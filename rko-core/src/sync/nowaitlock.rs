//! A non-blocking try-only lock with contention reporting.
//!
//! [`NoWaitLock`] provides only `try_lock` — no blocking. The guard's
//! [`unlock`](NoWaitLockGuard::unlock) method returns `true` if another
//! thread attempted (and failed) to acquire the lock during the hold
//! period, allowing the caller to take compensating action (e.g.
//! re-wake).

use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicU8, Ordering};

const UNLOCKED: u8 = 0;
const LOCKED: u8 = 1;
const LOCKED_CONTENDED: u8 = 2;

/// A non-blocking mutual-exclusion lock.
///
/// Only provides [`try_lock`](NoWaitLock::try_lock). The guard reports
/// contention on [`unlock`](NoWaitLockGuard::unlock), which is used by
/// `SocketFuture` to detect missed wakeups.
pub struct NoWaitLock<T: ?Sized> {
    state: AtomicU8,
    data: UnsafeCell<T>,
}

// SAFETY: The lock serializes all access to `data`.
unsafe impl<T: ?Sized + Send> Send for NoWaitLock<T> {}
// SAFETY: Interior mutability is serialized by the atomic state.
unsafe impl<T: ?Sized + Send> Sync for NoWaitLock<T> {}

impl<T> NoWaitLock<T> {
    /// Create a new unlocked `NoWaitLock`.
    pub const fn new(val: T) -> Self {
        Self {
            state: AtomicU8::new(UNLOCKED),
            data: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> NoWaitLock<T> {
    /// Try to acquire the lock without blocking.
    ///
    /// Returns `None` if the lock is already held. If the lock is held,
    /// the contention flag is set so the current holder's
    /// [`unlock`](NoWaitLockGuard::unlock) will return `true`.
    #[inline]
    pub fn try_lock(&self) -> Option<NoWaitLockGuard<'_, T>> {
        // Fast path: UNLOCKED → LOCKED.
        if self
            .state
            .compare_exchange(UNLOCKED, LOCKED, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            return Some(NoWaitLockGuard { lock: self });
        }

        // Slow path: set contention flag so the holder knows.
        let _ = self.state.compare_exchange(
            LOCKED,
            LOCKED_CONTENDED,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );

        None
    }
}

/// RAII guard for [`NoWaitLock`].
///
/// Provides `Deref`/`DerefMut` access to the protected data.
/// Call [`unlock`](NoWaitLockGuard::unlock) explicitly to learn whether
/// contention occurred, or simply drop to release.
#[must_use = "if unused, the lock will be immediately released"]
pub struct NoWaitLockGuard<'a, T: ?Sized> {
    lock: &'a NoWaitLock<T>,
}

impl<T: ?Sized> NoWaitLockGuard<'_, T> {
    /// Release the lock and return whether contention was detected.
    ///
    /// Returns `true` if another thread called `try_lock` (and failed)
    /// while this guard was held.
    #[inline]
    pub fn unlock(self) -> bool {
        let prev = self.lock.state.swap(UNLOCKED, Ordering::Release);
        // Prevent Drop from running (we already released).
        core::mem::forget(self);
        prev == LOCKED_CONTENDED
    }
}

impl<T: ?Sized> Deref for NoWaitLockGuard<'_, T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        // SAFETY: We hold the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized> DerefMut for NoWaitLockGuard<'_, T> {
    #[inline]
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: We hold the lock exclusively.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized> Drop for NoWaitLockGuard<'_, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.state.swap(UNLOCKED, Ordering::Release);
    }
}
