//! Slang shader compiler integration.
//!
//! Wraps the `shader-slang` crate to compile Slang source into SPIR-V
//! bytecode. Errors are mapped to the [`Error`](crate::error::Error) type.

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
    pub spirv: Vec<u8>,
    /// The Vulkan stage of the compiled entry point.
    pub stage: vk::ShaderStageFlags,
    /// The entry point name as it appears in the emitted SPIR-V (Slang may
    /// rename it, e.g. to `main`); the pipeline must name this exact string.
    pub entry: String,
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
    cache: std::sync::Mutex<std::collections::HashMap<ShaderCacheKey, std::rc::Rc<CompiledShader>>>,
}

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
    ) -> RenderResult<std::rc::Rc<CompiledShader>> {
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
    ) -> RenderResult<std::rc::Rc<CompiledShader>> {
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

    fn get_or_compile(
        &self,
        key: ShaderCacheKey,
        compile: impl FnOnce(&Compiler, &ShaderCacheKey) -> RenderResult<CompiledShader>,
    ) -> RenderResult<std::rc::Rc<CompiledShader>> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(compiled) = cache.get(&key) {
            return Ok(std::rc::Rc::clone(compiled));
        }
        let compiled = std::rc::Rc::new(compile(&self.compiler, &key)?);
        cache.insert(key, std::rc::Rc::clone(&compiled));
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
    /// `module_name` is used for diagnostics and as the module's logical name;
    /// it does not need to correspond to a file on disk.
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
            .load_module_from_source_string(module_name, &format!("{module_name}.slang"), source)
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
            .load_module_from_source_string(module_name, &format!("{module_name}.slang"), source)
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

/// A thin wrapper around a compiled program's reflection, exposing only the
/// layout queries the GPU-layout guard needs.
///
/// Holds the session and compiled [`shader_slang::ComponentType`] alive so the
/// owned reflection pointer stays valid for the wrapper's lifetime.
pub struct Reflection {
    _session: shader_slang::Session,
    _linked: shader_slang::ComponentType,
    reflection: *const shader_slang::reflection::Shader,
}

// The reflection object is owned by the held component type; sharing it behind
// `&self` is safe as long as this wrapper is alive.
unsafe impl Send for Reflection {}
unsafe impl Sync for Reflection {}

impl Reflection {
    /// Generate a `#[repr(C)]` Rust struct skeleton for a reflected Slang
    /// struct, with `bytemuck` derives and per-field offset comments.
    ///
    /// This realizes the shader-is-truth model: run it once when writing the
    /// host struct (manually, or via a build step) and the Rust side can
    /// never drift from the shader's byte layout. Offsets are from the
    /// compiled SPIR-V layout (`LayoutRules::Default`).
    pub fn struct_rust_source(&self, name: &str) -> RenderResult<String> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
            .ok_or_else(|| RenderError::Backend(format!("type '{name}' not found")))?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;

        let mut out = String::new();
        out.push_str(&format!(
            "#[repr(C)]\n#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]\npub struct {name} {{\n"
        ));
        for field in layout.fields() {
            let field_name = field.name().unwrap_or("<unnamed>");
            let field_layout = field.type_layout().ok_or_else(|| {
                RenderError::Backend(format!("field '{field_name}' has no layout"))
            })?;
            let (rust_ty, size) = rust_type(field_layout).ok_or_else(|| {
                RenderError::Backend(format!(
                    "field '{field_name}' type has no Rust equivalent yet"
                ))
            })?;
            // Field offsets are per-category, reported by the field's own
            // variable layout; take the max span across categories as the
            // byte offset in the blob.
            let mut offset = 0usize;
            for ci in 0..field_layout.category_count() {
                let c = field_layout.category_by_index(ci);
                offset = offset.max(field.offset(c));
            }
            out.push_str(&format!("    /// offset {offset}, {size} bytes\n"));
            out.push_str(&format!("    pub {field_name}: {rust_ty},\n"));
        }
        out.push('}');
        Ok(out)
    }

    /// Read the custom `[Attribute(...)]` annotations on a struct field, as
    /// editor metadata — e.g. `[EditorColor]`, `[Range(0, 1)]`: a name plus
    /// its typed arguments (`int`/`float`/string per arg). Fields are looked
    /// up through the type's layout so the variable node carrying the
    /// attributes is the one Slang attached them to.
    pub fn field_user_attributes(
        &self,
        struct_name: &str,
        field: &str,
    ) -> RenderResult<Vec<UserAttributeRef>> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(struct_name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{struct_name}': {e}")))?
            .ok_or_else(|| RenderError::Backend(format!("type '{struct_name}' not found")))?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{struct_name}'")))?;
        let field_layout = (0..layout.field_count())
            .find_map(|i| {
                let f = layout.field_by_index(i)?;
                (f.name() == Some(field)).then_some(f)
            })
            .ok_or_else(|| {
                RenderError::Backend(format!("field '{field}' not found in '{struct_name}'"))
            })?;
        let Some(var) = field_layout.variable() else {
            return Ok(Vec::new());
        };
        Ok(var
            .user_attributes()
            .map(|attr| {
                let name = attr.name().unwrap_or("<unnamed>").to_string();
                let args = (0..attr.argument_count())
                    .map(|i| {
                        if let Some(v) = attr.argument_value_string(i) {
                            UserAttributeArg::String(v.to_string())
                        } else if let Some(v) = attr.argument_value_int(i) {
                            UserAttributeArg::Int(v)
                        } else if let Some(v) = attr.argument_value_float(i) {
                            UserAttributeArg::Float(v)
                        } else {
                            UserAttributeArg::String(format!(
                                "<{:?}>",
                                attr.argument_type(i).map(|t| t.kind())
                            ))
                        }
                    })
                    .collect();
                UserAttributeRef { name, args }
            })
            .collect())
    }

    /// The `[numthreads(x, y, z)]` dispatch size of a compute entry point, or
    /// `None` for non-compute entries.
    pub fn compute_thread_group_size(&self, entry_name: &str) -> RenderResult<Option<[u32; 3]>> {
        let reflection = unsafe { &*self.reflection };
        let entry = reflection
            .find_entry_point_by_name(entry_name)
            .map_err(map_slang_error)?
            .ok_or_else(|| RenderError::Backend(format!("entry point '{entry_name}' not found")))?;
        if entry.stage() != shader_slang::Stage::Compute {
            return Ok(None);
        }
        let [x, y, z] = entry.compute_thread_group_size();
        Ok(Some([x as u32, y as u32, z as u32]))
    }

    /// Look up a struct type by name and return its layout.
    pub fn struct_layout(&self, name: &str) -> RenderResult<Layout<'_>> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
            .ok_or_else(|| {
                RenderError::Backend(format!("type '{name}' not found in reflection"))
            })?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;
        Ok(Layout { layout })
    }

    /// Derive the vertex input layout of a vertex entry point from its varying
    /// input parameters: location by declaration order, format from the
    /// parameter type, and offsets packed 4-byte aligned (the engine's vertex
    /// stream convention — matches `PodVertex`, `[f32;3]`, etc.).
    ///
    /// This replaces hand-written `VertexBufferLayout`s: the shader becomes the
    /// single source of truth for what a vertex looks like. Rejects entry
    /// points that are not vertex shaders or that use unsupported input types.
    pub fn vertex_layout(
        &self,
        entry_name: &str,
    ) -> RenderResult<crate::types::VertexBufferLayout> {
        let reflection = unsafe { &*self.reflection };
        let entry = reflection
            .find_entry_point_by_name(entry_name)
            .map_err(map_slang_error)?
            .ok_or_else(|| RenderError::Backend(format!("entry point '{entry_name}' not found")))?;
        if entry.stage() != shader_slang::Stage::Vertex {
            return Err(RenderError::Backend(format!(
                "entry point '{entry_name}' is not a vertex shader (stage {:?})",
                entry.stage()
            )));
        }

        let mut attributes = Vec::new();
        let mut offset = 0usize;
        // Only varying inputs describe vertex data; root parameters
        // (`Ptr<DrawData>` / `uniform Root`) and outputs do not. A struct-typed
        // input (the usual Slang shape, e.g. `VsInput input`) is unwrapped:
        // each struct field is one vertex attribute.
        for param in entry
            .parameters()
            .filter(|p| p.category() == Some(shader_slang::ParameterCategory::VaryingInput))
        {
            let param_layout = param.type_layout().ok_or_else(|| {
                RenderError::Backend(format!(
                    "vertex input '{}' has no reflected layout",
                    param.name().unwrap_or("<unnamed>")
                ))
            })?;
            let mut field_layouts = param_layout.fields().peekable();
            if field_layouts.peek().is_none() {
                // A scalar/vector input (no struct): the parameter itself is
                // the attribute.
                let format = vertex_input_format(param_layout).ok_or_else(|| {
                    RenderError::Backend(format!(
                        "vertex input '{}' has an unsupported type for vertex layouts",
                        param.name().unwrap_or("<unnamed>")
                    ))
                })?;
                attributes.push(crate::types::VertexAttribute {
                    location: attributes.len() as u32,
                    format,
                    offset: offset as u32,
                });
                offset = align4(offset + format_size(format));
            } else {
                for field in field_layouts {
                    let field_type = field
                        .type_layout()
                        .ok_or_else(|| RenderError::Backend("field has no layout".to_string()))?;
                    let format = vertex_input_format(field_type).ok_or_else(|| {
                        RenderError::Backend(format!(
                            "vertex input field '{}' has an unsupported type for vertex layouts",
                            field.name().unwrap_or("<unnamed>")
                        ))
                    })?;
                    attributes.push(crate::types::VertexAttribute {
                        location: attributes.len() as u32,
                        format,
                        offset: offset as u32,
                    });
                    offset = align4(offset + format_size(format));
                }
            }
        }
        Ok(crate::types::VertexBufferLayout {
            stride: align4(offset) as u32,
            attributes,
        })
    }
}

