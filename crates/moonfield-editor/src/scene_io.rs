//! Synchronous scene and asset I/O for the editor.
//!
//! Two halves, both display-free so they are unit-testable:
//!
//! - **Asset loading.** [`SplatCloudLoader`] is the `.ply` → `SplatCloud`
//!   [`AssetLoader`]; [`editor_asset_server`] bundles it into the
//!   `AssetServer` resource. [`load_splat_cloud`] routes a PLY path through
//!   that server (path-deduped) and spawns an entity holding a
//!   [`SplatCloudHandle`] component.
//! - **Scene save/load.** [`editor_scene_registry`] builds the
//!   `SceneRegistry` resource the hierarchy panel's Save/Load buttons run
//!   against: native Transform/Camera/hierarchy mappings, `Name` on the
//!   node's name field, `MeshRenderer` through the extras channel, and
//!   `SplatCloudHandle` as a path-backed handle entry.

use std::any::Any;
use std::path::{Path, PathBuf};

use moonfield_asset::{AssetError, AssetLoader, AssetServer, Assets};
use moonfield_ecs::{Entity, Name, Template, TemplateContext, TemplateError, World};
use moonfield_math::Transform;
use moonfield_render::MeshRenderer;
use moonfield_renderer::splat::cloud::{SplatCloud, SplatCloudHandle};
use moonfield_scene::{HandleTemplate, SceneError, SceneRegistry, SceneTemplate, NAME};

/// Loads `.ply` files into [`SplatCloud`] assets.
pub struct SplatCloudLoader;

impl AssetLoader for SplatCloudLoader {
    fn extensions(&self) -> &'static [&'static str] {
        &["ply"]
    }

    fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
        match SplatCloud::from_ply_file(path) {
            Ok(cloud) => Ok(Box::new(cloud)),
            Err(e) => Err(AssetError::Loader(e.to_string())),
        }
    }
}

/// The editor's asset server: PLY files resolve to `SplatCloud` assets.
pub fn editor_asset_server() -> AssetServer {
    let mut server = AssetServer::default();
    server.register_loader(SplatCloudLoader);
    server
}

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

fn splat_cloud_save(world: &World, entity: Entity) -> Option<serde_json::Value> {
    let handle = world.get_component::<SplatCloudHandle>(entity)?;
    let assets = world.get_resource::<Assets<SplatCloud>>()?;
    let source = assets.get(&handle.0)?.source()?;
    Some(serde_json::Value::String(source.to_string()))
}

/// Builds a [`SplatCloudHandle`] from a scene file's path string, resolving
/// the asset through the world's `AssetServer` (path-deduped).
///
/// Deliberately not `Clone`: moonfield-ecs blanket-implements `Template` for
/// every `Clone` type, which would collide with this impl. A bare
/// `HandleTemplate<SplatCloud>` cannot be used directly — its output is
/// `Handle<SplatCloud>`, but the entity component is the `SplatCloudHandle`
/// newtype the renderer queries.
struct SplatCloudHandleTemplate(PathBuf);

impl Template for SplatCloudHandleTemplate {
    type Output = SplatCloudHandle;

