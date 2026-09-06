//! Slang reflection: struct layouts, user attributes, and dispatch sizes.
//!
//! [`Reflection`] is a self-referential wrapper: it owns the Slang
//! `Session` and the linked [`shader_slang::ComponentType`], and holds a raw
//! pointer to the reflection object that the linked component owns. Its
//! invariants, which the hand-rolled `unsafe impl Send/Sync` below rest on:
//!
//! - The session and component type are stored inside the wrapper and are
//!   never touched after construction; they exist only to keep the pointed-to
//!   reflection object alive, so the pointer can never dangle while the
//!   wrapper lives.
//! - Every access goes through `&self` and is read-only: the reflected
//!   program is queried, never mutated, so shared references from several
//!   threads cannot race.
//!
//! Anything changing this type must preserve both invariants or drop the
//! `Send`/`Sync` impls.

use super::map_slang_error;
use crate::error::{Error as RenderError, Result as RenderResult};

/// A thin wrapper around a compiled program's reflection, exposing only the
/// layout queries the GPU-layout guard needs.
///
/// Holds the session and compiled [`shader_slang::ComponentType`] alive so the
/// owned reflection pointer stays valid for the wrapper's lifetime.
pub struct Reflection {
    pub(super) _session: shader_slang::Session,
    pub(super) _linked: shader_slang::ComponentType,
    pub(super) reflection: *const shader_slang::reflection::Shader,
}

// The reflection object is owned by the held component type; sharing it behind
// `&self` is safe as long as this wrapper is alive.
unsafe impl Send for Reflection {}
unsafe impl Sync for Reflection {}

impl Reflection {
    /// Generate a `#[repr(C)]` Rust struct skeleton for a reflected Slang
    /// struct, with `bytemuck` derives and per-field offset comments.
    ///
    /// This realizes the shader-is-truth model: run it once when writing the
    /// host struct (manually, or via a build step) and the Rust side can
    /// never drift from the shader's byte layout. Offsets are from the
    /// compiled SPIR-V layout (`LayoutRules::Default`).
    pub fn struct_rust_source(&self, name: &str) -> RenderResult<String> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
            .ok_or_else(|| RenderError::Backend(format!("type '{name}' not found")))?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;

