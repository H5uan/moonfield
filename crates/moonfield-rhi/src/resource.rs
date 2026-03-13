use std::result::Result;

pub trait Resource {
    fn get_native_handle(&self) -> Result<u64, crate::types::RhiError>;
}
