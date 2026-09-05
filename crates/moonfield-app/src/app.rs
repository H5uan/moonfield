use crate::{Plugin, PluginGroup};
use moonfield_ecs::{IntoSystemConfigs, Schedule, ScheduleLabel, World};
use std::any::TypeId;
use std::collections::{HashMap, HashSet};
use std::process::ExitCode;

/// Schedule label for systems that run once at app startup.
pub struct Startup;
/// Schedule label for systems that run first in every update phase, before
/// [`Update`]. The message buffer swap ([`moonfield_ecs::message_update_system`])
/// runs here, wired by [`App::add_message`].
pub struct First;
/// Schedule label for the fixed-timestep umbrella: [`App::update`] runs the
/// fixed loop zero or more times per frame (accumulated
/// [`moonfield_time::Time<Fixed>`](moonfield_time::Fixed) overstep divided by
/// the timestep), each iteration running [`FixedFirst`], [`FixedPreUpdate`],
/// [`FixedUpdate`], [`FixedPostUpdate`], [`FixedLast`] in order. Systems
/// registered directly under `FixedMain` run after those, inside every
/// iteration.
pub struct FixedMain;
/// Schedule label run last in every update tick, after the [`Update`] and
/// render pipeline; frame-end bookkeeping (window diff application, input
/// clearing) lives here (Bevy's `Last`).
pub struct Last;
/// Schedule label run first in every fixed-timestep iteration.
pub struct FixedFirst;
/// Schedule label run before [`FixedUpdate`] in every fixed iteration.
pub struct FixedPreUpdate;
/// Schedule label for fixed-timestep systems (physics, simulation) — the one
/// most callers want.
pub struct FixedUpdate;
/// Schedule label run after [`FixedUpdate`] in every fixed iteration.
pub struct FixedPostUpdate;
/// Schedule label run last in every fixed-timestep iteration.
pub struct FixedLast;
/// Schedule label for systems that run every frame, during [`App::update`].
pub struct Update;
/// Schedule label for main-world systems that prepare a frame before
/// extraction into the render world.
pub struct PreRender;
/// Schedule label for render-world systems, run by [`App::render`] after
/// [`PreRender`] and extraction.
pub struct Render;
/// Schedule label for render-world systems that prepare persistent GPU data
/// from the extracted snapshot.
pub struct RenderPrepare;
/// Schedule label for render-world systems that build per-frame render work
/// from extracted and prepared data.
pub struct RenderQueue;
/// Schedule label for systems that run once at app shutdown.
pub struct Shutdown;

impl ScheduleLabel for Startup {}
impl ScheduleLabel for First {}
impl ScheduleLabel for FixedMain {}
impl ScheduleLabel for FixedFirst {}
impl ScheduleLabel for FixedPreUpdate {}
impl ScheduleLabel for FixedUpdate {}
impl ScheduleLabel for FixedPostUpdate {}
impl ScheduleLabel for FixedLast {}
impl ScheduleLabel for Last {}
impl ScheduleLabel for Update {}
impl ScheduleLabel for PreRender {}
impl ScheduleLabel for RenderPrepare {}
impl ScheduleLabel for RenderQueue {}
impl ScheduleLabel for Render {}
impl ScheduleLabel for Shutdown {}

/// Exit code for the application, mirroring Bevy's `AppExit`.
///
/// Also acts as the resource marking that the app should exit its update
/// loop: insert it (e.g. via `Commands::insert_resource`) to make
/// [`App::update`] return `false` and the runner return its code. Most
/// systems just insert [`AppExit::SUCCESS`]; a nonzero code is set for
/// abnormal exits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AppExit {
    /// The process exit code.
    pub code: ExitCode,
}

impl AppExit {
    /// The default, successful exit.
    pub const SUCCESS: Self = Self {
        code: ExitCode::SUCCESS,
    };
    /// The generic failure exit.
    pub const FAILURE: Self = Self {
        code: ExitCode::FAILURE,
    };

    /// An exit with the given exit code.
    pub fn from_code(code: u8) -> Self {
        Self {
            code: ExitCode::from(code),
        }
    }

    /// The generic failure exit, for error paths.
    pub fn error() -> Self {
        Self::FAILURE
    }
}

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
/// Systems live in labeled [`Schedule`]s. The app drives [`Startup`] once,
/// [`Update`] every update, [`PreRender`] in the main world followed by
/// [`RenderPrepare`], [`RenderQueue`], and [`Render`] in the render world every
/// render tick, and [`Shutdown`] once.
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
    render_world: World,
    extract_systems: Vec<ExtractFn>,
    schedules: HashMap<TypeId, Schedule>,
    render_schedules: HashMap<TypeId, Schedule>,
    runner: Option<Runner>,
    initialized: bool,
}

