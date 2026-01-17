use crate::{types::*, Buffer, RhiError};

pub struct MetalBuffer {
    pub buffer: objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_metal::MTLBuffer>>,
}

impl std::any::Any for MetalBuffer {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<MetalBuffer>()
    }
}

impl Buffer for MetalBuffer {
    fn map(&self) -> Result<*mut u8, RhiError> {
        unsafe {
            let ptr = self.buffer.contents();
            Ok(ptr.as_ptr() as *mut u8)
        }
    }

    fn unmap(&self) {}
}