/// How a root parameter is delivered to the shader on descriptor-heap
/// pipelines: inline bytes (push-data / push constants) or a GPU address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootParamKind {
    /// The parameter's value is stored inline in the root blob (a `uniform`
    /// parameter — push-constant storage).
    Uniform,
    /// The parameter holds a GPU address (a `Ptr<T>` root — buffer device
    /// address).
    Pointer,
}

/// One root (non-varying) parameter of an entry point, with its placement in
/// the blob [`CommandBuffer::push_data`] receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootParam {
    /// The parameter name, e.g. `root`.
    pub name: String,
    /// Whether the parameter is inline data or a GPU address.
    pub kind: RootParamKind,
    /// Byte offset of the parameter within the root blob.
    pub offset: usize,
    /// Byte size of the parameter's storage in the blob.
    pub size: usize,
}

/// A root parameter's placement in the root blob, resolved once from a
/// [`RootBinder`]. Per-draw work is a stack write and a
/// [`CommandBuffer::push_data`](crate::CommandBuffer::push_data) at
/// [`RootParamPlace::offset`] — no allocation, no name lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootParamPlace {
    /// The parameter's byte offset in the root blob — the push-data offset.
    pub offset: usize,
    /// The parameter's storage size in bytes.
    pub size: usize,
    kind: RootParamKind,
}

