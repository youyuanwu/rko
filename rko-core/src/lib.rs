//! Rust wrappers for Linux kernel APIs.
//!
//! This crate provides safe(r) abstractions on top of the raw FFI
//! bindings in `rko-sys`. Hand-written modules live here; generated
//! bindings stay in `rko-sys`.

#![no_std]
#![feature(coerce_unsized, dispatch_from_dyn, unsize, arbitrary_self_types)]

pub mod alloc;
pub mod error;
pub mod fs;
pub mod iov;
pub mod kasync;
pub mod kunit;
pub mod module;
pub mod net;
pub mod prelude;
pub mod printk;
pub mod revocable;
pub mod sync;
pub mod task;
pub mod time;
pub mod types;
pub mod unsafe_list;
pub mod user;
pub mod workqueue;

/// Re-export proc macros from `rko-macros`.
pub use rko_macros::{FromBytes, rko_tests, vtable};

/// Re-export pin-init macros and traits for in-place initialization.
pub mod pin_init {
    pub use pinned_init::{
        Init, PinInit, init, init_from_closure, pin_init, pin_init_from_closure, try_init,
        try_pin_init,
    };
}

/// Produces a pointer to an object from a pointer to one of its fields.
///
/// # Safety
///
/// The pointer passed to this macro, and the pointer returned by this
/// macro, must both be in bounds of the same allocation.
///
/// # Examples
///
/// ```ignore
/// let ptr = container_of!(field_ptr, MyStruct, my_field);
/// ```
// UPSTREAM_REF: linux/rust/kernel/lib.rs (container_of! macro)
#[macro_export]
macro_rules! container_of {
    ($field_ptr:expr, $Container:ty, $($fields:tt)*) => {{
        let offset: usize = ::core::mem::offset_of!($Container, $($fields)*);
        let field_ptr = $field_ptr;
        let container_ptr = field_ptr.byte_sub(offset).cast::<$Container>();
        $crate::assert_same_type(field_ptr, (&raw const (*container_ptr).$($fields)*).cast_mut());
        container_ptr
    }}
}

/// Helper for [`container_of!`] — compile-time type check.
#[doc(hidden)]
pub fn assert_same_type<T>(_: T, _: T) {}
