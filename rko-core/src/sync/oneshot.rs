// SPDX-License-Identifier: GPL-2.0

//! A one-shot channel for sending a single value between threads.
//!
//! Uses a kernel `struct completion` for synchronization. The sender
//! writes the value and signals completion; the receiver blocks until
//! the value is available or a timeout expires.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::ptr::NonNull;
use core::sync::atomic::{AtomicU8, Ordering};

use crate::alloc::{AllocError, Flags, KBox};
use crate::sync::Completion;

/// Shared heap-allocated state with atomic refcount (starts at 2).
struct Inner<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    sent: UnsafeCell<bool>,
    completion: UnsafeCell<MaybeUninit<Completion>>,
    refcount: AtomicU8,
}

// SAFETY: `value` and `sent` are synchronized by the completion —
// sender writes before complete(), receiver reads after wait().
unsafe impl<T: Send> Send for Inner<T> {}
unsafe impl<T: Send> Sync for Inner<T> {}

impl<T> Inner<T> {
    fn completion_ptr(&self) -> *mut Completion {
        // SAFETY: Initialized in channel().
        unsafe { (*self.completion.get()).assume_init_mut() as *mut Completion }
    }
}

/// The sending half of a oneshot channel.
pub struct Sender<T> {
    inner: NonNull<Inner<T>>,
}

unsafe impl<T: Send> Send for Sender<T> {}

/// The receiving half of a oneshot channel.
pub struct Receiver<T> {
    inner: NonNull<Inner<T>>,
}

unsafe impl<T: Send> Send for Receiver<T> {}

/// Create a oneshot channel pair.
///
/// The sender can send exactly one value; the receiver blocks until
/// it arrives or the sender is dropped.
pub fn channel<T: Send>(flags: Flags) -> Result<(Sender<T>, Receiver<T>), AllocError> {
    let shared = KBox::new(
        Inner {
            value: UnsafeCell::new(MaybeUninit::uninit()),
            sent: UnsafeCell::new(false),
            completion: UnsafeCell::new(MaybeUninit::uninit()),
            refcount: AtomicU8::new(2),
        },
        flags,
    )?;
    let ptr = KBox::into_raw(shared);

    // SAFETY: ptr is valid, completion field is properly aligned.
    unsafe {
        let comp_ptr = (*ptr.as_ptr()).completion.get();
        Completion::init(comp_ptr.cast());
    }

    Ok((Sender { inner: ptr }, Receiver { inner: ptr }))
}

impl<T: Send> Sender<T> {
    /// Send a value, waking the receiver. Consumes the sender.
    pub fn send(self, value: T) {
        let inner = unsafe { self.inner.as_ref() };
        unsafe {
            (*inner.value.get()).write(value);
            *inner.sent.get() = true;
            (*inner.completion_ptr()).complete();
        }
        let ptr = self.inner;
        core::mem::forget(self);
        dec_ref(ptr);
    }
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        // Dropped without sending — signal so receiver doesn't block forever.
        let inner = unsafe { self.inner.as_ref() };
        unsafe { (*inner.completion_ptr()).complete() };
        dec_ref(self.inner);
    }
}

impl<T: Send> Receiver<T> {
    /// Block until a value is sent or the sender is dropped.
    ///
    /// Returns `Some(value)` if sent, `None` if sender was dropped.
    pub fn recv(self) -> Option<T> {
        let inner = unsafe { self.inner.as_ref() };
        unsafe { (*inner.completion_ptr()).wait() };
        let result = read_value(inner);
        let ptr = self.inner;
        core::mem::forget(self);
        dec_ref(ptr);
        result
    }

    /// Block with a timeout (in jiffies).
    ///
    /// Returns `Some(value)` if sent before timeout, `None` otherwise.
    pub fn recv_timeout(self, timeout_jiffies: u64) -> Option<T> {
        let inner = unsafe { self.inner.as_ref() };
        let remaining = unsafe { (*inner.completion_ptr()).wait_timeout(timeout_jiffies) };
        let result = if remaining > 0 {
            read_value(inner)
        } else {
            None
        };
        let ptr = self.inner;
        core::mem::forget(self);
        dec_ref(ptr);
        result
    }
}

impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        dec_ref(self.inner);
    }
}

/// Read the value after completion has been signalled.
fn read_value<T>(inner: &Inner<T>) -> Option<T> {
    // SAFETY: After wait() returns, the sender has either written
    // value+sent or dropped (sent stays false). No concurrent access.
    if unsafe { *inner.sent.get() } {
        Some(unsafe { (*inner.value.get()).assume_init_read() })
    } else {
        None
    }
}

/// Decrement refcount; deallocate when it reaches 0.
fn dec_ref<T>(ptr: NonNull<Inner<T>>) {
    let inner = unsafe { ptr.as_ref() };
    if inner.refcount.fetch_sub(1, Ordering::AcqRel) == 1 {
        // SAFETY: Both halves done. ptr came from KBox::into_raw.
        unsafe { drop(KBox::from_raw(ptr)) };
    }
}
