//! Opaque wrapper for kernel C structs.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

/// A wrapper for kernel C types that should not be directly read or
/// moved by Rust code.
///
/// `Opaque<T>` stores a `T` inside an `UnsafeCell<MaybeUninit<T>>`,
/// meaning:
/// - It may be uninitialized (e.g. a struct that the kernel fills in).
/// - Interior mutability: kernel functions may mutate it via raw
///   pointer even through a shared reference.
/// - Rust will not auto-derive `Send`/`Sync` — callers must opt in.
///
/// Typically used inside a `Pin`, since the kernel holds a pointer to
/// the inner `T`.
#[repr(transparent)]
pub struct Opaque<T> {
    inner: UnsafeCell<MaybeUninit<T>>,
}

impl<T> Opaque<T> {
    /// Create a new `Opaque` with uninitialized contents.
    pub const fn uninit() -> Self {
        Opaque {
            inner: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Return a raw pointer to the inner value.
    pub fn get(&self) -> *mut T {
        self.inner.get().cast()
    }
}
