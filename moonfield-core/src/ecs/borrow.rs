use std::sync::atomic::AtomicUsize;

pub struct SharedRuntimeBorrow(AtomicUsize);