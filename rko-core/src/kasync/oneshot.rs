// SPDX-License-Identifier: GPL-2.0

//! Async one-shot channel — `Future`-based variant of [`crate::sync::oneshot`].
//!
//! The [`Receiver`] implements `Future` and integrates with the async
//! executor via `Waker`. No threads are blocked while waiting.

use core::cell::UnsafeCell;
use core::future::Future;
use core::mem::MaybeUninit;
use core::pin::Pin;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, Ordering};
use core::task::{Context, Poll, Waker};

use crate::alloc::{AllocError, Flags, KBox};

/// State machine for the channel.
const STATE_EMPTY: u8 = 0; // No value, no completion
const STATE_SENT: u8 = 1; // Value written
const STATE_CLOSED: u8 = 2; // Sender dropped without sending

/// Shared heap-allocated state.
struct Inner<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    waker: UnsafeCell<Option<Waker>>,
    state: AtomicU8,
    refcount: AtomicU8,
}

// SAFETY: state transitions are atomic. value is written before state
// transitions to SENT. waker is only written by receiver under EMPTY
// state (before sender acts) and read by sender during send/drop.
// The atomic state ensures proper ordering.
unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

/// The sending half of an async oneshot channel.
pub struct Sender<T> {
    inner: NonNull<Inner<T>>,
}

unsafe impl<T: Send> Send for Sender<T> {}

/// The receiving half — implements [`Future`].
pub struct Receiver<T> {
    inner: NonNull<Inner<T>>,
}

unsafe impl<T: Send> Send for Receiver<T> {}

/// Create an async oneshot channel pair.
pub fn channel<T: Send>(flags: Flags) -> Result<(Sender<T>, Receiver<T>), AllocError> {
    let shared = KBox::new(
        Inner {
            value: UnsafeCell::new(MaybeUninit::uninit()),
            waker: UnsafeCell::new(None),
            state: AtomicU8::new(STATE_EMPTY),
            refcount: AtomicU8::new(2),
        },
        flags,
    )?;
    let ptr = KBox::into_raw(shared);
    Ok((Sender { inner: ptr }, Receiver { inner: ptr }))
}

impl<T: Send> Sender<T> {
    /// Send a value, waking the receiver's future.
    pub fn send(self, value: T) {
        let inner = unsafe { self.inner.as_ref() };
        // Write value before state transition.
        unsafe { (*inner.value.get()).write(value) };
        inner.state.store(STATE_SENT, Ordering::Release);
        // Wake the receiver if it's polling.
        wake(inner);
        let ptr = self.inner;
        core::mem::forget(self);
        dec_ref(ptr);
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        let inner = unsafe { self.inner.as_ref() };
        inner.state.store(STATE_CLOSED, Ordering::Release);
        wake(inner);
        dec_ref(self.inner);
    }
}

impl<T: Send> Future for Receiver<T> {
    type Output = Option<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = unsafe { self.inner.as_ref() };
        match inner.state.load(Ordering::Acquire) {
            STATE_SENT => {
                let val = unsafe { (*inner.value.get()).assume_init_read() };
                Poll::Ready(Some(val))
            }
            STATE_CLOSED => Poll::Ready(None),
            _ => {
                // Store waker for the sender to call.
                // SAFETY: Only the receiver writes the waker, and only
                // while state is EMPTY. The sender reads it only after
                // transitioning state away from EMPTY.
                unsafe { *inner.waker.get() = Some(cx.waker().clone()) };
                // Re-check state — sender may have raced between our
                // load and the waker store.
                match inner.state.load(Ordering::Acquire) {
                    STATE_SENT => {
                        let val = unsafe { (*inner.value.get()).assume_init_read() };
                        Poll::Ready(Some(val))
                    }
                    STATE_CLOSED => Poll::Ready(None),
                    _ => Poll::Pending,
                }
            }
        }
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        dec_ref(self.inner);
    }
}

fn wake<T>(inner: &Inner<T>) {
    // SAFETY: Only called after state transitions away from EMPTY,
    // so the receiver won't concurrently write the waker.
    if let Some(w) = unsafe { (*inner.waker.get()).take() } {
        w.wake();
    }
}

fn dec_ref<T>(ptr: NonNull<Inner<T>>) {
    let inner = unsafe { ptr.as_ref() };
    if inner.refcount.fetch_sub(1, Ordering::AcqRel) == 1 {
        unsafe { drop(KBox::from_raw(ptr)) };
    }
}
