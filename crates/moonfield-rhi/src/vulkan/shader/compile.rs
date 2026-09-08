//! Slang compilation: source text or files in, SPIR-V and reflections out.

use super::map_slang_error;
use super::reflection::Reflection;
use crate::error::{Error as RenderError, Result as RenderResult};
use ash::vk;

/// A compiled shader: SPIR-V bytecode plus the Vulkan stage Slang resolved for
/// its entry point.
///
/// The stage comes from the entry point's `[shader("...")]` annotation via
/// Slang reflection (`Shader::entry_points()`); the Rust side never guesses
/// it.
/// Pipeline construction validates a module's stage against the slot it is
/// handed to (`VERTEX` slot × vertex module, etc.), so a shader compiled with
/// the wrong annotation fails loudly instead of silently misbinding.
#[derive(Debug, Clone)]
pub struct CompiledShader {
    /// SPIR-V bytecode, ready for `vkCreateShaderModule`.
    pub(crate) spirv: Vec<u8>,
    /// The Vulkan stage of the compiled entry point.
    pub(crate) stage: vk::ShaderStageFlags,
    /// The entry point name as it appears in the emitted SPIR-V (Slang may
    /// rename it, e.g. to `main`); the pipeline must name this exact string.
    pub(crate) entry: String,
}

/// Extract the name of the (single) `OpEntryPoint` from SPIR-V bytecode.
///
/// The pipeline's `PipelineShaderStageCreateInfo::name` must match the name
/// actually emitted in the module; Slang emits `main` regardless of the
/// source-level name, so reflection's source name is not reliable.
fn spirv_entry_name(bytecode: &[u8]) -> Option<String> {
    // SPIR-V words: [magic, version, generator, bound, schema, ...].
    if bytecode.len() < 20 {
        return None;
    }
    let words: Vec<u32> = bytecode
        .as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| u32::from_le_bytes(*chunk))
        .collect();
    let mut i = 5;
    while i < words.len() {
        let word = words[i];
        let word_count = (word >> 16) as usize;
        let opcode = word & 0xFFFF;
        if word_count == 0 || i + word_count > words.len() {
            return None;
        }
        // OpEntryPoint (15): execution model, entry id, then the name string.
        if opcode == 15 && word_count >= 4 {
            let mut name = Vec::new();
            'words: for &word in &words[i + 3..i + word_count] {
                let chunk = word.to_le_bytes();
                for &b in &chunk {
                    if b == 0 {
                        break 'words;
                    }
                    name.push(b);
                }
            }
            return String::from_utf8(name).ok();
        }
        i += word_count;
    }
    None
}

/// Map a Slang reflection stage to its Vulkan `VkShaderStageFlagBits` value.
///
/// Only stages a pipeline can name today are mapped; unknown stages (e.g.
/// `Dispatch`/`Node`, which have no pipeline representation yet) error out.
fn to_vk_stage(stage: shader_slang::Stage) -> RenderResult<vk::ShaderStageFlags> {
    use shader_slang::Stage::*;
    Ok(match stage {
        Vertex => vk::ShaderStageFlags::VERTEX,
        Hull => vk::ShaderStageFlags::TESSELLATION_CONTROL,
        Domain => vk::ShaderStageFlags::TESSELLATION_EVALUATION,
        Geometry => vk::ShaderStageFlags::GEOMETRY,
        Fragment => vk::ShaderStageFlags::FRAGMENT,
        Compute => vk::ShaderStageFlags::COMPUTE,
        RayGeneration => vk::ShaderStageFlags::RAYGEN_KHR,
        Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
        AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
        ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
        Miss => vk::ShaderStageFlags::MISS_KHR,
        Callable => vk::ShaderStageFlags::CALLABLE_KHR,
        Mesh => vk::ShaderStageFlags::MESH_EXT,
        Amplification => vk::ShaderStageFlags::TASK_EXT,
        _ => {
            return Err(RenderError::Unsupported(format!(
                "shader stage has no pipeline representation: {:?}",
                stage
            )));
        }
    })
}

