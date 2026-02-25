// SPDX-License-Identifier: GPL-2.0

//! SuperBlock wrapper for filesystem implementations.

use core::marker::PhantomData;

use crate::error::Error;
use crate::types::{ARef, Opaque};
use rko_sys::rko::{dcache as dcache_b, fs as bindings};

use super::inode::{INode, NewINode};

type Result<T = ()> = core::result::Result<T, Error>;

/// I_NEW flag — set on freshly-allocated inodes from `iget_locked`.
const I_NEW: u32 = 1;

/// Wraps the kernel's `struct super_block`.
///
/// The kernel manages the super_block lifetime; this wrapper provides
/// typed access and safe helpers.
///
/// # Invariants
///
/// The inner pointer is valid for the entire duration of filesystem
/// callbacks (fill_super through kill_sb). `s_fs_info` points to `T`
/// data owned by the caller.
#[repr(transparent)]
pub struct SuperBlock<T: super::Type>(Opaque<bindings::super_block>, PhantomData<T>);

impl<T: super::Type> SuperBlock<T> {
    /// Creates a reference from a raw `*mut super_block`.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid and the caller must ensure the lifetime of
    /// the reference does not outlive the super_block.
    pub unsafe fn from_raw<'a>(ptr: *mut bindings::super_block) -> &'a Self {
        unsafe { &*ptr.cast() }
    }

    /// Returns the raw `*mut super_block`.
    pub fn as_ptr(&self) -> *mut bindings::super_block {
        self.0.get()
    }

    /// Sets basic super_block parameters for a simple filesystem.
    ///
    /// Should be called early in `fill_super`.
    pub fn init_simple(&self, params: &SuperParams) {
        let sb = self.0.get();
        // SAFETY: We have exclusive access during fill_super.
        // Note: s_op is set by the vtable before fill_super is called.
        unsafe {
            (*sb).s_maxbytes = params.maxbytes;
            (*sb).s_blocksize = 1u64 << params.blocksize_bits;
            (*sb).s_blocksize_bits = params.blocksize_bits;
            (*sb).s_magic = params.magic;
            (*sb).s_time_gran = params.time_gran;
        }
    }

    /// Stores a pointer to filesystem-private data in `s_fs_info`.
    ///
    /// # Safety
    ///
    /// The pointed-to data must live at least as long as the super_block
    /// and must not be aliased mutably.
    pub unsafe fn set_fs_info(&self, ptr: *mut core::ffi::c_void) {
        unsafe { (*self.0.get()).s_fs_info = ptr };
    }

    /// Retrieves the filesystem-private data from `s_fs_info`.
    ///
    /// # Safety
    ///
    /// The stored pointer must have been set via `set_fs_info` with the
    /// correct type and must still be valid.
    pub unsafe fn fs_info<D>(&self) -> *mut D {
        unsafe { (*self.0.get()).s_fs_info.cast() }
    }

    /// Looks up or allocates an inode by number.
    ///
    /// If the inode is already cached, returns `Ok(Err(aref))` with
    /// the existing inode. If freshly allocated (I_NEW set), returns
    /// `Ok(Ok(new_inode))` which must be initialized and unlocked.
    pub fn iget(&self, ino: u64) -> Result<core::result::Result<NewINode<T>, ARef<INode<T>>>> {
        let inode = unsafe { bindings::iget_locked(self.0.get(), ino) };
        if inode.is_null() {
            return Err(Error::new(-12)); // ENOMEM
        }
        let state = unsafe { (*inode).i_state.__state };
        if state & I_NEW != 0 {
            // Freshly allocated — caller must initialize.
            // SAFETY: We hold the only reference to a new inode.
            let aref = unsafe { ARef::from_raw(core::ptr::NonNull::new_unchecked(inode.cast())) };
            Ok(Ok(NewINode::new(aref)))
        } else {
            // Already cached — bump refcount and return.
            // iget_locked returns with an elevated refcount already.
            let aref = unsafe { ARef::from_raw(core::ptr::NonNull::new_unchecked(inode.cast())) };
            Ok(Err(aref))
        }
    }

    /// Creates the root dentry for the superblock from a root inode.
    ///
    /// Consumes the inode reference. On success, sets `sb->s_root`.
    /// On failure, returns ENOMEM.
    pub fn set_root(&self, root_inode: ARef<INode<T>>) -> Result {
        // d_make_root consumes the inode reference (calls iput on failure).
        let inode_ptr = ARef::into_raw(root_inode);
        let dentry =
            unsafe { dcache_b::d_make_root(inode_ptr.as_ptr().cast::<core::ffi::c_void>()) };
        if dentry.is_null() {
            return Err(Error::new(-12)); // ENOMEM
        }
        unsafe { (*self.0.get()).s_root = dentry };
        Ok(())
    }
}

/// Parameters for initializing a super_block.
pub struct SuperParams {
    /// Maximum file size.
    pub maxbytes: i64,
    /// Block size as log2 (e.g., 12 for 4096).
    pub blocksize_bits: u8,
    /// Filesystem magic number.
    pub magic: u64,
    /// Timestamp granularity in nanoseconds.
    pub time_gran: u32,
}
