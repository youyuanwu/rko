// SPDX-License-Identifier: GPL-2.0

//! Directory entry emitter — wraps `struct dir_context`.
//!
//! Used by `read_dir` implementations to emit directory entries
//! back to userspace via the kernel's `dir_emit` helper.

use rko_sys::rko::{fs as bindings, helpers as bindings_h};

/// File offset type.
pub type Offset = i64;

/// Inode number type.
pub type Ino = u64;

/// Type of a directory entry (DT_* constants).
#[repr(u8)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DirEntryType {
    /// Unknown type.
    Unknown = 0,
    /// Named pipe (FIFO).
    Fifo = 1,
    /// Character device.
    Chr = 2,
    /// Directory.
    Dir = 4,
    /// Block device.
    Blk = 6,
    /// Regular file.
    Reg = 8,
    /// Symbolic link.
    Lnk = 10,
    /// Unix domain socket.
    Sock = 12,
    /// Whiteout (union filesystems).
    Wht = 14,
}

/// Seek origin.
#[repr(i32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Whence {
    /// Seek from beginning of file.
    Start = 0, // SEEK_SET
    /// Seek from current position.
    Current = 1, // SEEK_CUR
    /// Seek from end of file.
    End = 2, // SEEK_END
}

impl Whence {
    /// Convert from raw kernel `whence` value.
    pub fn from_raw(v: i32) -> Option<Self> {
        match v {
            0 => Some(Self::Start),
            1 => Some(Self::Current),
            2 => Some(Self::End),
            _ => None,
        }
    }
}

/// Wraps the kernel's `struct dir_context` for emitting directory entries.
///
/// Passed to `read_dir` implementations. The kernel manages the buffer
/// and position tracking; this wrapper provides safe methods.
#[repr(transparent)]
pub struct DirEmitter(bindings::dir_context);

impl DirEmitter {
    /// Creates a `DirEmitter` from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid `dir_context` for the duration of
    /// the returned reference.
    pub(crate) unsafe fn from_raw<'a>(ptr: *mut bindings::dir_context) -> &'a mut Self {
        unsafe { &mut *ptr.cast() }
    }

    /// Current position in the directory stream.
    pub fn pos(&self) -> Offset {
        self.0.pos
    }

    /// Emit one directory entry.
    ///
    /// `pos_inc` is added to the internal position after emission.
    /// Returns `true` if the entry was accepted, `false` if the
    /// userspace buffer is full (stop iterating).
    pub fn emit(&mut self, pos_inc: Offset, name: &[u8], ino: Ino, etype: DirEntryType) -> bool {
        let ok = unsafe {
            bindings_h::rust_helper_dir_emit(
                &mut self.0,
                name.as_ptr().cast(),
                name.len() as i32,
                ino,
                etype as u8,
            )
        };
        if ok {
            self.0.pos += pos_inc;
        }
        ok
    }
}
