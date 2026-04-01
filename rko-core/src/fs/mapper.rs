// SPDX-License-Identifier: GPL-2.0

//! Block device mapper — reads folios from a block device's page cache.
//!
//! `Mapper` wraps the block device inode's address space and provides
//! `mapped_folio()` and `for_each_page()` for reading on-disk data.

use core::cmp;

use crate::error::Error;
use rko_sys::rko::{fs as bindings, helpers as bindings_h, pagemap as pagemap_b};

use super::{Offset, PAGE_SIZE};

type Result<T = ()> = core::result::Result<T, Error>;

/// A mapped folio from the block device's page cache.
///
/// Holds a folio reference and provides access to its data. The folio
/// is released (put) on drop.
pub struct MappedFolio {
    folio: *mut bindings::folio,
    data: *const u8,
    offset_in_folio: usize,
    len: usize,
}

impl MappedFolio {
    /// Access the mapped data as a byte slice.
    pub fn data(&self) -> &[u8] {
        // SAFETY: The folio is valid and mapped for `len` bytes.
        unsafe { core::slice::from_raw_parts(self.data.add(self.offset_in_folio), self.len) }
    }
}

impl Drop for MappedFolio {
    fn drop(&mut self) {
        // Unmap and release the folio.
        unsafe {
            bindings_h::rust_helper_kunmap_local(self.data as *const _);
            bindings_h::rust_helper_folio_put(self.folio);
        }
    }
}

/// Reads folios from a block device's address space.
///
/// Created from a superblock's block device inode. Provides page-cache
/// based reading of on-disk data.
pub struct Mapper {
    /// The address_space of the block device's inode.
    mapping: *mut bindings::address_space,
}

// SAFETY: Mapper holds a pointer to the bdev inode's address_space which
// is valid for the lifetime of the mounted filesystem.
unsafe impl Send for Mapper {}
unsafe impl Sync for Mapper {}

impl Mapper {
    /// Create a Mapper from a superblock's block device.
    ///
    /// # Safety
    ///
    /// The superblock must be block-device-backed (`SUPER_TYPE = BlockDev`)
    /// and `s_bdev` must be valid.
    pub unsafe fn from_sb(sb: *mut bindings::super_block) -> Self {
        // Get the block device's address_space directly via bd_mapping.
        let mapping = unsafe { bindings_h::rust_helper_sb_bdev_mapping(sb) };
        Self { mapping }
    }

    /// Read a folio from the block device at the given byte offset.
    ///
    /// Returns a `MappedFolio` with `data()` pointing to the page
    /// containing `offset`. The returned slice starts at `offset`
    /// within the page and extends to the end of the page.
    pub fn mapped_folio(&self, offset: Offset) -> Result<MappedFolio> {
        if offset < 0 {
            return Err(Error::new(-22)); // EINVAL
        }
        let offset_u = offset as u64;
        let page_index = offset_u / PAGE_SIZE as u64;
        let offset_in_page = (offset_u % PAGE_SIZE as u64) as usize;

        // Read the folio from the page cache (triggers I/O if not cached).
        // Use read_cache_folio with NULL filler (uses default readpage).
        let folio = unsafe {
            pagemap_b::read_cache_folio(
                self.mapping,
                page_index,
                core::ptr::null_mut(), // NULL filler = use default aops
                core::ptr::null_mut(), // no file context
            )
        };

        // Check for IS_ERR first (ERR_PTR is non-null).
        if unsafe { bindings_h::rust_helper_IS_ERR(folio.cast()) } {
            return Err(Error::new(
                unsafe { bindings_h::rust_helper_PTR_ERR(folio.cast()) } as i32,
            ));
        }

        if folio.is_null() {
            return Err(Error::new(-5)); // EIO
        }

        let data = unsafe { bindings_h::rust_helper_kmap_local_folio(folio, 0) };

        let len = PAGE_SIZE - offset_in_page;

        Ok(MappedFolio {
            folio,
            data: data.cast(),
            offset_in_folio: offset_in_page,
            len,
        })
    }

    /// Iterate over byte range `[offset, offset+len)`, one page at a time.
    ///
    /// Calls `cb` with each page's data slice. If `cb` returns
    /// `Ok(Some(val))`, iteration stops and returns `Ok(Some(val))`.
    /// Returns `Ok(None)` if the entire range was iterated.
    pub fn for_each_page<U>(
        &self,
        offset: Offset,
        len: Offset,
        mut cb: impl FnMut(&[u8]) -> Result<Option<U>>,
    ) -> Result<Option<U>> {
        if len <= 0 {
            return Ok(None);
        }

        let mut remain = len;
        let mut next = offset;

        while remain > 0 {
            let mapped = self.mapped_folio(next)?;
            let avail = cmp::min(mapped.data().len(), remain as usize);
            let ret = cb(&mapped.data()[..avail])?;
            if ret.is_some() {
                return Ok(ret);
            }
            next += avail as Offset;
            remain -= avail as Offset;
        }

        Ok(None)
    }
}