/// Slang compiler session wrapper.
pub struct Compiler {
    global_session: shader_slang::GlobalSession,
}

/// Compile-once cache of [`CompiledShader`]s, keyed by the compile inputs.
///
/// Every pipeline today compiles its shaders itself (`Compiler::new()`, then
/// `compile_file_to_spirv`), so an N-pipeline app compiles the same file N
/// times. This cache memoizes by `(file, source, entry, capabilities)`; the
/// caller still creates `vk::ShaderModule`s (they are device-bound) via
/// [`ShaderModule::from_compiled`], which is cheap.
///
/// Slang sessions are not thread-safe, so compilation happens under a mutex;
/// the cache itself is `Sync` for shared use from a render-world resource.
pub struct ShaderCache {
    compiler: Compiler,
    cache:
        std::sync::Mutex<std::collections::HashMap<ShaderCacheKey, std::sync::Arc<CompiledShader>>>,
    reflections:
        std::sync::Mutex<std::collections::HashMap<ShaderCacheKey, std::sync::Arc<Reflection>>>,
}

// SAFETY: every compiler access happens under one of the two mutexes (both
// `get_or_compile` and `compile_file_reflection` hold their lock while
// compiling), so the Slang session is used from one thread at a time; the
// cached values (`CompiledShader`, `Reflection`) are themselves
// `Send + Sync`. Slang's global session is documented as usable from
// multiple threads.
unsafe impl Send for ShaderCache {}
unsafe impl Sync for ShaderCache {}

/// The inputs that determine a compiled artifact. All variants are stored so
/// the key is the exact identity of a compile, not a hash of it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShaderCacheKey {
    module_name: String,
    /// Source text; empty for `compile_file` (the file is the identity).
    source: String,
    entry_point: String,
    capabilities: Vec<String>,
    /// Preprocessor macro definitions (shader-variant selectors).
    defines: Vec<(String, String)>,
}

impl ShaderCache {
    /// Create an empty cache with its own compiler session.
    pub fn new() -> RenderResult<Self> {
        Ok(Self {
            compiler: Compiler::new()?,
            cache: std::sync::Mutex::new(std::collections::HashMap::new()),
            reflections: std::sync::Mutex::new(std::collections::HashMap::new()),
        })
    }

