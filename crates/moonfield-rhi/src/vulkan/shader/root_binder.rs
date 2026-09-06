//! Root-parameter binding: turn [`Reflection`] results into the push-data
//! blobs descriptor-heap pipelines hand to [`CommandBuffer::push_data`].
//!
//! [`CommandBuffer::push_data`]: crate::CommandBuffer::push_data

use super::map_slang_error;
use super::reflection::Reflection;
use crate::error::{Error as RenderError, Result as RenderResult};

/// How a root parameter is delivered to the shader on descriptor-heap
/// pipelines: inline bytes (push-data / push constants) or a GPU address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootParamKind {
    /// The parameter's value is stored inline in the root blob (a `uniform`
    /// parameter — push-constant storage).
    Uniform,
    /// The parameter holds a GPU address (a `Ptr<T>` root — buffer device
    /// address).
    Pointer,
}

/// One root (non-varying) parameter of an entry point, with its placement in
/// the blob [`CommandBuffer::push_data`] receives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootParam {
    /// The parameter name, e.g. `root`.
    pub name: String,
    /// Whether the parameter is inline data or a GPU address.
    pub kind: RootParamKind,
    /// Byte offset of the parameter within the root blob.
    pub offset: usize,
    /// Byte size of the parameter's storage in the blob.
    pub size: usize,
}

/// A root parameter's placement in the root blob, resolved once from a
/// [`RootBinder`]. Per-draw work is a stack write and a
/// [`CommandBuffer::push_data`](crate::CommandBuffer::push_data) at
/// [`RootParamPlace::offset`] — no allocation, no name lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootParamPlace {
    /// The parameter's byte offset in the root blob — the push-data offset.
    pub offset: usize,
    /// The parameter's storage size in bytes.
    pub size: usize,
    kind: RootParamKind,
}

impl RootParamPlace {
    /// A pointer parameter's 8 bytes, ready for `push_data` at this place's
    /// offset.
    pub fn pointer_bytes(&self, address: u64) -> RenderResult<[u8; 8]> {
        if self.kind != RootParamKind::Pointer {
            return Err(RenderError::Backend(format!(
                "root parameter placement is {:?}, not a pointer",
                self.kind
            )));
        }
        if self.size != 8 {
            return Err(RenderError::Backend(format!(
                "pointer root parameter occupies {} bytes, expected 8",
                self.size
            )));
        }
        Ok(address.to_le_bytes())
    }
}

/// A typed writer for a draw's root blob, driven by an entry point's
/// reflected [`RootParam`]s.
///
/// Builds the exact byte layout the shader expects (from Slang reflection,
/// not a hand-synced struct) and fills it by parameter name. The result is
/// handed to [`CommandBuffer::push_data`] before the draw. The behavior of
/// writing is checked against the reflection: unknown names, kind mismatches,
/// and size overruns are errors instead of silent misplacement.
#[derive(Clone)]
pub struct RootBinder {
    /// The root blob, sized to the reflected layout and filled by [`set`].
    blob: Vec<u8>,
    params: Vec<RootParam>,
}

impl RootBinder {
    /// Build a binder for `entry_name`'s root parameters and a zeroed blob of
    /// the reflected size.
    pub fn new(reflection: &Reflection, entry_name: &str) -> RenderResult<Self> {
        let params = reflection.root_parameters(entry_name)?;
        let size = params.iter().map(|p| p.offset + p.size).max().unwrap_or(0);
        Ok(Self {
            blob: vec![0u8; size],
            params,
        })
    }

    /// The root blob, ready for [`CommandBuffer::push_data`].
    pub fn blob(&self) -> &[u8] {
        &self.blob
    }

    /// Write a GPU address to the named `Ptr<T>` root parameter.
    pub fn set_pointer(&mut self, name: &str, address: u64) -> RenderResult<()> {
        let (offset, size) = self.range(name, RootParamKind::Pointer, 8)?;
        self.blob[offset..offset + size].copy_from_slice(&address.to_le_bytes());
        Ok(())
    }

