use crate::App;
use std::any::Any;

/// A unit of application logic that can be registered with an [`App`].
///
/// Plugins are the primary mechanism for extending an application. They can be
/// implemented as structs or as plain functions via the blanket implementation
/// for `fn(&mut App)`.
///
/// # Example
///
/// ```
/// use moonfield_app::{App, Plugin};
///
/// struct MyPlugin;
///
/// impl Plugin for MyPlugin {
///     fn build(&self, app: &mut App) {
///         // configure the app
///     }
/// }
///
/// App::new().add_plugins(MyPlugin).run();
/// ```
pub trait Plugin: Any + Send + Sync {
    /// Configures the [`App`] to which this plugin is added.
    fn build(&self, app: &mut App);

    /// Called once after all plugins have been built but before the runner starts.
    fn finish(&self, _app: &mut App) {}

    /// Called once after the runner has returned.
    fn cleanup(&self, _app: &mut App) {}

    /// Returns a name used for duplicate-plugin detection and debugging.
    fn name(&self) -> &str {
        std::any::type_name::<Self>()
    }

    /// Whether this plugin may only be added once to a given [`App`].
    fn is_unique(&self) -> bool {
        true
    }
}

impl<T: Fn(&mut App) + Send + Sync + 'static> Plugin for T {
    fn build(&self, app: &mut App) {
        self(app);
    }

    fn name(&self) -> &str {
        std::any::type_name::<T>()
    }
}
