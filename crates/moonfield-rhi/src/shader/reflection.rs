//! Slang reflection: struct layouts, user attributes, and dispatch sizes.
//!
//! [`Reflection`] owns the Slang session and linked component. Queries and
//! destruction acquire the originating compiler's global-session mutex.
//! Returned values, including [`Layout`], contain owned data rather than
//! borrowed Slang reflection nodes.

use super::map_slang_error;
use crate::error::{Error as RenderError, Result as RenderResult};

/// A thin wrapper around a compiled program's reflection, exposing only the
/// layout queries the GPU-layout guard needs.
///
/// Holds the session and compiled [`shader_slang::ComponentType`] alive so the
/// program remains available for synchronized queries.
pub struct Reflection {
    state: std::sync::Mutex<Option<ReflectionState>>,
    global_session: std::sync::Arc<std::sync::Mutex<shader_slang::GlobalSession>>,
}

struct ReflectionState {
    linked: shader_slang::ComponentType,
    _session: shader_slang::Session,
}

impl Drop for Reflection {
    fn drop(&mut self) {
        let _global_session = self
            .global_session
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = self.state.get_mut().unwrap_or_else(|e| e.into_inner());
        drop(state.take());
    }
}

impl Reflection {
    pub(super) fn new(
        global_session: std::sync::Arc<std::sync::Mutex<shader_slang::GlobalSession>>,
        session: shader_slang::Session,
        linked: shader_slang::ComponentType,
    ) -> RenderResult<Self> {
        linked.layout(0).map_err(map_slang_error)?;
        Ok(Self {
            state: std::sync::Mutex::new(Some(ReflectionState {
                linked,
                _session: session,
            })),
            global_session,
        })
    }

    fn with_shader<T>(
        &self,
        query: impl FnOnce(&shader_slang::reflection::Shader) -> RenderResult<T>,
    ) -> RenderResult<T> {
        let _global_session = self
            .global_session
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let state = state.as_ref().expect("reflection state exists until drop");
        query(state.linked.layout(0).map_err(map_slang_error)?)
    }

    /// Generate a `#[repr(C)]` Rust struct skeleton for a reflected Slang
    /// struct, with `bytemuck` derives and per-field offset comments.
    ///
    /// Explicit padding and alignment match `LayoutRules::Default`. Matrix
    /// arrays include the reflected storage padding between rows or columns;
    /// their elements follow the shader's declared matrix layout mode.
    /// Unsupported field types or unrepresentable layouts return an error.
    pub fn struct_rust_source(&self, name: &str) -> RenderResult<String> {
        self.with_shader(|reflection| {
            let ty = reflection
                .find_type_by_name(name)
                .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
                .ok_or_else(|| RenderError::Backend(format!("type '{name}' not found")))?;
            let layout = reflection
                .type_layout(ty, shader_slang::LayoutRules::Default)
                .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;

            rust_struct_source(layout, name)
        })
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
        self.with_shader(|reflection| {
            let ty = reflection
                .find_type_by_name(struct_name)
                .map_err(|e| {
                    RenderError::Backend(format!("failed to find type '{struct_name}': {e}"))
                })?
                .ok_or_else(|| RenderError::Backend(format!("type '{struct_name}' not found")))?;
            let layout = reflection
                .type_layout(ty, shader_slang::LayoutRules::Default)
                .ok_or_else(|| {
                    RenderError::Backend(format!("no layout for type '{struct_name}'"))
                })?;
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
        })
    }

    /// The `[numthreads(x, y, z)]` dispatch size of a compute entry point, or
    /// `None` for non-compute entries.
    pub fn compute_thread_group_size(&self, entry_name: &str) -> RenderResult<Option<[u32; 3]>> {
        self.with_shader(|reflection| {
            let entry = reflection
                .find_entry_point_by_name(entry_name)
                .map_err(map_slang_error)?
                .ok_or_else(|| {
                    RenderError::Backend(format!("entry point '{entry_name}' not found"))
                })?;
            if entry.stage() != shader_slang::Stage::Compute {
                return Ok(None);
            }
            let [x, y, z] = entry.compute_thread_group_size();
            Ok(Some([x as u32, y as u32, z as u32]))
        })
    }

