//! [`SceneRegistry`]: the stable-name component registry behind the scene
//! file format.
//!
//! Every component that may appear in a scene document is registered under a
//! stable short name (`"transform"`, `"mesh_renderer"`, `"splat_cloud"`, … —
//! never a Rust type path, so renames do not break files). Two kinds of
//! entries exist:
//!
//! - **glTF-native entries** ([`register_native_transform`](SceneRegistry::register_native_transform),
//!   [`register_native_camera`](SceneRegistry::register_native_camera),
//!   [`register_native_hierarchy`](SceneRegistry::register_native_hierarchy)):
//!   [`save_scene`](crate::save_scene)/[`load_scene`](crate::load_scene)
//!   read and write node fields directly (TRS, the `cameras` array, the node
//!   tree). They exist as registry entries so that a world can opt out of a
//!   mapping entirely.
//! - **extras-channel entries** ([`register`](SceneRegistry::register),
//!   [`register_custom`](SceneRegistry::register_custom)): the component is
//!   serialized into `node.extras.components.<name>` via serde. The generic
//!   form covers plain `Clone + Serialize + DeserializeOwned` components;
//!   the custom form covers special cases like `Name` (a bare string, routed
//!   to `node.name` when registered under [`NAME`]) and path-backed handle
//!   components (save resolves the handle to its source path, load builds a
//!   [`HandleTemplate::Path`](crate::HandleTemplate::Path)).

use std::collections::HashMap;

use moonfield_ecs::{Component, Entity, World};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::{SceneError, SceneTemplate};

/// Registry key of the `Transform` ⇄ node TRS mapping.
pub const TRANSFORM: &str = "transform";
/// Registry key of the `Camera` ⇄ glTF camera mapping.
pub const CAMERA: &str = "camera";
/// Registry key of the hierarchy ⇄ node tree mapping.
pub const HIERARCHY: &str = "hierarchy";
/// Registry key routed to the node's native `name` field instead of
/// `extras.components`. Register `moonfield_ecs::Name` here via
/// [`SceneRegistry::register_custom`] with string-valued save/load hooks.
pub const NAME: &str = "name";

/// Save hook of an extras-channel entry: the component's JSON value, or
/// `None` when the entity does not carry it.
pub type SaveFn = fn(&World, Entity) -> Option<serde_json::Value>;

