//! Asynchronous revocable wrapper.
//!
//! [`AsyncRevocable<T>`] wraps a value and allows concurrent access via
//! guards while supporting revocation. Once revoked, no new guards can
//! be created and the inner value is dropped when the last outstanding
//! guard is released.
//!
//! Unlike the kernel's synchronous `Revocable<T>` (which uses RCU),
//! this variant uses atomic reference counting and is suitable for use
//! in async executor contexts.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering, fence};

const REVOKED_BIT: u32 = 1 << 31;

/// An asynchronously revocable wrapper around `T`.
///
/// # Invariants
///
/// - The low 31 bits of `usage_count` track active [`AsyncRevocableGuard`]s.
/// - Bit 31 (`REVOKED_BIT`) is set once [`revoke`](AsyncRevocable::revoke)
///   is called.
/// - `data` is initialized from construction until the moment it is dropped
///   (either in `revoke` when no guards exist, or in the last guard's `Drop`).
pub struct AsyncRevocable<T> {
    usage_count: AtomicU32,
    data: MaybeUninit<UnsafeCell<T>>,
}

// SAFETY: The atomic protocol serializes all access.
unsafe impl<T: Send> Send for AsyncRevocable<T> {}
// SAFETY: Guards provide shared/exclusive access via UnsafeCell.
unsafe impl<T: Send> Sync for AsyncRevocable<T> {}

impl<T> AsyncRevocable<T> {
    /// Create a new `AsyncRevocable` wrapping `data`.
    pub fn new(data: T) -> Self {
        Self {
            usage_count: AtomicU32::new(0),
            data: MaybeUninit::new(UnsafeCell::new(data)),
        }
    }

    /// Try to obtain a guard granting access to the inner value.
    ///
    /// Returns `None` if the value has been revoked.
    pub fn try_access(&self) -> Option<AsyncRevocableGuard<'_, T>> {
        loop {
            let val = self.usage_count.load(Ordering::Acquire);
            if val & REVOKED_BIT != 0 {
                return None;
            }

            // Debug check: guard count should never overflow 31 bits.
            debug_assert!(
                val < (REVOKED_BIT - 1),
                "AsyncRevocable guard count overflow"
            );

            match self.usage_count.compare_exchange_weak(
                val,
                val + 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    return Some(AsyncRevocableGuard {
                        inner: self,
                        _not_send: PhantomData,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    /// Revoke the wrapper, preventing new access.
    ///
    /// Returns `true` if no guards were active at the time of revocation
    /// (meaning the data was dropped immediately). Returns `false` if
    /// guards are still active or the value was already revoked.
    ///
    /// If guards are active, the last guard's [`Drop`] will drop the
    /// inner value.
    pub fn revoke(&self) -> bool {
        let prev = self.usage_count.fetch_or(REVOKED_BIT, Ordering::Release);
        if prev & REVOKED_BIT != 0 {
            // Already revoked.
            return false;
        }

        if prev == 0 {
            // No active guards — we must drop the data now.
            fence(Ordering::Acquire);
            // SAFETY: data is initialized (no guards exist, we just set REVOKED_BIT),
            // and no one else can access it after this point.
            unsafe { self.data.assume_init_ref().get().drop_in_place() };
            return true;
        }

        // Guards are still active — the last guard's Drop will handle cleanup.
        false
    }

    /// Check whether this wrapper has been revoked.
    pub fn is_revoked(&self) -> bool {
        self.usage_count.load(Ordering::Relaxed) & REVOKED_BIT != 0
    }
}

impl<T> Drop for AsyncRevocable<T> {
    fn drop(&mut self) {
        // If not yet revoked, we own the data and must drop it.
        let val = *self.usage_count.get_mut();
        if val & REVOKED_BIT == 0 {
            // SAFETY: data is initialized and no guards exist (we have &mut self).
            unsafe { self.data.assume_init_drop() };
        }
        // If revoked, data was already dropped by revoke() or the last guard.
    }
}

/// A guard providing access to the inner value of an [`AsyncRevocable`].
///
/// Explicitly `!Send` to prevent holding a guard across `await` points.
pub struct AsyncRevocableGuard<'a, T> {
    inner: &'a AsyncRevocable<T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> core::ops::Deref for AsyncRevocableGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // SAFETY: We hold an active guard, so data is initialized and not
        // yet dropped.
        unsafe { &*self.inner.data.assume_init_ref().get() }
    }
}

impl<T> AsyncRevocableGuard<'_, T> {
    /// Get a raw mutable pointer to the inner value.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other references (shared or mutable)
    /// to the inner value exist for the duration of the borrow, and
    /// that at most one `*mut T` is used at a time. This is safe when
    /// only one guard exists (e.g. the async executor's single-poll
    /// model).
    pub unsafe fn as_mut_ptr(&self) -> *mut T {
        // SAFETY: The caller guarantees exclusive access and the guard
        // ensures data is initialized.
        unsafe { self.inner.data.assume_init_ref().get() }
    }
}

impl<T> Drop for AsyncRevocableGuard<'_, T> {
    fn drop(&mut self) {
        let prev = self.inner.usage_count.fetch_sub(1, Ordering::Release);
        if prev == REVOKED_BIT | 1 {
            // Last guard + revoked: we must drop the data.
            fence(Ordering::Acquire);
            // SAFETY: We were the last guard and the value is revoked,
            // so no one else can access the data.
            unsafe { self.inner.data.assume_init_ref().get().drop_in_place() };
        }
    }
}