    /// Look up a struct type by name and return its layout.
    pub fn struct_layout(&self, name: &str) -> RenderResult<Layout<'_>> {
        self.with_shader(|reflection| {
            let ty = reflection
                .find_type_by_name(name)
                .map_err(|e| RenderError::Backend(format!("failed to find type '{name}': {e}")))?
                .ok_or_else(|| {
                    RenderError::Backend(format!("type '{name}' not found in reflection"))
                })?;
            let layout = reflection
                .type_layout(ty, shader_slang::LayoutRules::Default)
                .ok_or_else(|| RenderError::Backend(format!("no layout for type '{name}'")))?;
            Ok(Layout::from_slang(layout))
        })
    }

    /// The layout a `Ptr<T>` struct field dereferences with: the pointee's
    /// layout as baked into the emitted pointer arithmetic. This is NOT what
    /// [`struct_layout`](Self::struct_layout) reports for `T` — the default
    /// rules pad `float3` to 16-byte alignment, while pointer targets pack
    /// tightly (C-like natural layout). Host mirrors of `Ptr` pointees must
    /// match this layout, not the default one.
    pub fn field_pointee_layout(&self, struct_name: &str, field: &str) -> RenderResult<Layout<'_>> {
        self.with_shader(|reflection| {
            let ty = reflection
                .find_type_by_name(struct_name)
                .map_err(|e| {
                    RenderError::Backend(format!("failed to find type '{struct_name}': {e}"))
                })?
                .ok_or_else(|| {
                    RenderError::Backend(format!("type '{struct_name}' not found in reflection"))
                })?;
            let layout = reflection
                .type_layout(ty, shader_slang::LayoutRules::Default)
                .ok_or_else(|| {
                    RenderError::Backend(format!("no layout for type '{struct_name}'"))
                })?;
            let idx = layout.find_field_index_by_name(field);
            if idx < 0 {
                return Err(RenderError::Backend(format!(
                    "field '{field}' not found in '{struct_name}'"
                )));
            }
            let field_layout = layout
                .field_by_index(idx as u32)
                .and_then(|field| field.type_layout())
                .ok_or_else(|| {
                    RenderError::Backend(format!("field '{field}' has no type layout"))
                })?;
            if field_layout.kind() != shader_slang::TypeKind::Pointer {
                return Err(RenderError::Backend(format!(
                    "field '{struct_name}.{field}' is not a pointer"
                )));
            }
            let pointee = field_layout.element_type_layout().ok_or_else(|| {
                RenderError::Backend(format!("pointer field '{field}' has no pointee layout"))
            })?;
            Ok(Layout::from_slang(pointee))
        })
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

