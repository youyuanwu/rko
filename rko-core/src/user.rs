// SPDX-License-Identifier: GPL-2.0

//! Userspace memory access helpers.
//!
//! Provides [`Writer`] for copying data to userspace and [`Reader`] for
//! copying from userspace. These wrap `copy_to_user` / `copy_from_user`
//! with bounds checking and automatic pointer advancement.

use crate::error::Error;
use rko_sys::rko::helpers as bindings_h;

/// A writer that copies data to a userspace buffer.
///
/// Created from a raw userspace pointer and length (typically from a
/// `file_operations::read` callback). Each `write` call copies data
/// and advances the internal pointer.
pub struct Writer {
    ptr: *mut u8,
    len: usize,
}

impl Writer {
    /// Creates a new `Writer` from a userspace pointer and length.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid userspace pointer for `len` bytes, as
    /// provided by the kernel's read callback.
    pub(crate) unsafe fn new(ptr: *mut u8, len: usize) -> Self {
        Self { ptr, len }
    }

    /// Returns the remaining capacity in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if no more bytes can be written.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Writes all of `data` to the userspace buffer.
    ///
    /// Returns `EFAULT` if the buffer is too small or the copy fails.
    pub fn write(&mut self, data: &[u8]) -> Result<(), Error> {
        let n = data.len();
        if n > self.len {
            return Err(Error::EFAULT);
        }

        // SAFETY: ptr is a valid userspace address for self.len bytes.
        // data is a valid kernel buffer. copy_to_user handles fault detection.
        let pending = unsafe {
            bindings_h::rust_helper_copy_to_user(self.ptr.cast(), data.as_ptr().cast(), n as u64)
        };
        if pending != 0 {
            return Err(Error::EFAULT);
        }

        self.ptr = self.ptr.wrapping_add(n);
        self.len -= n;
        Ok(())
    }
}

/// A reader that copies data from a userspace buffer.
///
/// Created from a raw userspace pointer and length (typically from a
/// `file_operations::write` callback). Each `read` call copies data
/// and advances the internal pointer.
pub struct Reader {
    ptr: *const u8,
    len: usize,
}

impl Reader {
    /// Creates a new `Reader` from a userspace pointer and length.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid userspace pointer for `len` bytes, as
    /// provided by the kernel's write callback.
    #[allow(dead_code)] // Will be used when file::Operations::write is added
    pub(crate) unsafe fn new(ptr: *const u8, len: usize) -> Self {
        Self { ptr, len }
    }

    /// Returns the remaining bytes available to read.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns true if no more bytes can be read.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Reads from the userspace buffer into `buf`.
    ///
    /// Reads up to `buf.len()` bytes (or remaining, whichever is less).
    /// Returns the number of bytes actually read. Returns `EFAULT` if
    /// the copy fails.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let n = buf.len().min(self.len);
        if n == 0 {
            return Ok(0);
        }

        // SAFETY: ptr is a valid userspace address for self.len bytes.
        // buf is a valid kernel buffer. copy_from_user handles fault detection.
        let pending = unsafe {
            bindings_h::rust_helper_copy_from_user(
                buf.as_mut_ptr().cast(),
                self.ptr.cast(),
                n as u64,
            )
        };
        if pending != 0 {
            return Err(Error::EFAULT);
        }

        self.ptr = self.ptr.wrapping_add(n);
        self.len -= n;
        Ok(n)
    }
}
