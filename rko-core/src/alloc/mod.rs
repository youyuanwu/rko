//! Kernel memory allocation.
//!
//! Provides `Flags` (GFP flags), an `Allocator` trait, `Kmalloc`, and
//! `Vec<T, A>` / `KVec<T>` backed by kernel allocators.

mod allocator;
mod kbox;
mod kvec;
mod layout;

pub use allocator::Kmalloc;
pub use kbox::KBox;
pub use kvec::{KVec, Vec};

use core::ptr::NonNull;

/// Allocation error (equivalent to kernel `-ENOMEM`).
#[derive(Copy, Clone, Debug)]
pub struct AllocError;

impl core::fmt::Display for AllocError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("memory allocation failed")
    }
}

// GFP flags derived from generated bit-position constants in rko-sys.
use rko_sys::rko::gfp::*;

bitflags::bitflags! {
    /// Kernel GFP allocation flags (`gfp_t`).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Flags: u32 {
        const __GFP_DMA            = 1 << ___GFP_DMA_BIT;
        const __GFP_HIGHMEM        = 1 << ___GFP_HIGHMEM_BIT;
        const __GFP_DMA32          = 1 << ___GFP_DMA32_BIT;
        const __GFP_MOVABLE        = 1 << ___GFP_MOVABLE_BIT;
        const __GFP_RECLAIMABLE    = 1 << ___GFP_RECLAIMABLE_BIT;
        const __GFP_HIGH           = 1 << ___GFP_HIGH_BIT;
        const __GFP_IO             = 1 << ___GFP_IO_BIT;
        const __GFP_FS             = 1 << ___GFP_FS_BIT;
        const __GFP_ZERO           = 1 << ___GFP_ZERO_BIT;
        const __GFP_DIRECT_RECLAIM = 1 << ___GFP_DIRECT_RECLAIM_BIT;
        const __GFP_KSWAPD_RECLAIM = 1 << ___GFP_KSWAPD_RECLAIM_BIT;
        const __GFP_WRITE          = 1 << ___GFP_WRITE_BIT;
        const __GFP_NOWARN         = 1 << ___GFP_NOWARN_BIT;
        const __GFP_RETRY_MAYFAIL  = 1 << ___GFP_RETRY_MAYFAIL_BIT;
        const __GFP_NOFAIL         = 1 << ___GFP_NOFAIL_BIT;
        const __GFP_NORETRY        = 1 << ___GFP_NORETRY_BIT;
        const __GFP_COMP           = 1 << ___GFP_COMP_BIT;
        const __GFP_HARDWALL       = 1 << ___GFP_HARDWALL_BIT;
        const __GFP_THISNODE       = 1 << ___GFP_THISNODE_BIT;
        const __GFP_ACCOUNT        = 1 << ___GFP_ACCOUNT_BIT;
        const __GFP_ZEROTAGS       = 1 << ___GFP_ZEROTAGS_BIT;

        // Compound flags matching linux/gfp_types.h
        const __GFP_RECLAIM = Self::__GFP_DIRECT_RECLAIM.bits() | Self::__GFP_KSWAPD_RECLAIM.bits();
        const GFP_KERNEL    = Self::__GFP_RECLAIM.bits() | Self::__GFP_IO.bits() | Self::__GFP_FS.bits();
        const GFP_NOFS      = Self::__GFP_RECLAIM.bits() | Self::__GFP_IO.bits();
        const GFP_ATOMIC    = Self::__GFP_HIGH.bits() | Self::__GFP_KSWAPD_RECLAIM.bits();
        const GFP_NOWAIT    = Self::__GFP_KSWAPD_RECLAIM.bits();
    }
}

/// Kernel memory allocator trait.
///
/// # Safety
///
/// Implementors must return properly aligned, dereferenceable memory or
/// `AllocError`. `free` must accept any pointer previously returned by
/// `realloc` on the same allocator.
pub unsafe trait Allocator {
    /// Reallocate memory. If `ptr` is `None`, behaves as `alloc`.
    /// If `layout.size()` is 0, behaves as `free` and returns a
    /// zero-size allocation.
    ///
    /// # Safety
    ///
    /// `ptr` must be `None` or a pointer previously returned by this
    /// allocator with a layout compatible with `old_layout`.
    unsafe fn realloc(
        ptr: Option<NonNull<u8>>,
        layout: core::alloc::Layout,
        old_layout: core::alloc::Layout,
        flags: Flags,
    ) -> Result<NonNull<[u8]>, AllocError>;

    /// Free previously allocated memory.
    ///
    /// # Safety
    ///
    /// `ptr` must have been returned by this allocator with a layout
    /// compatible with `layout`.
    unsafe fn free(ptr: NonNull<u8>, layout: core::alloc::Layout);
}
