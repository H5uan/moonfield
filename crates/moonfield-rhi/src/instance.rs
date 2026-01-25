use std::sync::Arc;

use raw_window_handle::HasDisplayHandle;
use winit::window::Window;

use crate::types::RhiError;
use crate::{Adapter, Surface};

/// Trait for RHI instance functionality
///
/// Instance meaning the global entry/context for graphics api.
/// Each process should have only one instance.
pub trait Instance {
    /// Creates a surface for a given window
    fn create_surface(
        &self, window: &Window,
    ) -> Result<Arc<dyn Surface>, RhiError>;

    /// Enumerates available adapters
    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>>;
}

/// Creates a new RHI instance for the specified backend
pub fn create_instance(
    backend: crate::types::Backend,
) -> Result<Arc<dyn Instance>, RhiError> {
    tracing::debug!("Creating RHI instance for backend: {:?}", backend);
    match backend {
        #[cfg(feature = "vulkan")]
        crate::types::Backend::Vulkan => {
            #[cfg(windows)]
            {
                tracing::debug!("Using Vulkan backend");
                Ok(Arc::new(crate::backend::vulkan::VulkanInstance::new()?))
            }
            #[cfg(not(windows))]
            {
                tracing::warn!("Vulkan backend not supported on this platform");
                Err(RhiError::BackendNotSupported)
            }
        }
        #[cfg(target_os = "macos")]
        crate::types::Backend::Metal => {
            tracing::debug!("Using Metal backend");
            Ok(Arc::new(crate::backend::metal::MetalInstance::new()?))
        }
        #[cfg(all(windows, feature = "dx12"))]
        crate::types::Backend::Dx12 => {
            tracing::debug!("Using DirectX 12 backend");
            Ok(Arc::new(crate::backend::dx12::Dx12Instance::new()?))
        }
        _ => {
            tracing::error!("Requested backend {:?} is not supported", backend);
            Err(RhiError::BackendNotSupported)
        }
    }
}

/// Creates a new RHI instance with a window handle
pub fn create_instance_with_window(
    backend: crate::types::Backend, window: &Window,
) -> Result<Arc<dyn Instance>, RhiError> {
    tracing::debug!(
        "Creating RHI instance with window for backend: {:?}",
        backend
    );
    match backend {
        #[cfg(feature = "vulkan")]
        crate::types::Backend::Vulkan => {
            let display = window
                .display_handle()
                .map_err(|e| {
                    tracing::error!("Failed to get display handle: {}", e);
                    RhiError::InitializationFailed(e.to_string())
                })?
                .as_raw();
            tracing::debug!("Using Vulkan backend with window");
            Ok(Arc::new(
                crate::backend::vulkan::VulkanInstance::new_with_display(
                    display,
                )?,
            ))
        }
        #[cfg(target_os = "macos")]
        crate::types::Backend::Metal => {
            tracing::debug!("Using Metal backend with window");
            Ok(Arc::new(crate::backend::metal::MetalInstance::new()?))
        }
        #[cfg(all(windows, feature = "dx12"))]
        crate::types::Backend::Dx12 => {
            tracing::debug!("Using DirectX 12 backend with window");
            Ok(Arc::new(crate::backend::dx12::Dx12Instance::new()?))
        }
        _ => {
            tracing::error!(
                "Requested backend {:?} is not supported with window",
                backend
            );
            Err(RhiError::BackendNotSupported)
        }
    }
}
