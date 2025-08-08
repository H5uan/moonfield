use crate::Api;

/// Physical GPU
pub trait Adapter {
    type A: Api;

    fn open(&self);
    fn texture_format_capabilities(&self);
    fn surface_capabilities(&self, surface: &<Self::A as Api>::Surface);
}
