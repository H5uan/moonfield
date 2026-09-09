//! The render-world half of the `Shader` asset: extraction, and the
//! revision-matched compiled-artifact cache pipelines build from.
//!
//! Mirrors the mesh module's split: [`extract_shader_assets`] copies the
//! requested shader assets (with their [`AssetRevision`]s) and the
//! pipelines' [`PipelineShaders`] requests into the render world, and
//! [`prepare_shaders`] (`RenderPrepare`) compiles every request whose asset
//! revision advanced through the render-world [`PreparedShaders`] resource,
//! which owns the shared Slang [`ShaderCache`]. Compilation runs from the
//! asset's source text, so the cache's memoization keys see source changes;
//! a Vulkan device is not involved until a pipeline turns the prepared
//! artifacts into shader modules.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use moonfield_app::prelude::{Commands, Res, World};
use moonfield_asset::{AssetId, AssetRevision, Assets, Handle};
use moonfield_render_core::Extract;
use moonfield_rhi::{CompiledShader, Reflection, ShaderCache};
use moonfield_shader::Shader;

/// One entry point a pipeline compiles from its shader asset.
#[derive(Debug, Clone, Copy)]
pub struct ShaderEntry {
    /// The entry point name in the Slang source.
    pub name: &'static str,
    /// Extra SPIR-V capabilities (Slang capability atoms, e.g.
    /// `spvDescriptorHeapEXT`).
    pub capabilities: &'static [&'static str],
}

/// A pipeline's binding to a shader asset: which asset to compile, which
/// entry points to build modules for, and which entry's reflection drives
/// root binding. Entry points stay in the pipeline's code — the asset
/// carries no entry-point metadata.
#[derive(Debug, Clone, Copy)]
pub struct PipelineShader {
    /// The consuming pipeline's name; keys [`PipelineShaders`] and
    /// [`PreparedShaders`].
    pub pipeline: &'static str,
    /// The shader asset to compile from.
    pub shader: Handle<Shader>,
    /// The entry whose reflection resolves the pipeline's root parameters.
    pub reflect_entry: &'static str,
    /// The entry points to compile.
    pub entries: &'static [ShaderEntry],
}

/// Every pipeline's shader request. A main-world resource, populated by
/// whoever loads the shader assets (the editor, at startup) and cloned into
/// the render world by [`extract_shader_assets`].
#[derive(Debug, Default, Clone)]
pub struct PipelineShaders(Vec<PipelineShader>);

impl PipelineShaders {
    /// Register or replace a pipeline's shader request.
    pub fn push(&mut self, request: PipelineShader) {
        match self.0.iter_mut().find(|r| r.pipeline == request.pipeline) {
            Some(existing) => *existing = request,
            None => self.0.push(request),
        }
    }

    /// A pipeline's shader request by pipeline name.
    pub fn get(&self, pipeline: &str) -> Option<&PipelineShader> {
        self.0.iter().find(|r| r.pipeline == pipeline)
    }

    /// All registered requests.
    pub fn iter(&self) -> impl Iterator<Item = &PipelineShader> {
        self.0.iter()
    }
}

/// CPU shader data copied into the render world with the source revision.
pub struct ExtractedShader {
    /// Revision observed in the main-world asset store.
    pub revision: AssetRevision,
    /// The shader asset, cloned for the render world.
    pub shader: Shader,
}

/// Requested shader assets available to render-world preparation systems.
#[derive(Default)]
pub struct ExtractedShaders(HashMap<AssetId, ExtractedShader>);

impl ExtractedShaders {
    /// Get an extracted shader by asset id.
    pub fn get(&self, id: AssetId) -> Option<&ExtractedShader> {
        self.0.get(&id)
    }

    /// Number of extracted shader assets.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no shader assets are extracted.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// The compiled artifacts of one pipeline's shader: the module reflection
/// for root binding plus one compiled entry point per requested entry.
/// Device-free — a pipeline turns them into `ShaderModule`s per device.
pub struct PreparedShader {
    /// The source revision these artifacts were compiled from.
    revision: AssetRevision,
    reflection: Arc<Reflection>,
    entries: Vec<(&'static str, Arc<CompiledShader>)>,
}

impl PreparedShader {
    /// The module reflection compiled for the request's reflect entry.
    pub fn reflection(&self) -> &Reflection {
        &self.reflection
    }

    /// The compiled artifact of an entry point, if the request declared it.
    pub fn entry(&self, name: &str) -> Option<&CompiledShader> {
        self.entries
            .iter()
            .find(|(entry_name, _)| *entry_name == name)
            .map(|(_, compiled)| &**compiled)
    }