/// A handwritten extraction function: copies data out of the main world
/// (immutable) into the render world. Runs at the start of every
/// [`App::render`] call, right after the render world's entities have been
/// cleared — extraction is a full rebuild, so render-world entities are
/// never stable across frames.
type ExtractFn = Box<dyn FnMut(&World, &mut World) + Send + Sync>;

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
            render_world: World::new(),
            extract_systems: Vec::new(),
            schedules: HashMap::new(),
            render_schedules: HashMap::new(),
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

    /// Initialize [`Message`](moonfield_ecs::Message) handling for `M`:
    /// inserts the `Messages<M>` resource and registers its buffers for the
    /// once-per-frame swap, which runs in the [`First`] schedule (the swap
    /// system itself is added on the first `add_message` call).
    ///
    /// After this, systems can use `MessageReader<M>` / `MessageWriter<M>`
    /// params; messages live for two frames (see [`moonfield_ecs::Messages`]).
    pub fn add_message<M: moonfield_ecs::Message>(&mut self) -> &mut Self {
        if !self.world.contains_resource::<moonfield_ecs::Messages<M>>() {
            self.world
                .insert_resource(moonfield_ecs::Messages::<M>::default());
        }
        if !self
            .world
            .contains_resource::<moonfield_ecs::MessageRegistry>()
        {
            self.world
                .insert_resource(moonfield_ecs::MessageRegistry::default());
            self.add_systems(First, moonfield_ecs::message_update_system);
        }
        self.world
            .get_resource_mut::<moonfield_ecs::MessageRegistry>()
            .expect("MessageRegistry was just ensured")
            .register::<M>();
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

    /// Access the underlying render world immutably.
    pub fn render_world(&self) -> &World {
        &self.render_world
    }

    /// Access the underlying render world mutably.
    pub fn render_world_mut(&mut self) -> &mut World {
        &mut self.render_world
    }

    /// Registers a handwritten extraction function, run every frame at the
    /// start of [`App::render`] — after the render world's entities are
    /// cleared, before the [`Render`] schedule runs. The function copies
    /// data out of the main world (immutable) into the render world; it
    /// must not key cross-frame state by render-world [`moonfield_ecs::Entity`],
    /// which is rebuilt every frame.
    pub fn add_extract_system(
        &mut self,
        f: impl FnMut(&World, &mut World) + Send + Sync + 'static,
    ) -> &mut Self {
        self.extract_systems.push(Box::new(f));
        self
    }

    /// Register one or more systems into the schedule identified by `label`.
    /// Accepts a single system, a `.before()`/`.after()` ordering chain, or a
    /// tuple of either. Systems are ordinary functions whose parameters are
    /// system params (`Res<T>`, `ResMut<T>`, `Query<Q>`, `Local<T>`,
    /// `Commands`), or exclusive `FnMut(&mut World)` systems.
    ///
    /// ```ignore
    /// app.add_systems(Startup, setup_scene);
    /// app.add_systems(Update, (apply_gravity, integrate.after(&apply_gravity)));
    /// app.add_systems(PreRender, prepare_editor);
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

    /// Register one or more systems into a schedule that runs against the
    /// render world.
    ///
    /// ```ignore
    /// app.add_render_systems(Render, draw_scene);
    /// ```
    pub fn add_render_systems<L: ScheduleLabel, M>(
        &mut self,
        _label: L,
        systems: impl IntoSystemConfigs<M>,
    ) -> &mut Self {
        self.render_schedules
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

    /// Run a schedule against the render world once, if it exists.
    pub fn run_render_schedule<L: ScheduleLabel>(&mut self, _label: L) {
        if let Some(schedule) = self.render_schedules.get_mut(&TypeId::of::<L>()) {
            schedule.run(&mut self.render_world);
        }
    }

    /// Whether the schedule identified by `L` has no systems (or does not
    /// exist).
    fn schedule_is_empty<L: ScheduleLabel>(&self) -> bool {
        self.schedules
            .get(&TypeId::of::<L>())
            .is_none_or(Schedule::is_empty)
    }

    /// Set a custom runner function that replaces the default runner
    /// ([`run_once`], mirroring Bevy's `RunnerFn`).
    ///
    /// The runner receives `&mut App` and drives the application (typically
    /// via a winit event loop), returning the exit code. It is called once
    /// from [`App::run`] after all plugins have been finished; loop semantics
    /// are the runner's responsibility.
    ///
    /// # Example
    ///
    /// ```ignore
    /// app.set_runner(|app: &mut App| {
    ///     loop {
    ///         if !app.update() {
    ///             break;
    ///         }
    ///     }
    ///     AppExit::SUCCESS
    /// });
    /// ```
    pub fn set_runner(&mut self, runner: impl FnOnce(&mut App) -> AppExit + 'static) -> &mut Self {
        self.runner = Some(Box::new(runner));
        self
    }

    /// Take the runner, if set.
    pub fn take_runner(&mut self) -> Option<Runner> {
        self.runner.take()
    }

    /// Run the [`Startup`] schedule once.
    pub fn startup(&mut self) {
        self.initialized = true;
        self.run_schedule(Startup);
    }

    /// Run a single full tick: the [`First`] schedule, the fixed-timestep
    /// loop (zero or more [`FixedMain`] iterations), the [`Update`] schedule,
    /// the render pipeline ([`Self::render`]), then the [`Last`] schedule.
    /// Returns `false` if an [`AppExit`] resource was inserted (e.g. via
    /// `Commands::insert_resource`), signaling the loop should end.
    ///
    /// This is the per-frame counterpart of [`run_updates`]; it runs startup
    /// once on the first call. Rendering is part of the tick (Bevy's model),
    /// so a runner only needs to call this one method. The clocks advance in
    /// [`First`] via `moonfield_time::time_update_system` (registered by
    /// `TimePlugin`, driven by the `TimeUpdateStrategy` resource); tests drive
    /// deterministic time through that strategy.
    pub fn update(&mut self) -> bool {
        if !self.initialized {
            self.startup();
        }
        self.run_schedule(First);
        self.run_fixed_main_loop();
        self.run_schedule(Update);
        self.render();
        self.run_schedule(Last);
        !self.world.contains_resource::<AppExit>()
    }

    /// Accumulate the frame's virtual delta into `Time<Fixed>` and run the
    /// fixed schedules once per full timestep (see
    /// [`moonfield_time::run_fixed_main_schedule`]). No-op without the time
    /// resources (`TimePlugin`).
    fn run_fixed_main_loop(&mut self) {
        let world = &mut self.world;
        let schedules = &mut self.schedules;
        moonfield_time::run_fixed_main_schedule(world, |world| {
            for label in [
                TypeId::of::<FixedFirst>(),
                TypeId::of::<FixedPreUpdate>(),
                TypeId::of::<FixedUpdate>(),
                TypeId::of::<FixedPostUpdate>(),
                TypeId::of::<FixedLast>(),
                // Systems registered directly under the umbrella label run
                // last in every iteration.
                TypeId::of::<FixedMain>(),
            ] {
                if let Some(schedule) = schedules.get_mut(&label) {
                    schedule.run(world);
                }
            }
        });
    }

    /// Run one render tick: [`PreRender`] in the main world, extraction, then
    /// [`RenderPrepare`], [`RenderQueue`], and [`Render`] in the render world.
    /// Startup runs lazily on the first call so a backend that drives `render`
    /// without `update` still initializes.
    pub fn render(&mut self) {
        if !self.initialized {
            self.startup();
        }
        self.run_schedule(PreRender);
        self.render_world.clear();
        for extract in &mut self.extract_systems {
            extract(&self.world, &mut self.render_world);
        }
        self.run_render_schedule(RenderPrepare);
        self.run_render_schedule(RenderQueue);
        self.run_render_schedule(Render);
    }

    /// Run the update loop until exit is requested via [`AppExit`] or the
    /// [`Update`] schedule is empty. Headless apps can use this as their
    /// runner; windowed apps use `WinitPlugin`'s event-loop runner instead.
    /// Returns the final exit code.
    pub fn run_updates(&mut self) -> AppExit {
        loop {
            self.update();
            if self.world.contains_resource::<AppExit>() {
                break;
            }
            if self.schedule_is_empty::<Update>() {
                break;
            }
        }
        self.world()
            .get_resource::<AppExit>()
            .map(|exit| *exit)
            .unwrap_or(AppExit::SUCCESS)
    }

    /// Run the [`Shutdown`] schedule once.
    pub fn shutdown(&mut self) {
        if !self.initialized {
            return;
        }
        self.run_schedule(Shutdown);
        self.initialized = false;
    }

    /// Finishes all plugins, calls the runner (the default [`run_once`], or
    /// a custom runner set via [`set_runner`]), then cleans up all plugins.
    ///
    /// The runner receives `&mut App` and drives the application; `App` itself
    /// never owns a loop. Returns the runner's exit code.
    pub fn run(&mut self) -> AppExit {
        let plugins = std::mem::take(&mut self.plugins);

        for plugin in &plugins {
            plugin.finish(self);
        }

        // Always run through a runner: the default runs a single tick, a
        // plugin's runner owns the loop (e.g. winit's event loop).
        let runner = self.runner.take().unwrap_or_else(|| Box::new(run_once));
        let exit = runner(self);

        for plugin in &plugins {
            plugin.cleanup(self);
        }

        self.plugins = plugins;
        exit
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.initialized {
            self.shutdown();
        }
    }
}

/// The default runner: a single [`App::update`] tick, then the exit code
/// requested via the [`AppExit`] resource (mirrors Bevy's `run_once`).
///
/// Loops are the job of runner plugins: `WinitPlugin` replaces this with its
/// event-loop runner, and headless apps can use [`App::run_updates`] as their
/// runner.
pub fn run_once(app: &mut App) -> AppExit {
    app.update();
    app.world()
        .get_resource::<AppExit>()
        .map(|exit| *exit)
        .unwrap_or(AppExit::SUCCESS)
}

/// A runner function that drives the application (mirrors Bevy's `RunnerFn`).
///
/// A plugin that wants to control the app loop can set a runner via
/// [`App::set_runner`]. The runner is responsible for calling `app.update()`
/// each frame (rendering is part of the tick) and returning the exit code.
pub type Runner = Box<dyn FnOnce(&mut App) -> AppExit>;

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
