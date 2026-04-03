// SPDX-License-Identifier: GPL-2.0

//! SuperBlock wrapper for filesystem implementations.

use core::ffi::c_void;
use core::marker::PhantomData;

use crate::error::Error;
use crate::types::{ARef, Opaque};
use rko_sys::rko::{dcache as dcache_b, fs as bindings, helpers as bindings_h};

use super::inode::{INode, NewINode};

type Result<T = ()> = core::result::Result<T, Error>;

/// Extracts the inner type from a `ForeignOwnable` wrapper.
///
/// For `KBox<D>`: `Inner = D`. For `()`: `Inner = ()`.
/// Used by `SuperBlock::sb_data()` to return the correct type.
pub trait DataInner {
    /// The inner type stored behind the foreign pointer.
    type Inner;
}

impl DataInner for () {
    type Inner = ();
}

impl<T, A: crate::alloc::Allocator> DataInner for crate::alloc::Box<T, A> {
    type Inner = T;
}

/// I_NEW flag — set on freshly-allocated inodes from `iget_locked`.
const I_NEW: u32 = 1;

/// How a filesystem's superblocks are keyed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// Memory-backed, anonymous mount (`get_tree_nodev`).
    Independent,
    /// Block-device-backed mount (`get_tree_bdev`).
    BlockDev,
}

/// Superblock state: being initialized in `fill_super`.
///
/// Only setup methods are available (`set_magic`, `min_blocksize`, etc.).
/// `data()` and `iget()` are NOT available.
pub enum New {}

/// Superblock state: fully initialized (data stored).
///
/// All methods are available including `data()` and `iget()`.
pub enum Ready {}

/// Wraps the kernel's `struct super_block`.
///
/// The type parameter `S` tracks the superblock state:
/// - `New` — during `fill_super`, before data is stored
/// - `Ready` — after `fill_super`, data is accessible
///
/// # Invariants
///
/// The inner pointer is valid for the entire duration of filesystem
/// callbacks (fill_super through kill_sb).
#[repr(transparent)]
pub struct SuperBlock<T: super::FileSystem, S = Ready>(
    Opaque<bindings::super_block>,
    PhantomData<(T, S)>,
);

// --- Methods available in ALL states ---

impl<T: super::FileSystem, S> SuperBlock<T, S> {
    /// Returns the raw `*mut super_block`.
    pub fn as_ptr(&self) -> *mut bindings::super_block {
        self.0.get()
    }

    /// Sets the filesystem magic number.
    pub fn set_magic(&self, magic: usize) {
        unsafe { (*self.0.get()).s_magic = magic as u64 };
    }

    /// Returns whether the filesystem is mounted read-only.
    pub fn rdonly(&self) -> bool {
        unsafe { (*self.0.get()).s_flags & 1 != 0 }
    }

    /// Returns the raw block device pointer (`s_bdev`).
    pub fn bdev_raw(&self) -> *mut c_void {
        unsafe { (*self.0.get()).s_bdev }
    }

    /// Returns a typed block device reference.
    ///
    /// Only valid for block-device-backed filesystems (`SUPER_TYPE = BlockDev`).
    pub fn bdev(&self) -> &BlockDevice {
        // SAFETY: s_bdev is valid for block-device-backed filesystems.
        unsafe { &*(self.bdev_raw() as *const BlockDevice) }
    }

    /// Returns the total number of sectors on the block device.
    pub fn sector_count(&self) -> u64 {
        unsafe { bindings_h::rust_helper_bdev_nr_sectors(self.bdev_raw()) }
    }

    /// Sets the minimum block size. Returns the actual block size set.
    pub fn min_blocksize(&self, size: i32) -> i32 {
        unsafe { bindings_h::rust_helper_sb_min_blocksize(self.0.get(), size) }
    }
}

// --- Methods only available in New state (fill_super) ---