        let mut out = String::new();
        out.push_str(&format!(
            "#[repr(C)]\n#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]\npub struct {name} {{\n"
        ));
        for field in layout.fields() {
            let field_name = field.name().unwrap_or("<unnamed>");
            let field_layout = field.type_layout().ok_or_else(|| {
                RenderError::Backend(format!("field '{field_name}' has no layout"))
            })?;
            let (rust_ty, size) = rust_type(field_layout).ok_or_else(|| {
                RenderError::Backend(format!(
                    "field '{field_name}' type has no Rust equivalent yet"
                ))
            })?;
            // Field offsets are per-category, reported by the field's own
            // variable layout; take the max span across categories as the
            // byte offset in the blob.
            let mut offset = 0usize;
            for ci in 0..field_layout.category_count() {
                let c = field_layout.category_by_index(ci);
                offset = offset.max(field.offset(c));
            }
            out.push_str(&format!("    /// offset {offset}, {size} bytes\n"));
            out.push_str(&format!("    pub {field_name}: {rust_ty},\n"));
        }
        out.push('}');
        Ok(out)
    }

    /// Read the custom `[Attribute(...)]` annotations on a struct field, as
    /// editor metadata — e.g. `[EditorColor]`, `[Range(0, 1)]`: a name plus
    /// its typed arguments (`int`/`float`/string per arg). Fields are looked
    /// up through the type's layout so the variable node carrying the
    /// attributes is the one Slang attached them to.
    pub fn field_user_attributes(
        &self,
        struct_name: &str,
        field: &str,
    ) -> RenderResult<Vec<UserAttributeRef>> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(struct_name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{struct_name}': {e}")))?
            .ok_or_else(|| RenderError::Backend(format!("type '{struct_name}' not found")))?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{struct_name}'")))?;
        let field_layout = (0..layout.field_count())
            .find_map(|i| {
                let f = layout.field_by_index(i)?;
                (f.name() == Some(field)).then_some(f)
            })
            .ok_or_else(|| {
                RenderError::Backend(format!("field '{field}' not found in '{struct_name}'"))
            })?;
        let Some(var) = field_layout.variable() else {
            return Ok(Vec::new());
        };
        Ok(var
            .user_attributes()
            .map(|attr| {
                let name = attr.name().unwrap_or("<unnamed>").to_string();
                let args = (0..attr.argument_count())
                    .map(|i| {
                        if let Some(v) = attr.argument_value_string(i) {
                            UserAttributeArg::String(v.to_string())
                        } else if let Some(v) = attr.argument_value_int(i) {
                            UserAttributeArg::Int(v)
                        } else if let Some(v) = attr.argument_value_float(i) {
                            UserAttributeArg::Float(v)
                        } else {
                            UserAttributeArg::String(format!(
                                "<{:?}>",
                                attr.argument_type(i).map(|t| t.kind())
                            ))
                        }
                    })
                    .collect();
                UserAttributeRef { name, args }
            })
            .collect())
    }

    /// The `[numthreads(x, y, z)]` dispatch size of a compute entry point, or
    /// `None` for non-compute entries.
    pub fn compute_thread_group_size(&self, entry_name: &str) -> RenderResult<Option<[u32; 3]>> {
        let reflection = unsafe { &*self.reflection };
        let entry = reflection
            .find_entry_point_by_name(entry_name)
            .map_err(map_slang_error)?
            .ok_or_else(|| RenderError::Backend(format!("entry point '{entry_name}' not found")))?;
        if entry.stage() != shader_slang::Stage::Compute {
            return Ok(None);
        }
        let [x, y, z] = entry.compute_thread_group_size();
        Ok(Some([x as u32, y as u32, z as u32]))
    }

    /// Look up a struct type by name and return its layout.
    pub fn struct_layout(&self, name: &str) -> RenderResult<Layout<'_>> {
        let reflection = unsafe { &*self.reflection };
        let ty = reflection
            .find_type_by_name(name)
            .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
            .ok_or_else(|| {
                RenderError::Backend(format!("type '{name}' not found in reflection"))
            })?;
        let layout = reflection
            .type_layout(ty, shader_slang::LayoutRules::Default)
            .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;
        Ok(Layout { layout })
    }
}

/// A field's `[Attribute(...)]` annotation: name plus typed arguments.
///
/// `PartialEq` is manual because `UserAttributeArg::Float(f32)` is not `Eq`.
#[derive(Debug, Clone)]
pub struct UserAttributeRef {
    /// The attribute name, e.g. `EditorColor`.
    pub name: String,
    /// Positional arguments, in declaration order.
    pub args: Vec<UserAttributeArg>,
}
impl PartialEq for UserAttributeRef {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.args.len() == other.args.len()
            && self.args.iter().zip(&other.args).all(|(a, b)| a == b)
    }
}

/// A user-attribute argument value.
#[derive(Debug, Clone, PartialEq)]
pub enum UserAttributeArg {
    /// Integer constant.
    Int(i32),
    /// Float constant.
    Float(f32),
    /// String literal.
    String(String),
}

