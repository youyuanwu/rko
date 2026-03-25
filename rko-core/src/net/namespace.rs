//! Network namespace wrapper.
//!
//! The kernel's `struct net` represents a network namespace. We wrap it
//! as an opaque reference — `Namespace` IS the `struct net` in memory
//! (zero-sized wrapper over the extern symbol for `init_ns()`).

use core::ffi::c_void;
use core::ptr::NonNull;

use crate::types::AlwaysRefCounted;

/// A network namespace (`struct net`).
///
/// This is a zero-sized wrapper — a `&Namespace` is a pointer directly
/// to a `struct net`. We never construct a `Namespace` by value; we
/// only hand out references.
pub struct Namespace(());

// SAFETY: Network namespaces are refcounted and safe to use from any thread.
unsafe impl Send for Namespace {}
unsafe impl Sync for Namespace {}

// SAFETY: `get_net` / `put_net` maintain the kernel refcount,
// wired up via C helpers.
unsafe impl AlwaysRefCounted for Namespace {
    fn inc_ref(&self) {
        // SAFETY: self points to a valid struct net.
        unsafe { rko_sys::rko::helpers::rust_helper_get_net(self.as_ptr()) };
    }

    unsafe fn dec_ref(obj: NonNull<Self>) {
        // SAFETY: obj points to a valid Namespace with non-zero refcount.
        unsafe { rko_sys::rko::helpers::rust_helper_put_net(obj.as_ptr().cast()) };
    }
}

impl Namespace {
    /// Returns the init network namespace (`&init_net`).
    ///
    /// # Safety
    ///
    /// The init_net namespace lives for the entire kernel lifetime, so a
    /// `'static` reference is sound.
    pub fn init_ns() -> &'static Self {
        // SAFETY: `init_net` is a global symbol exported by the kernel.
        // It is valid for the entire kernel lifetime. We cast the pointer
        // to &Namespace — since Namespace is a ZST wrapper, &Namespace
        // is just a pointer to the struct net.
        unsafe {
            unsafe extern "C" {
                #[link_name = "init_net"]
                static INIT_NET: u8;
            }
            &*(&raw const INIT_NET as *const Namespace)
        }
    }

    /// Return the raw `struct net *` for passing to kernel socket APIs.
    ///
    /// Since `Namespace` is a ZST wrapper, `&self` IS the pointer to
    /// the `struct net`.
    pub(crate) fn as_ptr(&self) -> *mut c_void {
        (self as *const Namespace).cast_mut().cast()
    }
}
