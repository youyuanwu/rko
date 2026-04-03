// SPDX-License-Identifier: GPL-2.0

//! Heap-allocated, NUL-terminated C string.
//!
//! [`CString`] wraps a `KBox<[u8]>` containing a NUL-terminated string
//! with no interior NUL bytes. It implements `Deref<Target = CStr>` for
//! seamless use with kernel APIs.

use core::ffi::CStr;
use core::fmt;
use core::ops::Deref;

use crate::alloc::{Flags, KBox};
use crate::error::Error;

/// A heap-allocated, NUL-terminated byte string.
///
/// # Invariants
///
/// The `buf` always contains at least one byte (the NUL terminator),
/// ends with `b'\0'`, and contains no interior NUL bytes.
pub struct CString {
    buf: KBox<[u8]>,
}

impl CString {
    /// Creates a `CString` from a byte slice (without trailing NUL).
    ///
    /// A NUL terminator is appended automatically. Returns `EINVAL` if
    /// `data` contains interior NUL bytes.
    pub fn try_from_slice(data: &[u8], flags: Flags) -> Result<Self, Error> {
        if data.contains(&0) {
            return Err(Error::EINVAL);
        }
        let len = data.len().checked_add(1).ok_or(Error::ENOMEM)?;
        let mut buf = KBox::new_zeroed_bytes(len, flags)?;
        buf[..data.len()].copy_from_slice(data);
        buf[data.len()] = 0;
        Ok(Self { buf })
    }

    /// Creates a `CString` from a `&CStr` (copies the data).
    pub fn try_from_cstr(s: &CStr, flags: Flags) -> Result<Self, Error> {
        let bytes = s.to_bytes_with_nul();
        let buf = KBox::new_slice(bytes, flags)?;
        Ok(Self { buf })
    }

    /// Returns the string as a `&CStr`.
    pub fn as_cstr(&self) -> &CStr {
        // SAFETY: Invariant guarantees NUL-terminated, no interior NULs.
        unsafe { CStr::from_bytes_with_nul_unchecked(&self.buf) }
    }

    /// Returns a pointer to the C string (for passing to kernel APIs).
    pub fn as_char_ptr(&self) -> *const core::ffi::c_char {
        self.buf.as_ptr().cast()
    }

    /// Returns the length of the string (excluding the NUL terminator).
    pub fn len(&self) -> usize {
        self.buf.len() - 1
    }

    /// Returns true if the string is empty (only contains the NUL byte).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Consumes the `CString` and returns the underlying boxed byte slice.
    pub fn into_bytes_with_nul(self) -> KBox<[u8]> {
        self.buf
    }
}

impl Deref for CString {
    type Target = CStr;

    fn deref(&self) -> &CStr {
        self.as_cstr()
    }
}

impl fmt::Debug for CString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_cstr(), f)
    }
}

impl fmt::Display for CString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in self.as_cstr().to_bytes() {
            if b.is_ascii() && !b.is_ascii_control() {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

// SAFETY: CString owns its buffer, transferring via raw pointer.
unsafe impl crate::types::ForeignOwnable for CString {
    type Borrowed<'a> = &'a CStr;

    fn into_foreign(self) -> *const core::ffi::c_void {
        KBox::into_raw(self.buf).as_ptr().cast()
    }

    unsafe fn from_foreign(ptr: *const core::ffi::c_void) -> Self {
        // Reconstruct the KBox<[u8]> from the raw pointer.
        // SAFETY: ptr was produced by into_foreign above.
        let cstr = unsafe { CStr::from_ptr(ptr.cast()) };
        let len = cstr.to_bytes_with_nul().len();
        let slice_ptr = core::ptr::slice_from_raw_parts_mut(ptr.cast_mut().cast::<u8>(), len);
        let nn = unsafe { core::ptr::NonNull::new_unchecked(slice_ptr) };
        Self {
            buf: unsafe { KBox::from_raw(nn) },
        }
    }

    unsafe fn borrow<'a>(ptr: *const core::ffi::c_void) -> Self::Borrowed<'a> {
        // SAFETY: ptr was produced by into_foreign and points to a
        // NUL-terminated byte array.
        unsafe { CStr::from_ptr(ptr.cast()) }
    }
}