/// The Rust type and byte size for a reflected field of the given type
/// layout, for [`Reflection::struct_rust_source`].
fn rust_type(layout: &shader_slang::reflection::TypeLayout) -> Option<(&'static str, usize)> {
    use shader_slang::{ScalarType, TypeKind};
    let ty = layout.ty()?;
    match ty.kind() {
        TypeKind::Scalar => match ty.scalar_type() {
            ScalarType::Uint32 => Some(("u32", 4)),
            ScalarType::Int32 => Some(("i32", 4)),
            ScalarType::Float32 => Some(("f32", 4)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Float32 => match ty.column_count() {
            2 => Some(("[f32; 2]", 8)),
            3 => Some(("[f32; 3]", 12)),
            4 => Some(("[f32; 4]", 16)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Uint32 => match ty.column_count() {
            4 => Some(("[u32; 4]", 16)),
            _ => None,
        },
        TypeKind::Matrix if ty.scalar_type() == ScalarType::Float32 => {
            let rows = ty.row_count();
            let cols = ty.column_count();
            match (rows, cols) {
                (4, 4) => Some(("[f32; 16]", 64)),
                (4, 3) => Some(("[f32; 12]", 48)),
                (3, 3) => Some(("[f32; 9]", 36)),
                _ => None,
            }
        }
        TypeKind::Array => {
            // Arrays would need their element count as the Rust size; not
            // supported by the codegen yet.
            None
        }
        _ => None,
    }
}

/// A struct's GPU memory layout, queried from Slang reflection.
pub struct Layout<'a> {
    layout: &'a shader_slang::reflection::TypeLayout,
}

impl<'a> Layout<'a> {
    /// The total byte size of the struct under the compiled target's layout
    /// rules, across every parameter category the slang compiler reports.
    pub fn size(&self) -> usize {
        self.layout
            .categories()
            .map(|c| self.layout.size(c))
            .max()
            .unwrap_or(0)
    }

    /// The byte offset of a field by name, across the field's own categories.
    pub fn field_offset(&self, name: &str) -> RenderResult<usize> {
        let idx = self.layout.find_field_index_by_name(name);
        if idx < 0 {
            return Err(RenderError::Backend(format!(
                "field '{name}' not found in reflected struct"
            )));
        }
        let field = self
            .layout
            .field_by_index(idx as u32)
            .ok_or_else(|| RenderError::Backend("field disappeared".to_string()))?;
        let tl = field
            .type_layout()
            .ok_or_else(|| RenderError::Backend("field has no type layout".to_string()))?;
        Ok(tl.categories().map(|c| field.offset(c)).max().unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::Compiler;
    use super::*;

    /// `struct_rust_source` emits a `#[repr(C)]` struct matching the shader's
    /// reflected layout; `field_user_attributes` surfaces `[Attr(...)]` marks.
    #[test]
    fn codegen_and_user_attributes() {
        // The attributes are declared in `assets/shaders/editor_metadata.slang`
        // (Slang reflects only declared user attributes — `{Name}Attribute`
        // structs with `[__AttributeUsage(...)]`).
        const SOURCE: &str = concat!(
            include_str!("../../../../../assets/shaders/editor_metadata.slang"),
            r#"
            struct DrawData
            {
                column_major float4x4 mvp;
                float4 color;
                [EditorColor]
                float4 tint;
                [Range(0, 1)]
                float opacity;
            };
            struct VsInput { float3 position : POSITION; };
            struct VsOutput { float4 position : SV_POSITION; };
            [shader("vertex")]
            VsOutput main(VsInput input, uniform DrawData root)
            {
                VsOutput o;
                o.position = mul(root.mvp, float4(input.position, 1.0)) + root.tint * root.opacity;
                return o;
            }
        "#,
        );
        let compiler = Compiler::new().expect("compiler");
        let refl = compiler
            .compile_source_to_reflection("gen", SOURCE, "main")
            .expect("reflection");

        let src = refl.struct_rust_source("DrawData").expect("codegen");
        assert!(src.starts_with("#[repr(C)]"), "starts with repr(C): {src}");
        assert!(src.contains("pub mvp: [f32; 16]"), "matrix → array: {src}");
        assert!(src.contains("pub opacity: f32"), "scalar: {src}");
        assert!(src.contains("offset 0"), "first field at 0: {src}");

        // The payloads are asserted exactly — Slang reflects declared user
        // attributes on SPIR-V, so an empty result is a real regression.
        let tint = refl
            .field_user_attributes("DrawData", "tint")
            .expect("field exists");
        assert_eq!(
            tint,
            vec![UserAttributeRef {
                name: "EditorColor".into(),
                args: vec![],
            }]
        );
        let opacity = refl
            .field_user_attributes("DrawData", "opacity")
            .expect("field exists");
        assert_eq!(
            opacity,
            vec![UserAttributeRef {
                name: "Range".into(),
                args: vec![UserAttributeArg::Int(0), UserAttributeArg::Int(1)],
            }]
        );
    }
}