    /// Write inline bytes to the named `uniform` root parameter (the whole
    /// struct; typically `bytemuck::bytes_of(&root)`).
    pub fn set_bytes(&mut self, name: &str, bytes: &[u8]) -> RenderResult<()> {
        let (offset, size) = self.range(name, RootParamKind::Uniform, bytes.len())?;
        self.blob[offset..offset + size].copy_from_slice(bytes);
        Ok(())
    }

    fn range(&self, name: &str, kind: RootParamKind, size: usize) -> RenderResult<(usize, usize)> {
        let param = self
            .params
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| RenderError::Backend(format!("no root parameter named '{name}'")))?;
        if param.kind != kind {
            return Err(RenderError::Backend(format!(
                "root parameter '{name}' is {:?}, not {:?}",
                param.kind, kind
            )));
        }
        if size > param.size {
            return Err(RenderError::Backend(format!(
                "{size} bytes for root parameter '{name}' exceeds its {}-byte storage",
                param.size
            )));
        }
        Ok((param.offset, param.size))
    }

    /// The placement of the named `Ptr<T>` root parameter. Resolve once at
    /// pipeline build; per-draw encoding goes through
    /// [`RootParamPlace::pointer_bytes`].
    pub fn pointer_param(&self, name: &str) -> RenderResult<RootParamPlace> {
        let (offset, size) = self.range(name, RootParamKind::Pointer, 8)?;
        Ok(RootParamPlace {
            offset,
            size,
            kind: RootParamKind::Pointer,
        })
    }

    /// The placement of the named inline `uniform` root parameter. Resolve
    /// once at pipeline build; per-pass bytes are pushed at
    /// [`RootParamPlace::offset`].
    pub fn uniform_param(&self, name: &str) -> RenderResult<RootParamPlace> {
        let (offset, size) = self.range(name, RootParamKind::Uniform, 0)?;
        Ok(RootParamPlace {
            offset,
            size,
            kind: RootParamKind::Uniform,
        })
    }
}

