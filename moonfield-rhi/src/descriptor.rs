use crate::{
    AccelerationStructureFlags, AccelerationStructureGeometryFlags,
    AccelerationStructureUpdateMode, AddressMode, BufferAddress, BufferUsages,
    CompareFunction, Extent3d, ExternalTextureFormat,
    ExternalTextureTransferFunction, FilterMode, IndexFormat, MemoryHints,
    MipmapFilterMode, QueryType, SamplerBorderColor, TextureAspect,
    TextureDimension, TextureFormat, TextureUsages, TextureViewDimension,
    VertexFormat,
};

#[derive(Clone, Debug, Default)]
pub struct DeviceDescriptor<L> {
    pub label: L,
    pub memory_hints: MemoryHints,
}

impl<L> DeviceDescriptor<L> {
    /// Takes a closure and maps the label of the device descriptor into another
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> DeviceDescriptor<K> {
        DeviceDescriptor {
            label: fun(&self.label),
            memory_hints: self.memory_hints.clone(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BufferDescriptor<L> {
    /// Debug label of a buffer. This will show up in graphics debuggers for easy identification.
    pub label: L,
    /// Size of a buffer, in bytes.
    pub size: BufferAddress,
    /// Usages of a buffer. If the buffer is used in any way that isn't specified here, the operation
    /// will panic.
    ///
    /// Specifying only usages the application will actually perform may increase performance.
    /// see [`DownlevelFlags::UNRESTRICTED_INDEX_BUFFER`] for more information.
    pub usage: BufferUsages,
    /// Allows a buffer to be mapped immediately after they are made. It does not have to be [`BufferUsages::MAP_READ`] or
    /// [`BufferUsages::MAP_WRITE`], all buffers are allowed to be mapped at creation.
    ///
    /// If this is `true`, [`size`](#structfield.size) must be a multiple of
    /// [`COPY_BUFFER_ALIGNMENT`].
    pub mapped_at_creation: bool,
}

impl<L> BufferDescriptor<L> {
    /// Takes a closure and maps the label of the buffer descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> BufferDescriptor<K> {
        BufferDescriptor {
            label: fun(&self.label),
            size: self.size,
            usage: self.usage,
            mapped_at_creation: self.mapped_at_creation,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CommandEncoderDescriptor<L> {
    /// Debug label for the command encoder. This will show up in graphics debuggers for easy identification.
    pub label: L,
}

impl<L> CommandEncoderDescriptor<L> {
    /// Takes a closure and maps the label of the command encoder descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> CommandEncoderDescriptor<K> {
        CommandEncoderDescriptor { label: fun(&self.label) }
    }
}

impl<T> Default for CommandEncoderDescriptor<Option<T>> {
    fn default() -> Self {
        Self { label: None }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TextureViewDescriptor<L> {
    /// Debug label of the texture view. This will show up in graphics debuggers for easy identification.
    pub label: L,
    /// Format of the texture view. Either must be the same as the texture format or in the list
    /// of `view_formats` in the texture's descriptor.
    pub format: Option<TextureFormat>,
    /// The dimension of the texture view. For 1D textures, this must be `D1`. For 2D textures it must be one of
    /// `D2`, `D2Array`, `Cube`, and `CubeArray`. For 3D textures it must be `D3`
    pub dimension: Option<TextureViewDimension>,
    /// The allowed usage(s) for the texture view. Must be a subset of the usage flags of the texture.
    /// If not provided, defaults to the full set of usage flags of the texture.
    pub usage: Option<TextureUsages>,
    /// Aspect of the texture. Color textures must be [`TextureAspect::All`].
    pub aspect: TextureAspect,
    /// Base mip level.
    pub base_mip_level: u32,
    /// Mip level count.
    /// If `Some(count)`, `base_mip_level + count` must be less or equal to underlying texture mip count.
    /// If `None`, considered to include the rest of the mipmap levels, but at least 1 in total.
    pub mip_level_count: Option<u32>,
    /// Base array layer.
    pub base_array_layer: u32,
    /// Layer count.
    /// If `Some(count)`, `base_array_layer + count` must be less or equal to the underlying array count.
    /// If `None`, considered to include the rest of the array layers, but at least 1 in total.
    pub array_layer_count: Option<u32>,
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TextureDescriptor<L, V> {
    /// Debug label of the texture. This will show up in graphics debuggers for easy identification.
    pub label: L,
    /// Size of the texture. All components must be greater than zero. For a
    /// regular 1D/2D texture, the unused sizes will be 1. For 2DArray textures,
    /// Z is the number of 2D textures in that array.
    pub size: Extent3d,
    /// Mip count of texture. For a texture with no extra mips, this must be 1.
    pub mip_level_count: u32,
    /// Sample count of texture. If this is not 1, texture must have [`BindingType::Texture::multisampled`] set to true.
    pub sample_count: u32,
    /// Dimensions of the texture.
    pub dimension: TextureDimension,
    /// Format of the texture.
    pub format: TextureFormat,
    /// Allowed usages of the texture. If used in other ways, the operation will panic.
    pub usage: TextureUsages,
    /// Specifies what view formats will be allowed when calling `Texture::create_view` on this texture.
    ///
    /// View formats of the same format as the texture are always allowed.
    ///
    /// Note: currently, only the srgb-ness is allowed to change. (ex: `Rgba8Unorm` texture + `Rgba8UnormSrgb` view)
    pub view_formats: V,
}

impl<L, V> TextureDescriptor<L, V> {
    /// Takes a closure and maps the label of the texture descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> TextureDescriptor<K, V>
    where
        V: Clone, {
        TextureDescriptor {
            label: fun(&self.label),
            size: self.size,
            mip_level_count: self.mip_level_count,
            sample_count: self.sample_count,
            dimension: self.dimension,
            format: self.format,
            usage: self.usage,
            view_formats: self.view_formats.clone(),
        }
    }

    /// Maps the label and view formats of the texture descriptor into another.
    #[must_use]
    pub fn map_label_and_view_formats<K, M>(
        &self, l_fun: impl FnOnce(&L) -> K, v_fun: impl FnOnce(V) -> M,
    ) -> TextureDescriptor<K, M>
    where
        V: Clone, {
        TextureDescriptor {
            label: l_fun(&self.label),
            size: self.size,
            mip_level_count: self.mip_level_count,
            sample_count: self.sample_count,
            dimension: self.dimension,
            format: self.format,
            usage: self.usage,
            view_formats: v_fun(self.view_formats.clone()),
        }
    }

    #[must_use]
    pub fn mip_level_size(&self, level: u32) -> Option<Extent3d> {
        if level >= self.mip_level_count {
            return None;
        }

        Some(self.size.mip_level_size(level, self.dimension))
    }

    /// Computes the render extent of this texture.
    ///
    #[must_use]
    pub fn compute_render_extent(
        &self, mip_level: u32, plane: Option<u32>,
    ) -> Extent3d {
        let width = self.size.width >> mip_level;
        let height = self.size.height >> mip_level;

        let (width, height) = match (self.format, plane) {
            (TextureFormat::NV12 | TextureFormat::P010, Some(0)) => {
                (width, height)
            }
            (TextureFormat::NV12 | TextureFormat::P010, Some(1)) => {
                (width / 2, height / 2)
            }
            _ => {
                debug_assert!(!self.format.is_multi_planar_format());
                (width, height)
            }
        };

        Extent3d { width, height, depth_or_array_layers: 1 }
    }

    /// Returns the number of array layers.
    ///
    #[must_use]
    pub fn array_layer_count(&self) -> u32 {
        match self.dimension {
            TextureDimension::D1 | TextureDimension::D3 => 1,
            TextureDimension::D2 => self.size.depth_or_array_layers,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalTextureDescriptor<L> {
    /// Debug label of the external texture. This will show up in graphics
    /// debuggers for easy identification.
    pub label: L,

    /// Width of the external texture.
    pub width: u32,

    /// Height of the external texture.
    pub height: u32,

    /// Format of the external texture.
    pub format: ExternalTextureFormat,

    /// 4x4 column-major matrix with which to convert sampled YCbCr values
    /// to RGBA.
    /// This is ignored when `format` is [`ExternalTextureFormat::Rgba`].
    pub yuv_conversion_matrix: [f32; 16],

    /// 3x3 column-major matrix to transform linear RGB values in the source
    /// color space to linear RGB values in the destination color space. In
    /// combination with [`Self::src_transfer_function`] and
    /// [`Self::dst_transfer_function`] this can be used to ensure that
    /// [`ImageSample`] and [`ImageLoad`] operations return values in the
    /// desired destination color space rather than the source color space of
    /// the underlying planes.
    ///
    pub gamut_conversion_matrix: [f32; 9],

    /// Transfer function for the source color space. The *inverse* of this
    /// will be applied to decode non-linear RGB to linear RGB in the source
    /// color space.
    pub src_transfer_function: ExternalTextureTransferFunction,

    /// Transfer function for the destination color space. This will be applied
    /// to encode linear RGB to non-linear RGB in the destination color space.
    pub dst_transfer_function: ExternalTextureTransferFunction,

    /// Transform to apply to [`ImageSample`] coordinates.
    ///
    /// This is a 3x2 column-major matrix representing an affine transform from
    /// normalized texture coordinates to the normalized coordinates that should
    /// be sampled from the external texture's underlying plane(s).
    ///
    /// This transform may scale, translate, flip, and rotate in 90-degree
    /// increments, but the result of transforming the rectangle (0,0)..(1,1)
    /// must be an axis-aligned rectangle that falls within the bounds of
    /// (0,0)..(1,1).
    ///
    pub sample_transform: [f32; 6],

    /// Transform to apply to [`ImageLoad`] coordinates.
    ///
    /// This is a 3x2 column-major matrix representing an affine transform from
    /// non-normalized texel coordinates to the non-normalized coordinates of
    /// the texel that should be loaded from the external texture's underlying
    /// plane 0. For planes 1 and 2, if present, plane 0's coordinates are
    /// scaled according to the textures' relative sizes.
    ///
    /// This transform may scale, translate, flip, and rotate in 90-degree
    /// increments, but the result of transforming the rectangle (0,0)..([`width`],
    /// [`height`]) must be an axis-aligned rectangle that falls within the bounds
    /// of (0,0)..([`width`], [`height`]).
    ///
    pub load_transform: [f32; 6],
}

impl<L> ExternalTextureDescriptor<L> {
    /// Takes a closure and maps the label of the external texture descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> ExternalTextureDescriptor<K> {
        ExternalTextureDescriptor {
            label: fun(&self.label),
            width: self.width,
            height: self.height,
            format: self.format,
            yuv_conversion_matrix: self.yuv_conversion_matrix,
            sample_transform: self.sample_transform,
            load_transform: self.load_transform,
            gamut_conversion_matrix: self.gamut_conversion_matrix,
            src_transfer_function: self.src_transfer_function,
            dst_transfer_function: self.dst_transfer_function,
        }
    }

    /// The number of underlying planes used by the external texture.
    pub fn num_planes(&self) -> usize {
        match self.format {
            ExternalTextureFormat::Rgba => 1,
            ExternalTextureFormat::Nv12 => 2,
            ExternalTextureFormat::Yu12 => 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SamplerDescriptor<L> {
    /// Debug label of the sampler. This will show up in graphics debuggers for easy identification.
    pub label: L,
    /// How to deal with out of bounds accesses in the u (i.e. x) direction
    pub address_mode_u: AddressMode,
    /// How to deal with out of bounds accesses in the v (i.e. y) direction
    pub address_mode_v: AddressMode,
    /// How to deal with out of bounds accesses in the w (i.e. z) direction
    pub address_mode_w: AddressMode,
    /// How to filter the texture when it needs to be magnified (made larger)
    pub mag_filter: FilterMode,
    /// How to filter the texture when it needs to be minified (made smaller)
    pub min_filter: FilterMode,
    /// How to filter between mip map levels
    pub mipmap_filter: MipmapFilterMode,
    /// Minimum level of detail (i.e. mip level) to use
    pub lod_min_clamp: f32,
    /// Maximum level of detail (i.e. mip level) to use
    pub lod_max_clamp: f32,
    /// If this is enabled, this is a comparison sampler using the given comparison function.
    pub compare: Option<CompareFunction>,
    /// Must be at least 1. If this is not 1, all filter modes must be linear.
    pub anisotropy_clamp: u16,
    /// Border color to use when `address_mode` is [`AddressMode::ClampToBorder`]
    pub border_color: Option<SamplerBorderColor>,
}

impl<L: Default> Default for SamplerDescriptor<L> {
    fn default() -> Self {
        Self {
            label: Default::default(),
            address_mode_u: Default::default(),
            address_mode_v: Default::default(),
            address_mode_w: Default::default(),
            mag_filter: Default::default(),
            min_filter: Default::default(),
            mipmap_filter: Default::default(),
            lod_min_clamp: 0.0,
            lod_max_clamp: 32.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RenderBundleDescriptor<L> {
    /// Debug label of the render bundle encoder. This will show up in graphics debuggers for easy identification.
    pub label: L,
}

impl<L> RenderBundleDescriptor<L> {
    /// Takes a closure and maps the label of the render bundle descriptor into another.
    #[must_use]
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> RenderBundleDescriptor<K> {
        RenderBundleDescriptor { label: fun(&self.label) }
    }
}

impl<T> Default for RenderBundleDescriptor<Option<T>> {
    fn default() -> Self {
        Self { label: None }
    }
}

#[derive(Clone, Debug)]
pub struct QuerySetDescriptor<L> {
    /// Debug label for the query set.
    pub label: L,
    /// Kind of query that this query set should contain.
    pub ty: QueryType,
    /// Total count of queries the set contains. Must not be zero.
    /// Must not be greater than [`QUERY_SET_MAX_QUERIES`].
    pub count: u32,
}

impl<L> QuerySetDescriptor<L> {
    /// Takes a closure and maps the label of the query set descriptor into another.
    #[must_use]
    pub fn map_label<'a, K>(
        &'a self, fun: impl FnOnce(&'a L) -> K,
    ) -> QuerySetDescriptor<K> {
        QuerySetDescriptor {
            label: fun(&self.label),
            ty: self.ty,
            count: self.count,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// Descriptor for all size defining attributes of a single triangle geometry inside a bottom level acceleration structure.
pub struct BlasTriangleGeometrySizeDescriptor {
    /// Format of a vertex position, must be [`VertexFormat::Float32x3`]
    /// with just [`Features::EXPERIMENTAL_RAY_QUERY`]
    /// but [`Features::EXTENDED_ACCELERATION_STRUCTURE_VERTEX_FORMATS`] adds more.
    pub vertex_format: VertexFormat,
    /// Number of vertices.
    pub vertex_count: u32,
    /// Format of an index. Only needed if an index buffer is used.
    /// If `index_format` is provided `index_count` is required.
    pub index_format: Option<IndexFormat>,
    /// Number of indices. Only needed if an index buffer is used.
    /// If `index_count` is provided `index_format` is required.
    pub index_count: Option<u32>,
    /// Flags for the geometry.
    pub flags: AccelerationStructureGeometryFlags,
}

#[derive(Clone, Debug)]
/// Descriptor for all size defining attributes of all geometries inside a bottom level acceleration structure.
pub enum BlasGeometrySizeDescriptors {
    /// Triangle geometry version.
    Triangles {
        /// Descriptor for each triangle geometry.
        descriptors: Vec<BlasTriangleGeometrySizeDescriptor>,
    },
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Descriptor for creating a bottom level acceleration structure.
pub struct CreateBlasDescriptor<L> {
    /// Label for the bottom level acceleration structure.
    pub label: L,
    /// Flags for the bottom level acceleration structure.
    pub flags: AccelerationStructureFlags,
    /// Update mode for the bottom level acceleration structure.
    pub update_mode: AccelerationStructureUpdateMode,
}

impl<L> CreateBlasDescriptor<L> {
    /// Takes a closure and maps the label of the blas descriptor into another.
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> CreateBlasDescriptor<K> {
        CreateBlasDescriptor {
            label: fun(&self.label),
            flags: self.flags,
            update_mode: self.update_mode,
        }
    }
}

#[repr(C)]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
/// Descriptor for creating a top level acceleration structure.
pub struct CreateTlasDescriptor<L> {
    /// Label for the top level acceleration structure.
    pub label: L,
    /// Number of instances that can be stored in the acceleration structure.
    pub max_instances: u32,
    /// Flags for the bottom level acceleration structure.
    pub flags: AccelerationStructureFlags,
    /// Update mode for the bottom level acceleration structure.
    pub update_mode: AccelerationStructureUpdateMode,
}

impl<L> CreateTlasDescriptor<L> {
    /// Takes a closure and maps the label of the blas descriptor into another.
    pub fn map_label<K>(
        &self, fun: impl FnOnce(&L) -> K,
    ) -> CreateTlasDescriptor<K> {
        CreateTlasDescriptor {
            label: fun(&self.label),
            flags: self.flags,
            update_mode: self.update_mode,
            max_instances: self.max_instances,
        }
    }
}
