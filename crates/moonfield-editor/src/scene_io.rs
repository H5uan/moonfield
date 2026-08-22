//! Synchronous scene and asset I/O for the editor.
//!
//! Two halves, both display-free so they are unit-testable:
//!
//! - **Asset loading.** [`GltfLoader`] is the `.gltf`/`.glb` [`AssetLoader`]:
//!   it sniffs the file for the `KHR_gaussian_splatting` extension and
//!   produces a [`SplatCloud`] or a [`Mesh`] accordingly.
//!   [`editor_asset_server`] bundles it into the `AssetServer` resource.
//!   [`load_asset`] routes a path through that server (path-deduped) and
//!   spawns an entity holding the right handle component
//!   ([`SplatCloudHandle`] or [`MeshRenderer`]).
//! - **Scene save/load.** [`editor_scene_registry`] builds the
//!   `SceneRegistry` resource the hierarchy panel's Save/Load buttons run
//!   against: native Transform/Camera/hierarchy mappings, `Name` on the
//!   node's name field, and `MeshRenderer`/`SplatCloudHandle` as path-backed
//!   handle entries in the extras channel.

use std::any::Any;
use std::path::{Path, PathBuf};

use moonfield_asset::{AssetError, AssetLoader, AssetServer, Assets, Handle};
use moonfield_ecs::{Entity, Name, Template, TemplateContext, TemplateError, World};
use moonfield_math::Transform;
use moonfield_renderer::mesh::{Mesh, MeshHandle, MeshRenderer};
use moonfield_renderer::splat::cloud::{SplatCloud, SplatCloudHandle};
use moonfield_scene::{HandleTemplate, SceneError, SceneRegistry, SceneTemplate, NAME};

/// The color a mesh entity gets when it is created from a file: fresh
/// `load_asset` spawns and scene-file loads (the path-string entry does not
/// carry color). A neutral gray; the inspector edits it afterwards.
pub const DEFAULT_MESH_COLOR: [f32; 4] = [0.7, 0.7, 0.7, 1.0];

/// The splat extension name, quoted as a JSON key.
const SPLAT_EXTENSION_KEY: &[u8] = b"\"KHR_gaussian_splatting\"";

/// Whether the glTF/GLB bytes reference the `KHR_gaussian_splatting`
/// extension. A plain substring search is robust enough here: the extension
/// only appears as a JSON string key (`"KHR_gaussian_splatting"`), verbatim
/// in both the `.gltf` document and the GLB JSON chunk.
fn is_splat_gltf(bytes: &[u8]) -> bool {
    bytes
        .windows(SPLAT_EXTENSION_KEY.len())
        .any(|window| window == SPLAT_EXTENSION_KEY)
}

/// Loads `.gltf`/`.glb` files into [`SplatCloud`] or [`Mesh`] assets,
/// dispatched by file content (see [`is_splat_gltf`]).
///
/// `AssetServer::load::<T>` downcasts the payload to the requested `T`, so a
/// file whose content does not match the requested type yields
/// [`AssetError::TypeMismatch`] naturally.
pub struct GltfLoader;

impl AssetLoader for GltfLoader {
    fn extensions(&self) -> &'static [&'static str] {
        &["gltf", "glb"]
    }

    fn load(&self, path: &Path) -> Result<Box<dyn Any>, AssetError> {
        let bytes = std::fs::read(path)?;
        if is_splat_gltf(&bytes) {
            match SplatCloud::from_gltf_file(path) {
                Ok(cloud) => Ok(Box::new(cloud)),
                Err(e) => Err(AssetError::Loader(e.to_string())),
            }
        } else {
            match Mesh::from_gltf_file(path) {
                Ok(mesh) => Ok(Box::new(mesh)),
                Err(e) => Err(AssetError::Loader(e.to_string())),
            }
        }
    }
}

