//! Bevy-style plugin/application core.
//!
//! Provides a minimal `App` with `Plugin` registration, resources, and
//! lifecycle callbacks. This is intentionally lightweight and does not
//! depend on the full Bevy ecosystem.

use moonfield_base::{error, info};
use std::any::{Any, TypeId};
use std::collections::HashMap;

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

/// A plugin that can be registered with an [`App`].
///
/// Mirrors Bevy's `Plugin` trait: `build` is called once when the plugin is
/// added to the application.
pub trait Plugin: 'static {
    /// Human-readable plugin name.
    fn name(&self) -> &str;

    /// Build the plugin by configuring the application.
    fn build(&self, app: &mut App);
}

/// Application host with plugin lifecycle.
pub struct App {
    resources: Resources,
    plugins: Vec<Box<dyn Plugin>>,
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
    /// Create a new, empty application.
    pub fn new() -> Self {
        Self {
            resources: Resources::default(),
            plugins: Vec::new(),
            startup_fns: Vec::new(),
            shutdown_fns: Vec::new(),
            update_fns: Vec::new(),
            initialized: false,
        }
    }

    /// Register a plugin.
    pub fn add_plugin<P: Plugin>(&mut self, plugin: P) -> &mut Self {
        let name = plugin.name().to_string();
        let boxed = Box::new(plugin);
        boxed.build(self);
        info!("Registered plugin: {}", name);
        self.plugins.push(boxed);
        self
    }

    /// Insert a resource.
    pub fn insert_resource<R: 'static>(&mut self, resource: R) -> &mut Self {
        self.resources.insert(resource);
        self
    }

    /// Access resources immutably.
    pub fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Access resources mutably.
    pub fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
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
        info!("Application startup");
        // Run in reverse insertion order is not required; drain in order.
        for f in self.startup_fns.drain(..) {
            f(&mut self.resources);
        }
    }

    /// Run the update loop until a system returns `false` or no systems remain.
    pub fn run_updates(&mut self) {
        if !self.initialized {
            self.startup();
        }
        'outer: loop {
            for f in &mut self.update_fns {
                if !f(&mut self.resources) {
                    break 'outer;
                }
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
        info!("Application shutdown");
        for f in self.shutdown_fns.drain(..) {
            f(&mut self.resources);
        }
        moonfield_base::shutdown();
        self.initialized = false;
    }

    /// Run the full lifecycle and return the provided exit code.
    pub fn run<F>(&mut self, user_loop: F) -> i32
    where
        F: FnOnce(&mut App) -> i32 + 'static,
    {
        self.startup();
        let result = user_loop(self);
        self.shutdown();
        result
    }
}

impl Drop for App {
    fn drop(&mut self) {
        if self.initialized {
            error!("App dropped without explicit shutdown; running cleanup");
            self.shutdown();
        }
    }
}
