//! Kernel-allocator-backed `Vec<T, A>`.

use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr::NonNull;
use core::{fmt, ptr, slice};

use super::allocator::Kmalloc;
use super::layout::array_layout;
use super::{AllocError, Allocator, Flags};

/// A contiguous growable array backed by a kernel allocator.
///
/// Like `std::vec::Vec` but every allocation takes `Flags` (GFP flags),
/// and allocation failure returns `AllocError` instead of panicking.
pub struct Vec<T, A: Allocator = Kmalloc> {
    ptr: NonNull<T>,
    len: usize,
    cap: usize,
    _alloc: PhantomData<A>,
}

/// `KVec<T>` is `Vec<T, Kmalloc>` — the most common kernel vector type.
pub type KVec<T> = Vec<T, Kmalloc>;

// SAFETY: Vec is Send/Sync if T is, same as std Vec.
unsafe impl<T: Send, A: Allocator> Send for Vec<T, A> {}
unsafe impl<T: Sync, A: Allocator> Sync for Vec<T, A> {}

impl<T, A: Allocator> Default for Vec<T, A> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, A: Allocator> Vec<T, A> {
    /// Creates an empty `Vec` without allocating.
    pub const fn new() -> Self {
        Self {
            ptr: NonNull::dangling(),
            len: 0,
            cap: 0,
            _alloc: PhantomData,
        }
    }

    /// Creates a `Vec` with at least `capacity` slots pre-allocated.
    pub fn with_capacity(capacity: usize, flags: Flags) -> Result<Self, AllocError> {
        let mut v = Self::new();
        v.reserve(capacity, flags)?;
        Ok(v)
    }

    /// Returns the number of elements.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Returns true if empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the current capacity.
    pub const fn capacity(&self) -> usize {
        self.cap
    }

    /// Appends an element, growing if needed.
    pub fn push(&mut self, val: T, flags: Flags) -> Result<(), AllocError> {
        if self.len == self.cap {
            self.grow(flags)?;
        }
        // SAFETY: len < cap after grow, so ptr+len is valid.
        unsafe {
            self.ptr.as_ptr().add(self.len).write(val);
        }
        self.len += 1;
        Ok(())
    }

    /// Removes and returns the last element, or `None` if empty.
    pub fn pop(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        self.len -= 1;
        // SAFETY: len was > 0, element at self.len is initialized.
        Some(unsafe { self.ptr.as_ptr().add(self.len).read() })
    }

    /// Returns a slice of the initialized elements.
    pub fn as_slice(&self) -> &[T] {
        // SAFETY: ptr..ptr+len are initialized.
        unsafe { slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Returns a mutable slice of the initialized elements.
    pub fn as_mut_slice(&mut self) -> &mut [T] {
        // SAFETY: ptr..ptr+len are initialized.
        unsafe { slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }

    /// Removes all elements (drops them) without freeing the backing allocation.
    pub fn clear(&mut self) {
        self.truncate(0);
    }

    /// Shortens the vector, dropping elements beyond `new_len`.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len >= self.len {
            return;
        }
        let old_len = self.len;
        self.len = new_len;
        // SAFETY: elements [new_len..old_len] are initialized and need dropping.
        unsafe {
            let tail = slice::from_raw_parts_mut(self.ptr.as_ptr().add(new_len), old_len - new_len);
            ptr::drop_in_place(tail);
        }
    }

    /// Ensures at least `additional` more elements can be pushed without
    /// reallocating.
    pub fn reserve(&mut self, additional: usize, flags: Flags) -> Result<(), AllocError> {
        let needed = self.len.checked_add(additional).ok_or(AllocError)?;
        if needed <= self.cap {
            return Ok(());
        }
        self.realloc_to(needed, flags)
    }

    /// Grows capacity using doubling strategy.
    fn grow(&mut self, flags: Flags) -> Result<(), AllocError> {
        let new_cap = match self.cap {
            0 => 1,
            c => c.checked_mul(2).ok_or(AllocError)?,
        };
        self.realloc_to(new_cap, flags)
    }

    /// Reallocates to at least `new_cap` elements.
    fn realloc_to(&mut self, new_cap: usize, flags: Flags) -> Result<(), AllocError> {
        let new_layout = array_layout::<T>(new_cap)?;
        let old_layout = array_layout::<T>(self.cap)?;

        let old_ptr = if self.cap == 0 {
            None
        } else {
            Some(self.ptr.cast::<u8>())
        };

        // SAFETY: old_ptr is either None or a pointer to our current allocation
        // with old_layout. new_layout has correct size/align for [T; new_cap].
        let raw = unsafe { A::realloc(old_ptr, new_layout, old_layout, flags)? };

        self.ptr = raw.cast();
        self.cap = new_cap;
        Ok(())
    }
}

impl<T: Clone, A: Allocator> Vec<T, A> {
    /// Appends all elements from a slice (cloning each).
    pub fn extend_from_slice(&mut self, other: &[T], flags: Flags) -> Result<(), AllocError> {
        self.reserve(other.len(), flags)?;
        for item in other {
            // reserve guarantees space, push won't reallocate.
            self.push(item.clone(), flags)?;
        }
        Ok(())
    }
}

impl<T, A: Allocator> Drop for Vec<T, A> {
    fn drop(&mut self) {
        // Drop all elements.
        self.clear();
        // Free the backing allocation.
        if self.cap > 0
            && let Ok(layout) = array_layout::<T>(self.cap)
        {
            // SAFETY: ptr was allocated by A with this layout.
            unsafe { A::free(self.ptr.cast(), layout) };
        }
    }
}

impl<T, A: Allocator> Deref for Vec<T, A> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        self.as_slice()
    }
}

impl<T, A: Allocator> DerefMut for Vec<T, A> {
    fn deref_mut(&mut self) -> &mut [T] {
        self.as_mut_slice()
    }
}

impl<T: fmt::Debug, A: Allocator> fmt::Debug for Vec<T, A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_slice(), f)
    }
}

/// Consuming iterator for `Vec<T, A>`.
pub struct IntoIter<T, A: Allocator = Kmalloc> {
    vec: Vec<T, A>,
    pos: usize,
}

impl<T, A: Allocator> Iterator for IntoIter<T, A> {
    type Item = T;
    fn next(&mut self) -> Option<T> {
        if self.pos >= self.vec.len {
            return None;
        }
        // SAFETY: pos < len, element is initialized.
        let val = unsafe { self.vec.ptr.as_ptr().add(self.pos).read() };
        self.pos += 1;
        None.or(Some(val))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.vec.len - self.pos;
        (remaining, Some(remaining))
    }
}

impl<T, A: Allocator> Drop for IntoIter<T, A> {
    fn drop(&mut self) {
        // Drop remaining elements that weren't consumed.
        while self.pos < self.vec.len {
            unsafe { self.vec.ptr.as_ptr().add(self.pos).read() };
            self.pos += 1;
        }
        // Prevent Vec::drop from double-dropping elements.
        self.vec.len = 0;
    }
}

impl<T, A: Allocator> IntoIterator for Vec<T, A> {
    type Item = T;
    type IntoIter = IntoIter<T, A>;
    fn into_iter(self) -> IntoIter<T, A> {
        // We need to prevent Vec's Drop from running since IntoIter takes
        // ownership of the allocation.
        let iter = IntoIter {
            // SAFETY: We're transferring ownership. ManuallyDrop would be
            // cleaner but Vec fields are private, so we read then forget.
            vec: unsafe { ptr::read(&self) },
            pos: 0,
        };
        core::mem::forget(self);
        iter
    }
}