    fn build(&self, ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError> {
        let handle = HandleTemplate::<SplatCloud>::Path(self.0.clone()).build(ctx)?;
        Ok(SplatCloudHandle(handle))
    }
}

fn splat_cloud_load(value: &serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError> {
    let text = value
        .as_str()
        .ok_or_else(|| SceneError::Invalid("splat_cloud must be a path string".to_string()))?;
    Ok(Box::new(SplatCloudHandleTemplate(PathBuf::from(text))))
}

/// The scene registry the editor saves and loads with.
///
/// Transform, Camera, and the hierarchy ride the glTF node's native fields;
/// `Name` is registered under [`NAME`] so it lands on `node.name` instead of
/// the extras map; `MeshRenderer` is a plain serde extras entry; splat cloud
/// handles save as their cloud's source path and load back through the
/// `AssetServer` as [`HandleTemplate::Path`].
pub fn editor_scene_registry() -> SceneRegistry {
    let mut registry = SceneRegistry::new();
    registry.register_native_transform();
    registry.register_native_camera();
    registry.register_native_hierarchy();
    registry.register_custom(NAME, name_save, name_load);
    registry.register::<MeshRenderer>("mesh_renderer");
    registry.register_custom("splat_cloud", splat_cloud_save, splat_cloud_load);
    registry
}

/// Load a 3DGS PLY file into the world synchronously through the
/// `AssetServer` (created with the PLY loader if missing), and spawn an
/// entity named after the file holding the handle component. The server
/// dedups by path: loading the same file twice reuses the asset slot.
///
/// Returns the new entity, or a human-readable error.
pub fn load_splat_cloud(world: &mut World, path: &Path) -> Result<Entity, String> {
    if !world.contains_resource::<AssetServer>() {
        world.insert_resource(editor_asset_server());
    }

    // `AssetServer::load` needs `&mut AssetServer` and `&mut Assets<T>` at
    // once, but the world's resource storage hands out one borrow per
    // resource; take the server out, use it, and put it back (also on the
    // error path) — the same pattern `HandleTemplate::build` uses.
    let mut server = world
        .remove_resource::<AssetServer>()
        .expect("AssetServer was just ensured");
    let result = if world.contains_resource::<Assets<SplatCloud>>() {
        let mut assets = world
            .get_resource_mut::<Assets<SplatCloud>>()
            .expect("Assets<SplatCloud> was just checked");
        server.load(&mut assets, path)
    } else {
        // Keep failures side-effect free: the store only appears once a
        // cloud actually loads.
        let mut assets = Assets::<SplatCloud>::default();
        let result = server.load(&mut assets, path);
        if result.is_ok() {
            world.insert_resource(assets);
        }
        result
    };
    world.insert_resource(server);
    let handle = result.map_err(|e| e.to_string())?;

    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Splat Cloud".to_string());

    Ok(world.spawn((
        Name::new(name),
        Transform::IDENTITY,
        SplatCloudHandle(handle),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::{ChildOf, Children};
    use moonfield_scene::{load_scene_from_file, save_scene_to_file};

    /// Write a minimal but valid 1-vertex binary little-endian 3DGS PLY
    /// (all properties the parser requires) into a temp file. `tag` makes the
    /// file name unique per test so parallel tests never delete each other's
    /// files.
    fn write_test_ply(tag: &str) -> std::path::PathBuf {
        let mut properties: Vec<String> = Vec::new();
        properties.extend(["x", "y", "z"].map(str::to_string));
        properties.push("opacity".to_string());
        properties.extend((0..3).map(|i| format!("scale_{i}")));
        properties.extend((0..4).map(|i| format!("rot_{i}")));
        properties.extend((0..3).map(|i| format!("f_dc_{i}")));
        properties.extend((0..45).map(|i| format!("f_rest_{i}")));

        let mut bytes = b"ply\nformat binary_little_endian 1.0\n".to_vec();
        bytes.extend_from_slice(b"element vertex 1\n");
        for p in &properties {
            bytes.extend_from_slice(format!("property float {p}\n").as_bytes());
        }
        bytes.extend_from_slice(b"end_header\n");
        // One vertex: x=1, y=2, z=3, everything else zero.
        let mut vertex = vec![0.0f32; properties.len()];
        vertex[0] = 1.0;
        vertex[1] = 2.0;
        vertex[2] = 3.0;
        for v in vertex {
            bytes.extend_from_slice(&v.to_le_bytes());
        }

        let path = std::env::temp_dir().join(format!(
            "moonfield-editor-test-{}-{tag}.ply",
            std::process::id(),
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn test_ply_to_asset_to_entity_flow() {
        let path = write_test_ply("splat");
        let mut world = World::new();

        let entity = load_splat_cloud(&mut world, &path).expect("load succeeds");

        // The asset store was created and holds the parsed cloud.
        let clouds = world.get_resource::<Assets<SplatCloud>>().unwrap();
        assert_eq!(clouds.len(), 1);

        // The entity references it and resolves to the parsed data.
        let handle = world.get_component::<SplatCloudHandle>(entity).unwrap();
        let cloud = clouds.get(&handle.0).unwrap();
        assert_eq!(cloud.len(), 1);
        assert_eq!(cloud.scene().positions[0], [1.0, 2.0, 3.0]);
        assert_eq!(cloud.aabb(), ([1.0, 2.0, 3.0], [1.0, 2.0, 3.0]));
        drop(clouds);

        // The entity is named after the file and starts at the identity
        // transform (inspectable/editable in the editor).
        assert_eq!(
            world.get_component::<Name>(entity).unwrap().as_str(),
            path.file_name().unwrap().to_str().unwrap()
        );
        assert!(world.get_component::<Transform>(entity).is_some());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_splat_cloud_missing_file_errors() {
        let mut world = World::new();
        let result = load_splat_cloud(&mut world, Path::new("does/not/exist.ply"));
        assert!(result.is_err());
        // Nothing was inserted on failure.
        assert!(world.get_resource::<Assets<SplatCloud>>().is_none());
    }

    #[test]
    fn test_load_splat_cloud_dedups_path() {
        let path = write_test_ply("dedup");
        let mut world = World::new();

        let first = load_splat_cloud(&mut world, &path).expect("first load succeeds");
        let second = load_splat_cloud(&mut world, &path).expect("second load succeeds");

        // Two entities, one asset slot: the AssetServer path cache served the
        // second load.
        assert_ne!(first, second);
        assert_eq!(
            world.get_component::<SplatCloudHandle>(first).unwrap(),
            world.get_component::<SplatCloudHandle>(second).unwrap()
        );
        assert_eq!(world.get_resource::<Assets<SplatCloud>>().unwrap().len(), 1);

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_scene_roundtrip_with_splat_and_child() {
        let ply_path = write_test_ply("roundtrip");
        let scene_path = std::env::temp_dir().join(format!(
            "moonfield-editor-test-{}-scene.gltf",
            std::process::id()
        ));
        let registry = editor_scene_registry();

        // Source world: a splat cloud root plus a named, transformed child.
        let mut world = World::new();
        world.register_hierarchy();
        let splat = load_splat_cloud(&mut world, &ply_path).expect("load succeeds");
        let original_handle = world.get_component::<SplatCloudHandle>(splat).unwrap().0;
        let child = world.spawn((
            Name::new("child"),
            Transform::from_xyz(1.0, 2.0, 3.0),
            ChildOf(splat),
        ));

        save_scene_to_file(&world, &registry, &scene_path).expect("save succeeds");

        // Loading into the SAME world reuses the cached asset slot instead of
        // re-parsing the PLY.
        let same_world_roots =
            load_scene_from_file(&mut world, &registry, &scene_path).expect("reload succeeds");
        assert_eq!(same_world_roots.len(), 1);
        assert_eq!(
            world
                .get_component::<SplatCloudHandle>(same_world_roots[0])
                .unwrap()
                .0,
            original_handle
        );
        assert_eq!(world.get_resource::<Assets<SplatCloud>>().unwrap().len(), 1);

        // Fresh world: full roundtrip through the document.
        let mut fresh = World::new();
        fresh.register_hierarchy();
        fresh.insert_resource(editor_asset_server());
        fresh.insert_resource(Assets::<SplatCloud>::default());
        let roots =
            load_scene_from_file(&mut fresh, &registry, &scene_path).expect("load succeeds");

        assert_eq!(roots.len(), 1);
        let loaded_splat = roots[0];
        assert_eq!(
            fresh.get_component::<Name>(loaded_splat).unwrap().as_str(),
            ply_path.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(
            fresh.get_component::<Transform>(loaded_splat).unwrap(),
            &Transform::IDENTITY
        );
        let loaded_handle = fresh
            .get_component::<SplatCloudHandle>(loaded_splat)
            .unwrap()
            .0;
        let clouds = fresh.get_resource::<Assets<SplatCloud>>().unwrap();
        assert_eq!(clouds.len(), 1);
        let cloud = clouds.get(&loaded_handle).unwrap();
        assert_eq!(cloud.len(), 1);
        assert_eq!(cloud.scene().positions[0], [1.0, 2.0, 3.0]);
        assert_eq!(
            cloud.source(),
            Some(ply_path.display().to_string().as_str())
        );
        drop(clouds);

        let children = fresh.get_component::<Children>(loaded_splat).unwrap();
        assert_eq!(children.len(), 1);
        let loaded_child = children[0];
        assert_eq!(
            fresh.get_component::<Name>(loaded_child).unwrap().as_str(),
            world.get_component::<Name>(child).unwrap().as_str()
        );
        assert_eq!(
            fresh.get_component::<Transform>(loaded_child).unwrap(),
            world.get_component::<Transform>(child).unwrap()
        );
        assert_eq!(
            fresh
                .get_component::<ChildOf>(loaded_child)
                .unwrap()
                .parent(),
            loaded_splat
        );

        std::fs::remove_file(&ply_path).ok();
        std::fs::remove_file(&scene_path).ok();
    }
}
