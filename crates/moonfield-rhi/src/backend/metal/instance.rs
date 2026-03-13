use crate::{types::*, Instance, Surface, Adapter, RhiError};
use std::sync::Arc;

// Import tracing for logging
use tracing;

pub struct MetalInstance {}

impl MetalInstance {
    pub fn new() -> Result<Self, RhiError> {
        tracing::debug!("Creating Metal instance");
        tracing::info!("Metal instance created successfully");
        Ok(Self {})
    }
}

impl Instance for MetalInstance {
    fn create_surface(&self, window: &winit::window::Window) -> Result<Arc<dyn Surface>, RhiError> {
        tracing::debug!("Creating Metal surface for window");
        tracing::debug!("Metal surface created successfully");
        Ok(Arc::new(MetalSurface {}))
    }

    fn enumerate_adapters(&self) -> Vec<Arc<dyn Adapter>> {
        tracing::debug!("Enumerating Metal adapters");
        unsafe {
            let devices = objc2_metal::MTLCopyAllDevices();
            
            let adapters: Vec<Arc<dyn Adapter>> = (0..devices.count())
                .filter_map(|i| {
                    devices.objectAtIndex(i)
                })
                .map(|device| {
                    tracing::debug!("Found Metal device");
                    Arc::new(MetalAdapter {
                        device: objc2::rc::Retained::retain(device).unwrap(),
                    }) as Arc<dyn Adapter>
                })
                .collect();
                
            tracing::info!("Found {} Metal adapters", adapters.len());
            adapters
        }
    }
}