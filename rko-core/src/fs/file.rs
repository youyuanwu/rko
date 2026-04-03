// SPDX-License-Identifier: GPL-2.0

//! File abstractions — wraps `struct file`.
//!
//! Provides `File<T>` as a typed wrapper around the kernel's `struct file`,
//! parameterized by the filesystem type for type safety.

use core::marker::PhantomData;

use crate::types::Opaque;
use rko_sys::rko::{fs as bindings, helpers as bindings_h};

use super::dentry::DEntry;
use super::inode::INode;

/// Wraps the kernel's `struct file`.
///
/// `File<T>` is always used by reference (`&File<T>`) — the kernel
/// owns the `struct file` and manages its lifecycle. The type parameter
/// `T` ties it to a specific filesystem type for safety.
///
/// # Invariants
///
/// The inner pointer is valid for the duration of the VFS callback
/// that receives it.
#[repr(transparent)]
pub struct File<T: super::FileSystem>(Opaque<bindings::file>, PhantomData<T>);

// SAFETY: File is a transparent wrapper; the kernel ensures file
// validity during callbacks. Files can be shared across threads.
unsafe impl<T: super::FileSystem> Send for File<T> {}
unsafe impl<T: super::FileSystem> Sync for File<T> {}

impl<T: super::FileSystem> File<T> {
    /// Creates a reference from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid, non-null file pointer. The reference
    /// must not outlive the VFS callback.
    #[allow(dead_code)] // Will be used when read_dir trampoline passes File
    pub(crate) unsafe fn from_raw<'a>(ptr: *mut bindings::file) -> &'a Self {
        unsafe { &*ptr.cast() }
    }

    /// Returns the raw `*mut file` pointer.
    pub fn as_ptr(&self) -> *mut bindings::file {
        self.0.get()
    }

    /// Returns the open file flags (O_RDONLY, O_APPEND, etc).
    pub fn flags(&self) -> u32 {
        // SAFETY: f_flags is valid for the lifetime of the file.
        unsafe { (*self.0.get()).f_flags }
    }

    /// Returns the inode this file refers to.
    pub fn inode(&self) -> &INode<T> {
        // SAFETY: file_inode returns a valid inode for a valid file.
        let inode_ptr = unsafe { bindings_h::rust_helper_file_inode(self.0.get()) };
        unsafe { &*inode_ptr.cast() }
    }

    /// Returns the dentry this file was opened from.
    pub fn dentry(&self) -> &DEntry<T> {
        // SAFETY: f_path.dentry is valid for the lifetime of the file.
        let dentry_ptr = unsafe { (*self.0.get()).file__anon_0.f_path.dentry };
        unsafe { DEntry::from_raw(dentry_ptr.cast()) }
    }
}

/// Open file flag constants.
pub mod flags {
    /// Read-only.
    pub const O_RDONLY: u32 = 0o0;
    /// Write-only.
    pub const O_WRONLY: u32 = 0o1;
    /// Read-write.
    pub const O_RDWR: u32 = 0o2;
    /// Create file if it doesn't exist.
    pub const O_CREAT: u32 = 0o100;
    /// Fail if file exists (with O_CREAT).
    pub const O_EXCL: u32 = 0o200;
    /// Truncate file to zero length.
    pub const O_TRUNC: u32 = 0o1000;
    /// Append mode.
    pub const O_APPEND: u32 = 0o2000;
    /// Non-blocking I/O.
    pub const O_NONBLOCK: u32 = 0o4000;
    /// Directory only.
    pub const O_DIRECTORY: u32 = 0o200000;
    /// Don't follow symlinks.
    pub const O_NOFOLLOW: u32 = 0o400000;
}

/// File operation callbacks for directory files.
///
/// Implement this trait on your filesystem type to provide custom
/// `read_dir` behavior.
#[crate::vtable]
pub trait Operations: Sized + Send + Sync + 'static {
    /// The filesystem type these operations belong to.
    type FileSystem: super::FileSystem;

    /// Iterate directory entries.
    ///
    /// The `inode` is locked with `i_rwsem` in shared mode (guaranteed
    /// by the VFS `iterate_shared` contract). Use `emitter.pos()` for
    /// the current position and `emitter.emit()` for each entry.
    fn read_dir(
        file: &File<Self::FileSystem>,
        inode: &crate::types::Locked<'_, super::INode<Self::FileSystem>, super::inode::ReadSem>,
        emitter: &mut super::DirEmitter,
    ) -> super::Result<()>;

    /// Custom seek implementation.
    ///
    /// Override to provide custom seek behavior (e.g., SEEK_DATA/SEEK_HOLE
    /// for sparse files). Default: `generic_file_llseek` (kernel-provided).
    fn seek(
        _file: &File<Self::FileSystem>,
        _offset: super::Offset,
        _whence: super::Whence,
    ) -> super::Result<super::Offset> {
        Err(crate::error::Error::EINVAL)
    }

    /// Read data from a file into a userspace buffer.
    ///
    /// Override to provide custom read logic (e.g., pseudo-files that
    /// generate content on the fly). Write data to `buffer` using
    /// [`Writer::write`](crate::user::Writer::write) and update
    /// `*offset` to reflect the new file position.
    ///
    /// Returns the number of bytes written to `buffer`.
    ///
    /// Default: not overridden — `generic_file_read_iter` is used
    /// (reads from the page cache via `read_folio`).
    fn read(
        _file: &File<Self::FileSystem>,
        _buffer: &mut crate::user::Writer,
        _offset: &mut super::Offset,
    ) -> super::Result<usize> {
        Err(crate::error::Error::EINVAL)
    }

    /// Read data from a file using scatter-gather I/O.
    ///
    /// Modern alternative to [`read`](Self::read). Receives an
    /// [`IoVecIter`](crate::iov::IoVecIter) that supports scatter-gather
    /// buffers, splice, and async I/O. Write data to `iter` using
    /// [`IoVecIter::write`](crate::iov::IoVecIter::write).
    ///
    /// `offset` is the file position to read from.
    /// Returns the number of bytes written to the iterator.
    ///
    /// Default: not overridden — `generic_file_read_iter` is used.
    /// Takes priority over [`read`](Self::read) when both are implemented.
    fn read_iter(
        _file: &File<Self::FileSystem>,
        _iter: &mut crate::iov::IoVecIter,
        _offset: super::Offset,
    ) -> super::Result<usize> {
        Err(crate::error::Error::EINVAL)
    }
}