/// Load hook of an extras-channel entry: a template built from the JSON
/// value stored under `extras.components.<name>`.
pub type LoadFn = fn(&serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError>;

/// What a registry entry maps to in the glTF document.
pub(crate) enum EntryKind {
    /// `Transform` ⇄ the node's `translation`/`rotation`/`scale` fields.
    NativeTransform,
    /// `Camera` ⇄ the root `cameras` array + `node.camera`.
    NativeCamera,
    /// `ChildOf`/`Children` ⇄ the node tree.
    NativeHierarchy,
    /// A component serialized into `node.extras.components`.
    Extras { save: SaveFn, load: LoadFn },
}

fn save_generic<T: Component + Serialize>(
    world: &World,
    entity: Entity,
) -> Option<serde_json::Value> {
    let component = world.get_component::<T>(entity)?;
    serde_json::to_value(component).ok()
}

fn load_generic<T: Component + Clone + DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<Box<dyn SceneTemplate>, SceneError> {
    Ok(Box::new(serde_json::from_value::<T>(value.clone())?))
}

/// Maps stable component names to their glTF mappings. Stored as a world
/// resource; one registry is shared by [`save_scene`](crate::save_scene) and
/// [`load_scene`](crate::load_scene).
#[derive(Default)]
pub struct SceneRegistry {
    entries: HashMap<String, EntryKind>,
}

impl SceneRegistry {
    /// An empty registry; nothing is saved until entries are registered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register the `Transform` ⇄ node TRS mapping under [`TRANSFORM`].
    pub fn register_native_transform(&mut self) {
        self.entries
            .insert(TRANSFORM.to_string(), EntryKind::NativeTransform);
    }

    /// Register the `Camera` ⇄ glTF perspective camera mapping under
    /// [`CAMERA`]. `Camera::clear_color` has no glTF counterpart and rides in
    /// the node's `extras.camera` object.
    pub fn register_native_camera(&mut self) {
        self.entries
            .insert(CAMERA.to_string(), EntryKind::NativeCamera);
    }

    /// Register the hierarchy mapping under [`HIERARCHY`]: children recurse
    /// into `node.children` on save and are linked with `ChildOf` on load.
    /// Without it, every node is saved/loaded as an unlinked root.
    pub fn register_native_hierarchy(&mut self) {
        self.entries
            .insert(HIERARCHY.to_string(), EntryKind::NativeHierarchy);
    }

    /// Register a plain-data component under `name`. The component value is
    /// its own template (the blanket `Template` impl clones it), so the
    /// extras channel is a plain serde roundtrip.
    pub fn register<T>(&mut self, name: &str)
    where
        T: Component + Clone + Serialize + DeserializeOwned,
    {
        self.register_custom(name, save_generic::<T>, load_generic::<T>);
    }

    /// Register custom save/load hooks under `name`, for components whose
    /// file form is not their serde form: `Name` (a bare string, routed to
    /// `node.name` when registered under [`NAME`]) and path-backed handle
    /// components.
    pub fn register_custom(&mut self, name: &str, save: SaveFn, load: LoadFn) {
        self.entries
            .insert(name.to_string(), EntryKind::Extras { save, load });
    }

    /// Whether any entry is registered under `name`.
    pub fn contains(&self, name: &str) -> bool {
        self.entries.contains_key(name)
    }

    pub(crate) fn kind(&self, name: &str) -> Option<&EntryKind> {
        self.entries.get(name)
    }

    /// The extras-channel entries, sorted by name for deterministic output.
    pub(crate) fn extras_entries(&self) -> Vec<(&str, SaveFn)> {
        let mut entries: Vec<(&str, SaveFn)> = self
            .entries
            .iter()
            .filter_map(|(name, kind)| match kind {
                EntryKind::Extras { save, .. } => Some((name.as_str(), *save)),
                _ => None,
            })
            .collect();
        entries.sort_unstable_by(|a, b| a.0.cmp(b.0));
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
    struct Health {
        points: u32,
    }

    #[test]
    fn test_generic_entry_roundtrips_component_value() {
        let mut registry = SceneRegistry::new();
        registry.register::<Health>("health");
        assert!(registry.contains("health"));

        let mut world = World::new();
        let entity = world.spawn((Health { points: 3 },));
        let save = registry.extras_entries()[0].1;
        let value = save(&world, entity).unwrap();
        assert_eq!(value, serde_json::json!({ "points": 3 }));

        let EntryKind::Extras { load, .. } = registry.kind("health").unwrap() else {
            panic!("health entry must be an extras entry");
        };
        let template = load(&value).unwrap();
        template.insert_into_world(&mut world, entity).unwrap();
        assert_eq!(world.get_component::<Health>(entity).unwrap().points, 3);

        // Entities without the component save nothing.
        let other = world.spawn(());
        assert!(save(&world, other).is_none());
    }

    #[test]
    fn test_native_entries_are_distinguished_from_extras() {
        let mut registry = SceneRegistry::new();
        registry.register_native_transform();
        registry.register_native_camera();
        registry.register_native_hierarchy();
        assert!(matches!(
            registry.kind(TRANSFORM),
            Some(EntryKind::NativeTransform)
        ));
        assert!(matches!(
            registry.kind(CAMERA),
            Some(EntryKind::NativeCamera)
        ));
        assert!(matches!(
            registry.kind(HIERARCHY),
            Some(EntryKind::NativeHierarchy)
        ));
        assert!(registry.extras_entries().is_empty());
    }
}