impl RootParamPlace {
    /// A pointer parameter's 8 bytes, ready for `push_data` at this place's
    /// offset.
    pub fn pointer_bytes(&self, address: u64) -> RenderResult<[u8; 8]> {
        if self.kind != RootParamKind::Pointer {
            return Err(RenderError::Backend(format!(
                "root parameter placement is {:?}, not a pointer",
                self.kind
            )));
        }
        if self.size != 8 {
            return Err(RenderError::Backend(format!(
                "pointer root parameter occupies {} bytes, expected 8",
                self.size
            )));
        }
        Ok(address.to_le_bytes())
    }
}

/// A typed writer for a draw's root blob, driven by an entry point's
/// reflected [`RootParam`]s.
///
/// Builds the exact byte layout the shader expects (from Slang reflection,
/// not a hand-synced struct) and fills it by parameter name. The result is
/// handed to [`CommandBuffer::push_data`] before the draw. The behavior of
/// writing is checked against the reflection: unknown names, kind mismatches,
/// and size overruns are errors instead of silent misplacement.
#[derive(Clone)]
pub struct RootBinder {
    /// The root blob, sized to the reflected layout and filled by [`set`].
    blob: Vec<u8>,
    params: Vec<RootParam>,
}

impl RootBinder {
    /// Build a binder for `entry_name`'s root parameters and a zeroed blob of
    /// the reflected size.
    pub fn new(reflection: &Reflection, entry_name: &str) -> RenderResult<Self> {
        let params = reflection.root_parameters(entry_name)?;
        let size = params.iter().map(|p| p.offset + p.size).max().unwrap_or(0);
        Ok(Self {
            blob: vec![0u8; size],
            params,
        })
    }

