use crate::DynObject;

pub trait DynAdapter : DynObject{
    unsafe fn open();
    unsafe fn texture_format_capabilites();
    unsafe fn surface_capabilities();
}