fn rust_struct_source(
    layout: &shader_slang::reflection::TypeLayout,
    name: &str,
) -> RenderResult<String> {
    let category = shader_slang::ParameterCategory::Uniform;
    if layout.kind() != shader_slang::TypeKind::Struct
        || layout
            .categories()
            .any(|category| category != shader_slang::ParameterCategory::Uniform)
    {
        return Err(RenderError::Backend(format!(
            "type '{name}' is not an inline struct"
        )));
    }
    let alignment = usize::try_from(layout.alignment(category))
        .map_err(|_| RenderError::Backend(format!("invalid alignment for type '{name}'")))?;
    let total_size = layout.size(category);
    if !alignment.is_power_of_two()
        || alignment > (1 << 29)
        || !total_size.is_multiple_of(alignment)
    {
        return Err(RenderError::Backend(format!(
            "unrepresentable layout for type '{name}'"
        )));
    }
    let field_names: std::collections::HashSet<&str> =
        layout.fields().filter_map(|field| field.name()).collect();
    let mut padding_index = 0usize;
    let mut padding = |out: &mut String, offset: usize, size: usize| {
        if size == 0 {
            return;
        }
        let padding_name = loop {
            let candidate = format!("_padding{padding_index}");
            padding_index += 1;
            if !field_names.contains(candidate.as_str()) {
                break candidate;
            }
        };
        out.push_str(&format!(
            "    /// offset {offset}, {size} bytes\n    pub {padding_name}: [u8; {size}],\n"
        ));
    };
    let mut out = String::new();
    out.push_str(&format!(
        "#[repr(C)]\n#[repr(align({alignment}))]\n#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]\npub struct {name} {{\n"
    ));
    let mut cursor = 0usize;
    for field in layout.fields() {
        let field_name = field.name().unwrap_or("<unnamed>");
        let field_layout = field
            .type_layout()
            .ok_or_else(|| RenderError::Backend(format!("field '{field_name}' has no layout")))?;
        let (rust_ty, size) = rust_type(field_layout).ok_or_else(|| {
            RenderError::Backend(format!(
                "field '{field_name}' type has no Rust equivalent yet"
            ))
        })?;
        let offset = field.offset(category);
        let gap = offset.checked_sub(cursor).ok_or_else(|| {
            RenderError::Backend(format!("field '{field_name}' overlaps another field"))
        })?;
        if !offset.is_multiple_of(std::mem::align_of::<f32>()) || size > field_layout.size(category)
        {
            return Err(RenderError::Backend(format!(
                "unrepresentable layout for field '{field_name}'"
            )));
        }
        padding(&mut out, cursor, gap);
        out.push_str(&format!("    /// offset {offset}, {size} bytes\n"));
        out.push_str(&format!("    pub {field_name}: {rust_ty},\n"));
        cursor = offset
            .checked_add(size)
            .ok_or_else(|| RenderError::Backend(format!("field '{field_name}' size overflows")))?;
    }
    let tail = total_size
        .checked_sub(cursor)
        .ok_or_else(|| RenderError::Backend(format!("fields exceed type '{name}' storage")))?;
    padding(&mut out, cursor, tail);
    out.push('}');
    Ok(out)
}