impl<T: super::FileSystem> SuperBlock<T, New> {
    /// Creates a `New` reference from a raw `*mut super_block`.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid. Used by the vtable before `fill_super`.
    pub(crate) unsafe fn from_raw_new<'a>(ptr: *mut bindings::super_block) -> &'a Self {
        unsafe { &*ptr.cast() }
    }

    /// Sets basic super_block parameters for a simple filesystem.
    ///
    /// Should be called early in `fill_super`.
    pub fn init_simple(&self, params: &SuperParams) {
        let sb = self.0.get();
        // SAFETY: We have exclusive access during fill_super.
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
    /// The pointed-to data must live at least as long as the super_block.
    pub unsafe fn set_fs_info(&self, ptr: *mut c_void) {
        unsafe { (*self.0.get()).s_fs_info = ptr };
    }
}

// --- Methods only available in Ready state ---

impl<T: super::FileSystem> SuperBlock<T, Ready> {
    /// Creates a `Ready` reference from a raw `*mut super_block`.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid and `s_fs_info` must be initialized.
    pub unsafe fn from_raw<'a>(ptr: *mut bindings::super_block) -> &'a Self {
        unsafe { &*ptr.cast() }
    }

    /// Returns the per-superblock data stored in `s_fs_info`.
    ///
    /// The returned reference type depends on `T::Data`:
    /// - For `KBox<D>`: returns `&D` (the inner heap-allocated value)
    /// - For `()`: returns `&()`
    ///
    /// # Safety
    ///
    /// Only valid after `fill_super` has completed and before `kill_sb`.
    /// The caller must ensure `D` matches the actual inner type of `T::Data`.
    pub unsafe fn data<D>(&self) -> &D {
        let ptr = unsafe { (*self.0.get()).s_fs_info };
        // SAFETY: s_fs_info was set from ForeignOwnable::into_foreign in
        // fill_super_callback. The caller specifies the concrete inner type D.
        unsafe { &*ptr.cast::<D>() }
    }

    /// Returns a typed reference to the per-superblock data.
    ///
    /// The type is derived from `T::Data` automatically via `DataInner`.
    /// For `Data = KBox<D>`, returns `&D`. For `Data = ()`, returns `&()`.
    ///
    /// # Safety
    ///
    /// Only valid after `fill_super` has completed and before `kill_sb`.
    pub unsafe fn sb_data(&self) -> &<T::Data as DataInner>::Inner {
        unsafe { self.data() }
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
            return Err(Error::ENOMEM);
        }
        let state = unsafe { (*inode).i_state.__state };
        if state & I_NEW != 0 {
            // Freshly allocated — caller must initialize.
            // SAFETY: We hold the only reference to a new inode.
            let aref = unsafe { ARef::from_raw(core::ptr::NonNull::new_unchecked(inode.cast())) };
            Ok(Ok(NewINode::new(aref)))
        } else {
            // Already cached — iget_locked returns with elevated refcount.
            let aref = unsafe { ARef::from_raw(core::ptr::NonNull::new_unchecked(inode.cast())) };
            Ok(Err(aref))
        }
    }

    /// Creates the root dentry for the superblock from a root inode.
    ///
    /// Consumes the inode reference. On success, sets `sb->s_root`.
    pub fn set_root(&self, root_inode: ARef<INode<T>>) -> Result {
        // d_make_root consumes the inode reference (calls iput on failure).
        let inode_ptr = ARef::into_raw(root_inode);
        let dentry = unsafe { dcache_b::d_make_root(inode_ptr.as_ptr().cast::<c_void>()) };
        if dentry.is_null() {
            return Err(Error::ENOMEM);
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

/// Typed wrapper for `struct block_device`.
///
/// Always used by reference — the kernel owns the device.
#[repr(transparent)]
pub struct BlockDevice(crate::types::Opaque<core::ffi::c_void>);

impl BlockDevice {
    /// Returns the raw `*mut c_void` pointer for FFI.
    pub fn as_ptr(&self) -> *mut core::ffi::c_void {
        self.0.get()
    }
}
