use crate::{types::*, Swapchain, SwapchainImage, Extent2D, Format, RhiError};

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Dxgi::*,
};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12Swapchain {
    pub swapchain: IDXGISwapChain3,
    pub current_back_buffer_index: u32,
    pub format: Format,
    pub extent: Extent2D,
}

impl Dx12Swapchain {
    pub fn new(device: &super::device::Dx12Device, surface: &super::surface::Dx12Surface, desc: &SwapchainDescriptor) -> StdResult<Self, RhiError> {
        unsafe {
            let swapchain = create_dxgi_swapchain(device, surface, desc)?;
            
            let dx12_swapchain = Dx12Swapchain {
                swapchain,
                current_back_buffer_index: 0,
                format: desc.format,
                extent: desc.extent,
            };
            
            Ok(dx12_swapchain)
        }
    }
}

unsafe fn create_dxgi_swapchain(
    device: &super::device::Dx12Device,
    surface: &super::surface::Dx12Surface,
    desc: &SwapchainDescriptor,
) -> StdResult<IDXGISwapChain3, RhiError> {
    // Convert format to DXGI format
    let dxgi_format = match desc.format {
        Format::B8G8R8A8Unorm | Format::B8G8R8A8Srgb => DXGI_FORMAT_B8G8R8A8_UNORM,
        Format::R8G8B8A8Unorm | Format::R8G8B8A8Srgb => DXGI_FORMAT_R8G8B8A8_UNORM,
    };

    // Create swap chain descriptor
    let swapchain_desc = DXGI_SWAP_CHAIN_DESC1 {
        BufferCount: desc.image_count,
        Width: desc.extent.width,
        Height: desc.extent.height,
        Format: dxgi_format,
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        AlphaMode: DXGI_ALPHA_MODE_UNSPECIFIED,
        Scaling: DXGI_SCALING_STRETCH,
        Stereo: false.into(),
    };

    // Get DXGI factory from the device
    let adapter = {
        let mut adapter: Option<IDXGIAdapter> = None;
        let hr = device.device.QueryInterface(&mut adapter);
        if hr.is_err() {
            return Err(RhiError::SwapchainCreationFailed("Failed to get adapter from device".to_string()));
        }
        adapter.unwrap()
    };
    
    let factory: IDXGIFactory4 = {
        let mut factory: Option<IDXGIFactory> = None;
        let hr = adapter.GetParent(&mut factory);
        if hr.is_err() {
            return Err(RhiError::SwapchainCreationFailed("Failed to get factory from adapter".to_string()));
        }
        
        factory.unwrap().cast::<IDXGIFactory4>()
            .map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to get IDXGIFactory4: {}", e)))?
    };

    // Create the swap chain
    let hwnd = HWND(surface.window as isize);
    let swapchain: IDXGISwapChain1 = factory.CreateSwapChainForHwnd(
        &device.queue,
        hwnd,
        &swapchain_desc,
        None, // Don't restrict fullscreen
        None, // Don't use restrict tearing
    ).map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to create swapchain: {}", e)))?;

    // Cast to IDXGISwapChain3 for more features
    let swapchain: IDXGISwapChain3 = swapchain.cast::<IDXGISwapChain3>()
        .map_err(|e| RhiError::SwapchainCreationFailed(format!("Failed to cast swapchain: {}", e)))?;

    Ok(swapchain)
}

impl Swapchain for Dx12Swapchain {
    fn acquire_next_image(&self) -> StdResult<SwapchainImage, RhiError> {
        let current_index = unsafe {
            self.swapchain.GetCurrentBackBufferIndex()
        };
        
        Ok(SwapchainImage {
            index: current_index,
            image_view: current_index as usize,
            wait_semaphore: 0,
            signal_semaphore: 0,
        })
    }

    fn present(&self, _image: SwapchainImage) -> StdResult<(), RhiError> {
        unsafe {
            // Present the swapchain
            let hr = self.swapchain.Present(1, 0); // 1 for sync interval, 0 for present flags
            if hr.is_err() {
                return Err(RhiError::PresentFailed("Failed to present swapchain".to_string()));
            }
        }
        Ok(())
    }

    fn get_format(&self) -> Format {
        self.format
    }

    fn get_extent(&self) -> Extent2D {
        self.extent
    }
}