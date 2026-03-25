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

    /// Create a new `Opaque` with a known value.
    pub fn new(value: T) -> Self {
        Opaque {
            inner: UnsafeCell::new(MaybeUninit::new(value)),
        }
    }

    /// Return a raw pointer to the inner value.
    pub fn get(&self) -> *mut T {
        self.inner.get().cast()
    }

    /// Return a `*mut T` from a `*mut Opaque<T>` without creating a reference.
    ///
    /// This is useful for initializing the inner value in place.
    pub fn raw_get(slot: *mut Self) -> *mut T {
        let cell_ptr: *const UnsafeCell<MaybeUninit<T>> = slot.cast();
        UnsafeCell::raw_get(cell_ptr).cast::<T>()
    }

    /// Create a pin-initializer from an FFI initialization function.
    ///
    /// The closure receives a `*mut T` pointing at the slot to be
    /// initialized and must fill it in (the kernel side typically does
    /// this via a C helper call).
    // UPSTREAM_REF: linux/rust/kernel/types.rs Opaque::ffi_init
    pub fn ffi_init(init_fn: impl FnOnce(*mut T)) -> impl pinned_init::PinInit<Self> {
        // SAFETY: `init_fn` is responsible for fully initializing the T.
        // The MaybeUninit wrapper ensures no drop glue runs on partial init.
        unsafe {
            pinned_init::pin_init_from_closure::<_, core::convert::Infallible>(
                move |slot: *mut Self| {
                    init_fn(Self::raw_get(slot));
                    Ok(())
                },
            )
        }
    }
}
