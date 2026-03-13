use std::sync::Arc;
use crate::{types::*, Adapter, AdapterProperties, RhiError};

// Import Windows-specific DirectX 12 types
use windows::{
    core::*,
    Win32::Graphics::Dxgi::*,
};

// Import tracing for logging
use tracing;

// Use explicit Result type to avoid confusion with windows::core::Result
use std::result::Result as StdResult;

pub struct Dx12Adapter {
    pub adapter: IDXGIAdapter1,
    pub properties: AdapterProperties,
}

impl Dx12Adapter {
    pub fn new(adapter: IDXGIAdapter1) -> StdResult<Self, RhiError> {
        let mut desc: DXGI_ADAPTER_DESC1 = unsafe { std::mem::zeroed() };
        unsafe {
            let hr = adapter.GetDesc1(&mut desc);
            hr.map_err(|e| 
                RhiError::InitializationFailed(format!("Failed to get adapter description: {}", e))
            )?;
        }

        let name = unsafe {
            std::ffi::CStr::from_ptr(desc.Description.as_ptr() as *const i8)
                .to_string_lossy()
                .to_string()
        };

        let properties = AdapterProperties {
            name,
            vendor_id: desc.VendorId as u32,
            device_id: desc.DeviceId as u32,
        };

        Ok(Dx12Adapter { adapter, properties })
    }
}

impl Adapter for Dx12Adapter {
    fn request_device(&self) -> StdResult<Arc<dyn crate::Device>, RhiError> {
        tracing::debug!("Requesting DirectX 12 logical device");
        unsafe {
            let mut device: Option<ID3D12Device> = None;
            let hr = D3D12CreateDevice(
                &self.adapter,
                D3D_FEATURE_LEVEL_11_0,
                &mut device,
            );
            
            if hr.is_err() {
                tracing::error!("Failed to create D3D12 device");
                return Err(RhiError::DeviceCreationFailed("Failed to create D3D12 device".to_string()));
            }

            if let Some(d3d12_device) = device {
                let dx12_device = super::device::Dx12Device::new(&d3d12_device)
                    .map_err(|e| {
                        tracing::error!("Failed to initialize DirectX 12 device: {:?}", e);
                        e
                    })?;
                tracing::info!("DirectX 12 logical device created successfully");
                Ok(Arc::new(dx12_device) as Arc<dyn crate::Device>)
            } else {
                tracing::error!("D3D12 device creation returned null");
                Err(RhiError::DeviceCreationFailed("D3D12 device creation returned null".to_string()))
            }
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        tracing::debug!("Getting DirectX 12 adapter properties");
        self.properties.clone()
    }
}