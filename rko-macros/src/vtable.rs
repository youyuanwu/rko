// SPDX-License-Identifier: GPL-2.0

//! `#[vtable]` proc macro implementation.
//!
//! Handles two cases:
//! 1. **Trait definition** — adds `const HAS_<METHOD>: bool` for each method:
//!    - Methods with a default body: `HAS_<METHOD> = false` (overridable)
//!    - Methods without a default: `HAS_<METHOD> = true` (always present)
//! 2. **Impl block** — adds `const HAS_<METHOD>: bool = true` for each
//!    method present in the impl block (overrides the trait defaults).

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{Ident, Item, TraitItem, parse_macro_input};

/// Convert a method name to a `HAS_SCREAMING_SNAKE_CASE` identifier.
fn has_const_name(method: &str) -> Ident {
    let upper = method.to_uppercase();
    format_ident!("HAS_{}", upper, span = Span::call_site())
}

pub(crate) fn vtable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as Item);

    match input {
        Item::Trait(mut trait_item) => {
            // For each method, add a HAS_ const.
            // - Methods with default: HAS_ = false (can be overridden to true)
            // - Methods without default: HAS_ = true (required, always present)
            let mut consts = Vec::new();
            for item in &trait_item.items {
                if let TraitItem::Fn(method) = item {
                    let name = method.sig.ident.to_string();
                    let const_name = has_const_name(&name);
                    let default_val = method.default.is_none(); // required → true
                    consts.push((const_name, default_val));
                }
            }

            for (const_name, val) in consts {
                let const_item: TraitItem = syn::parse_quote! {
                    #[doc(hidden)]
                    const #const_name: bool = #val;
                };
                trait_item.items.push(const_item);
            }

            quote! { #trait_item }.into()
        }
        Item::Impl(mut impl_item) => {
            // Emit `const HAS_<METHOD>: bool = true` for every method
            // present in the impl block. This overrides the trait defaults.
            let method_names: Vec<String> = impl_item
                .items
                .iter()
                .filter_map(|item| {
                    if let syn::ImplItem::Fn(m) = item {
                        Some(m.sig.ident.to_string())
                    } else {
                        None
                    }
                })
                .collect();

            for name in &method_names {
                let const_name = has_const_name(name);
                let const_item: syn::ImplItem = syn::parse_quote! {
                    #[doc(hidden)]
                    const #const_name: bool = true;
                };
                impl_item.items.push(const_item);
            }

            quote! { #impl_item }.into()
        }
        _ => syn::Error::new_spanned(
            quote! {},
            "#[vtable] can only be applied to trait definitions or impl blocks",
        )
        .to_compile_error()
        .into(),
    }
}
