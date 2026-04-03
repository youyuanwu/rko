// SPDX-License-Identifier: GPL-2.0

//! Safe wrapper around `struct kmem_cache` (SLAB/SLUB allocator).
//!
//! `MemCache` manages a dedicated slab cache for fixed-size objects.
//! It wraps `kmem_cache_create` / `kmem_cache_destroy` with RAII and
//! provides typed alloc/free operations.

use core::ffi::c_void;

use rko_sys::rko::{helpers as bindings_h, slab as slab_b};

/// A kernel slab cache for fixed-size allocations.
///
/// Created via [`MemCache::try_new`] and automatically destroyed on drop.
/// Objects allocated from this cache must be freed back to the same cache
/// before the cache is destroyed.
pub struct MemCache {
    cache: *mut c_void,
}

/// SLAB flags for the inode cache.
const SLAB_RECLAIM_ACCOUNT: u64 = 1u64 << slab_b::_SLAB_RECLAIM_ACCOUNT;

impl MemCache {
    /// Creates a new slab cache.
    ///
    /// - `name`: displayed in `/proc/slabinfo` (must be `'static`).
    /// - `size`: object size in bytes.
    /// - `align`: minimum alignment (0 for natural alignment).
    pub fn try_new(
        name: &'static core::ffi::CStr,
        size: usize,
        align: usize,
    ) -> Result<Self, crate::error::Error> {
        let cache = unsafe {
            bindings_h::rust_helper_kmem_cache_create(
                name.as_ptr().cast_mut(),
                size as u32,
                align as u32,
                SLAB_RECLAIM_ACCOUNT,
                core::ptr::null_mut(),
            )
        };
        if cache.is_null() {
            return Err(crate::error::Error::ENOMEM);
        }
        Ok(Self { cache })
    }

    /// Creates a new slab cache with a constructor callback.
    ///
    /// `ctor` is called once per slab object when the slab page is first
    /// allocated (not on every `alloc`). Use for one-time initialization
    /// of embedded kernel structures (e.g., `inode_init_once`).
    ///
    /// # Safety
    ///
    /// `ctor` must be a valid C function pointer that correctly initializes
    /// the object at the given address.
    pub unsafe fn try_new_with_ctor(
        name: &'static core::ffi::CStr,
        size: usize,
        align: usize,
        ctor: unsafe extern "C" fn(*mut c_void),
    ) -> Result<Self, crate::error::Error> {
        let cache = unsafe {
            bindings_h::rust_helper_kmem_cache_create(
                name.as_ptr().cast_mut(),
                size as u32,
                align as u32,
                SLAB_RECLAIM_ACCOUNT,
                ctor as *mut isize,
            )
        };
        if cache.is_null() {
            return Err(crate::error::Error::ENOMEM);
        }
        Ok(Self { cache })
    }

    /// Returns the raw cache pointer for use with kernel APIs.
    pub fn as_ptr(&self) -> *mut c_void {
        self.cache
    }

    /// Allocates one object from this cache.
    ///
    /// Returns null on failure.
    pub fn alloc(&self, flags: super::Flags) -> *mut c_void {
        unsafe { slab_b::kmem_cache_alloc_noprof(self.cache, flags.bits()) }
    }

    /// Frees an object back to this cache.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated from this cache.
    pub unsafe fn free(&self, ptr: *mut c_void) {
        unsafe { slab_b::kmem_cache_free(self.cache, ptr) };
    }
}

impl Drop for MemCache {
    fn drop(&mut self) {
        unsafe { slab_b::kmem_cache_destroy(self.cache) };
    }
}

// SAFETY: kmem_cache is internally synchronized by the kernel.
unsafe impl Send for MemCache {}
unsafe impl Sync for MemCache {}
