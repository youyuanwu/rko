// SPDX-License-Identifier: GPL-2.0

//! Procedural macros for rko kernel module framework.
//!
//! Provides:
//! - `#[vtable]` — auto-generates `HAS_*` consts for trait methods with defaults
//! - `#[derive(FromBytes)]` — zero-copy byte parsing for `#[repr(C)]` structs

use proc_macro::TokenStream;

mod from_bytes;
mod rko_tests;
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

/// Attribute macro for in-kernel test modules.
///
/// Transforms a module containing `#[test]` functions into a runnable test
/// suite. Each `#[test]` fn gets structured assertion macros and the module
/// gains a `pub fn run() -> Result<(), Error>` that executes all tests with
/// PASS/FAIL output.
///
/// ```ignore
/// #[rko_tests]
/// mod tests {
///     #[test]
///     fn it_works() {
///         assert_eq!(1 + 1, 2);
///     }
/// }
///
/// // In module init:
/// tests::run()?;
/// ```
#[proc_macro_attribute]
pub fn rko_tests(_attr: TokenStream, item: TokenStream) -> TokenStream {
    use syn::parse_macro_input;
    rko_tests::rko_tests(parse_macro_input!(item))
        .unwrap_or_else(|e| e.into_compile_error())
        .into()
}
