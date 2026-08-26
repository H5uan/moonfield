//! Minimal App/Plugin framework for Moonfield, inspired by Bevy's `bevy_app`.
//!
//! This crate provides a lightweight plugin system without ECS:
//! - [`Plugin`] trait and function-pointer plugin support.
//! - [`App`] container for registering plugins and running the application.
//! - [`PluginGroup`] for bundling and configuring plugins (set/disable).

#![forbid(unsafe_code)]

mod app;
mod hierarchy;
mod plugin;
mod plugin_group;
mod time;

pub use app::{
    App, AppError, AppExit, First, FixedFirst, FixedLast, FixedMain, FixedPostUpdate,
    FixedPreUpdate, FixedUpdate, Plugins, PreRender, Render, RenderPrepare, RenderQueue, Runner,
    Shutdown, Startup, Update,
};
pub use hierarchy::HierarchyPlugin;
pub use moonfield_ecs::Resource;
pub use plugin::Plugin;
pub use plugin_group::{PluginGroup, PluginGroupBuilder};
pub use time::TimePlugin;

/// Common imports.
pub mod prelude {
    pub use crate::{
        App, AppExit, First, FixedFirst, FixedLast, FixedMain, FixedPostUpdate, FixedPreUpdate,
        FixedUpdate, HierarchyPlugin, Plugin, PluginGroup, PluginGroupBuilder, PreRender, Render,
        RenderPrepare, RenderQueue, Resource, Shutdown, Startup, TimePlugin, Update,
    };
    pub use moonfield_ecs::prelude::{
        ChildOf, Children, Commands, Component, Entity, EntityCommands, IntoSystem,
        IntoSystemConfigs, Local, Name, Query, Relationship, RelationshipTarget, Res, ResMut,
        Schedule, ScheduleLabel, System, World, WorldQuery,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::World;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct A;
    impl Plugin for A {
        fn build(&self, app: &mut App) {
            app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("A::build".to_string());
        }
        fn finish(&self, app: &mut App) {
            app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("A::finish".to_string());
        }
        fn cleanup(&self, app: &mut App) {
            app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("A::cleanup".to_string());
        }
    }

    struct B {
        name: &'static str,
    }
    impl Default for B {
        fn default() -> Self {
            Self { name: "B" }
        }
    }
    impl Plugin for B {
        fn build(&self, app: &mut App) {
            app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push(format!("{}::build", self.name));
        }
    }

    #[derive(Default)]
    struct C;
    impl Plugin for C {
        fn build(&self, app: &mut App) {
            app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("C::build".to_string());
        }
    }

    fn log_event(name: &str, app: &mut App) {
        app.get_resource_mut::<Arc<Mutex<Vec<String>>>>()
            .unwrap()
            .lock()
            .unwrap()
            .push(name.to_string());
    }

    fn fn_plugin(app: &mut App) {
        log_event("fn_plugin::build", app);
    }

    struct MyGroup;
    impl PluginGroup for MyGroup {
        fn build(self) -> PluginGroupBuilder {
            PluginGroupBuilder::start::<Self>()
                .add(A)
                .add(B::default())
                .add(C)
        }
    }

    fn make_app() -> (App, Arc<Mutex<Vec<String>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut app = App::new();
        app.insert_resource(events.clone());
        (app, events)
    }

    #[test]
    fn single_plugin_is_built() {
        let (mut app, events) = make_app();
        app.add_plugins(A);
        assert_eq!(events.lock().unwrap().as_slice(), &["A::build".to_string()]);
    }

    #[test]
    fn function_pointer_plugin_is_built() {
        let (mut app, events) = make_app();
        app.add_plugins(fn_plugin);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["fn_plugin::build".to_string()]
        );
    }

