// SPDX-License-Identifier: GPL-2.0

//! Folio wrappers — groups of contiguous pages in the page cache.
//!
//! `Folio` and `LockedFolio` have an optional state parameter `S`:
//! - `()` (default) — unspecified, no inode association
//! - `PageCache<T>` — belongs to filesystem `T`'s page cache,
//!   provides `inode()` accessor

use core::marker::PhantomData;
use core::ptr;

use crate::error::Error;
use crate::types::{AlwaysRefCounted, Opaque, ScopeGuard};
use rko_sys::rko::{fs as fs_b, helpers as bindings_h, mm_types as mm_b, pagemap as bindings_pg};

type Result<T = ()> = core::result::Result<T, Error>;

/// State marker: folio belongs to a page-cache for filesystem `T`.
pub struct PageCache<T: super::FileSystem>(PhantomData<T>);

/// Wraps the kernel's `struct folio`.
///
/// # Type parameter `S`
///
/// - `()` — unspecified folio (default)
/// - `PageCache<T>` — page-cache folio with `inode()` access
///
/// # Invariants
///
/// Instances are always ref-counted: a call to `folio_get` ensures
/// the allocation remains valid until the matching `folio_put`.
#[repr(C)]
pub struct Folio<S = ()>(Opaque<mm_b::folio>, PhantomData<S>);

// SAFETY: The type invariants guarantee that `Folio` is always ref-counted.
unsafe impl<S> AlwaysRefCounted for Folio<S> {
    fn inc_ref(&self) {
        // SAFETY: The shared reference implies a non-zero refcount.
        unsafe { bindings_h::rust_helper_folio_get(self.0.get()) };
    }

    unsafe fn dec_ref(obj: ptr::NonNull<Self>) {
        // SAFETY: The caller guarantees a non-zero refcount.
        unsafe { bindings_h::rust_helper_folio_put(obj.cast().as_ptr()) }
    }
}

impl<S> Folio<S> {
    /// Returns the byte position of this folio in its file.
    pub fn pos(&self) -> i64 {
        // SAFETY: Valid folio via shared reference.
        unsafe { bindings_h::rust_helper_folio_pos(self.0.get()) }
    }

    /// Returns the byte size of this folio.
    pub fn size(&self) -> usize {
        // SAFETY: Valid folio via shared reference.
        unsafe { bindings_h::rust_helper_folio_size(self.0.get()) as usize }
    }

    /// Flushes the data cache for the pages that make up the folio.
    pub fn flush_dcache(&self) {
        // SAFETY: Valid folio via shared reference.
        unsafe { bindings_h::rust_helper_flush_dcache_folio(self.0.get()) }
    }
}

impl<T: super::FileSystem> Folio<PageCache<T>> {
    /// Returns the inode that owns this page-cache folio.
    pub fn inode(&self) -> &super::inode::INode<T> {
        // SAFETY: For a page-cache folio, mapping->host is the owning inode.
        // The mapping field is at folio.folio__anon_0.folio__anon_0__anon_0.mapping.
        let folio_ptr = self.0.get();
        let mapping = unsafe {
            (*folio_ptr).folio__anon_0.folio__anon_0__anon_0.mapping as *mut fs_b::address_space
        };
        let host = unsafe { (*mapping).host };
        unsafe { &*(host as *const super::inode::INode<T>) }
    }
}

/// A locked [`Folio`]. Automatically unlocked on drop.
///
/// State parameter `S` matches the underlying `Folio<S>`.
pub struct LockedFolio<'a, S = ()>(&'a Folio<S>);

impl<S> LockedFolio<'_, S> {
    /// Creates a new locked folio from a raw pointer.
    ///
    /// # Safety
    ///
    /// The folio must be valid, locked, and the caller transfers unlock
    /// responsibility. The returned `LockedFolio` must not outlive the
    /// refcount that keeps the folio alive.
    pub(crate) unsafe fn from_raw(folio: *mut mm_b::folio) -> Self {
        // SAFETY: Caller guarantees the pointer is valid and locked.
        unsafe { Self(&*folio.cast()) }
    }

    /// Marks the folio as being up to date.
    pub fn mark_uptodate(&mut self) {
        // SAFETY: Valid folio via the locked reference.
        unsafe { bindings_h::rust_helper_folio_mark_uptodate(self.0.0.get()) }
    }

    fn for_each_page(
        &mut self,
        offset: usize,
        len: usize,
        mut cb: impl FnMut(&mut [u8]) -> Result,
    ) -> Result {
        let mut remaining = len;
        let mut next_offset = offset;

        let end = offset.checked_add(len).ok_or(Error::ERANGE)?;
        if end > self.size() {
            return Err(Error::EINVAL);
        }

        while remaining > 0 {
            let page_offset = next_offset & (super::PAGE_SIZE - 1);
            let usable = core::cmp::min(remaining, super::PAGE_SIZE - page_offset);
            let ptr = unsafe {
                bindings_h::rust_helper_kmap_local_folio(self.0.0.get(), next_offset as u64)
            };
            let _guard = ScopeGuard::new(|| unsafe {
                bindings_h::rust_helper_kunmap_local(ptr as *const _)
            });
            // SAFETY: kmap_local_folio returns a valid pointer for `usable` bytes.
            let s = unsafe { core::slice::from_raw_parts_mut(ptr.cast::<u8>(), usable) };
            cb(s)?;

            next_offset += usable;
            remaining -= usable;
        }

        Ok(())
    }

    /// Writes the given slice into the folio at `offset`.
    pub fn write(&mut self, offset: usize, data: &[u8]) -> Result {
        let mut remaining = data;
        self.for_each_page(offset, data.len(), |s| {
            s.copy_from_slice(&remaining[..s.len()]);
            remaining = &remaining[s.len()..];
            Ok(())
        })
    }

    /// Writes zeroes into the folio at `offset` for `len` bytes.
    pub fn zero_out(&mut self, offset: usize, len: usize) -> Result {
        self.for_each_page(offset, len, |s| {
            s.fill(0);
            Ok(())
        })
    }
}

impl<'a, T: super::FileSystem> LockedFolio<'a, PageCache<T>> {
    /// Returns the inode that owns this page-cache folio.
    pub fn inode(&self) -> &super::inode::INode<T> {
        self.0.inode()
    }
}

impl<S> core::ops::Deref for LockedFolio<'_, S> {
    type Target = Folio<S>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<S> Drop for LockedFolio<'_, S> {
    fn drop(&mut self) {
        // SAFETY: Valid folio; we hold the lock and release it here.
        unsafe { bindings_pg::folio_unlock(self.0.0.get()) }
    }
}
