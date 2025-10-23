use alloc::boxed::Box;

use super::{DynAdapter, DynObject};
use crate::{Api, Backend, DynSurface, InstanceError};

pub struct DynExposedAdapter {
    pub adapter: Box<dyn DynAdapter>,
}

impl DynExposedAdapter {
    pub fn backend(&self) -> Backend {
        self.adapter.backend()
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
        &self, display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
    ) {
    }

    unsafe fn enumerate_adapters(&self, surface_hint: Option<&dyn DynSurface>) {
    }
}
