pub trait Query {}

/// # Safety
/// This trait is unsafe because the implementer must ensure that the query
/// does not mutably access any component data.
pub unsafe trait QueryShared {}
