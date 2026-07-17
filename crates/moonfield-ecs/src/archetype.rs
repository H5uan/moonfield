use std::{alloc::Layout, any::TypeId};

#[derive(Copy, Clone, Debug)]
pub struct ArchetypeMeta {
    id: TypeId,
    layout: Layout,
    drop: unsafe fn(*mut u8),
    #[cfg(debug_assertions)]
    type_name: &'static str,
}

pub struct Archetype {
    meta: Vec<ArchetypeMeta>,
}
