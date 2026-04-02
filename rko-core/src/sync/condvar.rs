//! A condition variable backed by a kernel `wait_queue_head`.
//!
//! Ported from the in-tree Linux kernel Rust crate
//! (`linux/rust/kernel/sync/condvar.rs`).
// UPSTREAM_REF: linux/rust/kernel/sync/condvar.rs

use super::LockClassKey;
use super::lock::{Backend, Guard};
use crate::types::Opaque;
use core::marker::PhantomPinned;

/// TASK_NORMAL = TASK_INTERRUPTIBLE | TASK_UNINTERRUPTIBLE (not in bindings).
const TASK_NORMAL: u32 = 3;

/// A condition variable.
///
/// Wraps the kernel `wait_queue_head` and provides `wait` /
/// `notify_one` / `notify_all` semantics. The caller must hold a
/// [`Guard`] when calling [`wait`](CondVar::wait) — the guard is
/// released while sleeping and re-acquired before returning.
// UPSTREAM_REF: linux/rust/kernel/sync/condvar.rs CondVar
pub struct CondVar {
    wait_queue_head: Opaque<rko_sys::rko::wait::wait_queue_head>,
    _pin: PhantomPinned,
}

// SAFETY: The kernel wait_queue_head is designed for cross-thread use.
unsafe impl Send for CondVar {}
// SAFETY: All access is serialized by the internal spinlock in wait_queue_head.
unsafe impl Sync for CondVar {}

impl CondVar {
    /// Create a new condition variable initializer.
    ///
    /// Returns a [`PinInit`] that initializes a `CondVar` in place.
    ///
    /// [`PinInit`]: pinned_init::PinInit
    pub fn new(
        name: &'static core::ffi::CStr,
        key: &'static LockClassKey,
    ) -> impl pinned_init::PinInit<Self> {
        // SAFETY: We initialize every field. The wait_queue_head is
        // initialized via __init_waitqueue_head which expects a valid
        // pointer, name, and lock_class_key.
        unsafe {
            pinned_init::pin_init_from_closure::<_, core::convert::Infallible>(
                move |slot: *mut Self| {
                    core::ptr::addr_of_mut!((*slot)._pin).write(PhantomPinned);
                    // SAFETY: Initialize the wait_queue_head in place via
                    // the kernel's __init_waitqueue_head.
                    rko_sys::rko::wait::__init_waitqueue_head(
                        Opaque::raw_get(core::ptr::addr_of_mut!((*slot).wait_queue_head)),
                        name.as_ptr(),
                        key.as_ptr(),
                    );
                    Ok(())
                },
            )
        }
    }

    /// Block the current thread until notified.
    ///
    /// The lock behind `guard` is released while sleeping and
    /// re-acquired before this method returns. Spurious wakeups are
    /// possible — callers should re-check the predicate in a loop.
    pub fn wait<T: ?Sized, B: Backend>(&self, guard: &mut Guard<'_, T, B>) {
        let lock = guard.lock_ref();

        // Allocate a stack wait_queue_entry for sleeping.
        let mut wq_entry = core::mem::MaybeUninit::<rko_sys::rko::wait::wait_queue_entry>::zeroed();

        // SAFETY: prepare_to_wait_exclusive registers the entry on the
        // wait queue and sets the task state to TASK_INTERRUPTIBLE.
        // The wq_entry is stack-allocated and valid for this scope.
        unsafe {
            rko_sys::rko::wait::prepare_to_wait_exclusive(
                self.wait_queue_head.get(),
                wq_entry.as_mut_ptr(),
                1, // TASK_INTERRUPTIBLE
            );
        }

        // Release the lock before sleeping.
        // SAFETY: The guard guarantees we hold the lock.
        unsafe { B::unlock(lock.state.get(), &guard.state) };

        // SAFETY: Sleep until woken by notify_one/notify_all.
        unsafe { rko_sys::rko::helpers::rust_helper_schedule() };

        // Re-acquire the lock before returning.
        // SAFETY: The lock was initialized during Lock construction.
        guard.state = unsafe { B::lock(lock.state.get()) };

        // SAFETY: Deregister from the wait queue and restore task state.
        unsafe {
            rko_sys::rko::wait::finish_wait(self.wait_queue_head.get(), wq_entry.as_mut_ptr());
        }
    }

    /// Wake one waiter (exclusive).
    pub fn notify_one(&self) {
        // SAFETY: The wait_queue_head was initialized during CondVar construction.
        // TASK_NORMAL wakes both interruptible and uninterruptible tasks.
        // nr_exclusive=1 wakes at most one exclusive waiter.
        unsafe {
            rko_sys::rko::wait::__wake_up(
                self.wait_queue_head.get(),
                TASK_NORMAL,
                1,
                core::ptr::null_mut(),
            );
        }
    }

    /// Wake all waiters.
    pub fn notify_all(&self) {
        // SAFETY: The wait_queue_head was initialized during CondVar construction.
        // nr_exclusive=0 wakes all waiters.
        unsafe {
            rko_sys::rko::wait::__wake_up(
                self.wait_queue_head.get(),
                TASK_NORMAL,
                0,
                core::ptr::null_mut(),
            );
        }
    }
}