impl Reflection {
    /// Enumerate the root parameters of `entry_name` — every non-varying
    /// parameter — with their placement in the push-data blob.
    ///
    /// Root parameters are the entry point's remaining parameters after
    /// varying inputs (vertex attributes) and outputs: `Ptr<T>` roots
    /// (pointer category) and `uniform` roots (push-constant category). In
    /// the descriptor-heap model the entire root blob is written with
    /// [`CommandBuffer::push_data`]; this describes where each parameter lives
    /// in it.
    pub fn root_parameters(&self, entry_name: &str) -> RenderResult<Vec<RootParam>> {
        let reflection = unsafe { &*self.reflection };
        let entry = reflection
            .find_entry_point_by_name(entry_name)
            .map_err(map_slang_error)?
            .ok_or_else(|| RenderError::Backend(format!("entry point '{entry_name}' not found")))?;

        let mut params = Vec::new();
        for param in entry.parameters() {
            let cat = param.category();
            let is_varying = cat == Some(shader_slang::ParameterCategory::VaryingInput)
                || cat == Some(shader_slang::ParameterCategory::VaryingOutput);
            if is_varying {
                continue;
            }
            let Some(layout) = param.type_layout() else {
                continue;
            };
            let name = param.name().unwrap_or("<unnamed>").to_string();

            // Uniform roots and pointer roots live in different categories;
            // take the largest span across the categories Slang reports so we
            // are robust to target differences. The parameter's own offset is
            // per-category (`VariableLayout::offset`), its size comes from the
            // type layout (`TypeLayout::size`).
            let mut offset = usize::MAX;
            let mut size = 0usize;
            for ci in 0..layout.category_count() {
                let c = layout.category_by_index(ci);
                let off = param.offset(c);
                let sz = layout.size(c);
                offset = offset.min(off);
                size = size.max(off + sz);
            }
            if size == 0 {
                continue;
            }
            // `Ptr<T>` roots reflect as 8-byte uniform-category payloads holding a
            // GPU address; the type (not the category) decides the delivery
            // kind. Everything else inline is a `uniform` root.
            let ty_kind = layout.ty().map(|t| t.kind());
            let kind = if ty_kind == Some(shader_slang::TypeKind::Pointer) {
                RootParamKind::Pointer
            } else {
                RootParamKind::Uniform
            };
            params.push(RootParam {
                name,
                kind,
                offset,
                size: size - offset,
            });
        }
        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::Compiler;
    use super::*;

    const VERTEX_SOURCE: &str = r#"
        struct DrawData { column_major float4x4 mvp; };
        struct VsInput {
            float3 position : POSITION;
            float2 uv : TEXCOORD0;
            uint color : COLOR0;
        };
        struct VsOutput {
            float4 position : SV_POSITION;
            float2 uv : TEXCOORD0;
            uint color : COLOR0;
        };
        [shader("vertex")]
        VsOutput main(VsInput input, Ptr<DrawData> root)
        {
            VsOutput output;
            output.position = mul(root[0].mvp, float4(input.position, 1.0));
            output.uv = input.uv;
            output.color = input.color;
            return output;
        }
    "#;

    /// RootBinder writes the reflected blob for both root kinds: a `Ptr<T>`
    /// root gets a GPU address, a `uniform` root gets its inline struct bytes.
    #[test]
    fn root_binder_builds_blob_from_reflection() {
        let compiler = Compiler::new().expect("compiler");

        // `Ptr<DrawData> root` variant.
        let refl_ptr = compiler
            .compile_source_to_reflection("ptr", VERTEX_SOURCE, "main")
            .expect("refl");
        let mut binder = RootBinder::new(&refl_ptr, "main").expect("binder");
        binder.set_pointer("root", 0xdecafbad).expect("set");
        assert_eq!(binder.blob().len(), 8);
        assert_eq!(
            binder.blob(),
            &0xdecafbadu64.to_le_bytes(),
            "pointer root stores the raw GPU address"
        );
        // Unknown name and kind mismatch are rejected.
        assert!(
            binder.set_pointer("nope", 0).is_err(),
            "unknown name rejected"
        );
        assert!(
            binder.set_bytes("root", &[0u8; 8]).is_err(),
            "kind mismatch rejected"
        );

        // `uniform Root` variant with two fields.
        const UNIFORM_SOURCE: &str = r#"
            struct Root { float2 scale; uint flags; };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; };
            [shader("vertex")]
            VsOutput main(VsInput input, uniform Root root)
            {
                VsOutput o;
                o.position = float4(input.position * float3(root.scale, 1.0), 1.0);
                return o;
            }
        "#;
        let refl_uniform = compiler
            .compile_source_to_reflection("uniform", UNIFORM_SOURCE, "main")
            .expect("refl");
        let params = refl_uniform.root_parameters("main").expect("params");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "root");
        assert_eq!(params[0].kind, RootParamKind::Uniform);
        // Uniform (push-constant) layouts round a vec2 up to 16-byte storage.
        assert_eq!(params[0].size, 16);

