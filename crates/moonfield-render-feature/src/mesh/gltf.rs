//! Triangle-mesh import from standard glTF 2.0 files.
//!
//! v1 deliberately flattens the asset: **all** triangle primitives across all
//! meshes are merged into one positions + indices pair, with vertex offsets
//! applied to the indices. Non-indexed primitives get sequential indices.
//! POINTS primitives are skipped (those are splat clouds — see
//! [`crate::splat::io::gltf`]), as are node transforms and materials.

/// Errors returned by [`import_gltf_mesh`] / [`parse_gltf_mesh`].
#[derive(Debug, thiserror::Error)]
pub enum MeshGltfError {
    /// The glTF container or its buffers could not be imported.
    #[error("failed to import glTF: {0}")]
    Import(#[from] gltf::Error),
    /// The file holds no triangle geometry (it may be a splat cloud).
    #[error("glTF contains no triangle primitives")]
    NoTriangles,
    /// A triangle primitive without the mandatory `POSITION` attribute.
    #[error("triangle primitive has no POSITION attribute")]
    MissingPositions,
}

/// Import merged triangle geometry from in-memory glTF/GLB bytes.
///
/// External buffer references are not resolvable from a slice — use
/// [`super::Mesh::from_gltf_file`] for files that have them.
pub fn import_gltf_mesh(bytes: &[u8]) -> Result<(Vec<[f32; 3]>, Vec<u32>), MeshGltfError> {
    let (document, buffers, _images) = gltf::import_slice(bytes)?;
    parse_gltf_mesh(&document, &buffers)
}

/// Merge all triangle primitives of an imported glTF document into one
/// `(positions, indices)` pair.
pub fn parse_gltf_mesh(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Result<(Vec<[f32; 3]>, Vec<u32>), MeshGltfError> {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut found_triangles = false;

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            found_triangles = true;
            let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));
            let base = positions.len() as u32;
            let vertex_count = match reader.read_positions() {
                Some(iter) => {
                    positions.extend(iter);
                    positions.len() - base as usize
                }
                None => return Err(MeshGltfError::MissingPositions),
            };
            match reader.read_indices() {
                Some(read) => indices.extend(read.into_u32().map(|i| base + i)),
                None => indices.extend(base..base + vertex_count as u32),
            }
        }
    }

    if !found_triangles {
        return Err(MeshGltfError::NoTriangles);
    }
    Ok((positions, indices))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn u16_bytes(values: &[u16]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }

    /// Two meshes, one triangle each; the second mesh adds a quad. All
    /// POSITION accessors carry the mandatory min/max.
    fn triangle_and_quad_glb() -> Vec<u8> {
        let tri_pos = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        let tri_idx: [u16; 3] = [0, 1, 2];
        let quad_pos = [5.0, 5.0, 5.0, 6.0, 5.0, 5.0, 6.0, 6.0, 5.0, 5.0, 6.0, 5.0];
        let quad_idx: [u16; 6] = [0, 1, 2, 2, 3, 0];

        let mut bin = f32_bytes(&tri_pos);
        let tri_idx_off = bin.len();
        bin.extend_from_slice(&u16_bytes(&tri_idx));
        let quad_pos_off = bin.len();
        bin.extend_from_slice(&f32_bytes(&quad_pos));
        let quad_idx_off = bin.len();
        bin.extend_from_slice(&u16_bytes(&quad_idx));

        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [
                    {{"buffer": 0, "byteOffset": 0, "byteLength": 36}},
                    {{"buffer": 0, "byteOffset": {}, "byteLength": 6}},
                    {{"buffer": 0, "byteOffset": {}, "byteLength": 48}},
                    {{"buffer": 0, "byteOffset": {}, "byteLength": 12}}
                ],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                      "min": [0.0, 0.0, 0.0], "max": [1.0, 1.0, 0.0]}},
                    {{"bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR"}},
                    {{"bufferView": 2, "componentType": 5126, "count": 4, "type": "VEC3",
                      "min": [5.0, 5.0, 5.0], "max": [6.0, 6.0, 5.0]}},
                    {{"bufferView": 3, "componentType": 5123, "count": 6, "type": "SCALAR"}}
                ],
                "meshes": [
                    {{"primitives": [{{"attributes": {{"POSITION": 0}}, "indices": 1}}]}},
                    {{"primitives": [{{"attributes": {{"POSITION": 2}}, "indices": 3}}]}}
                ]
            }}"#,
            bin.len(),
            tri_idx_off,
            quad_pos_off,
            quad_idx_off
        );
        glb(&json, &bin)
    }

    #[test]
    fn test_merges_primitives_across_meshes_with_vertex_offsets() {
        let (positions, indices) = import_gltf_mesh(&triangle_and_quad_glb()).unwrap();
        assert_eq!(positions.len(), 7);
        assert_eq!(positions[0], [0.0, 0.0, 0.0]);
        assert_eq!(positions[3], [5.0, 5.0, 5.0]);
        // Triangle indices, then quad indices offset by 3.
        assert_eq!(indices, vec![0, 1, 2, 3, 4, 5, 5, 6, 3]);
    }

    #[test]
    fn test_non_indexed_primitive_gets_sequential_indices() {
        let pos = [0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 3.0, 0.0];
        let bin = f32_bytes(&pos);
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 36}}],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3",
                      "min": [0.0, 0.0, 0.0], "max": [2.0, 3.0, 0.0]}}
                ],
                "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}}}]}}]
            }}"#,
            bin.len()
        );
        let (positions, indices) = import_gltf_mesh(&glb(&json, &bin)).unwrap();
        assert_eq!(positions.len(), 3);
        assert_eq!(indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_aabb_covers_merged_geometry() {
        let (positions, indices) = import_gltf_mesh(&triangle_and_quad_glb()).unwrap();
        let mesh = crate::mesh::Mesh::new(positions, indices, None);
        assert_eq!(mesh.aabb(), ([0.0, 0.0, 0.0], [6.0, 6.0, 5.0]));
    }

    #[test]
    fn test_points_only_file_is_rejected() {
        let pos = [0.0, 0.0, 0.0];
        let bin = f32_bytes(&pos);
        let json = format!(
            r#"{{
                "asset": {{"version": "2.0"}},
                "buffers": [{{"byteLength": {}}}],
                "bufferViews": [{{"buffer": 0, "byteOffset": 0, "byteLength": 12}}],
                "accessors": [
                    {{"bufferView": 0, "componentType": 5126, "count": 1, "type": "VEC3",
                      "min": [0.0, 0.0, 0.0], "max": [0.0, 0.0, 0.0]}}
                ],
                "meshes": [{{"primitives": [{{"attributes": {{"POSITION": 0}}, "mode": 0}}]}}]
            }}"#,
            bin.len()
        );
        assert!(matches!(
            import_gltf_mesh(&glb(&json, &bin)),
            Err(MeshGltfError::NoTriangles)
        ));
    }

    #[test]
    fn test_invalid_bytes_are_rejected() {
        assert!(matches!(
            import_gltf_mesh(b"not a gltf"),
            Err(MeshGltfError::Import(_))
        ));
    }
}
