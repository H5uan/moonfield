use crate::{types::*, Surface, SurfaceCapabilities, Format, PresentMode, RhiError};

// Import tracing for logging
use tracing;

// Import Windows-specific DirectX 12 types
use windows::Win32::Foundation::*;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12Surface {
    pub window: *mut std::ffi::c_void,  // HWND
}

impl Dx12Surface {
    pub fn new(window: &winit::window::Window) -> StdResult<Self, RhiError> {
        // Get the HWND from the winit window
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        
        let window_handle = window.window_handle()
            .map_err(|e| RhiError::InitializationFailed(format!("Failed to get window handle: {}", e)))?;
        
        match window_handle.as_raw() {
            #[cfg(windows)]
            raw_window_handle::RawWindowHandle::Win32(handle) => {
                Ok(Dx12Surface {
                    window: handle.hwnd.get() as *mut std::ffi::c_void,
                })
            }
            _ => Err(RhiError::InitializationFailed("Unsupported window handle type".to_string())),
        }
    }
}

impl Surface for Dx12Surface {
    fn get_capabilities(&self, _adapter: &dyn crate::Adapter) -> SurfaceCapabilities {
        SurfaceCapabilities {
            formats: vec![Format::B8G8R8A8Unorm, Format::R8G8B8A8Unorm],
            present_modes: vec![PresentMode::Fifo, PresentMode::Immediate], // Basic support
            min_image_count: 2,
            max_image_count: 3,
        }
    }
}