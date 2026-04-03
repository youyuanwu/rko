// SPDX-License-Identifier: GPL-2.0

//! Filesystem registration (RAII wrapper for `register_filesystem`).

use core::ffi::c_void;
use core::mem::{align_of, size_of};
use core::pin::Pin;
use core::ptr;

use crate::alloc::MemCache;
use crate::error::Error;
use rko_sys::rko::{fs as bindings, fs_context as fc_bindings};

use super::FileSystem;
use super::inode::INodeWithData;
use super::vtable;

type Result<T = ()> = core::result::Result<T, Error>;

/// GFP_KERNEL via the alloc Flags.
pub(crate) const GFP_KERNEL: u32 = crate::alloc::Flags::GFP_KERNEL.bits();

/// RAII filesystem registration.
///
/// Holds the `file_system_type`, `fs_context_operations`, and the inode
/// slab cache. Must be pinned because the kernel retains pointers.
///
/// `#[repr(C)]` guarantees `fs_type` is at offset 0 — required because
/// `alloc_inode_callback` casts `sb->s_type` (a pointer to `fs_type`)
/// directly to `*const Registration`.
#[repr(C)]
pub struct Registration {
    pub(crate) fs_type: bindings::file_system_type,
    ctx_ops: fc_bindings::fs_context_operations,
    inode_cache: Option<MemCache>,
    registered: bool,
}

impl Registration {
    /// Creates a `Registration` for filesystem type `T`.
    ///
    /// Allocates the inode slab cache and wires all callbacks.
    pub fn new_for<T: FileSystem>() -> Result<Self> {
        let ctx_ops = vtable::fs_context_ops::<T>();

        let fs_type = bindings::file_system_type {
            name: T::NAME.as_ptr().cast_mut(),
            kill_sb: vtable::kill_sb_callback::<T> as *mut isize,
            ..Default::default()
        };

        // Create inode slab cache if INodeData is non-ZST.
        let inode_cache = if size_of::<T::INodeData>() != 0 {
            // SAFETY: inode_init_once_callback correctly calls inode_init_once
            // on the embedded inode field of INodeWithData<T::INodeData>.
            let cache = unsafe {
                MemCache::try_new_with_ctor(
                    T::NAME,
                    size_of::<INodeWithData<T::INodeData>>(),
                    align_of::<INodeWithData<T::INodeData>>(),
                    inode_init_once_callback::<T>,
                )?
            };
            Some(cache)
        } else {
            None
        };

        Ok(Self {
            fs_type,
            ctx_ops,
            inode_cache,
            registered: false,
        })
    }

    /// Returns a [`PinInit`] that creates and registers a filesystem in one step.
    ///
    /// This combines [`new_for`](Self::new_for) and [`register`](Self::register)
    /// into a single pin-initializer suitable for use with `KBox::pin_init`.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let reg = KBox::pin_init(Registration::pin_init::<MyFs>(), GFP_KERNEL)?;
    /// ```
    pub fn pin_init<T: FileSystem>() -> impl pinned_init::PinInit<Self, Error> {
        // SAFETY: We fully initialize the Registration in place:
        // 1. Write all fields via new_for()
        // 2. Set init_fs_context (requires stable address → must be pinned)
        // 3. Call register_filesystem (retains pointer → must not move)
        unsafe {
            pinned_init::pin_init_from_closure(move |slot: *mut Self| {
                // Initialize all fields in place.
                let reg = Self::new_for::<T>()?;
                slot.write(reg);

                // Now that we're at a stable address, wire and register.
                (*slot).fs_type.init_fs_context = init_fs_context_trampoline as *mut isize;
                let ret = bindings::register_filesystem(&mut (*slot).fs_type);
                if ret != 0 {
                    // Drop the partially-initialized Registration.
                    core::ptr::drop_in_place(slot);
                    return Err(Error::new(ret));
                }
                (*slot).registered = true;
                Ok(())
            })
        }
    }

    /// Registers the filesystem with the kernel.
    ///
    /// # Safety
    ///
    /// `self` must be pinned and must not move after this call.
    pub unsafe fn register(self: Pin<&mut Self>) -> Result {
        let this = unsafe { self.get_unchecked_mut() };
        this.fs_type.init_fs_context = init_fs_context_trampoline as *mut isize;

        let ret = unsafe { bindings::register_filesystem(&mut this.fs_type) };
        if ret != 0 {
            return Err(Error::new(ret));
        }
        this.registered = true;
        Ok(())
    }

    /// Returns a pointer to the `fs_context_operations` table.
    fn ctx_ops_ptr(&self) -> *mut fc_bindings::fs_context_operations {
        &self.ctx_ops as *const _ as *mut _
    }

    /// Returns the inode slab cache pointer.
    pub(crate) fn inode_cache(&self) -> *mut c_void {
        match &self.inode_cache {
            Some(cache) => cache.as_ptr(),
            None => ptr::null_mut(),
        }
    }
}

/// `inode_init_once` constructor for the slab cache.
///
/// Called once per slab object to initialize the kernel inode internals.
unsafe extern "C" fn inode_init_once_callback<T: FileSystem>(ptr: *mut c_void) {
    let obj = ptr.cast::<INodeWithData<T::INodeData>>();
    unsafe {
        bindings::inode_init_once(ptr::addr_of_mut!((*obj).inode));
    }
}

/// `init_fs_context` callback.
///
/// Recovers the `Registration` from `fc->fs_type` via `container_of`,
/// then sets `fc->ops`.
unsafe extern "C" fn init_fs_context_trampoline(fc: *mut fc_bindings::fs_context) -> i32 {
    let fs_type_ptr = unsafe { (*fc).fs_type };
    // SAFETY: fs_type_ptr points to the fs_type field of a Registration.
    let reg = unsafe { crate::container_of!(fs_type_ptr, Registration, fs_type) };
    unsafe {
        (*fc).ops = (*reg).ctx_ops_ptr();
    }
    0
}

impl Drop for Registration {
    fn drop(&mut self) {
        if self.registered {
            unsafe { bindings::unregister_filesystem(&mut self.fs_type) };
        }
        // MemCache::drop handles kmem_cache_destroy automatically.
    }
}

// SAFETY: Registration is only accessed from module init/exit.
unsafe impl Send for Registration {}
unsafe impl Sync for Registration {}
