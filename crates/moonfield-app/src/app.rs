use crate::{Plugin, PluginGroup};
use moonfield_ecs::{IntoSystem, System, World};
use std::collections::HashSet;

type StartupFn = Box<dyn FnOnce(&mut World)>;
type ShutdownFn = Box<dyn FnOnce(&mut World)>;
type UpdateFn = Box<dyn FnMut(&mut World) -> bool>;
type RenderFn = Box<dyn FnMut(&mut World)>;

/// Errors that can occur while adding a [`Plugin`] to an [`App`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    /// A plugin with the same name was already added.
    #[error("duplicate plugin {plugin_name:?}")]
    DuplicatePlugin { plugin_name: String },
}

/// The main application container.
///
/// An [`App`] holds registered plugins. Plugins are built when they are
/// added, and [`App::run`] calls `finish()`, runs the update loop, then
/// calls `cleanup()`.
///
/// # Runner
///
/// By default [`App::run`] runs its own update loop. A plugin can override
/// this by calling [`App::set_runner`]. The runner is a closure that receives
/// `&mut App` and drives the application.
#[must_use]
pub struct App {
    plugins: Vec<Box<dyn Plugin>>,
    plugin_names: HashSet<String>,
    world: World,
    startup_fns: Vec<StartupFn>,
    shutdown_fns: Vec<ShutdownFn>,
    update_fns: Vec<UpdateFn>,
    render_fns: Vec<RenderFn>,
    runner: Option<Runner>,
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
            world: World::new(),
            startup_fns: Vec::new(),
            shutdown_fns: Vec::new(),
            update_fns: Vec::new(),
            render_fns: Vec::new(),
            runner: None,
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
    pub fn add_boxed_plugin(&mut self, plugin: Box<dyn Plugin>) -> Result<(), AppError> {
        let name = plugin.name().to_string();
        if plugin.is_unique() && !self.plugin_names.insert(name.clone()) {
            return Err(AppError::DuplicatePlugin { plugin_name: name });
        }
        plugin.build(self);
        self.plugins.push(plugin);
        Ok(())
    }

    /// Inserts a resource into the app's world.
    pub fn insert_resource<R: moonfield_ecs::Resource>(&mut self, resource: R) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    /// Gets an immutable reference to a previously inserted resource.
    pub fn get_resource<R: moonfield_ecs::Resource>(&self) -> Option<std::cell::Ref<'_, R>> {
        self.world.get_resource::<R>()
    }

    /// Gets a mutable reference to a previously inserted resource.
    pub fn get_resource_mut<R: moonfield_ecs::Resource>(&self) -> Option<std::cell::RefMut<'_, R>> {
        self.world.get_resource_mut::<R>()
    }

    /// Access the underlying ECS world immutably.
    pub fn world(&self) -> &World {
        &self.world
    }

    /// Access the underlying ECS world mutably.
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Register a startup callback.
    pub fn add_startup_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut World) + 'static,
    {
        self.startup_fns.push(Box::new(f));
        self
    }

    /// Register a shutdown callback.
    pub fn add_shutdown_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnOnce(&mut World) + 'static,
    {
        self.shutdown_fns.push(Box::new(f));
        self
    }

    /// Register an update callback. Returning `false` ends the loop.
    pub fn add_update_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnMut(&mut World) -> bool + 'static,
    {
        self.update_fns.push(Box::new(f));
        self
    }

    /// Register an ECS system to run every frame.
    pub fn add_systems(&mut self, system: impl IntoSystem) -> &mut Self {
        let mut sys = system.system();
        self.update_fns.push(Box::new(move |world: &mut World| {
            sys.run(world);
            true
        }));
        self
    }

    /// Register a render system. Render systems run once per frame after the
    /// update phase, when a windowing backend calls [`App::render`]. Unlike
    /// update systems they cannot terminate the loop — their return value is
    /// discarded.
    ///
    /// Render systems are how plugins that do not own the event loop (e.g. an
    /// editor or a UI renderer) draw into the frame produced by the windowing
    /// backend, mirroring Bevy's render schedule.
    pub fn add_render_system<F>(&mut self, f: F) -> &mut Self
    where
        F: FnMut(&mut World) + 'static,
    {
        self.render_fns.push(Box::new(f));
        self
    }

    /// Register an ECS startup system to run once at startup.
    pub fn add_startup_system_ecs(&mut self, system: impl IntoSystem) -> &mut Self {
        let mut sys = system.system();
        self.startup_fns.push(Box::new(move |world: &mut World| {
            sys.run(world);
        }));
        self
    }

    /// Set a custom runner function that replaces the default update loop.
    ///
    /// The runner receives `&mut App` and drives the application (typically
    /// via a winit event loop). It is called once from [`App::run`] after
    /// all plugins have been finished.
    ///
    /// # Example
    ///
    /// ```ignore
    /// app.set_runner(Box::new(|app: &mut App| {
    ///     loop {
    ///         if !app.update() {
    ///             break;
    ///         }
    ///     }
    /// }));
    /// ```
    pub fn set_runner(&mut self, runner: Runner) -> &mut Self {
        self.runner = Some(runner);
        self
    }

    /// Take the runner, if set.
    pub fn take_runner(&mut self) -> Option<Runner> {
        self.runner.take()
    }

    /// Run startup systems.
    pub fn startup(&mut self) {
        moonfield_base::initialize();
        self.initialized = true;
        for f in self.startup_fns.drain(..) {
            f(&mut self.world);
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
        self.world.apply_commands();
        for f in &mut self.update_fns {
            if !f(&mut self.world) {
                return false;
            }
        }
        true
    }

    /// Run one render tick. Called by the windowing backend after
    /// [`App::update`] each frame; invokes every registered render system in
    /// registration order. Startup runs lazily on the first call so a backend
    /// that drives `render` without `update` still initializes.
    ///
    /// Render systems cannot terminate the loop.
    pub fn render(&mut self) {
        if !self.initialized {
            self.startup();
        }
        for f in &mut self.render_fns {
            f(&mut self.world);
        }
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
            f(&mut self.world);
        }
        moonfield_base::shutdown();
        self.initialized = false;
    }

    /// Finishes all plugins, runs the update loop (or a custom runner), then
    /// cleans up all plugins.
    ///
    /// If a runner was set via [`set_runner`], it is called instead of the
    /// default update loop. The runner receives `&mut App` and drives the
    /// application.
    pub fn run(&mut self) {
        let plugins = std::mem::take(&mut self.plugins);

        for plugin in &plugins {
            plugin.finish(self);
        }

        // If a plugin set a runner, delegate to it.
        if let Some(runner) = self.runner.take() {
            runner.0(self);
        } else {
            self.run_updates();
        }

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

/// A runner function that drives the application.
///
/// A plugin that wants to replace the default update loop can set a runner
/// via [`App::set_runner`]. The runner is responsible for calling
/// `app.update()` each frame.
pub struct Runner(pub Box<dyn FnOnce(&mut App)>);

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

    impl<A, MA, B, MB, C, MC, D, MD> Plugins<(PluginsTupleMarker, MA, MB, MC, MD)> for (A, B, C, D)
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

    impl<A, MA, B, MB, C, MC, D, MD, E, ME> Plugins<(PluginsTupleMarker, MA, MB, MC, MD, ME)>
        for (A, B, C, D, E)
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