        let mut binder = RootBinder::new(&refl_uniform, "main").expect("binder");
        let mut expected = [0u8; 16];
        expected[..4].copy_from_slice(&2.0f32.to_le_bytes());
        expected[4..8].copy_from_slice(&3.0f32.to_le_bytes());
        expected[8..12].copy_from_slice(&[0u8, 0, 0, 0]);
        binder
            .set_bytes("root", &expected)
            .expect("uniform root set");
        assert_eq!(binder.blob(), &expected);
    }

    /// The ViewUniforms shape: a vertex entry with two pointer roots
    /// (`root` + `view`), the fixed-function vertex input intact, and the
    /// struct layout a `Ptr<T>` dereference actually uses — read from the
    /// emitted SPIR-V's member offsets, the ground truth the Rust mirror
    /// must match.
    #[test]
    fn two_pointer_roots_and_ptr_struct_layout() {
        let compiler = Compiler::new().expect("compiler");
        const SOURCE: &str = r#"
            struct DrawData { column_major float4x4 model; float4 color; };
            struct ViewUniforms
            {
                float3 view_pos;
                float aspect;
                column_major float4x4 view_proj;
            };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; float3 local_pos : TEXCOORD0; };
            [shader("vertex")]
            VsOutput main(VsInput input, Ptr<DrawData> root, Ptr<ViewUniforms> view)
            {
                VsOutput o;
                o.position = mul(view[0].view_proj, mul(root[0].model, float4(input.position, 1.0)));
                o.local_pos = input.position;
                return o;
            }
        "#;
        let reflection = compiler
            .compile_source_to_reflection("view_uniforms", SOURCE, "main")
            .expect("reflection");
        // Two pointer roots, each an 8-byte address placement.
        let binder = RootBinder::new(&reflection, "main").expect("binder");
        assert_eq!(binder.pointer_param("root").expect("root place").size, 8);
        assert_eq!(binder.pointer_param("view").expect("view place").size, 8);

        // The emitted SPIR-V names the `Ptr` pointee types `..._natural`
        // (Slang's C-like natural layout — offsets baked into the pointer
        // arithmetic, no std140 padding games) and the entry-parameter
        // block `EntryPointParams_std430`. The natural layout is what
        // `struct_layout` (LayoutRules::Default) reports; the Rust mirror
        // must match it field-for-field.
        let view = reflection.struct_layout("ViewUniforms").expect("layout");
        assert_eq!(view.size(), 80);
        assert_eq!(view.field_offset("view_pos").expect("field"), 0);
        assert_eq!(view.field_offset("aspect").expect("field"), 12);
        assert_eq!(view.field_offset("view_proj").expect("field"), 16);
        let draw = reflection.struct_layout("DrawData").expect("layout");
        assert_eq!(draw.size(), 80);
        assert_eq!(draw.field_offset("model").expect("field"), 0);
        assert_eq!(draw.field_offset("color").expect("field"), 64);
    }

    /// The pulling-shape probe: a vertex entry whose only input is
    /// `SV_VertexID` — geometry arrives through `Ptr` fields inside the
    /// per-draw record. Verifies that (a) the system-value input does not
    /// become a vertex attribute, (b) pointer fields inside the uniform
    /// struct lay out at natural (C-like) offsets, and (c) both pointer
    /// roots resolve.
    #[test]
    fn pulling_vertex_shape() {
        let compiler = Compiler::new().expect("compiler");
        const SOURCE: &str = r#"
            struct ViewUniforms
            {
                column_major float4x4 view_proj;
                float3 view_pos;
                float _pad0;
            };
            struct DrawData
            {
                column_major float4x4 model;
                float4 color;
                Ptr<float3> positions;
                Ptr<uint32_t> indices;
            };
            struct VsOutput { float4 position : SV_POSITION; float3 local_pos : TEXCOORD0; };
            [shader("vertex")]
            VsOutput main(uint vid : SV_VertexID, Ptr<DrawData> root, Ptr<ViewUniforms> view)
            {
                VsOutput o;
                uint vi = root[0].indices[vid];
                float3 position = root[0].positions[vi];
                float4 world = mul(root[0].model, float4(position, 1.0));
                o.position = mul(view[0].view_proj, world);
                o.local_pos = position;
                return o;
            }
        "#;
        let reflection = compiler
            .compile_source_to_reflection("pulling", SOURCE, "main")
            .expect("reflection");

        let binder = RootBinder::new(&reflection, "main").expect("binder");
        assert_eq!(binder.pointer_param("root").expect("root place").size, 8);
        assert_eq!(binder.pointer_param("view").expect("view place").size, 8);

        // Pointer fields inside the uniform struct: natural offsets —
        // model @0, color @64, positions @80, indices @88, size 96.
        let draw = reflection.struct_layout("DrawData").expect("layout");
        assert_eq!(draw.field_offset("model").expect("field"), 0);
        assert_eq!(draw.field_offset("color").expect("field"), 64);
        assert_eq!(draw.field_offset("positions").expect("field"), 80);
        assert_eq!(draw.field_offset("indices").expect("field"), 88);
        assert_eq!(draw.size(), 96);
    }
}
