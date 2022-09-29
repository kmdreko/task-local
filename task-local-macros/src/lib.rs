//! This crate provides a derive macro for the task-local crate. This should
//! not be used directly.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, Error};

#[proc_macro_derive(TaskLocal)]
pub fn derive_task_local(item: TokenStream) -> TokenStream {
    let def: DeriveInput = match syn::parse(item) {
        Ok(def) => def,
        Err(err) => return err.into_compile_error().into(),
    };

    if !def.generics.params.is_empty() {
        return Error::new_spanned(
            def.generics,
            "generics are not supported by #[derive(TaskLocal)]",
        )
        .into_compile_error()
        .into();
    }

    let name = &def.ident;

    let output = quote! {
        impl ::task_local::TaskLocal for #name {
            fn key() -> &'static ::std::thread::LocalKey<::task_local::Storage<#name>> {
                ::std::thread_local!(static STORAGE: ::task_local::Storage<#name> = <::task_local::Storage<#name> as ::std::default::Default>::default());
                &STORAGE
            }
        }
    };

    output.into()
}
