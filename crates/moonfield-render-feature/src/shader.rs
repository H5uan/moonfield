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
//! the selected backend's device is not involved until a pipeline turns the
//! prepared artifacts into shader modules. The compilation target and
//! capability needs derive from the backend through
//! `moonfield_rhi::shader_target()` / `moonfield_rhi::shader_capabilities()`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use moonfield_app::prelude::{Commands, Res, World};
use moonfield_asset::{AssetId, AssetRevision, AssetServer, Assets, Handle};
use moonfield_render_core::Extract;
use moonfield_rhi::{CompiledShader, Reflection, ShaderCache};
use moonfield_shader::{Shader, SlangLoader};

/// One entry point a pipeline compiles from its shader asset.
#[derive(Debug, Clone, Copy)]
pub struct ShaderEntry {
    /// The entry point name in the Slang source.
    pub name: &'static str,
    /// Slang capability atoms this entry compiles with. Backend-neutral: the
    /// runtime needs of the selected backend come from
    /// `moonfield_rhi::shader_capabilities()`, not from a per-backend
    /// literal at the call site.
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
    /// The entry whose reflection the pipeline's root binding resolves
    /// through. The prepared reflection covers every declared entry (see
    /// [`PreparedShaders::compile`]); this names the primary one.
    pub reflect_entry: &'static str,
    /// The entry points to compile.
    pub entries: &'static [ShaderEntry],
}

