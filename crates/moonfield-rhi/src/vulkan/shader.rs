//! Slang shader compiler integration.
//!
//! Wraps the `shader-slang` crate to compile Slang source into SPIR-V
//! bytecode. Errors are mapped to the [`Error`](crate::error::Error) type.

use crate::error::{Error as RenderError, Result as RenderResult};

/// Slang compiler session wrapper.
pub struct Compiler {
    global_session: shader_slang::GlobalSession,
}

impl Compiler {
    /// Create a new Slang compiler instance.
    pub fn new() -> RenderResult<Self> {
        let global_session = shader_slang::GlobalSession::new().ok_or_else(|| {
            RenderError::Backend("failed to create Slang global session".to_string())
        })?;
        Ok(Self { global_session })
    }

    /// Compile Slang source code to SPIR-V for the given entry point.
    ///
    /// `module_name` is used for diagnostics and as the module's logical name;
    /// it does not need to correspond to a file on disk.
    pub fn compile_source_to_spirv(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
    ) -> RenderResult<Vec<u8>> {
        self.compile_source_to_spirv_impl(module_name, source, entry_point, &[])
    }

    /// Compile a Slang file to SPIR-V for the given entry point.
    pub fn compile_file_to_spirv(&self, path: &str, entry_point: &str) -> RenderResult<Vec<u8>> {
        self.compile_file_to_spirv_impl(path, entry_point, &[])
    }

    /// Compile a Slang file to SPIR-V with extra SPIR-V capabilities enabled.
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
    ) -> RenderResult<Vec<u8>> {
        self.compile_file_to_spirv_impl(path, entry_point, capabilities)
    }

    /// Compile Slang source to SPIR-V with extra capabilities (see
    /// [`compile_file_to_spirv_with_capabilities`]).
    pub fn compile_source_to_spirv_with_capabilities(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
        capabilities: &[&str],
    ) -> RenderResult<Vec<u8>> {
        self.compile_source_to_spirv_impl(module_name, source, entry_point, capabilities)
    }

    fn compile_file_to_spirv_impl(
        &self,
        path: &str,
        entry_point: &str,
        capabilities: &[&str],
    ) -> RenderResult<Vec<u8>> {
        let session = self.create_session(capabilities)?;
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
    ) -> RenderResult<Vec<u8>> {
        let session = self.create_session(capabilities)?;
        let module = session
            .load_module_from_source_string(module_name, &format!("{module_name}.slang"), source)
            .map_err(map_slang_error)?;
        self.finish_compile(&session, module, entry_point)
    }

    /// Create a Slang session targeting SPIR-V with the given capabilities.
    fn create_session(&self, capabilities: &[&str]) -> RenderResult<shader_slang::Session> {
        let mut options = shader_slang::CompilerOptions::default()
            .optimization(shader_slang::OptimizationLevel::High)
            .matrix_layout_row(true);
        for name in capabilities {
            let capability = self.global_session.find_capability(name);
            if !capability.is_unknown() {
                options = options.capability(capability);
            }
        }

        let profile = self.global_session.find_profile("glsl_450");
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

    /// Turn a loaded module into SPIR-V bytecode: pick the entry point, link
    /// the program, and extract the target code.
    fn finish_compile(
        &self,
        session: &shader_slang::Session,
        module: shader_slang::Module,
        entry_point: &str,
    ) -> RenderResult<Vec<u8>> {
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

        Ok(bytecode.as_slice().to_vec())
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

        let profile = self.global_session.find_profile("glsl_450");
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
