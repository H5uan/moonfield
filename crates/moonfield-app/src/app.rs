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

/// A type-erased resource container.
#[derive(Default)]
pub struct Resources {
    data: HashMap<TypeId, Box<dyn Any>>,
}

impl Resources {
    /// Insert a resource.
    pub fn insert<R: 'static>(&mut self, resource: R) {
        self.data.insert(TypeId::of::<R>(), Box::new(resource));
    }

    /// Get a reference to a resource.
    pub fn get<R: 'static>(&self) -> Option<&R> {
        self.data
            .get(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_ref::<R>())
    }

    /// Get a mutable reference to a resource.
    pub fn get_mut<R: 'static>(&mut self) -> Option<&mut R> {
        self.data
            .get_mut(&TypeId::of::<R>())
            .and_then(|boxed| boxed.downcast_mut::<R>())
    }
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
    resources: Resources,
    startup_fns: Vec<Box<dyn FnOnce(&mut Resources)>>,
    shutdown_fns: Vec<Box<dyn FnOnce(&mut Resources)>>,
    update_fns: Vec<Box<dyn FnMut(&mut Resources) -> bool>>,
    initialized: bool,
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
            resources: Resources::default(),
            startup_fns: Vec::new(),
            shutdown_fns: Vec::new(),
            update_fns: Vec::new(),
            initialized: false,
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

    /// Register a single plugin (convenience wrapper).
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        self.add_plugins(plugin);
        self
    }

    /// Registers a boxed plugin, returning an error if a unique plugin with
    /// the same name was already added.
    pub fn add_boxed_plugin(
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
    pub fn insert_resource<R: 'static>(&mut self, resource: R) -> &mut Self {
        self.resources.insert(resource);
        self
    }

    /// Gets an immutable reference to a previously inserted resource.
    pub fn get_resource<R: 'static>(&self) -> Option<&R> {
        self.resources.get()
    }

    /// Gets a mutable reference to a previously inserted resource.
    pub fn get_resource_mut<R: 'static>(&mut self) -> Option<&mut R> {
        self.resources.get_mut()
    }

    /// Access resources immutably.
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Access resources mutably.
    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }

    /// Sets the runner function that will be invoked by [`App::run`].
    pub fn set_runner(
        &mut self,
        runner: impl FnMut(&mut App) + 'static,
    ) -> &mut Self {
        self.runner = Box::new(runner);
        self
    }

    /// Register a startup callback.
    pub fn add_startup_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut Resources) + 'static,
    {
        self.startup_fns.push(Box::new(f));
        self
    }

    /// Register a shutdown callback.
    pub fn add_shutdown_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut Resources) + 'static,
    {
        self.shutdown_fns.push(Box::new(f));
        self
    }

    /// Register an update callback. Returning `false` ends the loop.
    pub fn add_update_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnMut(&mut Resources) -> bool + 'static,
    {
        self.update_fns.push(Box::new(f));
        self
    }

    /// Run startup systems.
    pub fn startup(&mut self) {
        moonfield_base::initialize();
        self.initialized = true;
        for f in self.startup_fns.drain(..) {
            f(&mut self.resources);
        }
    }

    /// Run a single update tick. Returns `false` if any system returned `false`.
    ///
    /// This is the per-frame counterpart of [`run_updates`]; it runs startup
    /// once on the first call, then invokes each update system exactly once.
    pub fn update(&mut self) -> bool {
        if !self.initialized {
            self.startup();
        }
        for f in &mut self.update_fns {
            if !f(&mut self.resources) {
                return false;
            }
        }
        true
    }

    /// Run the update loop until a system returns `false` or no systems remain.
    pub fn run_updates(&mut self) {
        loop {
            if !self.update() {
                break;
            }
            if self.update_fns.is_empty() {
                break;
            }
        }
    }

    /// Run shutdown systems.
    pub fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }
        for f in self.shutdown_fns.drain(..) {
            f(&mut self.resources);
        }
        moonfield_base::shutdown();
        self.initialized = false;
    }

    /// Finishes all plugins, runs the runner, and then cleans up all plugins.
    pub fn run(&mut self) {
        let plugins = std::mem::take(&mut self.plugins);

        for plugin in &plugins {
            plugin.finish(self);
        }

        let mut runner = std::mem::replace(&mut self.runner, Box::new(|_| {}));
        runner(self);
        self.runner = runner;

        for plugin in &plugins {
            plugin.cleanup(self);
        }

        self.plugins = plugins;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.initialized {
            self.shutdown();
        }
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