/// The Rust type and byte size for a reflected field of the given type
/// layout, for [`Reflection::struct_rust_source`].
fn rust_type(layout: &shader_slang::reflection::TypeLayout) -> Option<(String, usize)> {
    use shader_slang::{ScalarType, TypeKind};
    let ty = layout.ty()?;
    match ty.kind() {
        TypeKind::Scalar => match ty.scalar_type() {
            ScalarType::Uint32 => Some(("u32".into(), 4)),
            ScalarType::Int32 => Some(("i32".into(), 4)),
            ScalarType::Float32 => Some(("f32".into(), 4)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Float32 => match ty.column_count() {
            2 => Some(("[f32; 2]".into(), 8)),
            3 => Some(("[f32; 3]".into(), 12)),
            4 => Some(("[f32; 4]".into(), 16)),
            _ => None,
        },
        TypeKind::Vector if ty.scalar_type() == ScalarType::Uint32 => match ty.column_count() {
            4 => Some(("[u32; 4]".into(), 16)),
            _ => None,
        },
        TypeKind::Matrix if ty.scalar_type() == ScalarType::Float32 => {
            let size = layout.size(shader_slang::ParameterCategory::Uniform);
            let elements = (ty.row_count() as usize).checked_mul(ty.column_count() as usize)?;
            if !size.is_multiple_of(4) || size / 4 < elements {
                return None;
            }
            Some((format!("[f32; {}]", size / 4), size))
        }
        TypeKind::Array => {
            // Arrays would need their element count as the Rust size; not
            // supported by the codegen yet.
            None
        }
        _ => None,
    }
}

/// An owned snapshot of a struct's GPU memory layout.
pub struct Layout<'a> {
    size: usize,
    fields: Vec<(String, usize)>,
    _reflection: std::marker::PhantomData<&'a Reflection>,
}

impl<'a> Layout<'a> {
    fn from_slang(layout: &shader_slang::reflection::TypeLayout) -> Self {
        Self {
            size: layout
                .categories()
                .map(|category| layout.size(category))
                .max()
                .unwrap_or(0),
            fields: layout
                .fields()
                .map(|field| {
                    let offset = field
                        .type_layout()
                        .map(|layout| {
                            layout
                                .categories()
                                .map(|category| field.offset(category))
                                .max()
                                .unwrap_or(0)
                        })
                        .unwrap_or(0);
                    (field.name().unwrap_or("<unnamed>").to_string(), offset)
                })
                .collect(),
            _reflection: std::marker::PhantomData,
        }
    }

    /// The total byte size of the struct under the compiled target's layout
    /// rules, across every parameter category the slang compiler reports.
    pub fn size(&self) -> usize {
        self.size
    }

    /// The byte offset of a field by name, across the field's own categories.
    pub fn field_offset(&self, name: &str) -> RenderResult<usize> {
        self.fields
            .iter()
            .find(|(field_name, _)| field_name == name)
            .map(|(_, offset)| *offset)
            .ok_or_else(|| {
                RenderError::Backend(format!("field '{name}' not found in reflected struct"))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::compile::Compiler;
    use super::*;

    fn assert_generated_layout(reflection: &Reflection, name: &str, fields: &[&str]) {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT_PROBE: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "moonfield-layout-{}-{}",
            std::process::id(),
            NEXT_PROBE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&directory).expect("probe directory");
        let mut source = reflection
            .struct_rust_source(name)
            .expect("generated source");
        let layout = reflection.struct_layout(name).expect("layout");
        let alignment = reflection
            .with_shader(|shader| {
                let ty = shader
                    .find_type_by_name(name)
                    .map_err(map_slang_error)?
                    .expect("type");
                let layout = shader
                    .type_layout(ty, shader_slang::LayoutRules::Default)
                    .expect("type layout");
                Ok(layout.alignment(shader_slang::ParameterCategory::Uniform))
            })
            .expect("alignment");
        source.push_str(&format!(
            "\nfn main() {{ assert_eq!(std::mem::size_of::<{name}>(), {});\nassert_eq!(std::mem::align_of::<{name}>(), {alignment});\n",
            layout.size(),
        ));
        for field in fields {
            source.push_str(&format!(
                "assert_eq!(std::mem::offset_of!({name}, {field}), {});\n",
                layout.field_offset(field).expect("field offset")
            ));
        }
        source.push_str("}\n");
        let source_path = directory.join("probe.rs");
        let executable = directory.join(format!("probe{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&source_path, &source).expect("probe source");
        let test_executable = std::env::current_exe().expect("test executable");
        let profile = test_executable
            .ancestors()
            .find(|path| path.join("deps").is_dir() || path.join("build").is_dir())
            .expect("Cargo profile directory");
        let mut dependencies = Vec::new();
        if profile.join("deps").is_dir() {
            dependencies.push(profile.join("deps"));
        }
        if profile.join("build").is_dir() {
            let mut directories = vec![profile.join("build")];
            for _ in 0..2 {
                directories = directories
                    .into_iter()
                    .flat_map(|path| std::fs::read_dir(path).expect("artifact directory"))
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.is_dir())
                    .collect();
            }
            dependencies.extend(
                directories
                    .into_iter()
                    .map(|path| path.join("out"))
                    .filter(|path| path.is_dir()),
            );
        }
        let bytemuck = dependencies
            .iter()
            .flat_map(|path| std::fs::read_dir(path).expect("dependencies"))
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("libbytemuck-") && name.ends_with(".rlib"))
            })
            .max_by_key(|path| {
                path.metadata()
                    .and_then(|metadata| metadata.modified())
                    .ok()
            })
            .expect("bytemuck rlib");
        let mut command =
            std::process::Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()));
        command
            .arg("--edition=2024")
            .arg(&source_path)
            .arg("--extern")
            .arg(format!("bytemuck={}", bytemuck.display()));
        let metadata = bytemuck.with_extension("rmeta");
        if metadata.is_file() {
            command
                .arg("--extern")
                .arg(format!("bytemuck={}", metadata.display()));
        }
        for dependency in dependencies {
            command
                .arg("-L")
                .arg(format!("dependency={}", dependency.display()));
        }
        let output = command
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile generated struct");
        let execution = output.status.success().then(|| {
            std::process::Command::new(&executable)
                .output()
                .expect("run generated layout assertions")
        });
        std::fs::remove_dir_all(&directory).expect("remove probe directory");
        assert!(
            output.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&output.stderr)
        );
        let execution = execution.expect("compiled probe");
        assert!(
            execution.status.success(),
            "{}\n{source}",
            String::from_utf8_lossy(&execution.stderr)
        );
    }

    #[test]
    fn test_codegen_matches_inter_field_and_tail_padding() {
        const SOURCE: &str = r#"
            struct Padded { float scalar; float4 vector; float tail; };
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main(uniform Padded root, Ptr<float, Access.ReadWrite> output) {
                output[0] = root.scalar + root.vector.x + root.tail;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection("padding", SOURCE, &["main"], crate::ShaderTarget::Spirv)
            .expect("reflection");
        assert_generated_layout(&reflection, "Padded", &["scalar", "vector", "tail"]);
    }

    #[test]
    fn test_codegen_matches_padded_matrix_storage() {
        const SOURCE: &str = r#"
            struct Matrices { float3x3 row; column_major float4x3 column; float tail; };
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main(uniform Matrices root, Ptr<float, Access.ReadWrite> output) {
                output[0] = root.row[0][0] + root.column[0][0] + root.tail;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection("matrices", SOURCE, &["main"], crate::ShaderTarget::Spirv)
            .expect("reflection");
        assert_generated_layout(&reflection, "Matrices", &["row", "column", "tail"]);
    }

    #[test]
    fn test_codegen_padding_names_do_not_shadow_shader_fields() {
        const SOURCE: &str = r#"
            struct Collision { float _padding0; float4 vector; };
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main(uniform Collision root, Ptr<float, Access.ReadWrite> output) {
                output[0] = root._padding0 + root.vector.x;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection(
                "collision",
                SOURCE,
                &["main"],
                crate::ShaderTarget::Spirv,
            )
            .expect("reflection");
        assert_generated_layout(&reflection, "Collision", &["_padding0", "vector"]);
    }

    #[test]
    fn test_codegen_rejects_unsupported_fields() {
        const SOURCE: &str = r#"
            struct Unsupported { bool enabled; };
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main(uniform Unsupported root, Ptr<float, Access.ReadWrite> output) {
                output[0] = root.enabled ? 1.0 : 0.0;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection(
                "unsupported",
                SOURCE,
                &["main"],
                crate::ShaderTarget::Spirv,
            )
            .expect("reflection");
        assert!(reflection.struct_rust_source("Unsupported").is_err());
        assert!(reflection.struct_rust_source("Missing").is_err());
    }

    #[test]
    fn test_reflection_destruction_uses_the_global_session_lock() {
        use std::sync::{Arc, Barrier, mpsc};
        use std::time::Duration;

        const SOURCE: &str = r#"
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main() {}
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection("drop", SOURCE, &["main"], crate::ShaderTarget::Spirv)
            .expect("reflection");
        let global_session = Arc::clone(&reflection.global_session);
        let guard = global_session.lock().expect("session lock");
        let barrier = Arc::new(Barrier::new(2));
        let (sender, receiver) = mpsc::channel();
        let drop_thread = {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                drop(reflection);
                sender.send(()).expect("drop completion");
            })
        };
        barrier.wait();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(guard);
        drop_thread.join().expect("drop thread");
        receiver.recv().expect("drop completed");
    }

    #[test]
    fn test_layout_snapshots_survive_other_reflection_queries_and_compiler_drop() {
        const SOURCE: &str = r#"
            struct First { float scalar; float4 vector; };
            struct Second { float2 value; };
            [shader("compute")]
            [numthreads(1, 1, 1)]
            void main(uniform First first, uniform Second second,
                      Ptr<float, Access.ReadWrite> output) {
                output[0] = first.scalar + first.vector.x + second.value.x;
            }
        "#;
        let compiler = Compiler::new().expect("compiler");
        let reflection = compiler
            .compile_source_to_reflection(
                "snapshots",
                SOURCE,
                &["main"],
                crate::ShaderTarget::Spirv,
            )
            .expect("reflection");
        drop(compiler);
        let first = reflection.struct_layout("First").expect("first layout");
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for _ in 0..8 {
                    assert_eq!(
                        reflection
                            .struct_layout("Second")
                            .expect("second layout")
                            .size(),
                        16
                    );
                }
            });
            assert_eq!(first.size(), 32);
            assert_eq!(first.field_offset("vector").expect("field offset"), 16);
            assert!(first.field_offset("missing").is_err());
        });
    }

    /// `struct_rust_source` emits a `#[repr(C)]` struct matching the shader's
    /// reflected layout; `field_user_attributes` surfaces `[Attr(...)]` marks.
    ///
    /// The attributes are declared in `assets/shaders/editor_metadata.slang`
    /// (Slang reflects only declared user attributes — `{Name}Attribute`
    /// structs with `[__AttributeUsage(...)]`); the probe imports that module,
    /// with the virtual module path placed next to it so the import resolves.
    #[test]
    fn codegen_and_user_attributes() {
        const SOURCE: &str = r#"
            import editor_metadata;

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
        "#;
        let module_path = moonfield_asset::assets_dir()
            .join("shaders/__metadata_probe.slang")
            .display()
            .to_string();
        let compiler = Compiler::new().expect("compiler");
        let refl = compiler
            .compile_source_to_reflection(
                &module_path,
                SOURCE,
                &["main"],
                crate::ShaderTarget::Spirv,
            )
            .expect("reflection");

        let src = refl.struct_rust_source("DrawData").expect("codegen");
        assert!(src.starts_with("#[repr(C)]"), "starts with repr(C): {src}");
        assert!(src.contains("pub mvp: [f32; 16]"), "matrix → array: {src}");
        assert!(src.contains("pub opacity: f32"), "scalar: {src}");
        assert!(src.contains("offset 0"), "first field at 0: {src}");
        assert_generated_layout(&refl, "DrawData", &["mvp", "color", "tint", "opacity"]);

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

/// How a root parameter is delivered to the shader: inline bytes (a `uniform`
/// parameter) or a GPU address (a `Ptr<T>` root — a buffer binding on
/// backends without descriptor-push, a device address where they have one).
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
/// the root blob the backend's push-data command receives.
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

impl Reflection {
    /// Enumerate the root parameters of `entry_name` — every non-varying
    /// parameter — with their placement in the push-data blob.
    ///
    /// Root parameters are the entry point's remaining parameters after
    /// varying inputs (vertex attributes) and outputs: `Ptr<T>` roots
    /// (pointer category) and `uniform` roots (push-constant category). A
    /// backend writes the entire root blob through its push-data command;
    /// this describes where each parameter lives in it.
    pub fn root_parameters(&self, entry_name: &str) -> RenderResult<Vec<RootParam>> {
        self.with_shader(|reflection| {
            let entry = reflection
                .find_entry_point_by_name(entry_name)
                .map_err(map_slang_error)?
                .ok_or_else(|| {
                    RenderError::Backend(format!("entry point '{entry_name}' not found"))
                })?;

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
        })
    }
}
