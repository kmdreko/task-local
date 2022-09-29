//! This crate provides a derive macro for the task-global crate. This should
//! not be used directly.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error};

#[proc_macro_derive(TaskGlobal)]
pub fn derive_task_global(item: TokenStream) -> TokenStream {
    let def: DeriveInput = match syn::parse(item) {
        Ok(def) => def,
        Err(err) => return err.into_compile_error().into(),
    };

    if !def.generics.params.is_empty() {
        return Error::new_spanned(
            def.generics,
            "generics are not supported by #[derive(TaskGlobal)]",
        )
        .into_compile_error()
        .into();
    }

    let name = &def.ident;

    let output = quote! {
        impl ::task_global::TaskGlobal for #name {
            fn key() -> &'static ::std::thread::LocalKey<::task_global::Storage<#name>> {
                ::std::thread_local!(static STORAGE: ::task_global::Storage<#name> = <::task_global::Storage<#name> as ::std::default::Default>::default());
                &STORAGE
            }
        }
    };

    output.into()
}
