//! RCU support.
//!
//! Provides a guard that holds the RCU read-side lock. Currently a stub
//! until C helpers for `rcu_read_lock`/`rcu_read_unlock` are added.
// UPSTREAM_REF: linux/rust/kernel/sync/rcu.rs

/// Evidence that the RCU read side lock is held on the current thread/CPU.
///
/// Explicitly `!Send` because this property is per-thread/CPU.
///
/// # Invariants
///
/// The RCU read side lock is held while instances of this guard exist.
pub struct Guard {
    // PhantomData<*mut ()> makes this !Send.
    _not_send: core::marker::PhantomData<*mut ()>,
}

impl Guard {
    /// Acquire the RCU read side lock and return a guard.
    #[inline]
    pub fn new() -> Self {
        // SAFETY: Acquire the RCU read side lock.
        unsafe { rko_sys::rko::helpers::rust_helper_rcu_read_lock() };
        Self {
            _not_send: core::marker::PhantomData,
        }
    }

    /// Explicitly release the RCU read side lock.
    #[inline]
    pub fn unlock(self) {}
}

impl Default for Guard {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Guard {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: Release the RCU read side lock. The Guard invariant
        // guarantees we hold it.
        unsafe { rko_sys::rko::helpers::rust_helper_rcu_read_unlock() };
    }
}

/// Acquire the RCU read side lock.
#[inline]
pub fn read_lock() -> Guard {
    Guard::new()
}