/// The editor's asset server: glTF files resolve to `SplatCloud` or `Mesh`
/// assets by content.
pub fn editor_asset_server() -> AssetServer {
    let mut server = AssetServer::default();
    server.register_loader(GltfLoader);
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

fn mesh_renderer_save(world: &World, entity: Entity) -> Option<serde_json::Value> {
    let renderer = world.get_component::<MeshRenderer>(entity)?;
    let assets = world.get_resource::<Assets<Mesh>>()?;
    let source = assets.get(&renderer.mesh.0)?.source()?;
    Some(serde_json::Value::String(source.to_string()))
}

/// Builds a [`MeshRenderer`] from a scene file's path string, resolving the
/// asset through the world's `AssetServer` (path-deduped). Not `Clone`, for
/// the same blanket-impl reason as [`SplatCloudHandleTemplate`].
///
/// The entry stores only the mesh's source path; the loaded component gets
/// [`DEFAULT_MESH_COLOR`].
struct MeshRendererTemplate(PathBuf);

impl Template for MeshRendererTemplate {
    type Output = MeshRenderer;

    fn build(&self, ctx: &mut TemplateContext) -> Result<Self::Output, TemplateError> {
        let handle = HandleTemplate::<Mesh>::Path(self.0.clone()).build(ctx)?;
        Ok(MeshRenderer::new(MeshHandle(handle), DEFAULT_MESH_COLOR))
    }
}

fn mesh_renderer_load(value: &serde_json::Value) -> Result<Box<dyn SceneTemplate>, SceneError> {
    let text = value
        .as_str()
        .ok_or_else(|| SceneError::Invalid("mesh_renderer must be a path string".to_string()))?;
    Ok(Box::new(MeshRendererTemplate(PathBuf::from(text))))
}

/// The scene registry the editor saves and loads with.
///
/// Transform, Camera, and the hierarchy ride the glTF node's native fields;
/// `Name` is registered under [`NAME`] so it lands on `node.name` instead of
/// the extras map; `MeshRenderer` and `SplatCloudHandle` save as their
/// asset's source path and load back through the `AssetServer` as
/// [`HandleTemplate::Path`].
pub fn editor_scene_registry() -> SceneRegistry {
    let mut registry = SceneRegistry::new();
    registry.register_native_transform();
    registry.register_native_camera();
    registry.register_native_hierarchy();
    registry.register_custom(NAME, name_save, name_load);
    registry.register_custom("mesh_renderer", mesh_renderer_save, mesh_renderer_load);
    registry.register_custom("splat_cloud", splat_cloud_save, splat_cloud_load);
    registry
}

/// Run `AssetServer::load` against the world's resources. `load` needs
/// `&mut AssetServer` and `&mut Assets<T>` at once, but the world's resource
/// storage hands out one borrow per resource; take the server out, use it,
/// and put it back (also on the error path) — the same pattern
/// `HandleTemplate::build` uses.
fn load_with_server<T: Send + Sync + 'static>(
    world: &mut World,
    path: &Path,
) -> Result<Handle<T>, String> {
    let mut server = world
        .remove_resource::<AssetServer>()
        .expect("AssetServer was just ensured");
    let result = if world.contains_resource::<Assets<T>>() {
        let mut assets = world
            .get_resource_mut::<Assets<T>>()
            .expect("Assets<T> was just checked");
        server.load(&mut assets, path)
    } else {
        // Keep failures side-effect free: the store only appears once an
        // asset actually loads.
        let mut assets = Assets::<T>::default();
        let result = server.load(&mut assets, path);
        if result.is_ok() {
            world.insert_resource(assets);
        }
        result
    };
    world.insert_resource(server);
    result.map_err(|e| e.to_string())
}

