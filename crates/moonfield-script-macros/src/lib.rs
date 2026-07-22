//! Proc-macros for moonfield script system.
//!
//! # `#[script_function]`
//!
//! Generates static type-safe bindings for host functions exposed to scripts.
//! Handles argument marshaling and registration automatically.
//!
//! ```ignore
//! #[script_function]
//! fn record_frame(width: u32, height: u32) -> Result<bool, String> {
//!     // ...
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse_macro_input, FnArg, GenericArgument, ItemFn, Pat, PathArguments, ReturnType, Type,
};

/// Attribute macro that generates static type-safe bindings for a host function.
///
/// The annotated function must have the signature:
/// `fn name(param1: Type1, param2: Type2) -> Result<ReturnType, String>`
///
/// Supported parameter types: `u32`, `f64`, `bool`, `String`, `&HostValue`,
/// other integers (`u8`, `u16`, `u64`, `usize`, `i8`, `i16`, `i32`, `i64`,
/// `isize`) narrowed from JS numbers with a range check, `f32`, `Vec<u8>`
/// (from the typed-array/bytes representation), numeric `Vec<T>` (from the
/// array representation), and `Option<T>` wrappers of those (mapped to
/// optional TS parameters).
/// Supported return types: `u32`, `f64`, `bool`, `String`, `HostValue`, `()`, `Vec<u8>`
///
/// The macro can be used in any crate that depends on `moonfield-script`;
/// generated code refers to types via absolute `::moonfield_script::script` paths.
#[proc_macro_attribute]
pub fn script_function(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let func = parse_macro_input!(item as ItemFn);
    let fn_name = &func.sig.ident;
    let fn_name_str = fn_name.to_string();
    let struct_name = syn::Ident::new(
        &format!("{}_Fn", fn_name_str),
        proc_macro2::Span::call_site(),
    );

    let attrs = &func.attrs;
    let vis = &func.vis;
    let sig = &func.sig;

    // Extract parameter names and types.
    let params: Vec<ParamInfo> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pat_type) => {
                let name = match pat_type.pat.as_ref() {
                    Pat::Ident(ident) => ident.ident.to_string(),
                    _ => return None,
                };
                let ty = pat_type.ty.as_ref().clone();
                Some(ParamInfo { name, ty })
            }
            FnArg::Receiver(_) => None,
        })
        .collect();

    let param_names: Vec<&syn::Ident> = func
        .sig
        .inputs
        .iter()
        .filter_map(|arg| match arg {
            FnArg::Typed(pat_type) => match pat_type.pat.as_ref() {
                Pat::Ident(ident) => Some(&ident.ident),
                _ => None,
            },
            FnArg::Receiver(_) => None,
        })
        .collect();

    // Generate extraction code for each parameter.
    let extractions: Vec<_> = params
        .iter()
        .enumerate()
        .map(|(i, param)| {
            let name = &param.name;
            let index = i;
            extract_param_code(name, &param.ty, index)
        })
        .collect();

    // Determine the return type conversion.
    let ret_conversion = match &func.sig.output {
        ReturnType::Default => {
            quote! { Ok(::moonfield_script::script::HostValue::Null) }
        }
        ReturnType::Type(_, ty) => {
            let ty_str = quote!(#ty).to_string();
            if ty_str.contains("Result") {
                let inner_is_unit = ty_str.contains("()");
                if inner_is_unit {
                    quote! {
                        match #fn_name(#(#param_names),*) {
                            Ok(()) => Ok(::moonfield_script::script::HostValue::Null),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                } else {
                    quote! {
                        match #fn_name(#(#param_names),*) {
                            Ok(val) => Ok(::moonfield_script::script::HostValue::from(val)),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            } else {
                quote! {
                    Ok(::moonfield_script::script::HostValue::from(#fn_name(#(#param_names),*)))
                }
            }
        }
    };

    let stmt_tokens: Vec<proc_macro2::TokenStream> =
        func.block.stmts.iter().map(|s| quote!(#s)).collect();

    // Generate TypeScript signature for .d.ts generation.
    let ts_params: Vec<String> = params
        .iter()
        .map(|p| {
            let (ts_ty, optional) = rust_type_to_ts(&p.ty);
            if optional {
                format!("{}?: {}", p.name, ts_ty)
            } else {
                format!("{}: {}", p.name, ts_ty)
            }
        })
        .collect();
    let ts_ret = match &func.sig.output {
        ReturnType::Default => "void".to_string(),
        ReturnType::Type(_, ty) => return_type_to_ts(ty),
    };
    let ts_sig = format!(
        "declare function {}({}): {};",
        fn_name_str,
        ts_params.join(", "),
        ts_ret
    );

    // Use the original function verbatim via quote!(#func), then add the struct + impl.
    let output = quote! {
        #(#attrs)*
        #vis #sig {
            #(#stmt_tokens)*
        }

        #[doc(hidden)]
        #vis struct #struct_name;

        impl ::moonfield_script::script::ScriptFunction for #struct_name {
            const NAME: &'static str = #fn_name_str;

            fn call(args: &[::moonfield_script::script::HostValue]) -> Result<::moonfield_script::script::HostValue, String> {
                #(#extractions)*
                #ret_conversion
            }

            fn ts_signature() -> &'static str {
                #ts_sig
            }
        }
    };

    output.into()
}

struct ParamInfo {
    name: String,
    ty: Type,
}

/// Normalize a stringified Rust type by removing all whitespace, so that
/// e.g. `Option < u32 >` and `Option<u32>` compare equal.
fn normalize_ty(ty_str: &str) -> String {
    ty_str.split_whitespace().collect()
}

/// Map a Rust type to a TypeScript type string.
///
/// Returns the TypeScript type and whether the parameter is optional
/// (`Option<T>` maps to an optional TS parameter). The type is parsed
/// structurally via `syn`, so nested generics like `Vec<f32>` or
/// `Option<Vec<u8>>` map faithfully instead of degrading to `any`.
fn rust_type_to_ts(ty: &Type) -> (String, bool) {
    match ty {
        Type::Reference(r) => rust_type_to_ts(&r.elem),
        Type::Tuple(t) if t.elems.is_empty() => ("void".to_string(), false),
        Type::Path(tp) => {
            let Some(seg) = tp.path.segments.last() else {
                return ("any".to_string(), false);
            };
            match seg.ident.to_string().as_str() {
                "Option" => (
                    inner_angle_type(seg)
                        .map(|inner| rust_type_to_ts(inner).0)
                        .unwrap_or_else(|| "any".to_string()),
                    true,
                ),
                "Vec" => match inner_angle_type(seg) {
                    Some(inner) if is_type_named(inner, "u8") => ("Uint8Array".to_string(), false),
                    Some(inner) => (format!("{}[]", rust_type_to_ts(inner).0), false),
                    None => ("any[]".to_string(), false),
                },
                "u32" | "i32" | "u64" | "i64" | "usize" | "isize" | "f32" | "f64" => {
                    ("number".to_string(), false)
                }
                "bool" => ("boolean".to_string(), false),
                "String" | "str" => ("string".to_string(), false),
                _ => ("any".to_string(), false),
            }
        }
        _ => ("any".to_string(), false),
    }
}

/// Map a function's return type to a TypeScript return type.
///
/// `Result<T, E>` (the `#[script_function]` contract) maps to the TS type
/// of `T`; `()` maps to `void`; anything else maps via [`rust_type_to_ts`].
fn return_type_to_ts(ty: &Type) -> String {
    if let Type::Path(tp) = ty {
        if let Some(seg) = tp.path.segments.last() {
            if seg.ident == "Result" {
                if let Some(inner) = inner_angle_type(seg) {
                    return rust_type_to_ts(inner).0;
                }
            }
        }
    }
    rust_type_to_ts(ty).0
}

/// Extract the first generic type argument of a path segment
/// (e.g. `T` in `Foo<T, E>`).
fn inner_angle_type(seg: &syn::PathSegment) -> Option<&Type> {
    if let PathArguments::AngleBracketed(args) = &seg.arguments {
        if let Some(GenericArgument::Type(t)) = args.args.first() {
            return Some(t);
        }
    }
    None
}

/// Check whether a type is a simple path ending in `name` (e.g. `u8`).
fn is_type_named(ty: &Type, name: &str) -> bool {
    if let Type::Path(tp) = ty {
        tp.path.segments.last().is_some_and(|s| s.ident == name)
    } else {
        false
    }
}

/// Generate code to extract a parameter from `args` slice.
fn extract_param_code(name: &str, ty: &Type, index: usize) -> proc_macro2::TokenStream {
    let name_ident = syn::Ident::new(name, proc_macro2::Span::call_site());
    let index_lit = index;

    let ty_str = normalize_ty(&quote!(#ty).to_string());
    let ty_str = ty_str.trim_start_matches('&').to_string();

    // `Option<T>` parameters are optional: a missing or mistyped argument
    // yields `None` instead of an error.
    if let Some(inner) = ty_str
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let extraction = match inner {
            "u32" => quote! { |v| v.as_u32().map(|n| n as _) },
            "u8" | "u16" | "u64" | "usize" => int_extraction_closure(inner),
            "i8" | "i16" | "i32" | "i64" | "isize" => int_extraction_closure(inner),
            "f64" | "f32" => quote! { |v| v.as_f64().map(|n| n as _) },
            "bool" => quote! { |v| v.as_bool() },
            "String" | "str" => quote! { |v| v.as_str().map(|s| s.to_string()) },
            _ => numeric_vec_extraction(inner).unwrap_or_else(|| quote! { |_| Option::None }),
        };
        return quote! {
            let #name_ident: #ty = args.get(#index_lit).and_then(#extraction);
        };
    }

    // `Vec<u8>` extracts from the typed-array/bytes representation; other
    // numeric `Vec<T>` from the array representation.
    if let Some(extraction) = numeric_vec_extraction(&ty_str) {
        return quote! {
            let #name_ident: #ty = args
                .get(#index_lit)
                .and_then(#extraction)
                .ok_or_else(|| format!("arg {}: expected {}", #index_lit, #ty_str))?;
        };
    }

    // Match common types to generate appropriate extraction code.
    match ty_str.as_str() {
        "u32" => quote! {
            let #name_ident = args
                .get(#index_lit)
                .and_then(|v| v.as_u32())
                .ok_or_else(|| format!("arg {}: expected u32", #index_lit))?;
        },
        "f64" => quote! {
            let #name_ident = args
                .get(#index_lit)
                .and_then(|v| v.as_f64())
                .ok_or_else(|| format!("arg {}: expected f64", #index_lit))?;
        },
        "bool" => quote! {
            let #name_ident = args
                .get(#index_lit)
                .and_then(|v| v.as_bool())
                .ok_or_else(|| format!("arg {}: expected bool", #index_lit))?;
        },
        "String" => quote! {
            let #name_ident = args
                .get(#index_lit)
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .ok_or_else(|| format!("arg {}: expected string", #index_lit))?;
        },
        // Signed and wider unsigned integers narrow from HostValue's f64
        // number with a range check: negative and large values extract
        // correctly, and out-of-range or fractional values fail with a clear
        // error instead of silently truncating or flipping sign.
        "u8" | "u16" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
            let ty_ident = syn::Ident::new(&ty_str, proc_macro2::Span::call_site());
            quote! {
                let #name_ident = match args.get(#index_lit).and_then(|v| v.as_f64()) {
                    Some(n)
                        if n.fract() == 0.0
                            && n >= #ty_ident::MIN as f64
                            && n <= #ty_ident::MAX as f64 =>
                    {
                        n as #ty_ident
                    }
                    Some(n) => {
                        return Err(format!(
                            "arg {}: value {} does not fit in {}",
                            #index_lit, n, #ty_str
                        ));
                    }
                    None => {
                        return Err(format!("arg {}: expected {}", #index_lit, #ty_str));
                    }
                };
            }
        }
        "f32" => quote! {
            let #name_ident = args
                .get(#index_lit)
                .and_then(|v| v.as_f64().map(|n| n as f32))
                .ok_or_else(|| format!("arg {}: expected f32", #index_lit))?;
        },
        // For HostValue, pass through by reference (HostValue is not Clone:
        // its zero-copy view variants borrow the JS engine's backing store,
        // so the annotated function must take `&HostValue`).
        s if s.contains("HostValue") => quote! {
            let #name_ident = args
                .get(#index_lit)
                .unwrap_or(&::moonfield_script::script::HostValue::Null);
        },
        // Other types: try to clone the HostValue directly.
        _ => quote! {
            let #name_ident: #ty = args
                .get(#index_lit)
                .and_then(|v| v.as_u32().map(|n| n as #ty))
                .ok_or_else(|| format!("arg {}: type mismatch", #index_lit))?;
        },
    }
}

/// Closure `|v| -> Option<T>` extracting an integer of type `ty` from
/// HostValue's f64 number representation. Fractional and out-of-range
/// values yield `None` instead of silently truncating or flipping sign.
fn int_extraction_closure(ty: &str) -> proc_macro2::TokenStream {
    let ty_ident = syn::Ident::new(ty, proc_macro2::Span::call_site());
    quote! {
        |v| v.as_f64().and_then(|n| {
            if n.fract() == 0.0 && n >= #ty_ident::MIN as f64 && n <= #ty_ident::MAX as f64 {
                Some(n as #ty_ident)
            } else {
                None
            }
        })
    }
}

/// Closure `|v| -> Option<Vec<T>>` extracting a `Vec<u8>` from the
/// typed-array/bytes representation (`as_bytes`) or a numeric `Vec<T>` from
/// the array representation (`as_array`, converting each element — one bad
/// element fails the whole extraction). Returns `None` for non-vector and
/// unsupported element types.
fn numeric_vec_extraction(ty: &str) -> Option<proc_macro2::TokenStream> {
    let elem = ty.strip_prefix("Vec<")?.strip_suffix('>')?;
    if elem == "u8" {
        return Some(quote! { |v| v.as_bytes().map(|b| b.to_vec()) });
    }
    let elem_extraction = match elem {
        "u32" => quote! { |v| v.as_u32() },
        "u16" | "u64" | "usize" | "i8" | "i16" | "i32" | "i64" | "isize" => {
            int_extraction_closure(elem)
        }
        "f32" => quote! { |v| v.as_f64().map(|n| n as f32) },
        "f64" => quote! { |v| v.as_f64() },
        _ => return None,
    };
    Some(quote! {
        |v| v
            .as_array()
            .and_then(|arr| arr.iter().map(#elem_extraction).collect::<Option<Vec<_>>>())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(ty: &str) -> (String, bool) {
        rust_type_to_ts(&syn::parse_str::<Type>(ty).unwrap())
    }

    fn ret(ty: &str) -> String {
        return_type_to_ts(&syn::parse_str::<Type>(ty).unwrap())
    }

    #[test]
    fn primitives_map_to_ts() {
        assert_eq!(ts("u32"), ("number".to_string(), false));
        assert_eq!(ts("i64"), ("number".to_string(), false));
        assert_eq!(ts("f64"), ("number".to_string(), false));
        assert_eq!(ts("bool"), ("boolean".to_string(), false));
        assert_eq!(ts("String"), ("string".to_string(), false));
        assert_eq!(ts("&str"), ("string".to_string(), false));
    }

    #[test]
    fn option_marks_parameter_optional() {
        assert_eq!(ts("Option<u32>"), ("number".to_string(), true));
        assert_eq!(ts("Option<String>"), ("string".to_string(), true));
        assert_eq!(ts("Option<Vec<u8>>"), ("Uint8Array".to_string(), true));
    }

    #[test]
    fn vec_maps_to_typed_or_plain_arrays() {
        assert_eq!(ts("Vec<u8>"), ("Uint8Array".to_string(), false));
        assert_eq!(ts("Vec<f32>"), ("number[]".to_string(), false));
        assert_eq!(ts("Vec<String>"), ("string[]".to_string(), false));
    }

    #[test]
    fn unit_maps_to_void() {
        assert_eq!(ts("()"), ("void".to_string(), false));
    }

    #[test]
    fn unknown_types_fall_back_to_any() {
        assert_eq!(ts("MyStruct"), ("any".to_string(), false));
        assert_eq!(ts("HostValue"), ("any".to_string(), false));
        assert_eq!(
            ts("std::collections::HashMap<String, u32>"),
            ("any".to_string(), false)
        );
    }

    #[test]
    fn result_unwraps_ok_type() {
        assert_eq!(ret("Result<f64, String>"), "number");
        assert_eq!(ret("Result<bool, String>"), "boolean");
        assert_eq!(ret("Result<String, String>"), "string");
        assert_eq!(ret("Result<(), String>"), "void");
        assert_eq!(ret("Result<Vec<u8>, String>"), "Uint8Array");
        assert_eq!(ret("Result<Vec<f32>, String>"), "number[]");
        // Previously these degraded to `any` via string heuristics.
        assert_eq!(ret("std::result::Result<f64, String>"), "number");
    }

    #[test]
    fn plain_return_types_map_directly() {
        assert_eq!(ret("f64"), "number");
        assert_eq!(ret("()"), "void");
        assert_eq!(ret("MyStruct"), "any");
    }
}
