//! Atomic reference counting.
//!
//! Provides a `Refcount` type with acquire/release semantics suitable for
//! reference-counted smart pointers. Once C helper bindings for
//! `refcount_set`/`refcount_inc`/`refcount_dec_and_test` are added, the
//! implementation should delegate to those for kernel saturation semantics.
// UPSTREAM_REF: linux/rust/kernel/sync/refcount.rs

use core::sync::atomic::{AtomicI32, Ordering, fence};

/// Atomic reference counter.
///
/// A simplified port of the kernel's `refcount_t`. Currently backed by
/// `AtomicI32` directly; when C helpers for `refcount_inc` /
/// `refcount_dec_and_test` are wired up, this should delegate to those
/// for saturation semantics.
pub struct Refcount {
    refs: AtomicI32,
}

impl Refcount {
    /// Construct a new `Refcount` with the given initial value.
    pub fn new(value: i32) -> Self {
        Self {
            refs: AtomicI32::new(value),
        }
    }

    /// Set the reference count value.
    pub fn set(&self, value: i32) {
        self.refs.store(value, Ordering::Release);
    }

    /// Read the current reference count value.
    pub fn get(&self) -> i32 {
        self.refs.load(Ordering::Relaxed)
    }

    /// Increment the reference count.
    ///
    /// Caller must already hold a reference (the count must be > 0).
    pub fn inc(&self) {
        // Relaxed: caller already holds a ref, so the object is alive.
        let old = self.refs.fetch_add(1, Ordering::Relaxed);
        debug_assert!(old > 0, "Refcount::inc on zero refcount");
    }

    /// Decrement the reference count, returning `true` if it reached zero.
    ///
    /// Uses Release on the decrement and Acquire fence on zero for safe
    /// deallocation ordering (matching `std::sync::Arc`'s pattern).
    #[must_use]
    pub fn dec_and_test(&self) -> bool {
        if self.refs.fetch_sub(1, Ordering::Release) == 1 {
            fence(Ordering::Acquire);
            true
        } else {
            false
        }
    }
}

// SAFETY: Refcount uses atomic operations only.
unsafe impl Send for Refcount {}
// SAFETY: Refcount uses atomic operations only.
unsafe impl Sync for Refcount {}