    /// The source revision these artifacts were compiled from.
    pub fn revision(&self) -> AssetRevision {
        self.revision
    }
}

/// One pipeline's slot: the compiled artifacts, or the failure recorded for
/// the current revision (so a broken source is not recompiled every frame;
/// the pass keeps running the pipeline it already built).
struct Slot {
    asset: AssetId,
    revision: AssetRevision,
    result: Result<PreparedShader, String>,
}

/// Render-world resource: compiled shader artifacts keyed by pipeline name,
/// revision-matched against the source asset, plus the shared Slang compile
/// machinery. The cache is created lazily on the first compile — no Slang
/// session exists before one is needed.
///
/// Keyed by pipeline name rather than asset id because the compiled entry
/// set is pipeline-declared; a shader asset shared by two pipelines gets one
/// slot per pipeline (the underlying [`ShaderCache`] still memoizes the
/// identical compiles).
#[derive(Default)]
pub struct PreparedShaders {
    cache: Option<ShaderCache>,
    shaders: HashMap<&'static str, Slot>,
}

impl PreparedShaders {
    /// Whether `pipeline` has no record for `(asset, revision)` yet.
    pub fn needs_prepare(&self, pipeline: &str, asset: AssetId, revision: AssetRevision) -> bool {
        self.shaders
            .get(pipeline)
            .is_none_or(|slot| slot.asset != asset || slot.revision != revision)
    }

    /// The compiled artifacts for `pipeline`, when its last compile
    /// succeeded.
    pub fn get(&self, pipeline: &str) -> Option<&PreparedShader> {
        self.shaders
            .get(pipeline)
            .and_then(|slot| slot.result.as_ref().ok())
    }

    /// The error recorded for `pipeline`'s current revision, when its last
    /// compile failed.
    pub fn error(&self, pipeline: &str) -> Option<&str> {
        self.shaders
            .get(pipeline)
            .and_then(|slot| slot.result.as_ref().err())
            .map(String::as_str)
    }

    /// Drop slots whose pipeline is no longer requested.
    pub fn retain_pipelines(&mut self, requests: &PipelineShaders) {
        self.shaders
            .retain(|pipeline, _| requests.iter().any(|r| r.pipeline == *pipeline));
    }

    /// Number of recorded slots (ready or failed).
    pub fn len(&self) -> usize {
        self.shaders.len()
    }

    /// Whether no pipeline has a recorded slot.
    pub fn is_empty(&self) -> bool {
        self.shaders.is_empty()
    }

    /// Compile `request`'s entries from the extracted shader source and
    /// record the outcome. A failure replaces the slot for the new revision
    /// — the pass keeps running the pipeline it already built.
    pub fn compile(&mut self, request: &PipelineShader, extracted: &ExtractedShader) {
        let result = self.compile_request(request, extracted);
        self.shaders.insert(
            request.pipeline,
            Slot {
                asset: request.shader.id(),
                revision: extracted.revision,
                result,
            },
        );
    }

    fn compile_request(
        &mut self,
        request: &PipelineShader,
        extracted: &ExtractedShader,
    ) -> Result<PreparedShader, String> {
        let shader = &extracted.shader;
        let (module, source) = (shader.path(), shader.source());
        let cache = self.cache()?;
        let reflection = cache
            .compile_source_reflection(module, source, request.reflect_entry)
            .map_err(|e| e.to_string())?;
        let mut entries = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let compiled = cache
                .compile_source(module, source, entry.name, entry.capabilities, &[])
                .map_err(|e| e.to_string())?;
            entries.push((entry.name, compiled));
        }
        Ok(PreparedShader {
            revision: extracted.revision,
            reflection,
            entries,
        })
    }