    /// The root blob, ready for [`CommandBuffer::push_data`].
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// Write a GPU address to the named `Ptr<T>` root parameter.
    pub fn set_pointer(&mut self, name: &str, address: u64) -> RenderResult<()> {
        let (offset, size) = self.range(name, RootParamKind::Pointer, 8)?;
        self.blob[offset..offset + size].copy_from_slice(&address.to_le_bytes());
        Ok(())
    }

    /// Write inline bytes to the named `uniform` root parameter (the whole
    /// struct; typically `bytemuck::bytes_of(&root)`).
    pub fn set_bytes(&mut self, name: &str, bytes: &[u8]) -> RenderResult<()> {
        let (offset, size) = self.range(name, RootParamKind::Uniform, bytes.len())?;
        self.blob[offset..offset + size].copy_from_slice(bytes);
        Ok(())
    }

    fn range(&self, name: &str, kind: RootParamKind, size: usize) -> RenderResult<(usize, usize)> {
        let param = self
            .params
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| RenderError::Backend(format!("no root parameter named '{name}'")))?;
        if param.kind != kind {
            return Err(RenderError::Backend(format!(
                "root parameter '{name}' is {:?}, not {:?}",
                param.kind, kind
            )));
        }
        if size > param.size {
            return Err(RenderError::Backend(format!(
                "{size} bytes for root parameter '{name}' exceeds its {}-byte storage",
                param.size
            )));
        }
        Ok((param.offset, param.size))
    }

    /// The placement of the named `Ptr<T>` root parameter. Resolve once at
    /// pipeline build; per-draw encoding goes through
    /// [`RootParamPlace::pointer_bytes`].
    pub fn pointer_param(&self, name: &str) -> RenderResult<RootParamPlace> {
        let (offset, size) = self.range(name, RootParamKind::Pointer, 8)?;
        Ok(RootParamPlace {
            offset,
            size,
            kind: RootParamKind::Pointer,
        })
    }

    /// The placement of the named inline `uniform` root parameter. Resolve
    /// once at pipeline build; per-pass bytes are pushed at
    /// [`RootParamPlace::offset`].
    pub fn uniform_param(&self, name: &str) -> RenderResult<RootParamPlace> {
        let (offset, size) = self.range(name, RootParamKind::Uniform, 0)?;
        Ok(RootParamPlace {
            offset,
            size,
            kind: RootParamKind::Uniform,
        })
    }
}

impl Reflection {
    /// Enumerate the root parameters of `entry_name` — every non-varying
    /// parameter — with their placement in the push-data blob.
    ///
    /// Root parameters are the entry point's remaining parameters after
    /// varying inputs (vertex attributes) and outputs: `Ptr<T>` roots
    /// (pointer category) and `uniform` roots (push-constant category). In
    /// the descriptor-heap model the entire root blob is written with
    /// [`CommandBuffer::push_data`]; this describes where each parameter lives
    /// in it.
    pub fn root_parameters(&self, entry_name: &str) -> RenderResult<Vec<RootParam>> {
        let reflection = unsafe { &*self.reflection };
        let entry = reflection
            .find_entry_point_by_name(entry_name)
            .map_err(map_slang_error)?
            .ok_or_else(|| RenderError::Backend(format!("entry point '{entry_name}' not found")))?;

        let mut params = Vec::new();
        for param in entry.parameters() {
            let cat = param.category();
            let is_varying = cat == Some(shader_slang::ParameterCategory::VaryingInput)
                || cat == Some(shader_slang::ParameterCategory::VaryingOutput);
            if is_varying {
                continue;
            }
            let Some(layout) = param.type_layout() else {
                continue;
            };
            let name = param.name().unwrap_or("<unnamed>").to_string();

            // Uniform roots and pointer roots live in different categories;
            // take the largest span across the categories Slang reports so we
            // are robust to target differences. The parameter's own offset is
            // per-category (`VariableLayout::offset`), its size comes from the
            // type layout (`TypeLayout::size`).
            let mut offset = usize::MAX;
            let mut size = 0usize;
            for ci in 0..layout.category_count() {
                let c = layout.category_by_index(ci);
                let off = param.offset(c);
                let sz = layout.size(c);
                offset = offset.min(off);
                size = size.max(off + sz);
            }
            if size == 0 {
                continue;
            }
            // `Ptr<T>` roots reflect as 8-byte uniform-category payloads holding a
            // GPU address; the type (not the category) decides the delivery
            // kind. Everything else inline is a `uniform` root.
            let ty_kind = layout.ty().map(|t| t.kind());
            let kind = if ty_kind == Some(shader_slang::TypeKind::Pointer) {
                RootParamKind::Pointer
            } else {
                RootParamKind::Uniform
            };
            params.push(RootParam {
                name,
                kind,
                offset,
                size: size - offset,
            });
        }
        Ok(params)
    }
}

