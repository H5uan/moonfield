/// Marker trait for types that can be stored as singleton resources in [`World`].
///
/// Resources are unique (only one instance per type) and are accessed via
/// `world.get_resource::<T>()`.
pub trait Resource: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Resource for T {}
