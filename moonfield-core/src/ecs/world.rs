use crate::ecs::entities::Entities;

pub struct World {
    entities: Entities,
}

/// Marker trait for component
pub trait Component: Send + Sync + 'static {}
impl <T: Send + Sync + 'static> Component for T{}