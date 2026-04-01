// SPDX-License-Identifier: GPL-2.0

//! iomap — block I/O mapping abstraction.
//!
//! Provides `iomap::Operations` trait for filesystems to map file offsets
//! to block device addresses, and `ro_aops()` to create read-only
//! address_space_operations backed by iomap.

use core::ffi::c_void;
use core::marker::PhantomData;

use rko_sys::rko::{fs as fs_b, iomap as bindings};

use super::Offset;
use super::inode::INode;

/// iomap mapping type.
#[repr(u16)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Type {
    /// No blocks allocated, need allocation.
    Hole = bindings::IOMAP_HOLE as u16,
    /// Delayed allocation blocks.
    DelAlloc = bindings::IOMAP_DELALLOC as u16,
    /// Blocks allocated at addr.
    Mapped = bindings::IOMAP_MAPPED as u16,
    /// Blocks allocated at addr in unwritten state.
    Unwritten = bindings::IOMAP_UNWRITTEN as u16,
    /// Data inline in the inode.
    Inline = bindings::IOMAP_INLINE as u16,
}

/// Flags reported by the filesystem from iomap_begin.
pub mod map_flags {
    /// Blocks have been newly allocated and need zeroing.
    pub const NEW: u16 = 0x01;
    /// Dirty data needs to be written back.
    pub const DIRTY: u16 = 0x02;
    /// Shared extent (e.g. reflinked).
    pub const SHARED: u16 = 0x04;
    /// Multiple extents merged into one.
    pub const MERGED: u16 = 0x08;
}

/// Builder-style wrapper around `struct iomap`.
///
/// Filesystem's `begin()` callback fills this via setter methods.
#[repr(transparent)]
pub struct Map<'a>(bindings::iomap, PhantomData<&'a ()>);

impl<'a> Map<'a> {
    /// Set the mapping type.
    pub fn set_type(&mut self, t: Type) -> &mut Self {
        self.0.r#type = t as u16;
        self
    }

    /// Set the file offset of the mapping.
    pub fn set_offset(&mut self, v: Offset) -> &mut Self {
        self.0.offset = v;
        self
    }

    /// Set the length of the mapping in bytes.
    pub fn set_length(&mut self, len: u64) -> &mut Self {
        self.0.length = len;
        self
    }

    /// Set the flags for this mapping.
    pub fn set_flags(&mut self, flags: u16) -> &mut Self {
        self.0.flags = flags;
        self
    }

    /// Set the physical disk address of the mapping.
    pub fn set_addr(&mut self, addr: u64) -> &mut Self {
        self.0.addr = addr;
        self
    }

    /// Set the block device for this mapping.
    ///
    /// `bdev` is the raw `*mut block_device` from `SuperBlock::bdev_raw()`.
    pub fn set_bdev_raw(&mut self, bdev: *mut c_void) -> &mut Self {
        self.0.bdev = bdev.cast();
        self
    }
}

/// Trait for filesystems that provide iomap block mapping.
///
/// Implement `begin()` to map file offsets to block device addresses.
/// The kernel calls this during read/write I/O to determine where
/// data lives on disk.
pub trait Operations {
    /// The filesystem type this iomap is for.
    type FileSystem: super::FileSystem;

    /// Map a file range to a block device range.
    ///
    /// Fill `map` with the mapping for `pos..pos+length`.
    /// `srcmap` is used for copy-on-write scenarios.
    fn begin<'a>(
        inode: &'a INode<Self::FileSystem>,
        pos: Offset,
        length: Offset,
        flags: u32,
        map: &mut Map<'a>,
        srcmap: &mut Map<'a>,
    ) -> super::Result;

    /// Called after I/O completes on a mapped range (optional).
    fn end<'a>(
        _inode: &'a INode<Self::FileSystem>,
        _pos: Offset,
        _length: Offset,
        _written: isize,
        _flags: u32,
        _map: &Map<'a>,
    ) -> super::Result {
        Ok(())
    }
}

/// C trampoline for `iomap_ops.iomap_begin`.
unsafe extern "C" fn iomap_begin_trampoline<T: Operations>(
    inode: *mut fs_b::inode,
    pos: i64,
    length: i64,
    flags: u32,
    map: *mut bindings::iomap,
    srcmap: *mut bindings::iomap,
) -> i32 {
    let inode_ref = unsafe { &*(inode as *const INode<T::FileSystem>) };
    let map_ref = unsafe { &mut *(map as *mut Map<'_>) };
    let srcmap_ref = unsafe { &mut *(srcmap as *mut Map<'_>) };

    match T::begin(inode_ref, pos, length, flags, map_ref, srcmap_ref) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

/// C trampoline for `iomap_ops.iomap_end`.
unsafe extern "C" fn iomap_end_trampoline<T: Operations>(
    inode: *mut fs_b::inode,
    pos: i64,
    length: i64,
    written: isize,
    flags: u32,
    map: *mut bindings::iomap,
) -> i32 {
    let inode_ref = unsafe { &*(inode as *const INode<T::FileSystem>) };
    let map_ref = unsafe { &*(map as *const Map<'_>) };

    match T::end(inode_ref, pos, length, written, flags, map_ref) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

/// Returns a static `iomap_ops` vtable for type `T`.
pub const fn iomap_ops<T: Operations>() -> bindings::iomap_ops {
    bindings::iomap_ops {
        iomap_begin: iomap_begin_trampoline::<T> as *mut isize,
        iomap_end: iomap_end_trampoline::<T> as *mut isize,
    }
}

/// Create read-only `address_space_operations` backed by iomap.
///
/// The returned ops use `iomap_read_folio` and `iomap_readahead`
/// with the given `iomap_ops` for block mapping.
///
/// Store the returned ops in a `static` and pass to `NewINode::set_aops()`.
pub const fn ro_aops<T: Operations>() -> RoAops<T> {
    RoAops {
        ops: iomap_ops::<T>(),
        _marker: PhantomData,
    }
}

/// Holds the iomap_ops and provides access to the
/// address_space_operations read_folio callback.
pub struct RoAops<T: Operations> {
    ops: bindings::iomap_ops,
    _marker: PhantomData<T>,
}

impl<T: Operations> RoAops<T> {
    /// Returns a pointer to the `iomap_ops` for use in address_space
    /// read_folio callbacks.
    pub fn iomap_ops_ptr(&self) -> *const bindings::iomap_ops {
        &self.ops
    }
}

// SAFETY: RoAops contains only function pointers.
unsafe impl<T: Operations> Send for RoAops<T> {}
unsafe impl<T: Operations> Sync for RoAops<T> {}