/// Load a glTF asset into the world synchronously through the `AssetServer`
/// (created with the glTF loader if missing), and spawn an entity named
/// after the file holding the right handle component: files carrying the
/// `KHR_gaussian_splatting` extension become [`SplatCloud`] entities,
/// everything else becomes a [`Mesh`] entity with a [`MeshRenderer`] in
/// [`DEFAULT_MESH_COLOR`]. The server dedups by path: loading the same file
/// twice reuses the asset slot.
///
/// Returns the new entity, or a human-readable error.
pub fn load_asset(world: &mut World, path: &Path) -> Result<Entity, String> {
    if !world.contains_resource::<AssetServer>() {
        world.insert_resource(editor_asset_server());
    }

    let bytes =
        std::fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Asset".to_string());

    if is_splat_gltf(&bytes) {
        let handle = load_with_server::<SplatCloud>(world, path)?;
        Ok(world.spawn((
            Name::new(name),
            Transform::IDENTITY,
            SplatCloudHandle(handle),
        )))
    } else {
        let handle = load_with_server::<Mesh>(world, path)?;
        Ok(world.spawn((
            Name::new(name),
            Transform::IDENTITY,
            MeshRenderer::new(MeshHandle(handle), DEFAULT_MESH_COLOR),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moonfield_ecs::{ChildOf, Children};
    use moonfield_scene::{load_scene_from_file, save_scene_to_file};

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

    fn f32_bytes(values: &[f32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// A minimal but valid 1-splat `KHR_gaussian_splatting` GLB (all
    /// required attributes; the splat sits at [1, 2, 3]).
    fn test_splat_glb() -> Vec<u8> {
        let mut bin = f32_bytes(&[1.0, 2.0, 3.0]); // POSITION
        let rot_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.0, 0.0, 0.0, 1.0])); // ROTATION
        let scale_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[1.0, 1.0, 1.0])); // SCALE
        let opacity_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.5])); // OPACITY
        let dc_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&[0.1, 0.2, 0.3])); // SH_DEGREE_0_COEF_0
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "extensionsUsed": ["KHR_gaussian_splatting"],
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": {rot_off}, "byteLength": 16}},
                    {{"buffer": 0, "byteOffset": {scale_off}, "byteLength": 12}},
                    {{"buffer": 0, "byteOffset": {opacity_off}, "byteLength": 4}},
                    {{"buffer": 0, "byteOffset": {dc_off}, "byteLength": 12}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
                      "min": [1.0, 2.0, 3.0], "max": [1.0, 2.0, 3.0]}},
                    {{"bufferView": 1, "componentType": 5126, "count": 1, "type": "VEC4"}},
                    {{"bufferView": 2, "componentType": 5126, "count": 1, "type": "VEC3"}},
                    {{"bufferView": 3, "componentType": 5126, "count": 1, "type": "SCALAR"}},
                    {{"bufferView": 4, "componentType": 5126, "count": 1, "type": "VEC3"}}
                ],
                "meshes": [{{"primitives": [{{
                    "attributes": {{
                        "POSITION": 0,
                        "KHR_gaussian_splatting:ROTATION": 1,
                        "KHR_gaussian_splatting:SCALE": 2,
                        "KHR_gaussian_splatting:OPACITY": 3,
                        "KHR_gaussian_splatting:SH_DEGREE_0_COEF_0": 4
                    }},
                    "mode": 0,
                    "extensions": {{"KHR_gaussian_splatting": {{"kernel": "ellipse", "colorSpace": "srgb_rec709_display"}}}}
                }}]}}]
            }}"#,
            bin.len()
        );
        glb(&json, &bin)
    }

    /// A minimal but valid one-triangle GLB (non-indexed, no splat
    /// extension).
    fn test_mesh_glb() -> Vec<u8> {
        let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let bin = f32_bytes(&positions);
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

    /// Write `bytes` into a uniquely named temp file. `tag` makes the file
    /// name unique per test so parallel tests never delete each other's
    /// files.
    fn write_test_file(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moonfield-editor-test-{}-{tag}",
            std::process::id(),
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn test_splat_asset_to_entity_flow() {
        let path = write_test_file("splat.glb", &test_splat_glb());
        let mut world = World::new();

        let entity = load_asset(&mut world, &path).expect("load succeeds");

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
    fn test_mesh_asset_to_entity_flow() {
        let path = write_test_file("mesh.glb", &test_mesh_glb());
        let mut world = World::new();

        let entity = load_asset(&mut world, &path).expect("load succeeds");

        // The mesh store was created and holds the parsed mesh.
        let meshes = world.get_resource::<Assets<Mesh>>().unwrap();
        assert_eq!(meshes.len(), 1);
        let renderer = world.get_component::<MeshRenderer>(entity).unwrap();
        assert_eq!(renderer.color, DEFAULT_MESH_COLOR);
        let mesh = meshes.get(&renderer.mesh.0).unwrap();
        assert_eq!(mesh.positions().len(), 3);
        assert_eq!(mesh.indices(), &[0, 1, 2]);
        drop(meshes);

        assert_eq!(
            world.get_component::<Name>(entity).unwrap().as_str(),
            path.file_name().unwrap().to_str().unwrap()
        );
        assert!(world.get_component::<Transform>(entity).is_some());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn test_load_asset_missing_file_errors() {
        let mut world = World::new();
        let result = load_asset(&mut world, Path::new("does/not/exist.glb"));
        assert!(result.is_err());
        // Nothing was inserted on failure.
        assert!(world.get_resource::<Assets<SplatCloud>>().is_none());
        assert!(world.get_resource::<Assets<Mesh>>().is_none());
    }

    #[test]
    fn test_load_asset_dedups_path() {
        let path = write_test_file("dedup.glb", &test_splat_glb());
        let mut world = World::new();

        let first = load_asset(&mut world, &path).expect("first load succeeds");
        let second = load_asset(&mut world, &path).expect("second load succeeds");

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
    fn test_scene_roundtrip_with_splat_mesh_and_child() {
        let splat_path = write_test_file("roundtrip-splat.glb", &test_splat_glb());
        let mesh_path = write_test_file("roundtrip-mesh.glb", &test_mesh_glb());
        let scene_path = std::env::temp_dir().join(format!(
            "moonfield-editor-test-{}-scene.gltf",
            std::process::id()
        ));
        let registry = editor_scene_registry();

        // Source world: a splat cloud root plus a named, transformed mesh
        // child.
        let mut world = World::new();
        world.register_hierarchy();
        let splat = load_asset(&mut world, &splat_path).expect("load succeeds");
        let original_handle = world.get_component::<SplatCloudHandle>(splat).unwrap().0;
        let mesh_entity = load_asset(&mut world, &mesh_path).expect("load succeeds");
        let original_mesh_handle = world
            .get_component::<MeshRenderer>(mesh_entity)
            .unwrap()
            .mesh;
        world.insert_component(mesh_entity, ChildOf(splat));

        save_scene_to_file(&world, &registry, &scene_path).expect("save succeeds");

        // Loading into the SAME world reuses the cached asset slots instead
        // of re-parsing the files.
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
        let reloaded_mesh = world
            .get_component::<Children>(same_world_roots[0])
            .unwrap()[0];
        assert_eq!(
            world
                .get_component::<MeshRenderer>(reloaded_mesh)
                .unwrap()
                .mesh,
            original_mesh_handle
        );
        assert_eq!(world.get_resource::<Assets<SplatCloud>>().unwrap().len(), 1);
        assert_eq!(world.get_resource::<Assets<Mesh>>().unwrap().len(), 1);

        // Fresh world: full roundtrip through the document.
        let mut fresh = World::new();
        fresh.register_hierarchy();
        fresh.insert_resource(editor_asset_server());
        fresh.insert_resource(Assets::<SplatCloud>::default());
        fresh.insert_resource(Assets::<Mesh>::default());
        let roots =
            load_scene_from_file(&mut fresh, &registry, &scene_path).expect("load succeeds");

        assert_eq!(roots.len(), 1);
        let loaded_splat = roots[0];
        assert_eq!(
            fresh.get_component::<Name>(loaded_splat).unwrap().as_str(),
            splat_path.file_name().unwrap().to_str().unwrap()
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
            Some(splat_path.display().to_string().as_str())
        );
        drop(clouds);

        let children = fresh.get_component::<Children>(loaded_splat).unwrap();
        assert_eq!(children.len(), 1);
        let loaded_mesh = children[0];
        assert_eq!(
            fresh.get_component::<Name>(loaded_mesh).unwrap().as_str(),
            mesh_path.file_name().unwrap().to_str().unwrap()
        );
        assert_eq!(
            fresh.get_component::<Transform>(loaded_mesh).unwrap(),
            &Transform::IDENTITY
        );
        assert_eq!(
            fresh
                .get_component::<ChildOf>(loaded_mesh)
                .unwrap()
                .parent(),
            loaded_splat
        );
        // The mesh entry stores only the path: color comes back as the
        // default, the handle resolves to the re-parsed mesh.
        let renderer = fresh.get_component::<MeshRenderer>(loaded_mesh).unwrap();
        assert_eq!(renderer.color, DEFAULT_MESH_COLOR);
        let meshes = fresh.get_resource::<Assets<Mesh>>().unwrap();
        assert_eq!(meshes.len(), 1);
        let mesh = meshes.get(&renderer.mesh.0).unwrap();
        assert_eq!(mesh.positions().len(), 3);
        assert_eq!(
            mesh.source(),
            Some(mesh_path.display().to_string().as_str())
        );
        drop(meshes);

        std::fs::remove_file(&splat_path).ok();
        std::fs::remove_file(&mesh_path).ok();
        std::fs::remove_file(&scene_path).ok();
    }
}
