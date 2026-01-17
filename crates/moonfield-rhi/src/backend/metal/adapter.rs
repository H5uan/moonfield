use crate::{types::*, Adapter, Device, AdapterProperties, RhiError};
use std::sync::Arc;

// Import tracing for logging
use tracing;

pub struct MetalAdapter {
    pub device: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLDevice>>,
}

impl std::any::Any for MetalAdapter {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalAdapter>()
    }
}

impl Adapter for MetalAdapter {
    fn request_device(&self) -> Result<Arc<dyn Device>, RhiError> {
        tracing::debug!("Requesting Metal logical device");
        unsafe {
            let queue = self
                .device
                .newCommandQueue()
                .ok_or_else(|| {
                    tracing::error!("Failed to create Metal command queue");
                    RhiError::DeviceCreationFailed("Failed to create Metal command queue".to_string())
                })?;

            tracing::info!("Metal logical device created successfully");
            Ok(Arc::new(MetalDevice {
                device: self.device.clone(),
                queue,
            }))
        }
    }

    fn get_properties(&self) -> AdapterProperties {
        unsafe {
            let name = self.device.name().to_string();
            tracing::debug!("Getting Metal adapter properties: {}", name);
            AdapterProperties {
                name,
                vendor_id: 0,
                device_id: 0,
            }
        }
    }
}