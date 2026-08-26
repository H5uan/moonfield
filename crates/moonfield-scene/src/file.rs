//! The `.gltf` scene document: glTF 2.0 JSON as the text carrier.
//!
//! Document shape:
//!
//! - `asset: { version: "2.0", generator: "moonfield" }`, one scene whose
//!   `nodes` are the savable roots;
//! - node `name` ⇄ `Name` (when registered under [`NAME`](crate::NAME)),
//!   `translation`/`rotation`/`scale` ⇄ `Transform` (glTF quaternion order
//!   `[x, y, z, w]` matches glam's `Quat::from_xyzw`; glTF is Y-up
//!   right-handed like the engine, so TRS values cross verbatim),
//!   `children` ⇄ the `ChildOf`/`Children` hierarchy,
//!   `camera` ⇄ an index into the root `cameras` array (perspective `yfov` /
//!   `znear`; `Camera::clear_color` has no glTF field and rides in
//!   `extras.camera.clear_color`);
//! - every other registered component lives under
//!   `node.extras.components.<name>`.
//!
//! Roots are entities carrying at least one registered component and no
//! `ChildOf`. Unregistered components are skipped; `GlobalTransform` is
//! never registered — the hierarchy propagation systems recompute it after
//! load. Nodes whose transform is given as a `matrix` (not TRS) get no
//! `Transform` on load; orthographic cameras are likewise skipped.

use std::collections::HashSet;
use std::path::Path;

use gltf_json::scene::UnitQuaternion;
use gltf_json::validation::Checked;
use gltf_json::{camera, Index, Node, Root, Scene};
use moonfield_camera::Camera;
use moonfield_ecs::{ChildOf, Children, Entity, TemplateError, World};
use moonfield_math::{Quat, Transform, Vec3};

use crate::registry::{EntryKind, SceneRegistry, CAMERA, HIERARCHY, NAME, TRANSFORM};
use crate::{ResolvedScene, SceneTemplate};

