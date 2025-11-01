use std::boxed::Box;

use super::{DynAdapter, DynObject};
use crate::{AdapterInfo, Api, Backend, DynSurface, Feature, Capabilities,InstanceError, Instance};

pub struct DynExposedAdapter {
    pub adapter: Box<dyn DynAdapter>,
    pub info: AdapterInfo,
    pub features: Feature,
    pub capabilities: Capabilities,
}

impl DynExposedAdapter {
    pub fn backend(&self) -> Backend {
        self.info.backend
    }
}


pub trait DynInstance: DynObject {
    unsafe fn create_surface(
        &self, display_handle: raw_window_handle::DisplayHandle,
        window_handle: raw_window_handle::WindowHandle,
    ) -> Result<Box<dyn DynSurface>, InstanceError>;

    unsafe fn enumerate_adapters(&self, surface_hint: Option<&dyn DynSurface>);
}

impl<I: Instance + DynObject> DynInstance for I {
    unsafe fn create_surface(
        &self, display_handle: raw_window_handle::DisplayHandle,
        window_handle: raw_window_handle::WindowHandle,
    ) -> Result<Box<dyn DynSurface>, InstanceError> {
        todo!("Implement create_surface")
    }

    unsafe fn enumerate_adapters(&self, surface_hint: Option<&dyn DynSurface>) {
        todo!("Implement enumerate_adapters")
    }
}
