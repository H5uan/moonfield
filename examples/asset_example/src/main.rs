//! # Asset System Example
//!
//! Demonstrates usage of the asset management system.

use moonfield_core::asset::{AssetServer, AssetHandle, TextureAsset, TextureFormat, MeshAsset, VertexLayout, VertexAttribute, VertexFormat};
use moonfield_core::asset::loader::{TextureLoader, MeshLoader};

fn main() {
    // Initialize logging
    moonfield_core::logging::init_auto_logging().expect("Failed to initialize logging");

    // Create a new asset server
    let mut asset_server = AssetServer::new();

    // Register loaders for different asset types
    asset_server.register_loader::<TextureAsset>(Box::new(TextureLoader));
    asset_server.register_loader::<MeshAsset>(Box::new(MeshLoader));

    // Example 1: Loading a texture asset
    println!("Loading texture asset...");
    match asset_server.load::<TextureAsset>("assets/texture.png") {
        Ok(texture_handle) => {
            println!("Successfully loaded texture with handle: {:?}", texture_handle.as_handle());
            
            // Access the texture asset
            if let Some(texture) = asset_server.get(&texture_handle) {
                println!("Texture dimensions: {}x{}, format: {:?}", 
                    texture.width, texture.height, texture.format);
            }
        }
        Err(e) => {
            println!("Failed to load texture: {}", e);
            // Create a default texture instead
            let default_texture = TextureAsset {
                width: 128,
                height: 128,
                format: TextureFormat::Rgba8Unorm,
                data: vec![255; 128 * 128 * 4], // Simple white texture
            };
            let texture_handle = asset_server.add(default_texture);
            println!("Created default texture with handle: {:?}", texture_handle.as_handle());
        }
    }

    // Example 2: Loading a mesh asset
    println!("\nLoading mesh asset...");
    let default_mesh = MeshAsset {
        vertices: vec![
            -0.5, -0.5, 0.0,  // Vertex 1
             0.5, -0.5, 0.0,  // Vertex 2
             0.0,  0.5, 0.0,  // Vertex 3
        ],
        indices: vec![0, 1, 2],
        layout: VertexLayout {
            stride: 12, // 3 floats * 4 bytes each
            attributes: vec![
                VertexAttribute {
                    location: 0,
                    offset: 0,
                    format: VertexFormat::Float32x3,
                }
            ],
        },
    };
    
    let mesh_handle = asset_server.add(default_mesh);
    println!("Added mesh with handle: {:?}", mesh_handle.as_handle());

    // Access the mesh asset
    if let Some(mesh) = asset_server.get(&mesh_handle) {
        println!("Mesh has {} vertices and {} indices", 
            mesh.vertices.len() / 3, mesh.indices.len());
    }

    // Example 3: Working with asset handles
    println!("\nDemonstrating asset handle usage...");
    
    // Create multiple references to the same asset
    let mesh_handle_1 = mesh_handle.clone();
    let mesh_handle_2 = mesh_handle.clone();
    
    println!("All handles refer to the same asset:");
    println!("  Original: {:?}", mesh_handle.as_handle());
    println!("  Handle 1: {:?}", mesh_handle_1.as_handle());
    println!("  Handle 2: {:?}", mesh_handle_2.as_handle());
    
    // Verify they all access the same data
    if let Some(mesh) = asset_server.get(&mesh_handle_1) {
        println!("All handles access the same mesh data with {} vertices", 
            mesh.vertices.len() / 3);
    }

    println!("\nAsset management system demo completed!");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_asset_system_basic() {
        let mut asset_server = AssetServer::new();
        
        // Add a simple texture asset
        let texture = TextureAsset {
            width: 64,
            height: 64,
            format: TextureFormat::Rgba8Unorm,
            data: vec![255; 64 * 64 * 4],
        };
        
        let handle = asset_server.add(texture);
        
        // Verify the asset was added
        assert!(asset_server.contains(&handle));
        
        // Verify we can retrieve the asset
        let retrieved = asset_server.get(&handle).unwrap();
        assert_eq!(retrieved.width, 64);
        assert_eq!(retrieved.height, 64);
    }

    #[test]
    fn test_multiple_asset_types() {
        let mut asset_server = AssetServer::new();
        
        // Add a texture
        let texture = TextureAsset {
            width: 32,
            height: 32,
            format: TextureFormat::Rgba8Unorm,
            data: vec![128; 32 * 32 * 4],
        };
        let texture_handle = asset_server.add(texture);
        
        // Add a mesh
        let mesh = MeshAsset {
            vertices: vec![0.0, 0.0, 0.0],
            indices: vec![0],
            layout: VertexLayout {
                stride: 12,
                attributes: vec![VertexAttribute {
                    location: 0,
                    offset: 0,
                    format: VertexFormat::Float32x3,
                }]
            },
        };
        let mesh_handle = asset_server.add(mesh);
        
        // Verify both assets exist
        assert!(asset_server.contains(&texture_handle));
        assert!(asset_server.contains(&mesh_handle));
        
        // Verify we can access both
        assert!(asset_server.get(&texture_handle).is_some());
        assert!(asset_server.get(&mesh_handle).is_some());
    }
}