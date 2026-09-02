//! `#[derive(Reflect)]` for `moonfield-reflect`.
//!
//! Deliberately minimal: structs with named fields only (no tuple/unit
//! structs, no enums, no generics). `#[reflect(ignore)]` on a field excludes
//! it from reflection (it then needs no `Reflect` impl of its own).

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, parse_macro_input};

#[proc_macro_derive(Reflect, attributes(reflect))]
pub fn derive_reflect(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_reflect(&input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand_reflect(input: &DeriveInput) -> syn::Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Reflect derive does not support generics",
        ));
    }

    let fields = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return Err(syn::Error::new_spanned(
                    &input.ident,
                    "Reflect derive only supports structs with named fields",
                ));
            }
        },
        _ => {
            return Err(syn::Error::new_spanned(
                &input.ident,
                "Reflect derive only supports structs with named fields",
            ));
        }
    };

    let name = &input.ident;
    let mut info_entries = Vec::new();
    let mut get_arms = Vec::new();
    let mut get_mut_arms = Vec::new();

    for field in fields {
        if field.attrs.iter().any(|attr| {
            attr.path().is_ident("reflect")
                && attr
                    .parse_args::<syn::Ident>()
                    .is_ok_and(|arg| arg == "ignore")
        }) {
            continue;
        }
        let field_ident = field.ident.as_ref().unwrap();
        let field_name = field_ident.to_string();
        let field_ty = &field.ty;

        info_entries.push(quote! {
            ::moonfield_reflect::FieldInfo {
                name: #field_name,
                type_name: ::moonfield_reflect::type_name_of::<#field_ty>,
            }
        });
        get_arms.push(quote! {
            #field_name => Some(&self.#field_ident),
        });
        get_mut_arms.push(quote! {
            #field_name => Some(&mut self.#field_ident),
        });
    }

    Ok(quote! {
        impl ::moonfield_reflect::Reflect for #name {
            fn field_infos(&self) -> &'static [::moonfield_reflect::FieldInfo] {
                const FIELDS: &[::moonfield_reflect::FieldInfo] = &[#(#info_entries),*];
                FIELDS
            }

            fn field(&self, name: &str) -> Option<&dyn ::moonfield_reflect::Reflect> {
                match name {
                    #(#get_arms)*
                    _ => None,
                }
            }

            fn field_mut(&mut self, name: &str) -> Option<&mut dyn ::moonfield_reflect::Reflect> {
                match name {
                    #(#get_mut_arms)*
                    _ => None,
                }
            }

            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn ::core::any::Any {
                self
            }
        }
    })
}
