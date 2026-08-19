//! Synchronous scene loading for the editor: PLY file → `SplatCloud` asset →
//! entity holding a [`SplatCloudHandle`] component.
//!
//! Factored as a plain world-mutating function (no egui, no winit) so the
//! PLY → asset → entity flow is unit-testable without a display.

use std::path::Path;

use moonfield_asset::Assets;
use moonfield_ecs::{Entity, Name, World};
use moonfield_math::Transform;
use moonfield_renderer::splat::cloud::{SplatCloud, SplatCloudHandle};

/// Load a 3DGS PLY file into the world synchronously: parse it, insert it
/// into the `Assets<SplatCloud>` resource (created if missing), and spawn an
/// entity named after the file holding the handle component.
///
/// Returns the new entity, or a human-readable error.
pub fn load_splat_cloud(world: &mut World, path: &Path) -> Result<Entity, String> {
    let cloud = SplatCloud::from_ply_file(path).map_err(|e| e.to_string())?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Splat Cloud".to_string());

    if !world.contains_resource::<Assets<SplatCloud>>() {
        world.insert_resource(Assets::<SplatCloud>::default());
    }
    let handle = world
        .get_resource_mut::<Assets<SplatCloud>>()
        .expect("Assets<SplatCloud> was just ensured")
        .add(cloud);

    Ok(world.spawn((
        Name::new(name),
        Transform::IDENTITY,
        SplatCloudHandle(handle),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a minimal but valid 1-vertex binary little-endian 3DGS PLY
    /// (all properties the parser requires) into a temp file.
    fn write_test_ply() -> std::path::PathBuf {
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
            "moonfield-editor-test-{}-{}.ply",
            std::process::id(),
            "splat"
        ));
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn test_ply_to_asset_to_entity_flow() {
        let path = write_test_ply();
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
}
