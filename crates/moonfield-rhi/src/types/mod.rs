pub mod binding;
pub mod buffer;
pub mod error;
pub mod format;
pub mod pipeline;
pub mod render_pass;
pub mod surface;
pub mod vertex;

pub use binding::*;
pub use buffer::*;
pub use error::*;
pub use format::*;
pub use pipeline::*;
pub use render_pass::*;
pub use surface::*;
pub use vertex::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Vulkan,
    Metal,
    Dx12,
}

#[derive(Debug, Clone)]
pub struct AdapterProperties {
    pub name: String,
    pub vendor_id: u32,
    pub device_id: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rhi_error_variants() {
        let _err1 = error::RhiError::InitializationFailed("test".to_string());
        let _err2 = error::RhiError::DeviceCreationFailed("test".to_string());
        let _err3 =
            error::RhiError::SwapchainCreationFailed("test".to_string());
        let _err4 = error::RhiError::ShaderCompilationFailed(
            error::ShaderCompilationError::InvalidShaderCode(
                "test".to_string(),
            ),
        );
        let _err5 = error::RhiError::ResourceNotFound("test".to_string());
        let _err6 = error::RhiError::OutOfMemory("test".to_string());

        assert!(true);
    }

    #[test]
    fn test_error_conversions() {
        let err_from_str: error::RhiError = "test error".into();
        let err_from_string: error::RhiError = "test error".to_string().into();

        match err_from_str {
            error::RhiError::InitializationFailed(_) => {}
            _ => panic!("Expected InitializationFailed variant"),
        }

        match err_from_string {
            error::RhiError::InitializationFailed(_) => {}
            _ => panic!("Expected InitializationFailed variant"),
        }
    }
}
