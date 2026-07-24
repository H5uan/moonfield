//! Minimal App/Plugin framework for Moonfield, inspired by Bevy's `bevy_app`.
//!
//! This crate provides a lightweight plugin system without ECS:
//! - [`Plugin`] trait and function-pointer plugin support.
//! - [`App`] container for registering plugins and running the application.
//! - [`PluginGroup`] for bundling and configuring plugins (set/disable).

#![forbid(unsafe_code)]

mod app;
mod plugin;
mod plugin_group;

pub use app::{App, AppError, Plugins, Runner};
pub use moonfield_ecs::Resource;
pub use plugin::Plugin;
pub use plugin_group::{PluginGroup, PluginGroupBuilder};

/// Common imports.
pub mod prelude {
    pub use crate::{App, Plugin, PluginGroup, PluginGroupBuilder, Resource};
    pub use moonfield_ecs::prelude::{
        Commands, Component, Entity, IntoSystem, Query, System, World,
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
        app.add_render_system(|world: &mut World| {
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
        app.add_render_system(|world: &mut World| {
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
}