    fn cache(&mut self) -> Result<&ShaderCache, String> {
        if self.cache.is_none() {
            self.cache = Some(ShaderCache::new().map_err(|e| e.to_string())?);
        }
        Ok(self.cache.as_ref().expect("cache was just ensured"))
    }
}

/// Incrementally copy the requested shader assets (revision-matched, like
/// `extract_mesh_assets`) and the pipelines' shader requests into the render
/// world.
pub fn extract_shader_assets(
    requests: Extract<Option<Res<PipelineShaders>>>,
    assets: Extract<Option<Res<Assets<Shader>>>>,
    extracted: Option<Res<ExtractedShaders>>,
    commands: Commands,
) {
    let requests = requests.as_deref().cloned().unwrap_or_default();

    // An absent asset store clears the extracted set (retain-by-nothing).
    let mut referenced_ids: HashSet<AssetId> = HashSet::new();
    let mut updates = Vec::new();
    if let Some(assets) = assets.as_deref() {
        let referenced: Vec<Handle<Shader>> = requests
            .iter()
            .map(|request| request.shader)
            .filter(|handle| assets.contains(handle))
            .collect();
        referenced_ids = referenced.iter().map(|handle| handle.id()).collect();
        for handle in referenced {
            let Some(revision) = assets.revision(&handle) else {
                continue;
            };
            if extracted
                .as_deref()
                .and_then(|current| current.0.get(&handle.id()))
                .is_some_and(|shader| shader.revision == revision)
            {
                continue;
            }
            let Some(shader) = assets.get(&handle) else {
                continue;
            };
            updates.push((
                handle.id(),
                ExtractedShader {
                    revision,
                    shader: shader.clone(),
                },
            ));
        }
    }
    commands.insert_resource(requests);
    commands.queue(move |render_world| {
        let mut extracted = render_world
            .remove_resource::<ExtractedShaders>()
            .unwrap_or_default();
        extracted.0.retain(|id, _| referenced_ids.contains(id));
        for (id, shader) in updates {
            extracted.0.insert(id, shader);
        }
        render_world.insert_resource(extracted);
    });
}

/// `RenderPrepare` system: compile every requested shader whose asset
/// revision advanced. A failed compile is recorded for the new revision (no
/// per-frame retry of unchanged broken source) and the pass keeps running
/// the pipeline it already built — the keep-on-error behavior of
/// `prepare_meshes`, at pipeline level.
pub fn prepare_shaders(world: &mut World) {
    let Some(requests) = world.get_resource::<PipelineShaders>() else {
        return;
    };
    let Some(extracted) = world.get_resource::<ExtractedShaders>() else {
        return;
    };
    let mut prepared = world
        .get_resource_mut::<PreparedShaders>()
        .expect("PreparedShaders registered by RenderFeaturePlugin");
    prepared.retain_pipelines(&requests);
    for request in requests.iter() {
        let id = request.shader.id();
        let Some(shader) = extracted.get(id) else {
            continue;
        };
        if !prepared.needs_prepare(request.pipeline, id, shader.revision) {
            continue;
        }
        prepared.compile(request, shader);
        if let Some(error) = prepared.error(request.pipeline) {
            moonfield_log::error!(
                "failed to compile shader '{}' for pipeline '{}': {error}",
                shader.shader.path(),
                request.pipeline
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_app::{App, ExtractSchedule, RenderPrepare};

    const TEST_ENTRIES: &[ShaderEntry] = &[ShaderEntry {
        name: "main",
        capabilities: &[],
    }];

    const TEST_SHADER: &str = r#"
        struct DrawData { column_major float4x4 mvp; };
        [shader("vertex")]
        float4 main(float3 position : POSITION, Ptr<DrawData> root) : SV_POSITION
        {
            return mul(root[0].mvp, float4(position, 1.0));
        }
    "#;

    fn test_request(pipeline: &'static str, shader: Handle<Shader>) -> PipelineShader {
        PipelineShader {
            pipeline,
            shader,
            reflect_entry: "main",
            entries: TEST_ENTRIES,
        }
    }

    /// An app whose main world holds one shader asset per `(pipeline,
    /// source)` pair, each requested under its pipeline name.
    fn shader_world(entries: &[(&'static str, &str)]) -> (App, Vec<Handle<Shader>>) {
        let mut app = App::new();
        app.insert_resource(Assets::<Shader>::default());
        app.insert_resource(PipelineShaders::default());
        app.render_world_mut()
            .insert_resource(PreparedShaders::default());
        let handles = {
            let mut assets = app
                .world()
                .get_resource_mut::<Assets<Shader>>()
                .expect("Assets<Shader>");
            entries
                .iter()
                .map(|(pipeline, source)| {
                    assets.add(Shader::new(source.to_string(), format!("{pipeline}.slang")))
                })
                .collect::<Vec<_>>()
        };
        let mut requests = app
            .world()
            .get_resource_mut::<PipelineShaders>()
            .expect("PipelineShaders");
        for (&(pipeline, _), &handle) in entries.iter().zip(handles.iter()) {
            requests.push(test_request(pipeline, handle));
        }
        drop(requests);
        (app, handles)
    }

    #[test]
    fn test_extract_shader_assets_copies_requested_shaders_with_revisions() {
        let (mut app, handles) = shader_world(&[("wanted", TEST_SHADER)]);
        // A second asset nobody requests stays out of the render world.
        let unrequested = app
            .world()
            .get_resource_mut::<Assets<Shader>>()
            .expect("Assets<Shader>")
            .add(Shader::new(TEST_SHADER.to_string(), "other.slang".into()));
        app.add_render_systems(ExtractSchedule, extract_shader_assets);
        app.render();

        let render_world = app.render_world();
        let extracted = render_world
            .get_resource::<ExtractedShaders>()
            .expect("ExtractedShaders");
        assert_eq!(extracted.len(), 1);
        let shader = extracted.get(handles[0].id()).expect("requested shader");
        assert_eq!(shader.shader.source(), TEST_SHADER);
        assert_eq!(
            shader.revision,
            app.world()
                .get_resource::<Assets<Shader>>()
                .unwrap()
                .revision(&handles[0])
                .unwrap()
        );
        assert!(extracted.get(unrequested.id()).is_none());
        // The request list crosses into the render world too.
        assert!(
            render_world
                .get_resource::<PipelineShaders>()
                .expect("PipelineShaders")
                .get("wanted")
                .is_some()
        );
    }

    /// Revision bookkeeping without compiling: a recorded failure is not
    /// retried until the asset revision advances, and dropped pipelines are
    /// pruned.
    #[test]
    fn test_prepared_shaders_revision_bookkeeping() {
        let mut assets = Assets::<Shader>::default();
        let shader = assets.add(Shader::new("source".into(), "test.slang".into()));
        let revision = assets.revision(&shader).unwrap();

        let mut prepared = PreparedShaders::default();
        assert!(prepared.needs_prepare("test", shader.id(), revision));
        prepared.shaders.insert(
            "test",
            Slot {
                asset: shader.id(),
                revision,
                result: Err("boom".to_string()),
            },
        );
        assert!(!prepared.needs_prepare("test", shader.id(), revision));
        assert!(prepared.get("test").is_none());
        assert_eq!(prepared.error("test"), Some("boom"));

        // A revision advance (even without a source change) re-arms prepare.
        let _ = assets.get_mut(&shader).unwrap();
        let new_revision = assets.revision(&shader).unwrap();
        assert!(prepared.needs_prepare("test", shader.id(), new_revision));

        // A pipeline no longer requested loses its slot.
        let mut requests = PipelineShaders::default();
        requests.push(test_request("other", shader));
        prepared.retain_pipelines(&requests);
        assert!(prepared.is_empty());
        assert!(prepared.needs_prepare("test", shader.id(), new_revision));
    }

    /// The full extract → prepare flow compiles a real shader (Slang needs
    /// no Vulkan device) and records failures per revision.
    #[test]
    fn test_prepare_shaders_compiles_and_records_failures() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let (mut app, handles) = shader_world(&[("test", TEST_SHADER)]);
        app.add_render_systems(ExtractSchedule, extract_shader_assets);
        app.add_render_systems(RenderPrepare, prepare_shaders);

        app.render();
        let prepared = app
            .render_world()
            .get_resource::<PreparedShaders>()
            .expect("PreparedShaders");
        let compiled = prepared.get("test").expect("shader compiled");
        assert!(compiled.entry("main").is_some());
        let first_revision = compiled.revision();
        drop(prepared);

        // An unchanged revision does not recompile: the slot stays.
        app.render();
        let prepared = app
            .render_world()
            .get_resource::<PreparedShaders>()
            .unwrap();
        assert!(!prepared.needs_prepare("test", handles[0].id(), first_revision));
        drop(prepared);

        // Break the source: the revision advances, the failure is recorded
        // for the new revision, and it is not retried while unchanged.
        *app.world()
            .get_resource_mut::<Assets<Shader>>()
            .unwrap()
            .get_mut(&handles[0])
            .unwrap() = Shader::new("this is not slang".into(), "test.slang".into());
        app.render();
        let broken_revision = app
            .world()
            .get_resource::<Assets<Shader>>()
            .unwrap()
            .revision(&handles[0])
            .unwrap();
        let prepared = app
            .render_world()
            .get_resource::<PreparedShaders>()
            .unwrap();
        assert!(prepared.get("test").is_none());
        assert!(prepared.error("test").is_some());
        assert!(!prepared.needs_prepare("test", handles[0].id(), broken_revision));
    }
}
