// SPDX-License-Identifier: GPL-2.0

//! Little-endian types for on-disk structure parsing.
//!
//! Provides `LE<T>` for safe little-endian field access in `#[repr(C)]`
//! on-disk structures, and `FromBytes` for zero-copy parsing from byte slices.

/// A little-endian value of type `T`.
///
/// Stored in memory as little-endian bytes. Use `.value()` to convert
/// to native byte order.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct LE<T: LeInt>(T);

impl<T: LeInt> LE<T> {
    /// Convert to native byte order.
    pub fn value(self) -> T {
        T::from_le(self.0)
    }
}

/// Trait for integer types that support little-endian conversion.
pub trait LeInt: Copy {
    /// Convert from little-endian to native byte order.
    fn from_le(val: Self) -> Self;
}

impl LeInt for u8 {
    fn from_le(val: Self) -> Self {
        val
    }
}

impl LeInt for u16 {
    fn from_le(val: Self) -> Self {
        u16::from_le(val)
    }
}

impl LeInt for u32 {
    fn from_le(val: Self) -> Self {
        u32::from_le(val)
    }
}

impl LeInt for u64 {
    fn from_le(val: Self) -> Self {
        u64::from_le(val)
    }
}

/// Trait for types that can be safely read from a byte slice.
///
/// # Safety
///
/// Implementors must ensure the type is valid for any bit pattern
/// (all fields are integers or other `FromBytes` types). The type
/// must be `#[repr(C)]` with no padding requirements beyond alignment.
pub unsafe trait FromBytes: Sized {
    /// Read a value from `data` at byte offset `offset`.
    ///
    /// Returns `None` if the slice is too short.
    fn from_bytes(data: &[u8], offset: usize) -> Option<&Self> {
        let size = core::mem::size_of::<Self>();
        let align = core::mem::align_of::<Self>();
        if offset.checked_add(size)? > data.len() {
            return None;
        }
        let ptr = data.as_ptr().wrapping_add(offset);
        if !(ptr as usize).is_multiple_of(align) {
            return None;
        }
        // SAFETY: We checked bounds and alignment. The caller guarantees
        // all bit patterns are valid for Self.
        Some(unsafe { &*ptr.cast() })
    }

    /// Read a slice of values from `data`.
    ///
    /// Returns `None` if the slice length isn't a multiple of the struct size
    /// or alignment is wrong.
    fn from_bytes_to_slice(data: &[u8]) -> Option<&[Self]> {
        let size = core::mem::size_of::<Self>();
        if size == 0 || !data.len().is_multiple_of(size) {
            return None;
        }
        let ptr = data.as_ptr();
        let align = core::mem::align_of::<Self>();
        if !(ptr as usize).is_multiple_of(align) {
            return None;
        }
        let count = data.len() / size;
        // SAFETY: We checked bounds, alignment, and size divisibility.
        Some(unsafe { core::slice::from_raw_parts(ptr.cast(), count) })
    }
}

// SAFETY: LE<T> is repr(transparent) over T which is a plain integer,
// valid for any bit pattern.
unsafe impl<T: LeInt> FromBytes for LE<T> {}