/// Errors produced while saving or loading a scene document.
#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    /// JSON (de)serialization failure, from `serde_json`/`gltf-json`.
    #[error("scene json error: {0}")]
    Json(#[from] serde_json::Error),
    /// A template failed to build while applying the scene.
    #[error("scene template error: {0}")]
    Template(#[from] TemplateError),
    /// File I/O failure in the `_from_file`/`_to_file` helpers.
    #[error("scene io error: {0}")]
    Io(#[from] std::io::Error),
    /// The document is structurally invalid as a moonfield scene.
    #[error("invalid scene document: {0}")]
    Invalid(String),
}

fn has_registered_component(world: &World, registry: &SceneRegistry, entity: Entity) -> bool {
    if matches!(registry.kind(TRANSFORM), Some(EntryKind::NativeTransform))
        && world.get_component::<Transform>(entity).is_some()
    {
        return true;
    }
    if matches!(registry.kind(CAMERA), Some(EntryKind::NativeCamera))
        && world.get_component::<Camera>(entity).is_some()
    {
        return true;
    }
    registry
        .extras_entries()
        .iter()
        .any(|(_, save)| save(world, entity).is_some())
}

fn save_node(
    world: &World,
    registry: &SceneRegistry,
    root: &mut Root,
    entity: Entity,
    hierarchy: bool,
) -> Result<Index<Node>, SceneError> {
    let mut node = Node::default();
    let mut components = serde_json::Map::new();
    let mut camera_extras = None;

    if matches!(registry.kind(TRANSFORM), Some(EntryKind::NativeTransform)) {
        if let Some(transform) = world.get_component::<Transform>(entity) {
            node.translation = Some(transform.translation.to_array());
            let rotation = transform.rotation;
            node.rotation = Some(UnitQuaternion([
                rotation.x, rotation.y, rotation.z, rotation.w,
            ]));
            node.scale = Some(transform.scale.to_array());
        }
    }

    if matches!(registry.kind(CAMERA), Some(EntryKind::NativeCamera)) {
        if let Some(engine_camera) = world.get_component::<Camera>(entity) {
            let camera = camera::Camera {
                name: None,
                orthographic: None,
                perspective: Some(camera::Perspective {
                    aspect_ratio: None,
                    yfov: engine_camera.fov_y_radians,
                    zfar: None,
                    znear: engine_camera.near,
                    extensions: None,
                    extras: None,
                }),
                type_: Checked::Valid(camera::Type::Perspective),
                extensions: None,
                extras: None,
            };
            node.camera = Some(root.push(camera));
            camera_extras = Some(serde_json::json!({ "clear_color": engine_camera.clear_color }));
        }
    }

    for (name, save) in registry.extras_entries() {
        let Some(value) = save(world, entity) else {
            continue;
        };
        if name == NAME {
            // `Name` rides the node's native field, not the extras map.
            let Some(text) = value.as_str() else {
                return Err(SceneError::Invalid(format!(
                    "the '{NAME}' entry's save hook must produce a string"
                )));
            };
            node.name = Some(text.to_string());
        } else {
            components.insert(name.to_string(), value);
        }
    }

    let mut extras = serde_json::Map::new();
    if !components.is_empty() {
        extras.insert(
            "components".to_string(),
            serde_json::Value::Object(components),
        );
    }
    if let Some(camera_extras) = camera_extras {
        extras.insert("camera".to_string(), camera_extras);
    }
    if !extras.is_empty() {
        node.extras = Some(serde_json::value::to_raw_value(
            &serde_json::Value::Object(extras),
        )?);
    }

    if hierarchy {
        if let Some(children) = world.get_component::<Children>(entity) {
            let mut indices = Vec::new();
            for &child in children.iter() {
                // Children without any registered component carry no data;
                // they (and their subtrees) stay out of the document.
                if has_registered_component(world, registry, child) {
                    indices.push(save_node(world, registry, root, child, hierarchy)?);
                }
            }
            if !indices.is_empty() {
                node.children = Some(indices);
            }
        }
    }

    Ok(root.push(node))
}

/// Serialize the world's registered entities into a glTF document.
///
/// Roots are entities with at least one registered component and no
/// `ChildOf` (when the hierarchy mapping is unregistered, `ChildOf` is
/// ignored and every such entity becomes a flat root). Entities without any
/// registered component, and components without a registry entry, are
/// skipped.
pub fn save_scene(world: &World, registry: &SceneRegistry) -> Result<Root, SceneError> {
    let mut root = Root::default();
    root.asset.generator = Some("moonfield".to_string());

    let hierarchy = registry.contains(HIERARCHY);
    let mut scene = Scene {
        extensions: None,
        extras: None,
        name: None,
        nodes: Vec::new(),
    };
    for entity in world.iter_entities() {
        if !has_registered_component(world, registry, entity) {
            continue;
        }
        if hierarchy && world.get_component::<ChildOf>(entity).is_some() {
            continue; // reached through its parent's children
        }
        scene
            .nodes
            .push(save_node(world, registry, &mut root, entity, hierarchy)?);
    }
    root.scenes.push(scene);
    root.scene = Some(Index::new(0));
    Ok(root)
}

fn load_node(
    root: &Root,
    registry: &SceneRegistry,
    index: usize,
    visited: &mut HashSet<usize>,
) -> Result<ResolvedScene, SceneError> {
    if !visited.insert(index) {
        return Err(SceneError::Invalid(format!(
            "node {index} appears twice in the node tree (cycle or duplicate reference)"
        )));
    }
    let node = root
        .nodes
        .get(index)
        .ok_or_else(|| SceneError::Invalid(format!("node index {index} out of bounds")))?;

    let extras: Option<serde_json::Value> = match &node.extras {
        Some(raw) => Some(serde_json::from_str(raw.get())?),
        None => None,
    };

    let mut templates: Vec<Box<dyn SceneTemplate>> = Vec::new();

    if let Some(node_name) = &node.name {
        if let Some(EntryKind::Extras { load, .. }) = registry.kind(NAME) {
            templates.push(load(&serde_json::Value::String(node_name.clone()))?);
        }
    }

    if matches!(registry.kind(TRANSFORM), Some(EntryKind::NativeTransform))
        && node.matrix.is_none()
        && (node.translation.is_some() || node.rotation.is_some() || node.scale.is_some())
    {
        let translation = node.translation.map(Vec3::from_array).unwrap_or(Vec3::ZERO);
        let rotation = node
            .rotation
            .map(|q| Quat::from_xyzw(q.0[0], q.0[1], q.0[2], q.0[3]))
            .unwrap_or(Quat::IDENTITY);
        let scale = node.scale.map(Vec3::from_array).unwrap_or(Vec3::ONE);
        templates.push(Box::new(Transform {
            translation,
            rotation,
            scale,
        }));
    }

    if matches!(registry.kind(CAMERA), Some(EntryKind::NativeCamera)) {
        if let Some(camera_index) = node.camera {
            let gltf_camera = root.cameras.get(camera_index.value()).ok_or_else(|| {
                SceneError::Invalid(format!(
                    "camera index {} out of bounds",
                    camera_index.value()
                ))
            })?;
            // Only perspective cameras map onto the engine's Camera.
            if let Some(perspective) = &gltf_camera.perspective {
                let clear_color = extras
                    .as_ref()
                    .and_then(|v| v.get("camera"))
                    .and_then(|v| v.get("clear_color"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_else(|| Camera::default().clear_color);
                templates.push(Box::new(Camera {
                    fov_y_radians: perspective.yfov,
                    near: perspective.znear,
                    clear_color,
                }));
            }
        }
    }

    if let Some(components) = extras
        .as_ref()
        .and_then(|v| v.get("components"))
        .and_then(serde_json::Value::as_object)
    {
        for (name, value) in components {
            // Unknown keys are skipped, not an error: a scene written by a
            // newer registry still loads.
            if let Some(EntryKind::Extras { load, .. }) = registry.kind(name) {
                templates.push(load(value)?);
            }
        }
    }

    let mut children = Vec::new();
    if let Some(child_indices) = &node.children {
        for child_index in child_indices {
            children.push(load_node(root, registry, child_index.value(), visited)?);
        }
    }
    Ok(ResolvedScene::new(templates, children))
}

/// Parse the document into a glTF root.
///
/// Goes through `serde_json::Value` first to backfill `"nodes": []` on scene
/// objects that lack it: `gltf-json`'s `Scene::nodes` skips serialization
/// when empty but has no `#[serde(default)]`, so an empty scene (`{}`, which
/// [`save_scene`] produces for an empty world) would otherwise fail to
/// parse.
fn parse_root(text: &str) -> Result<Root, SceneError> {
    let mut value: serde_json::Value = serde_json::from_str(text)?;
    if let Some(scenes) = value.get_mut("scenes").and_then(|s| s.as_array_mut()) {
        for scene in scenes {
            if let Some(object) = scene.as_object_mut() {
                object
                    .entry("nodes")
                    .or_insert_with(|| serde_json::Value::Array(Vec::new()));
            }
        }
    }
    Ok(serde_json::from_value(value)?)
}

/// Parse a glTF scene document and spawn its entities into `world`,
/// returning the root entities.
///
/// Per node, templates are built through the registry — unknown
/// `extras.components` keys are skipped, not an error. Children link to
/// their parents via `ChildOf`, so the world must have called
/// [`World::register_hierarchy`] first. When the hierarchy mapping is not
/// registered, every node in the document still spawns, but as a flat,
/// unlinked root.
pub fn load_scene(
    world: &mut World,
    registry: &SceneRegistry,
    text: &str,
) -> Result<Vec<Entity>, SceneError> {
    let root = parse_root(text)?;
    let hierarchy = registry.contains(HIERARCHY);
    let mut roots = Vec::new();

    let scene_index = root.scene.map(|index| index.value()).unwrap_or(0);
    let Some(scene) = root.scenes.get(scene_index) else {
        return Ok(roots);
    };
    let mut visited = HashSet::new();
    for &node_index in &scene.nodes {
        let resolved = load_node(&root, registry, node_index.value(), &mut visited)?;
        if hierarchy {
            roots.push(resolved.apply(world)?);
        } else {
            for flat in resolved.flatten() {
                roots.push(flat.apply(world)?);
            }
        }
    }
    Ok(roots)
}

/// [`save_scene`] + pretty-print + write to `path`.
pub fn save_scene_to_file(
    world: &World,
    registry: &SceneRegistry,
    path: &Path,
) -> Result<(), SceneError> {
    let root = save_scene(world, registry)?;
    std::fs::write(path, root.to_string_pretty()?)?;
    Ok(())
}

/// Read `path` and [`load_scene`] its contents into `world`.
pub fn load_scene_from_file(
    world: &mut World,
    registry: &SceneRegistry,
    path: &Path,
) -> Result<Vec<Entity>, SceneError> {
    let text = std::fs::read_to_string(path)?;
    load_scene(world, registry, &text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;
    use std::path::PathBuf;

    use moonfield_asset::{AssetError, AssetLoader, AssetServer, Assets, Handle};
    use moonfield_ecs::{Name, Template, TemplateContext};
    use moonfield_render_feature::mesh::{Mesh, MeshHandle, MeshRenderer};

    use crate::HandleTemplate;

    fn name_save(world: &World, entity: Entity) -> Option<serde_json::Value> {
        let name = world.get_component::<Name>(entity)?;
        Some(serde_json::Value::String(name.as_str().to_string()))
    }

    fn name_load(value: &serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError> {
        let text = value
            .as_str()
            .ok_or_else(|| SceneError::Invalid("name must be a string".to_string()))?;
        Ok(Box::new(Name::new(text)))
    }

    fn test_registry() -> SceneRegistry {
        let mut registry = SceneRegistry::new();
        registry.register_native_transform();
        registry.register_native_camera();
        registry.register_native_hierarchy();
        registry.register_custom(NAME, name_save, name_load);
        registry
    }

    /// A scene: root (Name + Transform) → child (Name + Transform + Camera)
    /// → grandchild (Name + Transform).
    fn sample_world() -> (World, Entity, Entity, Entity) {
        let mut world = World::new();
        world.register_hierarchy();
        let root = world.spawn((
            Name::new("root"),
            Transform {
                translation: Vec3::new(1.0, 2.0, 3.0),
                rotation: Quat::from_rotation_y(0.5),
                scale: Vec3::new(2.0, 2.0, 2.0),
            },
        ));
        let child = world.spawn((
            Name::new("child"),
            Transform::from_xyz(0.0, 1.0, 0.0),
            Camera {
                fov_y_radians: 0.8,
                near: 0.5,
                clear_color: [0.1, 0.2, 0.3, 1.0],
            },
            ChildOf(root),
        ));
        let grandchild = world.spawn((
            Name::new("grandchild"),
            Transform {
                translation: Vec3::new(-1.0, 0.0, 0.5),
                rotation: Quat::IDENTITY,
                scale: Vec3::ONE,
            },
            ChildOf(child),
        ));
        (world, root, child, grandchild)
    }

    #[test]
    fn test_scene_roundtrip() {
        let (world, root, child, grandchild) = sample_world();
        let registry = test_registry();

        let document = save_scene(&world, &registry).unwrap();
        let text = document.to_string_pretty().unwrap();

        let mut loaded = World::new();
        loaded.register_hierarchy();
        let roots = load_scene(&mut loaded, &registry, &text).unwrap();

        assert_eq!(roots.len(), 1);
        let loaded_root = roots[0];
        assert_eq!(
            loaded.get_component::<Name>(loaded_root).unwrap().as_str(),
            "root"
        );
        assert_eq!(
            loaded.get_component::<Transform>(loaded_root).unwrap(),
            world.get_component::<Transform>(root).unwrap()
        );

        let children = loaded.get_component::<Children>(loaded_root).unwrap();
        assert_eq!(children.len(), 1);
        let loaded_child = children[0];
        assert_eq!(
            loaded.get_component::<Name>(loaded_child).unwrap().as_str(),
            "child"
        );
        assert_eq!(
            loaded.get_component::<Transform>(loaded_child).unwrap(),
            world.get_component::<Transform>(child).unwrap()
        );
        assert_eq!(
            loaded.get_component::<Camera>(loaded_child).unwrap(),
            world.get_component::<Camera>(child).unwrap()
        );

        let grandchildren = loaded.get_component::<Children>(loaded_child).unwrap();
        assert_eq!(grandchildren.len(), 1);
        let loaded_grandchild = grandchildren[0];
        assert_eq!(
            loaded
                .get_component::<Name>(loaded_grandchild)
                .unwrap()
                .as_str(),
            "grandchild"
        );
        // Exact f32 equality: the quaternion rides the document verbatim.
        assert_eq!(
            loaded
                .get_component::<Transform>(loaded_grandchild)
                .unwrap(),
            world.get_component::<Transform>(grandchild).unwrap()
        );
    }

    #[test]
    fn test_output_is_valid_gltf() {
        let (world, _, _, _) = sample_world();
        let registry = test_registry();

        let document = save_scene(&world, &registry).unwrap();
        assert_eq!(document.asset.version, "2.0");
        assert_eq!(document.asset.generator.as_deref(), Some("moonfield"));
        assert_eq!(document.cameras.len(), 1);

        // The serialized text parses back as a glTF root.
        let text = document.to_string_pretty().unwrap();
        let parsed = Root::from_str(&text).unwrap();
        assert_eq!(parsed.nodes.len(), 3);
        assert_eq!(parsed.cameras.len(), 1);
        assert_eq!(parsed.scenes[parsed.scene.unwrap().value()].nodes.len(), 1);
    }

    #[test]
    fn test_unknown_extras_key_is_skipped() {
        let registry = test_registry();
        let text = r#"{
            "asset": { "version": "2.0", "generator": "test" },
            "scene": 0,
            "scenes": [{ "nodes": [0] }],
            "nodes": [{
                "name": "Solo",
                "translation": [1.0, 2.0, 3.0],
                "extras": { "components": { "unknown_thing": { "foo": 1 } } }
            }]
        }"#;

        let mut world = World::new();
        world.register_hierarchy();
        let roots = load_scene(&mut world, &registry, text).unwrap();

        assert_eq!(roots.len(), 1);
        assert_eq!(
            world.get_component::<Name>(roots[0]).unwrap().as_str(),
            "Solo"
        );
        assert_eq!(
            world
                .get_component::<Transform>(roots[0])
                .unwrap()
                .translation,
            Vec3::new(1.0, 2.0, 3.0)
        );
    }

    #[test]
    fn test_empty_scene_roundtrip() {
        let world = World::new();
        let registry = test_registry();

        let document = save_scene(&world, &registry).unwrap();
        assert!(document.nodes.is_empty());
        let text = document.to_string_pretty().unwrap();

        let mut loaded = World::new();
        loaded.register_hierarchy();
        let roots = load_scene(&mut loaded, &registry, &text).unwrap();
        assert!(roots.is_empty());
    }

    // ---------------------------------------------------------------
    // Handle component roundtrip
    // ---------------------------------------------------------------

    /// Loads `.fake` files into `String` assets.
    struct FakeLoader;

    impl AssetLoader for FakeLoader {
        fn extensions(&self) -> &'static [&'static str] {
            &["fake"]
        }

        fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
            Ok(Box::new(std::fs::read_to_string(path)?))
        }
    }

    /// Where each handle was loaded from; the save hook needs it to write
    /// the path back into the document.
    #[derive(Default)]
    struct HandlePaths(std::collections::HashMap<Handle<String>, PathBuf>);

    fn cloud_save(world: &World, entity: Entity) -> Option<serde_json::Value> {
        let handle = world.get_component::<Handle<String>>(entity)?;
        let paths = world.get_resource::<HandlePaths>()?;
        let path = paths.0.get(handle)?;
        Some(serde_json::Value::String(
            path.to_string_lossy().into_owned(),
        ))
    }

    fn cloud_load(value: &serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError> {
        let text = value
            .as_str()
            .ok_or_else(|| SceneError::Invalid("splat_cloud must be a path string".to_string()))?;
        Ok(Box::new(HandleTemplate::<String>::Path(PathBuf::from(
            text,
        ))))
    }

    #[test]
    fn test_handle_component_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "moonfield-scene-test-handle-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cloud.fake");
        std::fs::write(&path, "point cloud payload").unwrap();

        // Source world: load the asset, remember its path, attach the handle.
        let mut world = World::new();
        world.register_hierarchy();
        let mut server = AssetServer::default();
        server.register_loader(FakeLoader);
        let mut assets = Assets::<String>::default();
        let handle = server.load(&mut assets, &path).unwrap();
        let mut paths = HandlePaths::default();
        paths.0.insert(handle, path.clone());
        world.insert_resource(server);
        world.insert_resource(assets);
        world.insert_resource(paths);
        world.spawn((handle,));

        let mut registry = SceneRegistry::new();
        registry.register_custom("splat_cloud", cloud_save, cloud_load);

        let document = save_scene(&world, &registry).unwrap();
        let text = document.to_string_pretty().unwrap();
        // The handle became a plain path string in extras.
        assert!(text.contains("cloud.fake"));

        // Load into a fresh world with its own asset storage.
        let mut loaded = World::new();
        loaded.register_hierarchy();
        let mut server = AssetServer::default();
        server.register_loader(FakeLoader);
        loaded.insert_resource(server);
        loaded.insert_resource(Assets::<String>::default());
        let roots = load_scene(&mut loaded, &registry, &text).unwrap();

        assert_eq!(roots.len(), 1);
        let loaded_handle = loaded.get_component::<Handle<String>>(roots[0]).unwrap();
        let assets = loaded.get_resource::<Assets<String>>().unwrap();
        assert_eq!(
            assets.get(loaded_handle).map(String::as_str),
            Some("point cloud payload")
        );

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn test_file_roundtrip_through_disk() {
        let (world, _, _, _) = sample_world();
        let registry = test_registry();

        let path = std::env::temp_dir().join(format!(
            "moonfield-scene-test-file-{}.gltf",
            std::process::id()
        ));
        save_scene_to_file(&world, &registry, &path).unwrap();

        let mut loaded = World::new();
        loaded.register_hierarchy();
        let roots = load_scene_from_file(&mut loaded, &registry, &path).unwrap();
        assert_eq!(roots.len(), 1);
        assert_eq!(
            loaded.get_component::<Name>(roots[0]).unwrap().as_str(),
            "root"
        );

        std::fs::remove_file(&path).unwrap();
    }

    // ---------------------------------------------------------------
    // MeshRenderer path entry (same shape the editor registers)
    // ---------------------------------------------------------------

    /// The color a mesh entity loaded from a scene file gets; mirrors the
    /// editor's default (the path-string entry does not carry color).
    const DEFAULT_MESH_COLOR: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

    /// Assemble a GLB container from a JSON document and a binary blob.
    fn glb(json: &str, bin: &[u8]) -> Vec<u8> {
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut bin = bin.to_vec();
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = b"glTF".to_vec();
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    /// A minimal but valid one-triangle GLB (non-indexed).
    fn test_mesh_glb() -> Vec<u8> {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let bin: Vec<u8> = positions.iter().flat_map(|v| v.to_le_bytes()).collect();
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                      "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}}
                ],
                "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}}}]}}]
            }}"#,
            bin.len()
        );
        glb(&json, &bin)
    }

    /// Loads `.glb` files into `Mesh` assets (the editor's `GltfLoader`
    /// shape, minus the splat sniffing).
    struct MeshGlbLoader;

    impl AssetLoader for MeshGlbLoader {
        fn extensions(&self) -> &'static [&'static str] {
            &["glb"]
        }

        fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
            match Mesh::from_gltf_file(path) {
                Ok(mesh) => Ok(Box::new(mesh)),
                Err(e) => Err(AssetError::Loader(e.to_string())),
            }
        }
    }

    fn mesh_renderer_save(world: &World, entity: Entity) -> Option<serde_json::Value> {
        let renderer = world.get_component::<MeshRenderer>(entity)?;
        let assets = world.get_resource::<Assets<Mesh>>()?;
        let source = assets.get(&renderer.mesh.0)?.source()?;
        Some(serde_json::Value::String(source.to_string()))
    }

    /// Builds a [`MeshRenderer`] from a scene file's path string, resolving
    /// the asset through the world's `AssetServer`.
    ///
    /// Deliberately not `Clone`: moonfield-ecs blanket-implements `Template`
    /// for every `Clone` type, which would collide with this impl. A bare
    /// `HandleTemplate<Mesh>` cannot be used directly — its output is
    /// `Handle<Mesh>`, but the entity component is the `MeshRenderer`
    /// newtype wrapper the renderer queries.
    struct MeshRendererTemplate(PathBuf);

    impl Template for MeshRendererTemplate {
        type Output = MeshRenderer;

        fn build(&self, ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError> {
            let handle = HandleTemplate::<Mesh>::Path(self.0.clone()).build(ctx)?;
            Ok(MeshRenderer::new(MeshHandle(handle), DEFAULT_MESH_COLOR))
        }
    }

    fn mesh_renderer_load(value: &serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError> {
        let text = value.as_str().ok_or_else(|| {
            SceneError::Invalid("mesh_renderer must be a path string".to_string())
        })?;
        Ok(Box::new(MeshRendererTemplate(PathBuf::from(text))))
    }

    #[test]
    fn test_mesh_renderer_path_entry_roundtrip() {
        let dir =
            std::env::temp_dir().join(format!("moonfield-scene-test-mesh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("triangle.glb");
        std::fs::write(&path, test_mesh_glb()).unwrap();

        // Source world: a mesh asset with a source path + an entity carrying
        // the MeshRenderer.
        let mut world = World::new();
        world.register_hierarchy();
        let mut meshes = Assets::<Mesh>::default();
        let handle = meshes.add(Mesh::from_gltf_file(&path).unwrap());
        world.insert_resource(meshes);
        let entity = world.spawn((
            Name::new("mesh"),
            MeshRenderer::new(MeshHandle(handle), [1.0, 0.0, 0.0, 1.0]),
        ));
        assert!(world.get_component::<MeshRenderer>(entity).is_some());

        let mut registry = SceneRegistry::new();
        registry.register_custom(NAME, name_save, name_load);
        registry.register_custom("mesh_renderer", mesh_renderer_save, mesh_renderer_load);

        let document = save_scene(&world, &registry).unwrap();
        let text = document.to_string_pretty().unwrap();
        // The handle became a plain path string in extras.
        assert!(text.contains("triangle.glb"), "{text}");

        // Load into a fresh world with its own asset storage.
        let mut loaded = World::new();
        loaded.register_hierarchy();
        let mut server = AssetServer::default();
        server.register_loader(MeshGlbLoader);
        loaded.insert_resource(server);
        loaded.insert_resource(Assets::<Mesh>::default());
        let roots = load_scene(&mut loaded, &registry, &text).unwrap();

        assert_eq!(roots.len(), 1);
        let renderer = loaded.get_component::<MeshRenderer>(roots[0]).unwrap();
        // The entry stores only the path: color comes back as the default.
        assert_eq!(renderer.color, DEFAULT_MESH_COLOR);
        let meshes = loaded.get_resource::<Assets<Mesh>>().unwrap();
        let mesh = meshes.get(&renderer.mesh.0).unwrap();
        assert_eq!(mesh.positions().len(), 3);
        assert_eq!(mesh.indices(), &[0, 1, 2]);
        assert_eq!(mesh.source(), Some(path.display().to_string().as_str()));

        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }
}
