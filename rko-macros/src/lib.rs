// SPDX-License-Identifier: GPL-2.0

//! Procedural macros for rko kernel module framework.
//!
//! Provides:
//! - `#[vtable]` — auto-generates `HAS_*` consts for trait methods with defaults
//! - `#[derive(FromBytes)]` — zero-copy byte parsing for `#[repr(C)]` structs

use proc_macro::TokenStream;

mod from_bytes;
mod vtable;

/// Attribute macro for vtable traits and their implementations.
///
/// # On trait definitions
///
/// For each method that has a default implementation, a `const HAS_<METHOD>: bool = false`
/// is added to the trait. This allows vtable constructors to check whether a concrete
/// type has overridden the default.
///
/// ```ignore
/// #[vtable]
/// pub trait FileSystem {
///     fn fill_super(...) -> Result; // required — no HAS_ const
///     fn statfs(...) -> Result { ... } // optional — generates HAS_STATFS
/// }
/// ```
///
/// # On impl blocks
///
/// For each method that the impl block overrides (i.e., methods that have a
/// default in the trait), a `const HAS_<METHOD>: bool = true` is emitted.
///
/// ```ignore
/// #[vtable]
/// impl FileSystem for MyFs {
///     fn fill_super(...) -> Result { ... }
///     fn statfs(...) -> Result { ... } // HAS_STATFS = true
/// }
/// ```
#[proc_macro_attribute]
pub fn vtable(_attr: TokenStream, item: TokenStream) -> TokenStream {
    vtable::vtable(_attr, item)
}

/// Derive macro for zero-copy byte parsing.
///
/// Generates `unsafe impl FromBytes for T {}` and ensures `#[repr(C)]`
/// is present. All fields must be valid for any bit pattern.
///
/// ```ignore
/// #[derive(FromBytes)]
/// #[repr(C)]
/// pub struct Header {
///     pub magic: LE<u32>,
///     pub size: LE<u64>,
/// }
/// ```
#[proc_macro_derive(FromBytes)]
pub fn derive_from_bytes(item: TokenStream) -> TokenStream {
    from_bytes::derive(item)
}
