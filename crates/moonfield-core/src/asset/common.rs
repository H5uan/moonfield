//! # Common Asset Types
//!
//! Defines common asset types used throughout the engine.



use super::{Asset, AssetHandle};

/// Texture asset wrapper.
#[derive(Debug, Clone)]
pub struct TextureAsset {
    /// Width of the texture.
    pub width: u32,
    /// Height of the texture.
    pub height: u32,
    /// Format of the texture.
    pub format: TextureFormat,
    /// Raw texture data.
    pub data: Vec<u8>,
}

/// Texture format enumeration.
#[derive(Debug, Clone)]
pub enum TextureFormat {
    Rgba8Unorm,
    Rgba8Srgb,
    Bgra8Unorm,
    Bgra8Srgb,
    R32Float,
    Rg32Float,
    Rgba32Float,
    Depth32Float,
}

impl Asset for TextureAsset {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Mesh asset containing vertex and index data.
#[derive(Debug, Clone)]
pub struct MeshAsset {
    /// Vertex buffer data.
    pub vertices: Vec<f32>,
    /// Index buffer data.
    pub indices: Vec<u32>,
    /// Vertex layout information.
    pub layout: VertexLayout,
}

/// Vertex layout descriptor.
#[derive(Debug, Clone)]
pub struct VertexLayout {
    /// Size of each vertex in bytes.
    pub stride: u32,
    /// Attributes for the vertex layout.
    pub attributes: Vec<VertexAttribute>,
}

/// Vertex attribute descriptor.
#[derive(Debug, Clone)]
pub struct VertexAttribute {
    /// Location of the attribute in the shader.
    pub location: u32,
    /// Offset of the attribute in the vertex.
    pub offset: u32,
    /// Format of the attribute.
    pub format: VertexFormat,
}

/// Vertex format enumeration.
#[derive(Debug, Clone)]
pub enum VertexFormat {
    Float32,
    Float32x2,
    Float32x3,
    Float32x4,
    Uint32,
    Uint32x2,
    Uint32x3,
    Uint32x4,
}

impl Asset for MeshAsset {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Material asset containing shader and property information.
#[derive(Debug, Clone)]
pub struct MaterialAsset {
    /// Handle to the shader used by this material.
    pub shader_handle: Option<String>, // Using string for now until we have proper shader assets
    /// Properties for the material.
    pub properties: MaterialProperties,
}

/// Material properties container.
#[derive(Debug, Clone)]
pub struct MaterialProperties {
    /// Albedo texture handle.
    pub albedo_texture: Option<AssetHandle<TextureAsset>>,
    /// Normal map texture handle.
    pub normal_texture: Option<AssetHandle<TextureAsset>>,
    /// Metallic-roughness texture handle.
    pub metallic_roughness_texture: Option<AssetHandle<TextureAsset>>,
    /// Base color factor.
    pub base_color_factor: [f32; 4],
    /// Metallic factor.
    pub metallic_factor: f32,
    /// Roughness factor.
    pub roughness_factor: f32,
}

impl Asset for MaterialAsset {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Shader asset wrapper.
#[derive(Debug, Clone)]
pub struct ShaderAsset {
    /// Shader source code or bytecode.
    pub source: Vec<u8>,
    /// Shader type (vertex, fragment, compute, etc.).
    pub shader_type: ShaderType,
    /// Entry point function name.
    pub entry_point: String,
}

/// Shader type enumeration.
#[derive(Debug, Clone)]
pub enum ShaderType {
    Vertex,
    Fragment,
    Compute,
    Geometry,
    Hull,
    Domain,
}

impl Asset for ShaderAsset {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

/// Animation clip asset.
#[derive(Debug, Clone)]
pub struct AnimationAsset {
    /// Name of the animation.
    pub name: String,
    /// Duration of the animation in seconds.
    pub duration: f32,
    /// Animation channels.
    pub channels: Vec<AnimationChannel>,
}

/// Animation channel containing keyframe data.
#[derive(Debug, Clone)]
pub struct AnimationChannel {
    /// Target path for the animation.
    pub target_path: String,
    /// Keyframes for translation.
    pub translation_keys: Vec<(f32, [f32; 3])>, // (time, translation)
    /// Keyframes for rotation.
    pub rotation_keys: Vec<(f32, [f32; 4])>,    // (time, quaternion)
    /// Keyframes for scale.
    pub scale_keys: Vec<(f32, [f32; 3])>,       // (time, scale)
}

impl Asset for AnimationAsset {
    fn type_id(&self) -> std::any::TypeId {
        std::any::TypeId::of::<Self>()
    }
    
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}