    /// Compile a file for `entry_point`, memoized by
    /// `(path, entry, caps, defines)`.
    pub fn compile_file(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<std::sync::Arc<CompiledShader>> {
        let key = ShaderCacheKey {
            module_name: path.to_string(),
            source: String::new(),
            entry_point: entry_point.to_string(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            defines: defines
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.get_or_compile(key, |compiler, key| {
            compiler.with_caps(
                &key.module_name,
                &key.entry_point,
                &key.capabilities,
                &key.defines,
            )
        })
    }

    /// Compile in-memory source for `entry_point`, memoized by the source text
    /// and compile options.
    pub fn compile_source(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<std::sync::Arc<CompiledShader>> {
        let key = ShaderCacheKey {
            module_name: module_name.to_string(),
            source: source.to_string(),
            entry_point: entry_point.to_string(),
            capabilities: capabilities.iter().map(|s| s.to_string()).collect(),
            defines: defines
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        };
        self.get_or_compile(key, |compiler, key| {
            compiler.with_caps_source(
                &key.module_name,
                &key.source,
                &key.entry_point,
                &key.capabilities,
                &key.defines,
            )
        })
    }

    /// Compile a file and return its reflection, memoized by
    /// `(path, entry)`. The reflection wrapper keeps its session and linked
    /// component alive, so the cached value stays valid.
    pub fn compile_file_reflection(
        &self,
        path: &str,
        entry_point: &str,
    ) -> RenderResult<std::sync::Arc<Reflection>> {
        let key = ShaderCacheKey {
            module_name: path.to_string(),
            source: String::new(),
            entry_point: entry_point.to_string(),
            capabilities: Vec::new(),
            defines: Vec::new(),
        };
        self.get_or_reflect(key, |compiler, key| {
            compiler.compile_file_to_reflection(&key.module_name, &key.entry_point)
        })
    }

    /// Compile in-memory source and return its reflection, memoized by the
    /// source text and entry point — the source-text counterpart of
    /// [`compile_file_reflection`](Self::compile_file_reflection).
    pub fn compile_source_reflection(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
    ) -> RenderResult<std::sync::Arc<Reflection>> {
        let key = ShaderCacheKey {
            module_name: module_name.to_string(),
            source: source.to_string(),
            entry_point: entry_point.to_string(),
            capabilities: Vec::new(),
            defines: Vec::new(),
        };
        self.get_or_reflect(key, |compiler, key| {
            compiler.compile_source_to_reflection(&key.module_name, &key.source, &key.entry_point)
        })
    }

    fn get_or_reflect(
        &self,
        key: ShaderCacheKey,
        reflect: impl FnOnce(&Compiler, &ShaderCacheKey) -> RenderResult<Reflection>,
    ) -> RenderResult<std::sync::Arc<Reflection>> {
        let mut reflections = self.reflections.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(reflection) = reflections.get(&key) {
            return Ok(std::sync::Arc::clone(reflection));
        }
        let reflection = std::sync::Arc::new(reflect(&self.compiler, &key)?);
        reflections.insert(key, std::sync::Arc::clone(&reflection));
        Ok(reflection)
    }

    fn get_or_compile(
        &self,
        key: ShaderCacheKey,
        compile: impl FnOnce(&Compiler, &ShaderCacheKey) -> RenderResult<CompiledShader>,
    ) -> RenderResult<std::sync::Arc<CompiledShader>> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(compiled) = cache.get(&key) {
            return Ok(std::sync::Arc::clone(compiled));
        }
        let compiled = std::sync::Arc::new(compile(&self.compiler, &key)?);
        cache.insert(key, std::sync::Arc::clone(&compiled));
        Ok(compiled)
    }
}

impl Compiler {
    /// Create a new Slang compiler instance.
    pub fn new() -> RenderResult<Self> {
        let global_session = shader_slang::GlobalSession::new().ok_or_else(|| {
            RenderError::Backend("failed to create Slang global session".to_string())
        })?;
        Ok(Self { global_session })
    }

    /// Compile a file, forwarding extra capabilities and macro definitions.
    /// Shared by [`ShaderCache`], which stores them in its key.
    pub(crate) fn with_caps(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[String],
        defines: &[(String, String)],
    ) -> RenderResult<CompiledShader> {
        let caps: Vec<&str> = capabilities.iter().map(String::as_str).collect();
        let defs: Vec<(&str, &str)> = defines
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if caps.is_empty() && defs.is_empty() {
            self.compile_file_to_spirv(path, entry_point)
        } else {
            self.compile_file_to_spirv_with_options(path, entry_point, &caps, &defs)
        }
    }

    /// Compile in-memory source, forwarding extra capabilities and macro
    /// definitions.
    pub(crate) fn with_caps_source(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[String],
        defines: &[(String, String)],
    ) -> RenderResult<CompiledShader> {
        let caps: Vec<&str> = capabilities.iter().map(String::as_str).collect();
        let defs: Vec<(&str, &str)> = defines
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if caps.is_empty() && defs.is_empty() {
            self.compile_source_to_spirv(module_name, source, entry_point)
        } else {
            self.compile_source_to_spirv_with_options(
                module_name,
                source,
                entry_point,
                &caps,
                &defs,
            )
        }
    }

    /// Compile Slang source code for the given entry point.
    ///
    /// `module_name` is the module's logical name and the path hint Slang
    /// resolves `import` statements against: a real file path lets the
    /// source import sibling modules from that file's directory (the asset
    /// layer passes the asset's path); a plain name leaves imports
    /// resolving against the process working directory.
    pub fn compile_source_to_spirv(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
    ) -> RenderResult<CompiledShader> {
        self.compile_source_to_spirv_impl(module_name, source, entry_point, &[], &[])
    }

    /// Compile a Slang file for the given entry point.
    pub fn compile_file_to_spirv(
        &self,
        path: &str,
        entry_point: &str,
    ) -> RenderResult<CompiledShader> {
        self.compile_file_to_spirv_impl(path, entry_point, &[], &[])
    }

    /// Compile a Slang file with extra SPIR-V capabilities enabled.
    ///
    /// Capability names are Slang capability atoms (e.g. `spvDescriptorHeapEXT`
    /// for the `VK_EXT_descriptor_heap` shader path — `ResourceDescriptorHeap[]`
    /// then lowers to untyped pointer heap access without descriptor bindings).
    /// Unknown names are ignored so callers can pass driver-dependent lists.
    pub fn compile_file_to_spirv_with_capabilities(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[&str],
    ) -> RenderResult<CompiledShader> {
        self.compile_file_to_spirv_impl(path, entry_point, capabilities, &[])
    }

    /// Compile a Slang file with extra capabilities and preprocessor macro
    /// definitions. Macros select shader variants (feature toggles, material
    /// flags) without duplicating source files.
    pub fn compile_file_to_spirv_with_options(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<CompiledShader> {
        self.compile_file_to_spirv_impl(path, entry_point, capabilities, defines)
    }

    /// Compile Slang source with extra capabilities (see
    /// [`compile_file_to_spirv_with_capabilities`]).
    pub fn compile_source_to_spirv_with_capabilities(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[&str],
    ) -> RenderResult<CompiledShader> {
        self.compile_source_to_spirv_impl(module_name, source, entry_point, capabilities, &[])
    }

    /// Compile Slang source with extra capabilities and macro definitions.
    pub fn compile_source_to_spirv_with_options(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<CompiledShader> {
        self.compile_source_to_spirv_impl(module_name, source, entry_point, capabilities, defines)
    }

    fn compile_file_to_spirv_impl(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<CompiledShader> {
        let session = self.create_session(capabilities, defines)?;
        let module = session.load_module(path).map_err(map_slang_error)?;
        self.finish_compile(&session, module, entry_point)
    }

    /// Compile in-memory Slang source to SPIR-V. Shared with
    /// [`compile_source_to_spirv`], which supplies no capabilities.
    fn compile_source_to_spirv_impl(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<CompiledShader> {
        let session = self.create_session(capabilities, defines)?;
        let module = session
            .load_module_from_source_string(module_name, module_name, source)
            .map_err(map_slang_error)?;
        self.finish_compile(&session, module, entry_point)
    }

    /// Create a Slang session targeting SPIR-V with the given capabilities
    /// and preprocessor macro definitions.
    fn create_session(
        &self,
        capabilities: &[&str],
        defines: &[(&str, &str)],
    ) -> RenderResult<shader_slang::Session> {
        let mut options = shader_slang::CompilerOptions::default()
            .optimization(shader_slang::OptimizationLevel::High)
            .matrix_layout_row(true);
        for name in capabilities {
            let capability = self.global_session.find_capability(name);
            if !capability.is_unknown() {
                options = options.capability(capability);
            }
        }
        for (key, value) in defines {
            options = options.macro_define(key, value).map_err(map_slang_error)?;
        }

        let profile = self.global_session.find_profile("spirv_1_5");
        let target_desc = shader_slang::TargetDesc::default()
            .format(shader_slang::CompileTarget::Spirv)
            .profile(profile)
            .options(&options);
        let targets = [target_desc];

        let session_desc = shader_slang::SessionDesc::default()
            .targets(&targets)
            .options(&options);

        self.global_session
            .create_session(&session_desc)
            .ok_or_else(|| RenderError::Backend("failed to create Slang session".to_string()))
    }

    /// Turn a loaded module into a [`CompiledShader`]: pick the entry point,
    /// link the program, extract the target code, and resolve the entry point's
    /// stage from Slang reflection.
    fn finish_compile(
        &self,
        session: &shader_slang::Session,
        module: shader_slang::Module,
        entry_point: &str,
    ) -> RenderResult<CompiledShader> {
        let entry = module
            .find_entry_point_by_name(entry_point)
            .ok_or_else(|| {
                RenderError::Backend(format!("entry point '{}' not found", entry_point))
            })?;

        let program = session
            .create_composite_component_type(&[module.into(), entry.into()])
            .map_err(map_slang_error)?;

        let linked = program.link().map_err(map_slang_error)?;
        let bytecode = linked.entry_point_code(0, 0).map_err(map_slang_error)?;

        // The linked program's reflection names the entry point's stage (the
        // `[shader("...")]` annotation); copy the stage out while `linked` is
        // still alive.
        let reflection = linked.layout(0).map_err(map_slang_error)?;
        let reflected_entry = reflection
            .find_entry_point_by_name(entry_point)
            .map_err(map_slang_error)?
            .ok_or_else(|| {
                RenderError::Backend(format!(
                    "entry point '{}' missing from linked program reflection",
                    entry_point
                ))
            })?;
        let stage = to_vk_stage(reflected_entry.stage())?;
        // The pipeline must name the entry point exactly as it appears in the
        // emitted SPIR-V (Slang emits `main` regardless of the source name);
        // reflection `name_override` only report s source-level overrides.
        let entry = spirv_entry_name(bytecode.as_slice()).ok_or_else(|| {
            RenderError::Backend("emitted SPIR-V has no OpEntryPoint".to_string())
        })?;

        Ok(CompiledShader {
            spirv: bytecode.as_slice().to_vec(),
            stage,
            entry,
        })
    }

    /// Compile a Slang file and return a reflection object that computes struct
    /// layouts on demand. Keeps the whole compile pipeline (session, program,
    /// linked component) alive so every reflection pointer stays valid for the
    /// returned wrapper's lifetime.
    pub fn compile_file_to_reflection(
        &self,
        path: &str,
        entry_point: &str,
    ) -> RenderResult<Reflection> {
        let options = shader_slang::CompilerOptions::default()
            .optimization(shader_slang::OptimizationLevel::High)
            .matrix_layout_row(true);

        let profile = self.global_session.find_profile("spirv_1_4");
        let target_desc = shader_slang::TargetDesc::default()
            .format(shader_slang::CompileTarget::Spirv)
            .profile(profile)
            .options(&options);
        let targets = [target_desc];

        let session_desc = shader_slang::SessionDesc::default()
            .targets(&targets)
            .options(&options);

        let session = self
            .global_session
            .create_session(&session_desc)
            .ok_or_else(|| RenderError::Backend("failed to create Slang session".to_string()))?;

        let module = session.load_module(path).map_err(map_slang_error)?;

        let entry = module
            .find_entry_point_by_name(entry_point)
            .ok_or_else(|| {
                RenderError::Backend(format!("entry point '{}' not found", entry_point))
            })?;

        let program = session
            .create_composite_component_type(&[module.into(), entry.into()])
            .map_err(map_slang_error)?;

        let linked = program.link().map_err(map_slang_error)?;

        // `layout(0)` returns a reference owned by `linked`; keep `linked` (and
        // its dependencies) alive in the wrapper and store the raw pointer.
        let reflection =
            linked.layout(0).map_err(map_slang_error)? as *const shader_slang::reflection::Shader;

        Ok(Reflection {
            _session: session,
            _linked: linked,
            reflection,
        })
    }

    /// Compile in-memory source and return a reflection object, like
    /// [`compile_file_to_reflection`](Self::compile_file_to_reflection) but
    /// without a file on disk. `module_name` is used for diagnostics and as
    /// the module's logical name.
    pub fn compile_source_to_reflection(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
    ) -> RenderResult<Reflection> {
        let options = shader_slang::CompilerOptions::default()
            .optimization(shader_slang::OptimizationLevel::High)
            .matrix_layout_row(true);

        let profile = self.global_session.find_profile("spirv_1_4");
        let target_desc = shader_slang::TargetDesc::default()
            .format(shader_slang::CompileTarget::Spirv)
            .profile(profile)
            .options(&options);
        let targets = [target_desc];

        let session_desc = shader_slang::SessionDesc::default()
            .targets(&targets)
            .options(&options);

        let session = self
            .global_session
            .create_session(&session_desc)
            .ok_or_else(|| RenderError::Backend("failed to create Slang session".to_string()))?;

        let module = session
            .load_module_from_source_string(module_name, module_name, source)
            .map_err(map_slang_error)?;

        let entry = module
            .find_entry_point_by_name(entry_point)
            .ok_or_else(|| {
                RenderError::Backend(format!("entry point '{}' not found", entry_point))
            })?;

        let program = session
            .create_composite_component_type(&[module.into(), entry.into()])
            .map_err(map_slang_error)?;

        let linked = program.link().map_err(map_slang_error)?;

        let reflection =
            linked.layout(0).map_err(map_slang_error)? as *const shader_slang::reflection::Shader;

        Ok(Reflection {
            _session: session,
            _linked: linked,
            reflection,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KERNEL: &str = r#"
        [shader("compute")]
        void main(uint3 tid : SV_DispatchThreadID, Ptr<uint32_t, Access.ReadWrite> out)
        {
            out[tid.x] = tid.x;
        }
    "#;

    /// A source whose behavior changes with a preprocessor macro — the
    /// shader-variant test bed.
    const VARIANT_SOURCE: &str = r#"
        [shader("compute")]
        void main(uint3 tid : SV_DispatchThreadID, Ptr<uint32_t, Access.ReadWrite> out)
        {
        #ifdef VARIANT
            out[tid.x] = VARIANT;
        #else
            out[tid.x] = 0;
        #endif
        }
    "#;

    /// The cache must return the same artifact for identical inputs and a
    /// different one when the key differs — without recompiling.
    #[test]
    fn shader_cache_memoizes_by_key() {
        let cache = ShaderCache::new().expect("cache");
        let first = cache
            .compile_source("memo", KERNEL, "main", &[], &[])
            .expect("compile");
        let second = cache
            .compile_source("memo", KERNEL, "main", &[], &[])
            .expect("compile");
        assert!(
            std::sync::Arc::ptr_eq(&first, &second),
            "same key must share the artifact"
        );
        assert_eq!(first.stage, vk::ShaderStageFlags::COMPUTE);
        assert_eq!(first.entry, "main");
        // Different entry point → different key → new artifact.
        let other = cache
            .compile_source("memo", KERNEL, "other", &[], &[])
            .expect_err("unknown entry point must fail");
        assert!(matches!(other, RenderError::Backend(_)));

        // Different macro definitions → distinct variants, each memoized.
        let variant_a = cache
            .compile_source("memo", VARIANT_SOURCE, "main", &[], &[("VARIANT", "1")])
            .expect("variant a");
        let variant_a_again = cache
            .compile_source("memo", VARIANT_SOURCE, "main", &[], &[("VARIANT", "1")])
            .expect("variant a again");
        let variant_b = cache
            .compile_source("memo", VARIANT_SOURCE, "main", &[], &[("VARIANT", "2")])
            .expect("variant b");
        assert!(std::sync::Arc::ptr_eq(&variant_a, &variant_a_again));
        assert!(
            !std::sync::Arc::ptr_eq(&variant_a, &variant_b),
            "different defines must be different artifacts"
        );
    }

    /// One file can host a compute entry and a graphics entry sharing structs
    /// (GPU-culling/skinning pattern); reflection reports each stage and the
    /// compute thread-group size.
    #[test]
    fn multi_stage_file_reflects_compute_and_graphics() {
        const MULTI: &str = r#"
            struct Payload { uint3 tid; };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; };
            [shader("compute")]
            [numthreads(8, 4, 1)]
            void cull_main(uint3 tid : SV_DispatchThreadID, Ptr<Payload, Access.ReadWrite> payload)
            {
                payload[tid.x].tid = tid;
            }
            [shader("vertex")]
            VsOutput vs_main(VsInput input)
            {
                VsOutput o;
                o.position = float4(input.position, 1.0);
                return o;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        // Each entry links into its own program, so reflection is per entry.
        let vs_refl = compiler
            .compile_source_to_reflection("multi", MULTI, "vs_main")
            .expect("reflection");
        assert_eq!(
            vs_refl.compute_thread_group_size("vs_main").expect("vs"),
            None,
            "non-compute entry has no thread-group size"
        );
        let cull_refl = compiler
            .compile_source_to_reflection("multi", MULTI, "cull_main")
            .expect("reflection");
        assert_eq!(
            cull_refl
                .compute_thread_group_size("cull_main")
                .expect("cs"),
            Some([8, 4, 1]),
            "compute entry reports its [numthreads] size"
        );
        // Both entries compile from the same module; each names its own stage.
        let cull = compiler
            .compile_source_to_spirv("multi", MULTI, "cull_main")
            .expect("cull compile");
        let vs = compiler
            .compile_source_to_spirv("multi", MULTI, "vs_main")
            .expect("vs compile");
        assert_eq!(cull.stage, vk::ShaderStageFlags::COMPUTE);
        assert_eq!(
            cull.entry, "main",
            "SPIR-V emits `main` for the compute entry regardless of source name"
        );
        assert_eq!(vs.stage, vk::ShaderStageFlags::VERTEX);
    }

    /// A source-string module compiled under a real-path module name resolves
    /// same-directory `import`s: the probe's (virtual) file lives in
    /// `assets/shaders/gs/` and imports the shared Gaussian math.
    #[test]
    fn source_import_resolves_through_module_name_path() {
        let compiler = Compiler::new().expect("compiler");
        let module_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/shaders/gs/__import_probe.slang"
        );
        const PROBE: &str = r#"
            import gaussian;

            [shader("compute")]
            [numthreads(64, 1, 1)]
            void probe(uint3 tid : SV_DispatchThreadID,
                       Ptr<float, Access.ReadWrite> out_buf)
            {
                SplatView view;
                view.w = float3x3(1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0);
                view.cam_pos = float3(0.0, 0.0, -2.0);
                view.fx = 500.0;
                view.fy = 500.0;
                view.pp = float2(64.0, 64.0);

                Gaussian3D g;
                g.mean = float3(0.1, -0.2, 1.0);
                g.log_scale = float3(-2.0, -1.5, -1.0);
                g.rotation = float4(0.923, 0.2, 0.3, 0.1);
                g.logit_opacity = 0.0;
                g.sh_dc = float3(0.5, 0.6, 0.7);

                float3x3 cov = cov3d(g);
                ProjectedSplat p = project(g, view);
                float3 color = eval_color(float3(0.0, 0.0, 1.0), g);

                out_buf[0] = cov[0][0];
                out_buf[1] = p.screen.x;
                out_buf[2] = p.conic.x;
                out_buf[3] = p.depth;
                out_buf[4] = color.x;
            }
        "#;
        let shader = compiler
            .compile_source_to_spirv(module_path, PROBE, "probe")
            .expect("import resolves through the module-name path hint");
        assert!(!shader.spirv.is_empty());
    }
}