impl PipelineShader {
    /// A fingerprint of everything in the request that changes the compiled
    /// artifacts: the reflection entry and the entry set (names and
    /// capabilities). Changing any of these without touching the source must
    /// still re-prepare, so [`PreparedShaders`] slots compare the fingerprint
    /// alongside the source revision. Process-local — fingerprints are
    /// compared within one run, never persisted.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.reflect_entry.hash(&mut hasher);
        for entry in self.entries {
            entry.name.hash(&mut hasher);
            entry.capabilities.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// Every pipeline's shader request. Application requests take precedence
/// over feature startup defaults. Extraction copies them into the render world.
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

/// The compiled artifacts of one pipeline's shader: option-matched reflections
/// for root binding plus one compiled entry point per requested entry.
/// Device-free — a pipeline turns them into `ShaderModule`s per device.
pub struct PreparedShader {
    /// The source revision these artifacts were compiled from.
    revision: AssetRevision,
    /// The prepared-artifact generation: bumped by [`PreparedShaders`] on
    /// every successful compile, never on failure. Pipelines key their
    /// rebuilds on this — not on the source revision — so a failed
    /// recompile leaves the pipeline built from the previous generation
    /// untouched.
    generation: u64,
    reflection: Arc<Reflection>,
    entry_reflections: Vec<(&'static str, Arc<Reflection>)>,
    entries: Vec<(&'static str, Arc<CompiledShader>)>,
}

impl PreparedShader {
    /// The reflection group containing the request's primary reflect entry.
    /// Other capability groups are accessed through `reflection_for_entry`.
    pub fn reflection(&self) -> &Reflection {
        &self.reflection
    }

    /// The reflection compiled with this entry's bytecode capabilities.
    /// Entries with identical options share a linked reflection program.
    pub fn reflection_for_entry(&self, name: &str) -> moonfield_rhi::Result<&Reflection> {
        self.entry_reflections
            .iter()
            .find(|(entry, _)| *entry == name)
            .map(|(_, reflection)| &**reflection)
            .ok_or_else(|| {
                moonfield_rhi::Error::Backend(format!(
                    "prepared shader is missing reflection for '{name}'"
                ))
            })
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

    /// The prepared-artifact generation of these artifacts.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// One pipeline's slot: the compiled artifacts, or the failure recorded for
/// the current revision (so a broken source is not recompiled every frame;
/// the pass keeps running the pipeline it already built).
struct Slot {
    asset: AssetId,
    revision: AssetRevision,
    /// The [`PipelineShader::fingerprint`] the slot was compiled from.
    fingerprint: u64,
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
    /// Hands out [`PreparedShader::generation`] values; bumps only on
    /// successful compiles, so generation 0 never names a usable artifact.
    next_generation: u64,
}

impl PreparedShaders {
    /// Whether `request`'s pipeline has no record matching its asset, its
    /// asset's `revision`, and its [`PipelineShader::fingerprint`].
    pub fn needs_prepare(&self, request: &PipelineShader, revision: AssetRevision) -> bool {
        self.shaders.get(request.pipeline).is_none_or(|slot| {
            slot.asset != request.shader.id()
                || slot.revision != revision
                || slot.fingerprint != request.fingerprint()
        })
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
    /// record the outcome. A success bumps the artifact generation; a
    /// failure replaces the slot for the new revision without one, so
    /// pipelines built from the previous generation keep running untouched.
    pub fn compile(&mut self, request: &PipelineShader, extracted: &ExtractedShader) {
        let mut result = self.compile_request(request, extracted);
        if let Ok(prepared) = &mut result {
            self.next_generation += 1;
            prepared.generation = self.next_generation;
        }
        self.shaders.insert(
            request.pipeline,
            Slot {
                asset: request.shader.id(),
                revision: extracted.revision,
                fingerprint: request.fingerprint(),
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
        let mut groups: Vec<(&[&str], Vec<&'static str>)> = Vec::new();
        let mut seen = HashSet::new();
        for entry in request.entries {
            if !seen.insert(entry.name) {
                return Err(format!("duplicate shader entry '{}'", entry.name));
            }
            if let Some((_, entries)) = groups
                .iter_mut()
                .find(|(caps, _)| *caps == entry.capabilities)
            {
                entries.push(entry.name);
            } else {
                groups.push((entry.capabilities, vec![entry.name]));
            }
        }
        if !seen.contains(request.reflect_entry) {
            if let Some((_, entries)) = groups.iter_mut().find(|(caps, _)| caps.is_empty()) {
                entries.push(request.reflect_entry);
            } else {
                groups.push((&[], vec![request.reflect_entry]));
            }
        }
        let mut entry_reflections = Vec::new();
        for (caps, entries) in groups {
            let reflection = cache
                .compile_source_reflection_with_options(module, source, &entries, caps, &[])
                .map_err(|error| error.to_string())?;
            entry_reflections.extend(
                entries
                    .into_iter()
                    .map(|entry| (entry, Arc::clone(&reflection))),
            );
        }
        let reflection = entry_reflections
            .iter()
            .find(|(entry, _)| *entry == request.reflect_entry)
            .map(|(_, reflection)| Arc::clone(reflection))
            .expect("reflect entry was included");
        let mut compiled = Vec::with_capacity(request.entries.len());
        for entry in request.entries {
            let artifact = cache
                .compile_source(module, source, entry.name, entry.capabilities, &[])
                .map_err(|e| e.to_string())?;
            compiled.push((entry.name, artifact));
        }
        Ok(PreparedShader {
            revision: extracted.revision,
            // Assigned by `compile` once the compile succeeds.
            generation: 0,
            reflection,
            entry_reflections,
            entries: compiled,
        })
    }

    fn cache(&mut self) -> Result<&ShaderCache, String> {
        if self.cache.is_none() {
            self.cache =
                Some(ShaderCache::new(moonfield_rhi::shader_target()).map_err(|e| e.to_string())?);
        }
        Ok(self.cache.as_ref().expect("cache was just ensured"))
    }
}

/// A feature's default shader: pipeline name, file under `<assets>/shaders/`,
/// and request builder. The name allows configuration checks before file I/O.
pub type FeatureShaderSpec<'a> = (&'static str, &'a str, fn(Handle<Shader>) -> PipelineShader);

/// Load and register a feature's shader needs: each [`FeatureShaderSpec`]
/// loads `<assets>/shaders/<file>` through the world's [`AssetServer`] and
/// pushes the built request into [`PipelineShaders`] only when its pipeline
/// has no request. An existing request is preserved without loading the default.
///
/// This is how the rendering features declare their own shaders at startup —
/// the application supplies the asset configuration (its own `AssetServer`
/// and `Assets<Shader>` are reused when present; otherwise a minimal server
/// with the `.slang` loader and an empty store appear). The shader directory
/// resolves through [`moonfield_asset::assets_dir`], including its
/// `MOONFIELD_ASSETS_DIR` override. Loading is synchronous; a failure is
/// logged and skipped, and the affected pass runs without its pipeline until
/// the shader prepares.
pub fn load_feature_shaders(world: &mut World, shaders: &[FeatureShaderSpec<'_>]) {
    if !world.contains_resource::<AssetServer>() {
        let mut server = AssetServer::default();
        server.register_loader(SlangLoader);
        world.insert_resource(server);
    }
    world.init_resource::<Assets<Shader>>();
    world.init_resource::<PipelineShaders>();
    let shader_dir = moonfield_asset::assets_dir().join("shaders");
    for (pipeline, file, request) in shaders {
        if world
            .get_resource::<PipelineShaders>()
            .expect("PipelineShaders was just ensured")
            .get(pipeline)
            .is_some()
        {
            continue;
        }
        let path = shader_dir.join(file);
        // `AssetServer::load` needs `&mut AssetServer` and
        // `&mut Assets<Shader>` at once; the scope holds the server out of
        // the world's storage while the store is borrowed from the world.
        let result = world.resource_scope(|world, server: &mut AssetServer| {
            let mut assets = world
                .get_resource_mut::<Assets<Shader>>()
                .expect("Assets<Shader> was just ensured");
            server.load(&mut assets, &path)
        });
        match result {
            Ok(handle) => {
                let request = request(handle);
                assert_eq!(
                    request.pipeline, *pipeline,
                    "feature shader builder must match its declared pipeline"
                );
                world
                    .get_resource_mut::<PipelineShaders>()
                    .expect("PipelineShaders was just ensured")
                    .push(request);
            }
            Err(error) => {
                moonfield_log::error!("failed to load shader {}: {error}", path.display())
            }
        }
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

/// `PrepareAssets` set system: compile every requested shader whose asset
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
        if !prepared.needs_prepare(request, shader.revision) {
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
    use moonfield_app::{App, ExtractSchedule, Render};

    #[test]
    fn test_prepared_reflections_match_entry_capabilities() {
        const SOURCE: &str = r#"
            [shader("compute")] [numthreads(1, 1, 1)]
            void first(Ptr<uint, Access.ReadWrite> output) { output[0] = 1; }
            [shader("compute")] [numthreads(1, 1, 1)]
            void second(Ptr<uint, Access.ReadWrite> output) { output[0] = 2; }
            [shader("compute")] [numthreads(1, 1, 1)]
            void heap(Ptr<uint, Access.ReadWrite> output) { output[0] = 3; }
        "#;
        let mut assets = Assets::<Shader>::default();
        let handle = assets.add(Shader::new(SOURCE.into(), "entry_caps.slang".into()));
        const ENTRIES: &[ShaderEntry] = &[
            ShaderEntry {
                name: "first",
                capabilities: &[],
            },
            ShaderEntry {
                name: "second",
                capabilities: &[],
            },
            ShaderEntry {
                name: "heap",
                capabilities: moonfield_rhi::shader_capabilities(),
            },
        ];
        let request = PipelineShader {
            pipeline: "entry_caps",
            shader: handle,
            reflect_entry: "first",
            entries: ENTRIES,
        };
        let mut prepared = PreparedShaders::default();
        prepared.compile(
            &request,
            &ExtractedShader {
                revision: assets.revision(&handle).unwrap(),
                shader: assets.get(&handle).unwrap().clone(),
            },
        );
        let shader = prepared.get("entry_caps").expect("prepared shader");
        let first = shader.reflection_for_entry("first").unwrap();
        let second = shader.reflection_for_entry("second").unwrap();
        let heap = shader.reflection_for_entry("heap").unwrap();
        assert!(std::ptr::eq(first, second));
        assert!(!std::ptr::eq(first, heap));
        assert!(first.root_parameters("second").is_ok());
        assert!(heap.root_parameters("heap").is_ok());
        assert!(shader.reflection_for_entry("missing").is_err());
    }
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

    #[test]
    fn test_feature_shader_defaults_preserve_explicit_requests() {
        let mut world = World::new();
        let mut assets = Assets::<Shader>::default();
        let custom = assets.add(Shader::new(TEST_SHADER.into(), "custom.slang".into()));
        world.insert_resource(assets);
        let mut requests = PipelineShaders::default();
        requests.push(test_request("test", custom));
        world.insert_resource(requests);
        load_feature_shaders(
            &mut world,
            &[("test", "core_3d.slang", |shader| {
                test_request("test", shader)
            })],
        );
        assert_eq!(
            world
                .get_resource::<PipelineShaders>()
                .unwrap()
                .get("test")
                .unwrap()
                .shader,
            custom
        );
        assert_eq!(world.get_resource::<Assets<Shader>>().unwrap().len(), 1);
    }

    #[test]
    fn test_feature_shader_defaults_load_only_missing_requests() {
        let mut world = World::new();
        let specs: &[FeatureShaderSpec<'_>] = &[("test", "core_3d.slang", |shader| {
            test_request("test", shader)
        })];
        load_feature_shaders(&mut world, specs);
        let handle = world
            .get_resource::<PipelineShaders>()
            .unwrap()
            .get("test")
            .unwrap()
            .shader;
        load_feature_shaders(&mut world, specs);
        assert_eq!(
            world
                .get_resource::<PipelineShaders>()
                .unwrap()
                .get("test")
                .unwrap()
                .shader,
            handle
        );
        assert_eq!(world.get_resource::<Assets<Shader>>().unwrap().len(), 1);
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

    /// Revision and fingerprint bookkeeping without compiling: a recorded
    /// failure is not retried until the asset revision or the request
    /// changes, and dropped pipelines are pruned.
    #[test]
    fn test_prepared_shaders_revision_bookkeeping() {
        let mut assets = Assets::<Shader>::default();
        let shader = assets.add(Shader::new("source".into(), "test.slang".into()));
        let revision = assets.revision(&shader).unwrap();
        let request = test_request("test", shader);

        let mut prepared = PreparedShaders::default();
        assert!(prepared.needs_prepare(&request, revision));
        prepared.shaders.insert(
            "test",
            Slot {
                asset: shader.id(),
                revision,
                fingerprint: request.fingerprint(),
                result: Err("boom".to_string()),
            },
        );
        assert!(!prepared.needs_prepare(&request, revision));
        assert!(prepared.get("test").is_none());
        assert_eq!(prepared.error("test"), Some("boom"));

        // A revision advance (even without a source change) re-arms prepare.
        let _ = assets.get_mut(&shader).unwrap();
        let new_revision = assets.revision(&shader).unwrap();
        assert!(prepared.needs_prepare(&request, new_revision));

        // A request change at an unchanged revision also re-arms prepare.
        let changed = PipelineShader {
            reflect_entry: "other",
            ..request
        };
        assert_ne!(changed.fingerprint(), request.fingerprint());
        assert!(prepared.needs_prepare(&changed, revision));

        // A pipeline no longer requested loses its slot.
        let mut requests = PipelineShaders::default();
        requests.push(test_request("other", shader));
        prepared.retain_pipelines(&requests);
        assert!(prepared.is_empty());
        assert!(prepared.needs_prepare(&request, new_revision));
    }

    /// The generation counts successful compiles only: a failed recompile
    /// leaves the last usable artifacts' generation untouched, so pipelines
    /// keyed on it never rebuild from a broken source.
    #[test]
    fn test_prepared_shader_generation_bumps_only_on_success() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let mut assets = Assets::<Shader>::default();
        let shader = assets.add(Shader::new(TEST_SHADER.into(), "test.slang".into()));
        let request = test_request("test", shader);
        let mut prepared = PreparedShaders::default();

        let extracted = |assets: &Assets<Shader>| ExtractedShader {
            revision: assets.revision(&shader).unwrap(),
            shader: assets.get(&shader).unwrap().clone(),
        };

        prepared.compile(&request, &extracted(&assets));
        let first_generation = prepared.get("test").unwrap().generation();

        // Break the source: the failed compile is recorded but hands out no
        // generation, and `get` stops serving artifacts.
        *assets.get_mut(&shader).unwrap() = Shader::new("not slang".into(), "test.slang".into());
        prepared.compile(&request, &extracted(&assets));
        assert!(prepared.get("test").is_none());
        assert!(prepared.error("test").is_some());

        // Fix the source: the next success gets a new generation.
        *assets.get_mut(&shader).unwrap() = Shader::new(TEST_SHADER.into(), "test.slang".into());
        prepared.compile(&request, &extracted(&assets));
        let third_generation = prepared.get("test").unwrap().generation();
        assert!(first_generation < third_generation);
        assert_eq!(third_generation - first_generation, 1);
    }

    /// The full extract → prepare flow compiles a real shader (Slang needs
    /// no Vulkan device) and records failures per revision.
    #[test]
    fn test_prepare_shaders_compiles_and_records_failures() {
        let _gpu = crate::test_util::GPU_LOCK.lock().unwrap();
        let (mut app, handles) = shader_world(&[("test", TEST_SHADER)]);
        app.add_render_systems(ExtractSchedule, extract_shader_assets);
        app.add_render_systems(Render, prepare_shaders);

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
        assert!(!prepared.needs_prepare(&test_request("test", handles[0]), first_revision));
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
        assert!(!prepared.needs_prepare(&test_request("test", handles[0]), broken_revision));
    }
}
