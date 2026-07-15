use crate::{App, AppError, Plugin};
use std::{
    any::TypeId,
    collections::{HashMap, VecDeque},
};

/// Combines multiple [`Plugin`]s into a single configurable unit.
///
/// # Example
///
/// ```
/// use moonfield_app::{App, Plugin, PluginGroup, PluginGroupBuilder};
///
/// #[derive(Default)]
/// struct A;
/// impl Plugin for A { fn build(&self, _: &mut App) {} }
///
/// #[derive(Default)]
/// struct B;
/// impl Plugin for B { fn build(&self, _: &mut App) {} }
///
/// struct MyPlugins;
/// impl PluginGroup for MyPlugins {
///     fn build(self) -> PluginGroupBuilder {
///         PluginGroupBuilder::start::<Self>()
///             .add(A)
///             .add(B)
///     }
/// }
///
/// App::new().add_plugins(MyPlugins).run();
/// ```
pub trait PluginGroup: Sized {
    /// Configures the plugins to be added.
    fn build(self) -> PluginGroupBuilder;

    /// Sets the value of the given [`Plugin`], if it exists in the group.
    fn set<T: Plugin>(self, plugin: T) -> PluginGroupBuilder {
        self.build().set(plugin)
    }

    /// Disables the given [`Plugin`], if it exists in the group.
    fn disable<T: Plugin>(self) -> PluginGroupBuilder {
        self.build().disable::<T>()
    }
}

impl PluginGroup for PluginGroupBuilder {
    fn build(self) -> PluginGroupBuilder {
        self
    }
}

struct PluginEntry {
    plugin: Box<dyn Plugin>,
    enabled: bool,
}

/// Facilitates the creation and configuration of a [`PluginGroup`].
pub struct PluginGroupBuilder {
    plugins: HashMap<TypeId, PluginEntry>,
    order: VecDeque<TypeId>,
}

impl PluginGroupBuilder {
    /// Starts a new builder for the given [`PluginGroup`].
    pub fn start<PG: PluginGroup>() -> Self {
        Self {
            plugins: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Returns whether the builder contains the given plugin type.
    pub fn contains<T: Plugin>(&self) -> bool {
        self.plugins.contains_key(&TypeId::of::<T>())
    }

    /// Adds a plugin to the end of the group.
    ///
    /// If the plugin was already present, it is moved to the end.
    pub fn add<T: Plugin>(mut self, plugin: T) -> Self {
        let id = TypeId::of::<T>();
        self.order.retain(|&existing| existing != id);
        self.order.push_back(id);
        self.plugins.insert(
            id,
            PluginEntry {
                plugin: Box::new(plugin),
                enabled: true,
            },
        );
        self
    }

    /// Replaces an existing plugin of the same type.
    ///
    /// # Panics
    ///
    /// Panics if the plugin type is not already in the group.
    pub fn set<T: Plugin>(mut self, plugin: T) -> Self {
        let id = TypeId::of::<T>();
        let entry = self.plugins.get_mut(&id).unwrap_or_else(|| {
            panic!(
                "Plugin {} does not exist in this PluginGroup",
                std::any::type_name::<T>()
            )
        });
        entry.plugin = Box::new(plugin);
        self
    }

    /// Disables a plugin so it is not added to the app.
    ///
    /// # Panics
    ///
    /// Panics if the plugin type is not already in the group.
    pub fn disable<T: Plugin>(mut self) -> Self {
        let id = TypeId::of::<T>();
        let entry = self
            .plugins
            .get_mut(&id)
            .unwrap_or_else(|| panic!("Cannot disable a plugin that does not exist"));
        entry.enabled = false;
        self
    }

    /// Enables a previously disabled plugin.
    ///
    /// # Panics
    ///
    /// Panics if the plugin type is not already in the group.
    pub fn enable<T: Plugin>(mut self) -> Self {
        let id = TypeId::of::<T>();
        let entry = self
            .plugins
            .get_mut(&id)
            .unwrap_or_else(|| panic!("Cannot enable a plugin that does not exist"));
        entry.enabled = true;
        self
    }

    /// Consumes the builder and adds all enabled plugins to the app in order.
    pub fn finish(self, app: &mut App) {
        let mut plugins = self.plugins;
        for id in &self.order {
            if let Some(entry) = plugins.remove(id) {
                if entry.enabled {
                    if let Err(AppError::DuplicatePlugin { plugin_name }) =
                        app.add_boxed_plugin(entry.plugin)
                    {
                        panic!(
                            "Error adding plugin {plugin_name} in group: plugin was already added in application"
                        );
                    }
                }
            }
        }
    }
}