/// A field's `[Attribute(...)]` annotation: name plus typed arguments.
///
/// `PartialEq` is manual because `UserAttributeArg::Float(f32)` is not `Eq`.
#[derive(Debug, Clone)]
pub struct UserAttributeRef {
    /// The attribute name, e.g. `EditorColor`.
    pub name: String,
    /// Positional arguments, in declaration order.
    pub args: Vec<UserAttributeArg>,
}
impl PartialEq for UserAttributeRef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.args.len() == other.args.len()
            && self.args.iter().zip(&other.args).all(|(a, b)| a == b)
    }
}

/// A user-attribute argument value.
#[derive(Debug, Clone, PartialEq)]
pub enum UserAttributeArg {
    /// Integer constant.
    Int(i32),
    /// Float constant.
    Float(f32),
    /// String literal.
    String(String),
}

/// The Rust type and byte size for a reflected field of the given type
/// layout, for [`Reflection::struct_rust_source`].
fn rust_type(layout: &shader_slang::reflection::TypeLayout) -> Option<(&'static str, usize)> {
    use shader_slang::{ScalarType, TypeKind};
    let ty = layout.ty()?;
    match ty.kind() {
        TypeKind::Scalar => match ty.scalar_type() {
            ScalarType::Uint32 => Some(("u32", 4)),
            ScalarType::Int32 => Some(("i32", 4)),
            ScalarType::Float32 => Some(("f32", 4)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Float32 => match ty.column_count() {
            2 => Some(("[f32; 2]", 8)),
            3 => Some(("[f32; 3]", 12)),
            4 => Some(("[f32; 4]", 16)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Uint32 => match ty.column_count() {
            4 => Some(("[u32; 4]", 16)),
            _ => None,
        },
        TypeKind::Matrix if ty.scalar_type() == ScalarType::Float32 => {
            let rows = ty.row_count();
            let cols = ty.column_count();
            match (rows, cols) {
                (4, 4) => Some(("[f32; 16]", 64)),
                (4, 3) => Some(("[f32; 12]", 48)),
                (3, 3) => Some(("[f32; 9]", 36)),
                _ => None,
            }
        }
        TypeKind::Array => {
            // Arrays would need their element count as the Rust size; not
            // supported by the codegen yet.
            None
        }
        _ => None,
    }
}

/// Map a varying input's reflected type to a [`VertexFormat`].
///
/// Supported: `float2/3/4` vectors and `uint` scalars (packed colors). The
/// scalar type comes from the reflected type; a vector's element count from
/// its column count.
fn vertex_input_format(
    layout: &shader_slang::reflection::TypeLayout,
) -> Option<crate::types::VertexFormat> {
    use shader_slang::{ScalarType, TypeKind};
    let ty = layout.ty()?;
    match ty.kind() {
        TypeKind::Vector if ty.scalar_type() == ScalarType::Float32 => match ty.column_count() {
            2 => Some(crate::types::VertexFormat::Float32x2),
            3 => Some(crate::types::VertexFormat::Float32x3),
            4 => Some(crate::types::VertexFormat::Float32x4),
            _ => None,
        },
        TypeKind::Scalar if ty.scalar_type() == ScalarType::Uint32 => {
            Some(crate::types::VertexFormat::Uint32)
        }
        _ => None,
    }
}

fn format_size(format: crate::types::VertexFormat) -> usize {
    use crate::types::VertexFormat;
    match format {
        VertexFormat::Float32x2 => 8,
        VertexFormat::Float32x3 => 12,
        VertexFormat::Float32x4 => 16,
        VertexFormat::Uint32 => 4,
    }
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// A struct's GPU memory layout, queried from Slang reflection.
pub struct Layout<'a> {
    layout: &'a shader_slang::reflection::TypeLayout,
}

impl<'a> Layout<'a> {
    /// The total byte size of the struct under the compiled target's layout
    /// rules, across every parameter category the slang compiler reports.
    pub fn size(&self) -> usize {
        self.layout
            .categories()
            .map(|c| self.layout.size(c))
            .max()
            .unwrap_or(0)
    }

    /// The byte offset of a field by name, across the field's own categories.
    pub fn field_offset(&self, name: &str) -> RenderResult<usize> {
        let idx = self.layout.find_field_index_by_name(name);
        if idx < 0 {
            return Err(RenderError::Backend(format!(
                "field '{name}' not found in reflected struct"
            )));
        }
        let field = self
            .layout
            .field_by_index(idx as u32)
            .ok_or_else(|| RenderError::Backend("field disappeared".to_string()))?;
        let tl = field
            .type_layout()
            .ok_or_else(|| RenderError::Backend("field has no type layout".to_string()))?;
        Ok(tl.categories().map(|c| field.offset(c)).max().unwrap_or(0))
    }
}

fn map_slang_error(err: shader_slang::Error) -> RenderError {
    let message = match err {
        shader_slang::Error::Code(code) => format!("Slang error code: {}", code),
        shader_slang::Error::Blob(blob) => {
            blob.as_str().unwrap_or("unknown Slang error").to_string()
        }
    };
    RenderError::ShaderCompilation(message)
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
            std::rc::Rc::ptr_eq(&first, &second),
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
        assert!(std::rc::Rc::ptr_eq(&variant_a, &variant_a_again));
        assert!(
            !std::rc::Rc::ptr_eq(&variant_a, &variant_b),
            "different defines must be different artifacts"
        );
    }

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

    const VERTEX_SOURCE: &str = r#"
        struct DrawData { column_major float4x4 mvp; };
        struct VsInput {
            float3 position : POSITION;
            float2 uv : TEXCOORD0;
            uint color : COLOR0;
        };
        struct VsOutput {
            float4 position : SV_POSITION;
            float2 uv : TEXCOORD0;
            uint color : COLOR0;
        };
        [shader("vertex")]
        VsOutput main(VsInput input, Ptr<DrawData> root)
        {
            VsOutput output;
            output.position = mul(root[0].mvp, float4(input.position, 1.0));
            output.uv = input.uv;
            output.color = input.color;
            return output;
        }
    "#;

    /// Reflection-derived vertex layout: locations by declaration order,
    /// formats from the types, offsets packed 4-byte aligned.
    #[test]
    fn reflection_derives_vertex_layout() {
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection("vert", VERTEX_SOURCE, "main")
            .expect("reflection");
        let layout = reflection.vertex_layout("main").expect("layout");

        // Compact packing: 12 (pos) + 8 (uv) + 4 (color) = 24, offsets
        // contiguous — the engine's vertex stream convention.
        assert_eq!(layout.stride, 24);
        assert_eq!(layout.attributes.len(), 3);
        assert_eq!(
            (layout.attributes[0].location, layout.attributes[0].offset),
            (0, 0)
        );
        assert_eq!(
            layout.attributes[0].format,
            crate::types::VertexFormat::Float32x3
        );
        assert_eq!(
            (layout.attributes[1].location, layout.attributes[1].offset),
            (1, 12)
        );
        assert_eq!(
            layout.attributes[1].format,
            crate::types::VertexFormat::Float32x2
        );
        assert_eq!(
            (layout.attributes[2].location, layout.attributes[2].offset),
            (2, 20)
        );
    }

    /// RootBinder writes the reflected blob for both root kinds: a `Ptr<T>`
    /// root gets a GPU address, a `uniform` root gets its inline struct bytes.
    #[test]
    fn root_binder_builds_blob_from_reflection() {
        let compiler = Compiler::new().expect("compiler");

        // `Ptr<DrawData> root` variant.
        let refl_ptr = compiler
            .compile_source_to_reflection("ptr", VERTEX_SOURCE, "main")
            .expect("refl");
        let mut binder = RootBinder::new(&refl_ptr, "main").expect("binder");
        binder.set_pointer("root", 0xdecafbad).expect("set");
        assert_eq!(binder.blob().len(), 8);
        assert_eq!(
            binder.blob(),
            &0xdecafbadu64.to_le_bytes(),
            "pointer root stores the raw GPU address"
        );
        // Unknown name and kind mismatch are rejected.
        assert!(
            binder.set_pointer("nope", 0).is_err(),
            "unknown name rejected"
        );
        assert!(
            binder.set_bytes("root", &[0u8; 8]).is_err(),
            "kind mismatch rejected"
        );

        // `uniform Root` variant with two fields.
        const UNIFORM_SOURCE: &str = r#"
            struct Root { float2 scale; uint flags; };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; };
            [shader("vertex")]
            VsOutput main(VsInput input, uniform Root root)
            {
                VsOutput o;
                o.position = float4(input.position * float3(root.scale, 1.0), 1.0);
                return o;
            }
        "#;
        let refl_uniform = compiler
            .compile_source_to_reflection("uniform", UNIFORM_SOURCE, "main")
            .expect("refl");
        let params = refl_uniform.root_parameters("main").expect("params");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "root");
        assert_eq!(params[0].kind, RootParamKind::Uniform);
        // Uniform (push-constant) layouts round a vec2 up to 16-byte storage.
        assert_eq!(params[0].size, 16);

        let mut binder = RootBinder::new(&refl_uniform, "main").expect("binder");
        let mut expected = [0u8; 16];
        expected[..4].copy_from_slice(&2.0f32.to_le_bytes());
        expected[4..8].copy_from_slice(&3.0f32.to_le_bytes());
        expected[8..12].copy_from_slice(&[0u8, 0, 0, 0]);
        binder
            .set_bytes("root", &expected)
            .expect("uniform root set");
        assert_eq!(binder.blob(), &expected);
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

    /// `struct_rust_source` emits a `#[repr(C)]` struct matching the shader's
    /// reflected layout; `field_user_attributes` surfaces `[Attr(...)]` marks.
    #[test]
    fn codegen_and_user_attributes() {
        // The attributes are declared in `assets/shaders/editor_metadata.slang`
        // (Slang reflects only declared user attributes — `{Name}Attribute`
        // structs with `[__AttributeUsage(...)]`).
        const SOURCE: &str = concat!(
            include_str!("../../../../assets/shaders/editor_metadata.slang"),
            r#"
            struct DrawData
            {
                column_major float4x4 mvp;
                float4 color;
                [EditorColor]
                float4 tint;
                [Range(0, 1)]
                float opacity;
            };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; };
            [shader("vertex")]
            VsOutput main(VsInput input, uniform DrawData root)
            {
                VsOutput o;
                o.position = mul(root.mvp, float4(input.position, 1.0)) + root.tint * root.opacity;
                return o;
            }
        "#,
        );
        let compiler = Compiler::new().expect("compiler");
        let refl = compiler
            .compile_source_to_reflection("gen", SOURCE, "main")
            .expect("reflection");

        let src = refl.struct_rust_source("DrawData").expect("codegen");
        assert!(src.starts_with("#[repr(C)]"), "starts with repr(C): {src}");
        assert!(src.contains("pub mvp: [f32; 16]"), "matrix → array: {src}");
        assert!(src.contains("pub opacity: f32"), "scalar: {src}");
        assert!(src.contains("offset 0"), "first field at 0: {src}");

        // The payloads are asserted exactly — Slang reflects declared user
        // attributes on SPIR-V, so an empty result is a real regression.
        let tint = refl
            .field_user_attributes("DrawData", "tint")
            .expect("field exists");
        assert_eq!(
            tint,
            vec![UserAttributeRef {
                name: "EditorColor".into(),
                args: vec![],
            }]
        );
        let opacity = refl
            .field_user_attributes("DrawData", "opacity")
            .expect("field exists");
        assert_eq!(
            opacity,
            vec![UserAttributeRef {
                name: "Range".into(),
                args: vec![UserAttributeArg::Int(0), UserAttributeArg::Int(1)],
            }]
        );
    }

    /// The ViewUniforms shape: a vertex entry with two pointer roots
    /// (`root` + `view`), the fixed-function vertex input intact, and the
    /// struct layout a `Ptr<T>` dereference actually uses — read from the
    /// emitted SPIR-V's member offsets, the ground truth the Rust mirror
    /// must match.
    #[test]
    fn two_pointer_roots_and_ptr_struct_layout() {
        let compiler = Compiler::new().expect("compiler");
        const SOURCE: &str = r#"
            struct DrawData { column_major float4x4 model; float4 color; };
            struct ViewUniforms
            {
                float3 view_pos;
                float aspect;
                column_major float4x4 view_proj;
            };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; float3 local_pos : TEXCOORD0; };
            [shader("vertex")]
            VsOutput main(VsInput input, Ptr<DrawData> root, Ptr<ViewUniforms> view)
            {
                VsOutput o;
                o.position = mul(view[0].view_proj, mul(root[0].model, float4(input.position, 1.0)));
                o.local_pos = input.position;
                return o;
            }
        "#;
        let reflection = compiler
            .compile_source_to_reflection("view_uniforms", SOURCE, "main")
            .expect("reflection");
        // Two pointer roots, each an 8-byte address placement.
        let binder = RootBinder::new(&reflection, "main").expect("binder");
        assert_eq!(binder.pointer_param("root").expect("root place").size, 8);
        assert_eq!(binder.pointer_param("view").expect("view place").size, 8);
        // The varying input is untouched by the pointer roots.
        let layout = reflection.vertex_layout("main").expect("vertex layout");
        assert_eq!(layout.attributes.len(), 1);
        assert_eq!(
            layout.attributes[0].format,
            crate::types::VertexFormat::Float32x3
        );

        // The emitted SPIR-V names the `Ptr` pointee types `..._natural`
        // (Slang's C-like natural layout — offsets baked into the pointer
        // arithmetic, no std140 padding games) and the entry-parameter
        // block `EntryPointParams_std430`. The natural layout is what
        // `struct_layout` (LayoutRules::Default) reports; the Rust mirror
        // must match it field-for-field.
        let view = reflection.struct_layout("ViewUniforms").expect("layout");
        assert_eq!(view.size(), 80);
        assert_eq!(view.field_offset("view_pos").expect("field"), 0);
        assert_eq!(view.field_offset("aspect").expect("field"), 12);
        assert_eq!(view.field_offset("view_proj").expect("field"), 16);
        let draw = reflection.struct_layout("DrawData").expect("layout");
        assert_eq!(draw.size(), 80);
        assert_eq!(draw.field_offset("model").expect("field"), 0);
        assert_eq!(draw.field_offset("color").expect("field"), 64);
    }

    /// The pulling-shape probe: a vertex entry whose only input is
    /// `SV_VertexID` — geometry arrives through `Ptr` fields inside the
    /// per-draw record. Verifies that (a) the system-value input does not
    /// become a vertex attribute, (b) pointer fields inside the uniform
    /// struct lay out at natural (C-like) offsets, and (c) both pointer
    /// roots resolve.
    #[test]
    fn pulling_vertex_shape() {
        let compiler = Compiler::new().expect("compiler");
        const SOURCE: &str = r#"
            struct ViewUniforms
            {
                column_major float4x4 view_proj;
                float3 view_pos;
                float _pad0;
            };
            struct DrawData
            {
                column_major float4x4 model;
                float4 color;
                Ptr<float3> positions;
                Ptr<uint32_t> indices;
            };
            struct VsOutput { float4 position : SV_POSITION; float3 local_pos : TEXCOORD0; };
            [shader("vertex")]
            VsOutput main(uint vid : SV_VertexID, Ptr<DrawData> root, Ptr<ViewUniforms> view)
            {
                VsOutput o;
                uint vi = root[0].indices[vid];
                float3 position = root[0].positions[vi];
                float4 world = mul(root[0].model, float4(position, 1.0));
                o.position = mul(view[0].view_proj, world);
                o.local_pos = position;
                return o;
            }
        "#;
        let reflection = compiler
            .compile_source_to_reflection("pulling", SOURCE, "main")
            .expect("reflection");

        let binder = RootBinder::new(&reflection, "main").expect("binder");
        assert_eq!(binder.pointer_param("root").expect("root place").size, 8);
        assert_eq!(binder.pointer_param("view").expect("view place").size, 8);

        // (a) System-value inputs reflect with category `None` and the
        // semantic name (e.g. "SV_VERTEXID") — the varying-input filter
        // excludes them, so a pulling vertex shader derives an empty
        // vertex layout.
        let layout = reflection.vertex_layout("main").expect("vertex layout");
        assert!(
            layout.attributes.is_empty(),
            "SV_VertexID must not become a vertex attribute"
        );

        // (b) Pointer fields inside the uniform struct: natural offsets —
        // model @0, color @64, positions @80, indices @88, size 96.
        let draw = reflection.struct_layout("DrawData").expect("layout");
        assert_eq!(draw.field_offset("model").expect("field"), 0);
        assert_eq!(draw.field_offset("color").expect("field"), 64);
        assert_eq!(draw.field_offset("positions").expect("field"), 80);
        assert_eq!(draw.field_offset("indices").expect("field"), 88);
        assert_eq!(draw.size(), 96);
    }
}
