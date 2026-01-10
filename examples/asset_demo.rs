//! # Asset Management System Example
//!
//! Demonstrates the usage of the asset management system.

use moonfield_core::asset::{AssetServer, TextureAsset, TextureFormat, MeshAsset, VertexLayout, VertexAttribute, VertexFormat};
use moonfield_core::asset::loader::{TextureLoader, MeshLoader};

fn main() {
    // Initialize logging
    moonfield_core::logging::init_auto_logging().expect("Failed to initialize logging");

    println!("Moonfield Asset Management System Demo");
    println!("=====================================");

    // Create a new asset server
    let mut asset_server = AssetServer::new();

    // Register loaders for different asset types
    asset_server.register_loader::<TextureAsset>(Box::new(TextureLoader));
    asset_server.register_loader::<MeshAsset>(Box::new(MeshLoader));

    // Example 1: Adding a texture asset directly
    println!("\n1. Adding a texture asset directly:");
    let texture = TextureAsset {
        width: 512,
        height: 512,
        format: TextureFormat::Rgba8Unorm,
        data: vec![255; 512 * 512 * 4], // Simple texture data
    };
    let texture_handle = asset_server.add(texture);
    println!("   Texture added with handle: {:?}", texture_handle.as_handle());

    // Access the texture asset
    if let Some(texture) = asset_server.get(&texture_handle) {
        println!("   Texture dimensions: {}x{}", texture.width, texture.height);
    }

    // Example 2: Adding a mesh asset
    println!("\n2. Adding a mesh asset:");
    let mesh = MeshAsset {
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
    let mesh_handle = asset_server.add(mesh);
    println!("   Mesh added with handle: {:?}", mesh_handle.as_handle());

    // Access the mesh asset
    if let Some(mesh) = asset_server.get(&mesh_handle) {
        println!("   Mesh has {} vertices and {} indices", 
            mesh.vertices.len() / 3, mesh.indices.len());
    }

    // Example 3: Demonstrating asset handle usage
    println!("\n3. Asset handle functionality:");
    let mesh_handle_1 = mesh_handle.clone();
    let mesh_handle_2 = mesh_handle.clone();
    
    println!("   All handles refer to the same asset:");
    println!("     Original: {:?}", mesh_handle.as_handle());
    println!("     Handle 1: {:?}", mesh_handle_1.as_handle());
    println!("     Handle 2: {:?}", mesh_handle_2.as_handle());
    
    // Verify they all access the same data
    if let Some(mesh) = asset_server.get(&mesh_handle_1) {
        println!("   All handles access the same mesh data with {} vertices", 
            mesh.vertices.len() / 3);
    }

    println!("\nAsset management system demo completed successfully!");
    println!("The system provides:");
    println!("  - Centralized asset management with AssetServer");
    println!("  - Type-safe asset handles for safe resource access");
    println!("  - Support for multiple asset types (textures, meshes, materials, etc.)");
    println!("  - Asset loading from files with registered loaders");
    println!("  - Proper memory management through the handle system");
}