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
use syn::{parse_macro_input, FnArg, ItemFn, Pat, ReturnType, Type};

/// Attribute macro that generates static type-safe bindings for a host function.
///
/// The annotated function must have the signature:
/// `fn name(param1: Type1, param2: Type2) -> Result<ReturnType, String>`
///
/// Supported parameter types: `u32`, `f64`, `bool`, `String`, `HostValue`,
/// and `Option<T>` wrappers of those (mapped to optional TS parameters).
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
            let ty = &p.ty;
            let ty_str = normalize_ty(&quote!(#ty).to_string());
            let (ts_ty, optional) = rust_type_to_ts(&ty_str);
            if optional {
                format!("{}?: {}", p.name, ts_ty)
            } else {
                format!("{}: {}", p.name, ts_ty)
            }
        })
        .collect();
    let ts_ret = match &func.sig.output {
        ReturnType::Default => "void".to_string(),
        ReturnType::Type(_, ty) => {
            let ty_str = quote!(#ty).to_string();
            return_type_to_ts(&ty_str).to_string()
        }
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

/// Map a Rust type string to a TypeScript type string.
///
/// Returns the TypeScript type and whether the parameter is optional
/// (`Option<T>` maps to an optional TS parameter).
fn rust_type_to_ts(ty_str: &str) -> (&'static str, bool) {
    if let Some(inner) = ty_str
        .strip_prefix("Option<")
        .and_then(|s| s.strip_suffix('>'))
    {
        return (rust_type_to_ts(inner).0, true);
    }
    let ts = match ty_str.trim_start_matches('&') {
        "u32" | "i32" | "u64" | "i64" | "usize" | "isize" | "f32" | "f64" => "number",
        "bool" => "boolean",
        "String" | "str" | "&str" => "string",
        "Vec<u8>" => "Uint8Array",
        _ => "any",
    };
    (ts, false)
}

/// Map a return type string to a TypeScript return type.
fn return_type_to_ts(ty_str: &str) -> &'static str {
    let ty_str = ty_str.trim();
    if ty_str.contains("Result") {
        if ty_str.contains("()") {
            "void"
        } else if ty_str.contains("bool") {
            "boolean"
        } else if ty_str.contains("String") && !ty_str.contains("Vec<u8>") {
            "string"
        } else if ty_str.contains("Vec<u8>") {
            "Uint8Array"
        } else {
            "any"
        }
    } else if ty_str == "()" {
        "void"
    } else {
        "any"
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
            "u32" | "u64" | "usize" => quote! { |v| v.as_u32().map(|n| n as _) },
            "i32" | "i64" | "isize" => quote! { |v| v.as_u32().map(|n| n as _) },
            "f64" | "f32" => quote! { |v| v.as_f64().map(|n| n as _) },
            "bool" => quote! { |v| v.as_bool() },
            "String" | "str" => quote! { |v| v.as_str().map(|s| s.to_string()) },
            _ => quote! { |_| Option::None },
        };
        return quote! {
            let #name_ident: #ty = args.get(#index_lit).and_then(#extraction);
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
        // For HostValue, pass through directly.
        s if s.contains("HostValue") => quote! {
            let #name_ident = args
                .get(#index_lit)
                .cloned()
                .unwrap_or(::moonfield_script::script::HostValue::Null);
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
