// SPDX-License-Identifier: GPL-2.0

//! `ForeignOwnable` — trait for data stored in C `void *` fields.
//!
//! Types implementing this trait can be safely converted to/from raw
//! pointers for storage in kernel structures (e.g., `s_fs_info`,
//! `i_private`).

use core::ffi::c_void;

/// Trait for Rust types that can be stored in kernel `void *` fields.
///
/// # Safety
///
/// Implementors must ensure:
/// - `into_foreign` produces a valid pointer that `from_foreign` can
///   reconstruct the original value from.
/// - `from_foreign` is only called once per `into_foreign` call.
/// - `borrow` does not take ownership.
pub unsafe trait ForeignOwnable: Sized + Send {
    /// The type returned by [`borrow`](Self::borrow).
    ///
    /// For `KBox<T>`: `Borrowed<'a> = &'a T`.
    /// For `CString`: `Borrowed<'a> = &'a CStr`.
    /// For `()`: `Borrowed<'a> = ()`.
    type Borrowed<'a>
    where
        Self: 'a;

    /// Converts the object into a raw pointer.
    ///
    /// The caller is responsible for eventually calling `from_foreign`
    /// to reclaim the object (or it will leak).
    fn into_foreign(self) -> *const c_void;

    /// Reconstructs the object from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must have been produced by a prior call to `into_foreign`,
    /// and must not have been passed to `from_foreign` yet.
    unsafe fn from_foreign(ptr: *const c_void) -> Self;

    /// Temporarily borrows the object behind the pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must have been produced by `into_foreign` and not yet
    /// reclaimed. The returned borrow must not outlive the foreign
    /// storage.
    unsafe fn borrow<'a>(ptr: *const c_void) -> Self::Borrowed<'a>;
}

// SAFETY: `()` has no data — all conversions are trivial.
unsafe impl ForeignOwnable for () {
    type Borrowed<'a> = ();

    fn into_foreign(self) -> *const c_void {
        core::ptr::null()
    }

    unsafe fn from_foreign(_ptr: *const c_void) -> Self {}

    unsafe fn borrow<'a>(_ptr: *const c_void) -> Self::Borrowed<'a> {}
}
