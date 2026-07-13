//! Slang shader compiler integration.
//!
//! Wraps the `shader-slang` crate to compile Slang source into SPIR-V
//! bytecode. Errors are mapped to the Lunaris [`Error`] type.

use crate::error::{Error as LunarisError, Result as LunarisResult};
use shader_slang::Downcast;

/// Slang compiler session wrapper.
pub struct Compiler {
    global_session: shader_slang::GlobalSession,
}

impl Compiler {
    /// Create a new Slang compiler instance.
    pub fn new() -> LunarisResult<Self> {
        let global_session = shader_slang::GlobalSession::new()
            .ok_or_else(|| LunarisError::Backend("failed to create Slang global session".to_string()))?;
        Ok(Self { global_session })
    }

    /// Compile Slang source code to SPIR-V for the given entry point.
    ///
    /// `module_name` is used for diagnostics and does not need to correspond to
    /// a file on disk.
    pub fn compile_source_to_spirv(
        &self,
        module_name: &str,
        source: &str,
        entry_point: &str,
    ) -> LunarisResult<Vec<u8>> {
        // `shader-slang` 0.1 exposes file-based `load_module`. Compile from
        // source by writing to a temporary file.
        let temp_dir = std::env::temp_dir();
        let file_name = format!("{}.slang", module_name);
        let temp_path = temp_dir.join(&file_name);

        std::fs::write(&temp_path, source)
            .map_err(|e| LunarisError::Backend(format!("failed to write temp shader file: {}", e)))?;

        let result = self.compile_file_to_spirv(temp_path.to_string_lossy().as_ref(), entry_point);

        // Best-effort cleanup; ignore errors.
        let _ = std::fs::remove_file(&temp_path);

        result
    }

    /// Compile a Slang file to SPIR-V for the given entry point.
    pub fn compile_file_to_spirv(&self, path: &str, entry_point: &str) -> LunarisResult<Vec<u8>> {
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
            .ok_or_else(|| LunarisError::Backend("failed to create Slang session".to_string()))?;

        let module = session
            .load_module(path)
            .map_err(map_slang_error)?;

        let entry = module
            .find_entry_point_by_name(entry_point)
            .ok_or_else(|| LunarisError::Backend(format!("entry point '{}' not found", entry_point)))?;

        let program = session
            .create_composite_component_type(&[
                module.downcast().clone(),
                entry.downcast().clone(),
            ])
            .map_err(map_slang_error)?;

        let linked = program.link().map_err(map_slang_error)?;
        let bytecode = linked.entry_point_code(0, 0).map_err(map_slang_error)?;

        Ok(bytecode.as_slice().to_vec())
    }
}

fn map_slang_error(err: shader_slang::Error) -> LunarisError {
    let message = match err {
        shader_slang::Error::Code(code) => format!("Slang error code: {}", code),
        shader_slang::Error::Blob(blob) => blob.as_str().unwrap_or("unknown Slang error").to_string(),
    };
    LunarisError::ShaderCompilation(message)
}
