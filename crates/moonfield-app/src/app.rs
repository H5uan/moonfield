use crate::{Plugin, PluginGroup};
use std::{
    any::{Any, TypeId},
    collections::{HashMap, HashSet},
};

/// Errors that can occur while adding a [`Plugin`] to an [`App`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// A plugin with the same name was already added.
    #[error("duplicate plugin {plugin_name:?}")]
    DuplicatePlugin { plugin_name: String },
}

/// The main application container.
///
/// An [`App`] holds registered plugins and a runner. Plugins are built when
/// they are added, and the runner is invoked by [`App::run`].
#[must_use]
pub struct App {
    plugins: Vec<Box<dyn Plugin>>,
    plugin_names: HashSet<String>,
    runner: Box<dyn FnMut(&mut App)>,
    resources: HashMap<TypeId, Box<dyn Any>>,
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl App {
    /// Creates a new, empty [`App`].
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            plugin_names: HashSet::new(),
            runner: Box::new(|_| {}),
            resources: HashMap::new(),
        }
    }

    /// Adds one or more [`Plugin`]s or [`PluginGroup`]s to the app.
    ///
    /// Accepts a single plugin, a single plugin group, or a tuple of plugins
    /// and plugin groups.
    pub fn add_plugins<M>(&mut self, plugins: impl Plugins<M>) -> &mut Self {
        plugins.add_to_app(self);
        self
    }

    /// Registers a boxed plugin, returning an error if a unique plugin with
    /// the same name was already added.
    pub(crate) fn add_boxed_plugin(
        &mut self,
        plugin: Box<dyn Plugin>,
    ) -> Result<(), AppError> {
        let name = plugin.name().to_string();
        if plugin.is_unique() && !self.plugin_names.insert(name.clone()) {
            return Err(AppError::DuplicatePlugin { plugin_name: name });
        }
        plugin.build(self);
        self.plugins.push(plugin);
        Ok(())
    }

    /// Inserts a resource into the app.
    pub fn insert_resource<R: Any>(&mut self, resource: R) -> &mut Self {
        self.resources.insert(TypeId::of::<R>(), Box::new(resource));
        self
    }

    /// Gets an immutable reference to a previously inserted resource.
    pub fn get_resource<R: Any>(&self) -> Option<&R> {
        self.resources
            .get(&TypeId::of::<R>())
            .and_then(|r| r.downcast_ref::<R>())
    }

    /// Gets a mutable reference to a previously inserted resource.
    pub fn get_resource_mut<R: Any>(&mut self) -> Option<&mut R> {
        self.resources
            .get_mut(&TypeId::of::<R>())
            .and_then(|r| r.downcast_mut::<R>())
    }

    /// Sets the runner function that will be invoked by [`App::run`].
    pub fn set_runner(
        &mut self,
        runner: impl FnMut(&mut App) + 'static,
    ) -> &mut Self {
        self.runner = Box::new(runner);
        self
    }

    /// Finishes all plugins, runs the runner, and then cleans up all plugins.
    pub fn run(&mut self) {
        // Temporarily move the plugin registry out so we can call lifecycle
        // hooks while holding `&mut self`.
        let plugins = std::mem::take(&mut self.plugins);

        for plugin in &plugins {
            plugin.finish(self);
        }

        // Temporarily swap the runner out so we can call it with `&mut self`
        // without borrowing the runner field at the same time.
        let mut runner = std::mem::replace(&mut self.runner, Box::new(|_| {}));
        runner(self);
        self.runner = runner;

        for plugin in &plugins {
            plugin.cleanup(self);
        }

        self.plugins = plugins;
    }
}

/// Types that can be passed to [`App::add_plugins`].
pub trait Plugins<Marker>: sealed::Plugins<Marker> {}

impl<Marker, T: sealed::Plugins<Marker>> Plugins<Marker> for T {}

/// Marker types for [`Plugins`] implementations.
pub mod plugin_markers {
    /// Marker for a single [`Plugin`].
    pub struct PluginMarker;
    /// Marker for a single [`PluginGroup`].
    pub struct PluginGroupMarker;
    /// Marker for a tuple of plugins.
    pub struct PluginsTupleMarker;
}

/// Sealed implementations of [`Plugins`].
pub mod sealed {
    use super::*;
    use plugin_markers::*;

    /// Internal trait for types that can be passed to [`App::add_plugins`].
    pub trait Plugins<Marker> {
        /// Adds the represented plugins to the app.
        fn add_to_app(self, app: &mut App);
    }

    impl<P: Plugin> Plugins<PluginMarker> for P {
        fn add_to_app(self, app: &mut App) {
            if let Err(AppError::DuplicatePlugin { plugin_name }) =
                app.add_boxed_plugin(Box::new(self))
            {
                panic!(
                    "Error adding plugin {plugin_name}: plugin was already added in application"
                );
            }
        }
    }

    impl<G: PluginGroup> Plugins<PluginGroupMarker> for G {
        fn add_to_app(self, app: &mut App) {
            self.build().finish(app);
        }
    }

    impl Plugins<PluginsTupleMarker> for () {
        fn add_to_app(self, _app: &mut App) {}
    }

    impl<A, MA> Plugins<(PluginsTupleMarker, MA)> for (A,)
    where
        A: Plugins<MA>,
    {
        fn add_to_app(self, app: &mut App) {
            let (a,) = self;
            a.add_to_app(app);
        }
    }

    impl<A, MA, B, MB> Plugins<(PluginsTupleMarker, MA, MB)> for (A, B)
    where
        A: Plugins<MA>,
        B: Plugins<MB>,
    {
        fn add_to_app(self, app: &mut App) {
            let (a, b) = self;
            a.add_to_app(app);
            b.add_to_app(app);
        }
    }

    impl<A, MA, B, MB, C, MC> Plugins<(PluginsTupleMarker, MA, MB, MC)> for (A, B, C)
    where
        A: Plugins<MA>,
        B: Plugins<MB>,
        C: Plugins<MC>,
    {
        fn add_to_app(self, app: &mut App) {
            let (a, b, c) = self;
            a.add_to_app(app);
            b.add_to_app(app);
            c.add_to_app(app);
        }
    }

    impl<A, MA, B, MB, C, MC, D, MD> Plugins<(PluginsTupleMarker, MA, MB, MC, MD)>
        for (A, B, C, D)
    where
        A: Plugins<MA>,
        B: Plugins<MB>,
        C: Plugins<MC>,
        D: Plugins<MD>,
    {
        fn add_to_app(self, app: &mut App) {
            let (a, b, c, d) = self;
            a.add_to_app(app);
            b.add_to_app(app);
            c.add_to_app(app);
            d.add_to_app(app);
        }
    }

    impl<A, MA, B, MB, C, MC, D, MD, E, ME>
        Plugins<(PluginsTupleMarker, MA, MB, MC, MD, ME)> for (A, B, C, D, E)
    where
        A: Plugins<MA>,
        B: Plugins<MB>,
        C: Plugins<MC>,
        D: Plugins<MD>,
        E: Plugins<ME>,
    {
        fn add_to_app(self, app: &mut App) {
            let (a, b, c, d, e) = self;
            a.add_to_app(app);
            b.add_to_app(app);
            c.add_to_app(app);
            d.add_to_app(app);
            e.add_to_app(app);
        }
    }
}
