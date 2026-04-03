// SPDX-License-Identifier: GPL-2.0

//! `Locked<T, L>` — RAII lock guard generic over lock type.
//!
//! Provides a `Lockable` trait that kernel types (e.g. `INode`) can
//! implement for specific lock semantics (e.g. `ReadSem`).

use core::marker::PhantomData;
use core::ops::Deref;

/// A lock backend that can be applied to a lockable object.
///
/// # Safety
///
/// Implementors must ensure `raw_lock` and `unlock` correctly
/// acquire and release the underlying kernel lock.
pub unsafe trait Lockable<L> {
    /// Acquire the lock.
    fn raw_lock(&self);

    /// Release the lock.
    ///
    /// # Safety
    ///
    /// The lock must currently be held by the caller.
    unsafe fn unlock(&self);
}

/// RAII lock guard. Holds a reference to a `Lockable` object and
/// automatically releases the lock on drop.
pub struct Locked<'a, T: Lockable<L> + ?Sized, L> {
    inner: &'a T,
    owns_lock: bool,
    _lock: PhantomData<L>,
}

impl<'a, T: Lockable<L> + ?Sized, L> Locked<'a, T, L> {
    /// Acquire the lock and return a guard.
    pub fn new(inner: &'a T) -> Self {
        inner.raw_lock();
        Self {
            inner,
            owns_lock: true,
            _lock: PhantomData,
        }
    }

    /// Create a proof that `inner` is already locked.
    ///
    /// The lock is NOT released on drop — the caller (e.g., the kernel
    /// VFS) is responsible for unlocking.
    ///
    /// # Safety
    ///
    /// The lock must currently be held and must remain held for the
    /// lifetime `'a`.
    pub unsafe fn borrowed(inner: &'a T) -> Self {
        Self {
            inner,
            owns_lock: false,
            _lock: PhantomData,
        }
    }
}

impl<T: Lockable<L> + ?Sized, L> Deref for Locked<'_, T, L> {
    type Target = T;
    fn deref(&self) -> &T {
        self.inner
    }
}

impl<T: Lockable<L> + ?Sized, L> Drop for Locked<'_, T, L> {
    fn drop(&mut self) {
        if self.owns_lock {
            // SAFETY: We acquired the lock in `new`.
            unsafe { self.inner.unlock() };
        }
    }
}
