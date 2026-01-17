use std::sync::Arc;
use crate::{types::*, Instance, Surface, Adapter, RhiError};
use winit::window::Window;

// Import tracing for logging
use tracing;

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Dxgi::*,
    Win32::Foundation::*,
    Win32::System::LibraryLoader::GetModuleHandleW,
};

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

// Initialize the DirectX 12 Agility SDK
pub fn init_dx12_agility_sdk() -> StdResult<(), RhiError> {
    tracing::debug!("Initializing DirectX 12 Agility SDK");
    // Attempt to initialize the Agility SDK
    unsafe {
        // Load the Agility SDK DLL if available
        let _result = GetModuleHandleW(w!("D3D12Core.dll"));
        // Note: In a real implementation, you would dynamically load the Agility SDK DLL
        // This is a placeholder implementation
    }
    
    tracing::info!("DirectX 12 Agility SDK initialized");
    Ok(())
}

pub struct Dx12Instance {
    pub factory: IDXGIFactory4,
}

impl Dx12Instance {
    pub fn new() -> StdResult<Self, RhiError> {
        tracing::debug!("Creating DirectX 12 instance");
        init_dx12_agility_sdk()?;
        
        unsafe {
            let factory: IDXGIFactory4 = create_factory()
                .map_err(|e| {
                    tracing::error!("Failed to create DXGI factory: {:?}", e);
                    e
                })?;
            tracing::info!("DirectX 12 instance created successfully");
            Ok(Dx12Instance { factory })
        }
    }

    pub fn new_with_window(&self, _window: &Window) -> StdResult<Self, RhiError> {
        tracing::debug!("Creating DirectX 12 instance with window");
        // For now, just return a new instance
        Self::new()
    }
}

impl Instance for Dx12Instance {
    fn create_surface(&self, window: &winit::window::Window) -> StdResult<Arc<dyn Surface>, RhiError> {
        tracing::debug!("Creating DirectX 12 surface for window");
        let dx12_surface = Dx12Surface::new(window)
            .map_err(|e| {
                tracing::error!("Failed to create DirectX 12 surface: {:?}", e);
                e
            })?;
        tracing::debug!("DirectX 12 surface created successfully");
        Ok(Arc::new(dx12_surface) as Arc<dyn Surface>)
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        tracing::debug!("Enumerating DirectX 12 adapters");
        let mut adapters = Vec::new();
        unsafe {
            let mut i = 0;
            loop {
                match self.factory.EnumAdapters1(i) {
                    Ok(adapter) => {
                        tracing::debug!("Found DirectX 12 adapter");
                        // Check if this is a D3D12-compatible adapter
                        let hr = D3D12CreateDevice(
                            &adapter,
                            D3D_FEATURE_LEVEL_11_0,
                            std::ptr::null_mut() as *mut Option<ID3D12Device>,
                        );

                        if hr.is_ok() {
                            if let Ok(dx12_adapter) = Dx12Adapter::new(adapter) {
                                adapters.push(Arc::new(dx12_adapter) as Arc<dyn Adapter>);
                            }
                        }
                    }
                    Err(_) => {
                        tracing::debug!("No more DirectX 12 adapters found");
                        break; // No more adapters
                    }
                }
                i += 1;
            }
        }
        tracing::info!("Found {} DirectX 12 adapters", adapters.len());
        adapters
    }
}

unsafe fn create_factory() -> StdResult<IDXGIFactory4, RhiError> {
    // Create DXGI factory
    let mut factory4: Option<IDXGIFactory4> = None;
    
    // Try to create a debug factory first if we're in debug mode
    #[cfg(debug_assertions)]
    {
        let mut debug_controller: Option<ID3D12Debug> = None;
        let hr = D3D12GetDebugInterface(&mut debug_controller);
        if hr.is_ok() {
            if let Some(debug) = debug_controller {
                let _ = debug.EnableDebugLayer();
            }
        }
    }

    // Create the factory
    let hr = CreateDXGIFactory1(&mut factory4);
    if hr.is_err() {
        return Err(RhiError::InitializationFailed(format!("Failed to create DXGI factory: {}", hr.err().unwrap())));
    }
    
    if let Some(f) = factory4 {
        Ok(f)
    } else {
        Err(RhiError::InitializationFailed("Factory creation returned null".to_string()))
    }
}