    #[test]
    fn tuple_plugins_are_built_in_order() {
        let (mut app, events) = make_app();
        app.add_plugins((A, B::default(), C));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build", "B::build", "C::build"]
        );
    }

    #[test]
    fn plugin_group_adds_all_plugins() {
        let (mut app, events) = make_app();
        app.add_plugins(MyGroup);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build", "B::build", "C::build"]
        );
    }

    #[test]
    fn plugin_group_disable_prevents_adding() {
        let (mut app, events) = make_app();
        app.add_plugins(MyGroup.disable::<B>());
        assert_eq!(events.lock().unwrap().as_slice(), &["A::build", "C::build"]);
    }

    #[test]
    fn plugin_group_set_replaces_plugin() {
        let (mut app, events) = make_app();
        app.add_plugins(MyGroup.set(B { name: "B2" }));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build", "B2::build", "C::build"]
        );
    }

    #[test]
    fn duplicate_unique_plugin_is_rejected() {
        let (mut app, _events) = make_app();
        app.add_plugins(A);
        let err = app.add_boxed_plugin(Box::new(A));
        assert_eq!(
            err,
            Err(AppError::DuplicatePlugin {
                plugin_name: "moonfield_app::tests::A".to_string()
            })
        );
    }

    #[test]
    #[should_panic(expected = "plugin was already added in application")]
    fn duplicate_unique_plugin_panics_via_add_plugins() {
        let (mut app, _events) = make_app();
        app.add_plugins((A, A));
    }

    #[test]
    fn run_invokes_finish_run_updates_and_cleanup() {
        let (mut app, events) = make_app();
        app.add_plugins(A);
        // A has no update systems, so run_updates completes immediately.
        app.run();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build", "A::finish", "A::cleanup"]
        );
    }

    #[test]
    fn render_systems_run_after_update() {
        let (mut app, events) = make_app();
        app.add_plugins(A);
        app.render_world_mut().insert_resource(events.clone());
        app.add_render_systems(Render, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("render".to_string());
        });

        app.update();
        app.render();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build".to_string(), "render".to_string()]
        );
    }

    #[test]
    fn render_initializes_lazily_without_update() {
        let (mut app, events) = make_app();
        app.add_plugins(A);
        app.render_world_mut().insert_resource(events.clone());
        app.add_render_systems(Render, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("render".to_string());
        });

        // No update() call — render() must still trigger startup.
        app.render();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &["A::build".to_string(), "render".to_string()]
        );
    }

    #[test]
    fn test_pre_render_reads_main_world_and_render_reads_render_world() {
        struct MainMarker;
        struct RenderMarker;

        let mut app = App::new();
        app.insert_resource(MainMarker);
        app.render_world_mut().insert_resource(RenderMarker);
        app.add_systems(PreRender, |world: &mut World| {
            assert!(world.contains_resource::<MainMarker>());
            assert!(!world.contains_resource::<RenderMarker>());
        });
        app.add_render_systems(Render, |world: &mut World| {
            assert!(world.contains_resource::<RenderMarker>());
            assert!(!world.contains_resource::<MainMarker>());
        });

        app.render();
    }

    #[test]
    fn test_render_frame_runs_all_stages_in_order() {
        let events = Arc::new(Mutex::new(Vec::<String>::new()));
        let mut app = App::new();
        app.insert_resource(events.clone());
        app.render_world_mut().insert_resource(events.clone());
        app.add_systems(PreRender, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("pre_render".to_string());
        });
        app.add_extract_system(|main_world, render_world| {
            let events = main_world
                .get_resource::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .clone();
            events.lock().unwrap().push("extract".to_string());
            assert!(render_world.contains_resource::<Arc<Mutex<Vec<String>>>>());
        });
        app.add_render_systems(RenderPrepare, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("render_prepare".to_string());
        });
        app.add_render_systems(RenderQueue, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("render_queue".to_string());
        });
        app.add_render_systems(Render, |world: &mut World| {
            world
                .get_resource_mut::<Arc<Mutex<Vec<String>>>>()
                .unwrap()
                .lock()
                .unwrap()
                .push("render".to_string());
        });

        app.render();

        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                "pre_render",
                "extract",
                "render_prepare",
                "render_queue",
                "render"
            ]
        );
    }

    #[test]
    fn update_loop_exits_on_app_exit_resource() {
        use crate::{AppExit, Update};
        use moonfield_ecs::{Commands, ResMut};

        struct Frames(usize);

        fn count_frame(mut frames: ResMut<Frames>, commands: Commands) {
            frames.0 += 1;
            if frames.0 == 3 {
                commands.insert_resource(AppExit);
            }
        }

        let mut app = App::new();
        app.insert_resource(Frames(0));
        app.add_systems(Update, count_frame);
        app.run_updates();

        // Three update ticks ran; the AppExit queued in the third ended the loop.
        assert_eq!(app.world().get_resource::<Frames>().unwrap().0, 3);
    }

    #[test]
    fn non_unique_plugin_can_be_added_twice() {
        #[derive(Default)]
        struct D;
        impl Plugin for D {
            fn is_unique(&self) -> bool {
                false
            }
            fn build(&self, app: &mut App) {
                log_event("D::build", app);
            }
        }

        let (mut app, events) = make_app();
        app.add_plugins((D, D));
        assert_eq!(events.lock().unwrap().as_slice(), &["D::build", "D::build"]);
    }

    #[test]
    fn add_message_enables_reader_writer_params_with_frame_retention() {
        use moonfield_ecs::{IntoSystemConfigs, MessageReader, MessageWriter, ResMut};

        struct Ping(u32);

        #[derive(Default)]
        struct Outbox(u32);
        #[derive(Default)]
        struct Seen(Vec<u32>);

        fn write_ping(mut outbox: ResMut<Outbox>, mut writer: MessageWriter<Ping>) {
            outbox.0 += 1;
            writer.write(Ping(outbox.0));
        }
        fn read_pings(mut reader: MessageReader<Ping>, mut seen: ResMut<Seen>) {
            for ping in reader.read() {
                seen.0.push(ping.0);
            }
        }

        let mut app = App::new();
        app.insert_resource(Outbox::default());
        app.insert_resource(Seen::default());
        app.add_message::<Ping>();
        app.add_systems(Update, (write_ping, read_pings.after(&write_ping)));

        app.update();
        app.update();
        // Each written ping was consumed by the reader exactly once.
        assert_eq!(app.world().get_resource::<Seen>().unwrap().0, vec![1, 2]);

        app.update();
        assert_eq!(app.world().get_resource::<Seen>().unwrap().0, vec![1, 2, 3]);
    }

    #[test]
    fn fixed_update_runs_zero_one_many_times_per_frame() {
        use moonfield_ecs::{Res, ResMut};
        use moonfield_time::{Fixed, Time, Virtual};
        use std::time::Duration;

        #[derive(Default)]
        struct FixedRuns(u32);
        #[derive(Default)]
        struct FixedDeltas(Vec<(Duration, Duration)>);
        #[derive(Default)]
        struct UmbrellaRuns(u32);

        fn count_fixed(
            mut runs: ResMut<FixedRuns>,
            mut deltas: ResMut<FixedDeltas>,
            time: Res<Time>,
        ) {
            runs.0 += 1;
            deltas.0.push((time.delta(), time.elapsed()));
        }
        fn count_umbrella(mut runs: ResMut<UmbrellaRuns>) {
            runs.0 += 1;
        }

        let mut app = App::new();
        app.add_plugin(TimePlugin);
        app.insert_resource(FixedRuns::default());
        app.insert_resource(FixedDeltas::default());
        app.insert_resource(UmbrellaRuns::default());
        app.add_systems(FixedUpdate, count_fixed);
        // Systems registered directly under the FixedMain umbrella run inside
        // every iteration too.
        app.add_systems(FixedMain, count_umbrella);
        app.world_mut()
            .get_resource_mut::<Time<Fixed>>()
            .unwrap()
            .set_timestep_hz(2.0); // 500 ms steps

        let advance = |app: &mut App, ms: u64| {
            app.world_mut()
                .get_resource_mut::<Time<Virtual>>()
                .unwrap()
                .advance_by(Duration::from_millis(ms));
        };

        // 400 ms of virtual time: no full step.
        advance(&mut app, 400);
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 0);
        assert_eq!(app.world().get_resource::<UmbrellaRuns>().unwrap().0, 0);

        // +200 ms → 600 ms accumulated: exactly one step.
        advance(&mut app, 200);
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 1);
        assert_eq!(app.world().get_resource::<UmbrellaRuns>().unwrap().0, 1);
        // During the fixed run the generic Time was the fixed clock.
        assert_eq!(
            app.world().get_resource::<FixedDeltas>().unwrap().0,
            vec![(Duration::from_millis(500), Duration::from_millis(500))]
        );
        // …and afterwards it is virtual time again.
        assert_eq!(
            app.world().get_resource::<Time>().unwrap().delta(),
            Duration::from_millis(200)
        );

        // +1.1 s → two more steps; 200 ms stays in the overstep accumulator.
        advance(&mut app, 1100);
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 3);
        assert_eq!(
            app.world()
                .get_resource::<Time<Fixed>>()
                .unwrap()
                .overstep(),
            Duration::from_millis(200)
        );
        // Fixed elapsed only ever advances by whole timesteps.
        assert_eq!(
            app.world().get_resource::<Time<Fixed>>().unwrap().elapsed(),
            Duration::from_millis(1500)
        );
    }

    #[test]
    fn fixed_update_respects_virtual_pause() {
        use moonfield_ecs::ResMut;
        use moonfield_time::{Time, Virtual};
        use std::time::Duration;

        #[derive(Default)]
        struct FixedRuns(u32);
        fn count_fixed(mut runs: ResMut<FixedRuns>) {
            runs.0 += 1;
        }

        let mut app = App::new();
        app.add_plugin(TimePlugin);
        app.insert_resource(FixedRuns::default());
        app.add_systems(FixedUpdate, count_fixed);

        app.world_mut()
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .advance_by(Duration::from_secs(1));
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 64);

        // Paused virtual time: no delta, no fixed steps.
        app.world_mut()
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .pause();
        app.world_mut()
            .get_resource_mut::<Time<Virtual>>()
            .unwrap()
            .advance_by(Duration::ZERO);
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 64);
    }

    #[test]
    fn fixed_schedules_never_run_without_time_plugin() {
        use moonfield_ecs::ResMut;

        #[derive(Default)]
        struct FixedRuns(u32);
        fn count_fixed(mut runs: ResMut<FixedRuns>) {
            runs.0 += 1;
        }

        let mut app = App::new();
        app.insert_resource(FixedRuns::default());
        app.add_systems(FixedUpdate, count_fixed);
        app.update();
        assert_eq!(app.world().get_resource::<FixedRuns>().unwrap().0, 0);
    }
}
