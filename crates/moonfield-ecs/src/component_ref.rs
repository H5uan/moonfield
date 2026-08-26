use crate::{archetype::Archetype, entity_ref::EntityRef, Component};

pub trait ComponentRef<'a> {
    type Ref;
    type Column;
    type Component: Component;

    fn get_component(entity: EntityRef<'a>) -> Option<Self::Ref>;

    unsafe fn from_raw(raw: *mut Self::Component) -> Self;

    fn get_column(archetype: &'a Archetype) -> Option<Self::Column>;
}
