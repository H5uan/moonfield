use winit::raw_window_handle;

use crate::{Api, InstanceDescriptor};

pub trait Instance: Sized {
    type A: Api;
    fn init(desc: &InstanceDescriptor);
    fn create_surface(
        &self, display_handle: raw_window_handle::RawDisplayHandle,
        window_handle: raw_window_handle::RawWindowHandle,
    );
    fn enumerate_adapeters(&self);
}
