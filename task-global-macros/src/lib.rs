use proc_macro::TokenStream;
use quote::quote;
use syn::DeriveInput;

#[proc_macro_derive(TaskGlobal)]
pub fn derive_task_global(item: TokenStream) -> TokenStream {
    let def: DeriveInput = syn::parse(item).unwrap();

    if !def.generics.params.is_empty() {
        panic!("generics are not supported");
    }

    let name = &def.ident;

    let output = quote! {
        impl ::task_global::TaskGlobal for #name {
            fn key() -> &'static ::std::thread::LocalKey<::task_global::TaskGlobalStorage<#name>> {
                ::std::thread_local!(static STORAGE: ::task_global::TaskGlobalStorage<#name> = <::task_global::TaskGlobalStorage<#name> as ::std::default::Default>::default());
                &STORAGE
            }
        }
    };

    output.into()
}
