// SPDX-License-Identifier: GPL-2.0

//! Safe wrapper around the kernel's `struct completion`.
//!
//! A one-shot signalling mechanism: one thread calls `wait()` (blocks),
//! another calls `complete()` (wakes the waiter). Used to bridge async
//! futures back to synchronous callers.

use rko_sys::rko::helpers as h;
use rko_sys::rko::wait as wait_b;

/// A kernel completion.
///
/// Must be initialized in place via `init()` — cannot be moved after
/// initialization because the internal `list_head` contains self-pointers.
pub struct Completion {
    inner: wait_b::completion,
}

// SAFETY: completion is internally synchronized by the kernel.
unsafe impl Send for Completion {}
unsafe impl Sync for Completion {}

impl Completion {
    /// Initialize a `Completion` in place through a raw pointer.
    ///
    /// Use with `MaybeUninit` on the stack:
    /// ```ignore
    /// let mut comp = MaybeUninit::<Completion>::uninit();
    /// unsafe { Completion::init(comp.as_mut_ptr()); }
    /// let comp = unsafe { comp.assume_init_mut() };
    /// comp.complete();
    /// ```
    ///
    /// # Safety
    ///
    /// `ptr` must point to valid, writable, properly aligned memory for
    /// a `Completion`. The memory must not be moved after this call.
    pub unsafe fn init(ptr: *mut Self) {
        // SAFETY: ptr is valid, inner is at offset 0.
        unsafe { h::rust_helper_init_completion(&mut (*ptr).inner) };
    }

    /// Block the current thread until `complete()` is called, with timeout.
    ///
    /// `timeout_jiffies` is the maximum time to wait in jiffies.
    /// Returns the remaining jiffies (0 if timed out).
    pub fn wait_timeout(&mut self, timeout_jiffies: u64) -> u64 {
        unsafe { h::rust_helper_wait_for_completion_timeout(&mut self.inner, timeout_jiffies) }
    }

    /// Block the current thread until `complete()` is called (no timeout).
    pub fn wait(&mut self) {
        unsafe { h::rust_helper_wait_for_completion(&mut self.inner) }
    }

    /// Signal the completion, waking one waiter.
    pub fn complete(&mut self) {
        unsafe { h::rust_helper_complete(&mut self.inner) }
    }
}
