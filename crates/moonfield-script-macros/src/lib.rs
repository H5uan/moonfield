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
/// Supported parameter types: `u32`, `f64`, `bool`, `String`, `HostValue`
/// Supported return types: `u32`, `f64`, `bool`, `String`, `HostValue`, `()`, `Vec<u8>`
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
            quote! { Ok(HostValue::Null) }
        }
        ReturnType::Type(_, ty) => {
            let ty_str = quote!(#ty).to_string();
            if ty_str.contains("Result") {
                let inner_is_unit = ty_str.contains("()");
                if inner_is_unit {
                    quote! {
                        match #fn_name(#(#param_names),*) {
                            Ok(()) => Ok(HostValue::Null),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                } else {
                    quote! {
                        match #fn_name(#(#param_names),*) {
                            Ok(val) => Ok(HostValue::from(val)),
                            Err(e) => Err(e.to_string()),
                        }
                    }
                }
            } else {
                quote! {
                    Ok(HostValue::from(#fn_name(#(#param_names),*)))
                }
            }
        }
    };

    let stmt_tokens: Vec<proc_macro2::TokenStream> = func
        .block
        .stmts
        .iter()
        .map(|s| quote!(#s))
        .collect();

    // Use the original function verbatim via quote!(#func), then add the struct + impl.
    let output = quote! {
        #(#attrs)*
        #vis #sig {
            #(#stmt_tokens)*
        }

        #[doc(hidden)]
        #vis struct #struct_name;

        impl crate::script::ScriptFunction for #struct_name {
            const NAME: &'static str = #fn_name_str;

            fn call(args: &[crate::script::HostValue]) -> Result<crate::script::HostValue, String> {
                #(#extractions)*
                #ret_conversion
            }
        }
    };

    output.into()
}

struct ParamInfo {
    name: String,
    ty: Type,
}

/// Generate code to extract a parameter from `args` slice.
fn extract_param_code(name: &str, ty: &Type, index: usize) -> proc_macro2::TokenStream {
    let name_ident = syn::Ident::new(name, proc_macro2::Span::call_site());
    let index_lit = index;

    let ty_str = quote!(#ty).to_string();
    // Strip leading "& " if present (e.g., `&str` → `str`).
    let ty_str = ty_str.trim_start_matches("& ").to_string();

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
                .unwrap_or(moonfield_script::script::HostValue::Null);
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