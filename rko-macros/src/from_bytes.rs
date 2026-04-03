// SPDX-License-Identifier: GPL-2.0

//! `#[derive(FromBytes)]` proc macro implementation.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, parse_macro_input};

pub(crate) fn derive(item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    // Verify #[repr(C)] is present.
    let has_repr_c = input.attrs.iter().any(|attr| {
        if attr.path().is_ident("repr")
            && let Ok(nested) = attr.parse_args::<syn::Ident>()
        {
            return nested == "C";
        }
        false
    });

    if !has_repr_c {
        return syn::Error::new_spanned(
            &input.ident,
            "#[derive(FromBytes)] requires #[repr(C)] on the struct",
        )
        .to_compile_error()
        .into();
    }

    let expanded = quote! {
        // SAFETY: The struct is #[repr(C)] and all fields must be valid
        // for any bit pattern (integers, LE<T>, [u8; N], etc.).
        // The user is responsible for ensuring this contract.
        unsafe impl #impl_generics ::rko_core::types::FromBytes for #name #ty_generics #where_clause {}
    };

    expanded.into()
}
