// SPDX-License-Identifier: GPL-2.0

//! I/O vector iterator wrapper.
//!
//! Wraps the kernel's `struct iov_iter` for scatter-gather I/O. Used by
//! `file_operations::read_iter` and `write_iter` callbacks to transfer
//! data between kernel and userspace buffers.

use crate::error::Error;
use rko_sys::rko::net as net_b;

/// Wraps `struct iov_iter` for reading/writing to userspace scatter-gather
/// buffers.
///
/// For read paths (reading FROM a file INTO userspace), use [`write`](Self::write)
/// to push data into the iterator. For write paths (writing TO a file FROM
/// userspace), use [`read`](Self::read) to pull data from the iterator.
///
/// This naming follows the kernel convention: "write to iter" means the
/// caller writes data into the iter's destination buffers.
pub struct IoVecIter {
    ptr: *mut net_b::iov_iter,
}

impl IoVecIter {
    /// Creates an `IoVecIter` from a raw `iov_iter` pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid pointer to an initialized `iov_iter` that
    /// remains valid for the lifetime of this wrapper.
    pub(crate) unsafe fn from_raw(ptr: *mut net_b::iov_iter) -> Self {
        Self { ptr }
    }

    /// Returns the number of bytes remaining in the iterator.
    pub fn count(&self) -> usize {
        // SAFETY: ptr is valid per the type invariant.
        unsafe { (*self.ptr).iov_iter__anon_0.iov_iter__anon_0__anon_0.count as usize }
    }

    /// Returns true if no bytes remain.
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }

    /// Writes `data` into the iterator's destination buffers (read path).
    ///
    /// Copies kernel data into the userspace buffers described by the
    /// iterator. Returns the number of bytes actually written.
    /// Returns `EFAULT` if the copy fails entirely.
    pub fn write(&mut self, data: &[u8]) -> Result<usize, Error> {
        let n = data.len().min(self.count());
        if n == 0 {
            return Ok(0);
        }
        // SAFETY: data is a valid kernel buffer, self.ptr is a valid iov_iter.
        // _copy_to_iter copies from kernel to userspace via the iterator.
        let copied =
            unsafe { net_b::_copy_to_iter(data.as_ptr().cast(), n as u64, self.ptr) } as usize;
        if copied == 0 && n > 0 {
            return Err(Error::EFAULT);
        }
        Ok(copied)
    }

    /// Writes all of `data` into the iterator, or returns an error.
    pub fn write_all(&mut self, data: &[u8]) -> Result<(), Error> {
        let written = self.write(data)?;
        if written != data.len() {
            return Err(Error::EFAULT);
        }
        Ok(())
    }

    /// Reads from the iterator's source buffers into `buf` (write path).
    ///
    /// Copies userspace data from the iterator into a kernel buffer.
    /// Returns the number of bytes actually read.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        let n = buf.len().min(self.count());
        if n == 0 {
            return Ok(0);
        }
        // SAFETY: buf is a valid kernel buffer, self.ptr is a valid iov_iter.
        // _copy_from_iter copies from userspace to kernel via the iterator.
        let copied =
            unsafe { net_b::_copy_from_iter(buf.as_mut_ptr().cast(), n as u64, self.ptr) } as usize;
        if copied == 0 && n > 0 {
            return Err(Error::EFAULT);
        }
        Ok(copied)
    }
}
