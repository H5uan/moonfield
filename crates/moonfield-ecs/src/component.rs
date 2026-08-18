/// Marker trait for types that can be used as components.
///
/// Automatically implemented for all `Send + Sync + 'static` types.
pub trait Component: Send + Sync + 'static {}
impl<T: Send + Sync + 'static> Component for T {}
