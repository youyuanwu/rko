//! `Kmalloc` allocator — wraps kernel `krealloc_node_align_noprof` / `kfree`.

use core::alloc::Layout;
use core::ptr::NonNull;

use super::{AllocError, Allocator, Flags};
use rko_sys::rko::slab;

/// NUMA "no node" sentinel — allocate from any node.
const NUMA_NO_NODE: i32 = -1;

/// Kmalloc-backed allocator.
pub struct Kmalloc;

unsafe impl Allocator for Kmalloc {
    unsafe fn realloc(
        ptr: Option<NonNull<u8>>,
        layout: Layout,
        _old_layout: Layout,
        flags: Flags,
    ) -> Result<NonNull<[u8]>, AllocError> {
        let size = layout.size();
        let align = layout.align();
        let raw_ptr = ptr
            .map(|p| p.as_ptr() as *const core::ffi::c_void)
            .unwrap_or(core::ptr::null());

        let result = unsafe {
            slab::krealloc_node_align_noprof(
                raw_ptr,
                size as rko_sys::rko::types::size_t,
                align as u64,
                flags.bits(),
                NUMA_NO_NODE,
            )
        };

        if size == 0 {
            // krealloc with size 0 frees and returns ZERO_SIZE_PTR.
            // Return a dangling but aligned pointer with len 0.
            return Ok(NonNull::slice_from_raw_parts(NonNull::dangling(), 0));
        }

        NonNull::new(result as *mut u8)
            .map(|p| NonNull::slice_from_raw_parts(p, size))
            .ok_or(AllocError)
    }

    unsafe fn free(ptr: NonNull<u8>, _layout: Layout) {
        unsafe { slab::kfree(ptr.as_ptr() as *const core::ffi::c_void) };
    }
}
