// SPDX-License-Identifier: GPL-2.0

//! Filesystem registration (RAII wrapper for `register_filesystem`).

use core::ffi::c_void;
use core::mem::{align_of, size_of};
use core::pin::Pin;
use core::ptr;

use crate::error::Error;
use rko_sys::rko::{
    fs as bindings, fs_context as fc_bindings, gfp as gfp_b, helpers as bindings_h, slab as slab_b,
};

use super::FileSystem;
use super::inode::INodeWithData;
use super::vtable;

type Result<T = ()> = core::result::Result<T, Error>;

/// SLAB flags for inode cache.
const SLAB_RECLAIM_ACCOUNT: u64 = 1u64 << slab_b::_SLAB_RECLAIM_ACCOUNT;

/// GFP_KERNEL = __GFP_RECLAIM | __GFP_IO | __GFP_FS
///            = (DIRECT_RECLAIM | KSWAPD_RECLAIM) | IO | FS
pub(crate) const GFP_KERNEL: u32 = (1 << gfp_b::___GFP_DIRECT_RECLAIM_BIT)
    | (1 << gfp_b::___GFP_KSWAPD_RECLAIM_BIT)
    | (1 << gfp_b::___GFP_IO_BIT)
    | (1 << gfp_b::___GFP_FS_BIT);

/// RAII filesystem registration.
///
/// Holds the `file_system_type`, `fs_context_operations`, and the inode
/// slab cache. Must be pinned because the kernel retains pointers.
pub struct Registration {
    // IMPORTANT: fs_type must be the first field so that
    // `fc->fs_type` can be cast directly to `*const Registration`.
    fs_type: bindings::file_system_type,
    ctx_ops: fc_bindings::fs_context_operations,
    inode_cache: *mut c_void,
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
            let cache = unsafe {
                bindings_h::rust_helper_kmem_cache_create(
                    T::NAME.as_ptr().cast(),
                    size_of::<INodeWithData<T::INodeData>>() as u32,
                    align_of::<INodeWithData<T::INodeData>>() as u32,
                    SLAB_RECLAIM_ACCOUNT,
                    inode_init_once_callback::<T> as *mut isize,
                )
            };
            if cache.is_null() {
                return Err(Error::new(-12)); // ENOMEM
            }
            cache
        } else {
            ptr::null_mut()
        };

        Ok(Self {
            fs_type,
            ctx_ops,
            inode_cache,
            registered: false,
        })
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
        self.inode_cache
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
/// Recovers the `Registration` from `fc->fs_type` (first field),
/// then sets `fc->ops`.
unsafe extern "C" fn init_fs_context_trampoline(fc: *mut fc_bindings::fs_context) -> i32 {
    let fs_type_ptr = unsafe { (*fc).fs_type };
    let reg = fs_type_ptr as *const Registration;
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
        if !self.inode_cache.is_null() {
            unsafe { slab_b::kmem_cache_destroy(self.inode_cache) };
        }
    }
}

// SAFETY: Registration is only accessed from module init/exit.
unsafe impl Send for Registration {}
unsafe impl Sync for Registration {}
