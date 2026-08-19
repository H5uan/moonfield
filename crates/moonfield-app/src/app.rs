use crate::{Plugin, PluginGroup};
use moonfield_ecs::{IntoSystemConfigs, Schedule, ScheduleLabel, World};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};

/// Schedule label for systems that run once at app startup.
pub struct Startup;
/// Schedule label for systems that run every frame, during [`App::update`].
pub struct Update;
/// Schedule label for render-phase systems, run by [`App::render`] after the
/// update phase when a windowing backend drives the frame.
pub struct Render;
/// Schedule label for systems that run once at app shutdown.
pub struct Shutdown;

impl ScheduleLabel for Startup {}
impl ScheduleLabel for Update {}
impl ScheduleLabel for Render {}
impl ScheduleLabel for Shutdown {}

/// Resource marking that the app should exit its update loop.
///
/// Insert it (e.g. via `Commands::insert_resource`) to make [`App::update`]
/// return `false`. This replaces the old convention of update systems
/// returning `bool`: systems no longer have return values.
#[derive(Debug, Default, Clone, Copy)]
pub struct AppExit;

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
/// # Schedules
///
/// Systems live in labeled [`Schedule`]s. The app drives four of them:
/// [`Startup`] (once, lazily on the first update/render), [`Update`] (every
/// [`App::update`]), [`Render`] (every [`App::render`], called by the
/// windowing backend after the update), and [`Shutdown`] (once, from
/// [`App::shutdown`]).
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
    schedules: HashMap<TypeId, Schedule>,
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
            schedules: HashMap::new(),
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

    /// Register lifecycle hooks for component `T` in the app's world.
    ///
    /// See [`moonfield_ecs::ComponentHooks`] for firing points and semantics.
    pub fn register_component_hooks<T: moonfield_ecs::Component>(
        &mut self,
    ) -> &mut moonfield_ecs::ComponentHooks {
        self.world.register_component_hooks::<T>()
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

    /// Register one or more systems into the schedule identified by `label`.
    ///
    /// Accepts a single system, a `.before()`/`.after()` ordering chain, or a
    /// tuple of either. Systems are ordinary functions whose parameters are
    /// system params (`Res<T>`, `ResMut<T>`, `Query<Q>`, `Local<T>`,
    /// `Commands`), or exclusive `FnMut(&mut World)` systems.
    ///
    /// ```ignore
    /// app.add_systems(Startup, setup_scene);
    /// app.add_systems(Update, (apply_gravity, integrate.after(&apply_gravity)));
    /// app.add_systems(Render, editor_render);
    /// ```
    pub fn add_systems<L: ScheduleLabel, M>(
        &mut self,
        _label: L,
        systems: impl IntoSystemConfigs<M>,
    ) -> &mut Self {
        self.schedules
            .entry(TypeId::of::<L>())
            .or_default()
            .add_systems(systems);
        self
    }

    /// Run the schedule identified by `label` once, if it exists.
    pub fn run_schedule<L: ScheduleLabel>(&mut self, _label: L) {
        if let Some(schedule) = self.schedules.get_mut(&TypeId::of::<L>()) {
            schedule.run(&mut self.world);
        }
    }

    /// Whether the schedule identified by `L` has no systems (or does not
    /// exist).
    fn schedule_is_empty<L: ScheduleLabel>(&self) -> bool {
        self.schedules
            .get(&TypeId::of::<L>())
            .is_none_or(Schedule::is_empty)
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

    /// Run the [`Startup`] schedule once.
    pub fn startup(&mut self) {
        moonfield_base::initialize();
        self.initialized = true;
        self.run_schedule(Startup);
    }

    /// Run a single update tick: the [`Update`] schedule once. Returns `false`
    /// if an [`AppExit`] resource was inserted (e.g. via
    /// `Commands::insert_resource`), signaling the loop should end.
    ///
    /// This is the per-frame counterpart of [`run_updates`]; it runs startup
    /// once on the first call.
    pub fn update(&mut self) -> bool {
        if !self.initialized {
            self.startup();
        }
        self.run_schedule(Update);
        !self.world.contains_resource::<AppExit>()
    }

    /// Run one render tick: the [`Render`] schedule once. Called by the
    /// windowing backend after [`App::update`] each frame, mirroring Bevy's
    /// render schedule. Startup runs lazily on the first call so a backend
    /// that drives `render` without `update` still initializes.
    pub fn render(&mut self) {
        if !self.initialized {
            self.startup();
        }
        self.run_schedule(Render);
    }

    /// Run the update loop until exit is requested via [`AppExit`] or the
    /// [`Update`] schedule is empty.
    pub fn run_updates(&mut self) {
        loop {
            if !self.update() {
                break;
            }
            if self.schedule_is_empty::<Update>() {
                break;
            }
        }
    }

    /// Run the [`Shutdown`] schedule once.
    pub fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }
        self.run_schedule(Shutdown);
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
