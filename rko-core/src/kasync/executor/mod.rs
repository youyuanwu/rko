//! Async executor traits and supporting types.
//!
//! Defines the [`Executor`] trait (spawn + stop), the [`ArcWake`] trait
//! for bridging `Arc<T>` with `core::task::Waker`, [`RevocableTask`]
//! for task cancellation, and [`AutoStopHandle`] for RAII executor
//! shutdown.

pub mod workqueue;

use crate::error::Error;
use crate::sync::{Arc, ArcBorrow};
use core::future::Future;
use core::task::{RawWaker, RawWakerVTable, Waker};

// ---------------------------------------------------------------------------
// ArcWake
// ---------------------------------------------------------------------------

/// Trait for types that can be woken through an `Arc<Self>`.
///
/// This bridges the kernel's `Arc<T>` reference counting with the
/// standard `core::task::Waker` mechanism.
pub trait ArcWake: Send + Sync {
    /// Wake the task associated with this `Arc`.
    ///
    /// Consumes one reference count.
    fn wake(self: Arc<Self>);

    /// Wake the task without consuming the `Arc`.
    ///
    /// The default implementation clones and delegates to [`wake`](ArcWake::wake).
    fn wake_by_ref(self: &Arc<Self>) {
        self.clone().wake();
    }
}

/// Create a [`Waker`] from a reference to an `Arc<T: ArcWake>`.
///
/// The returned `Waker` holds its own reference (the refcount is
/// incremented).
pub fn ref_waker<T: ArcWake + 'static>(arc: &Arc<T>) -> Waker {
    let arc_clone = arc.clone();
    let data = Arc::into_raw(arc_clone) as *const ();

    // SAFETY: The vtable functions below correctly manage the Arc refcount
    // and the data pointer originated from Arc::into_raw.
    unsafe { Waker::from_raw(RawWaker::new(data, raw_waker_vtable::<T>())) }
}

/// Build the vtable for `Arc<T: ArcWake>`.
fn raw_waker_vtable<T: ArcWake + 'static>() -> &'static RawWakerVTable {
    &RawWakerVTable::new(
        // clone: increment refcount
        |data| {
            // SAFETY: data came from Arc::into_raw and the Arc is still alive
            // (the Waker holds a reference).
            let arc = unsafe { Arc::<T>::from_raw(data as *const T) };
            let cloned = arc.clone();
            // Don't drop the original — the existing Waker still owns it.
            core::mem::forget(arc);
            RawWaker::new(Arc::into_raw(cloned) as *const (), raw_waker_vtable::<T>())
        },
        // wake: consume the reference
        |data| {
            // SAFETY: data came from Arc::into_raw.
            let arc = unsafe { Arc::<T>::from_raw(data as *const T) };
            ArcWake::wake(arc);
        },
        // wake_by_ref: wake without consuming
        |data| {
            // SAFETY: data came from Arc::into_raw and the Arc is still alive.
            let arc = unsafe { Arc::<T>::from_raw(data as *const T) };
            ArcWake::wake_by_ref(&arc);
            // Don't drop — the Waker still owns this reference.
            core::mem::forget(arc);
        },
        // drop: decrement refcount
        |data| {
            // SAFETY: data came from Arc::into_raw.
            let _arc = unsafe { Arc::<T>::from_raw(data as *const T) };
            // _arc drops here, decrementing the refcount.
        },
    )
}

// ---------------------------------------------------------------------------
// RevocableTask
// ---------------------------------------------------------------------------

/// A spawned task handle that supports revocation and flushing.
///
/// - [`revoke`](RevocableTask::revoke) prevents the task from being polled
///   again and removes it from the executor's task list.
/// - [`flush`](RevocableTask::flush) waits for any in-flight execution to
///   complete (analogous to `cancel_work_sync`).
pub trait RevocableTask: Send + Sync {
    /// Revoke the task — prevent further polling and remove from the
    /// executor's task list.
    fn revoke(&self);

    /// Flush — block until any in-flight execution of this task completes.
    fn flush(&self);
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Trait for async executors that can spawn `Future`-based tasks.
///
/// Executors manage the lifecycle of spawned tasks and support ordered
/// shutdown via [`stop`](Executor::stop).
pub trait Executor: Send + Sync {
    /// Spawn a future as a new task on this executor.
    ///
    /// Returns a handle that can be used to revoke/flush the task.
    fn spawn(
        self: ArcBorrow<'_, Self>,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Arc<dyn RevocableTask>, Error>;

    /// Stop the executor — revoke and flush all tasks.
    ///
    /// After this call, no new tasks can be spawned and all existing
    /// tasks have been fully cancelled.
    fn stop(&self);
}

// ---------------------------------------------------------------------------
// AutoStopHandle
// ---------------------------------------------------------------------------

/// RAII wrapper that calls [`Executor::stop`] on drop.
///
/// Ensures orderly shutdown of an executor when the owning module is
/// unloaded. Store this in your module struct.
pub struct AutoStopHandle<T: Executor> {
    executor: Arc<T>,
}

impl<T: Executor> AutoStopHandle<T> {
    /// Create a new `AutoStopHandle` wrapping the given executor.
    pub fn new(executor: Arc<T>) -> Self {
        Self { executor }
    }

    /// Get an `ArcBorrow` to the underlying executor (for spawning).
    pub fn executor(&self) -> ArcBorrow<'_, T> {
        self.executor.as_arc_borrow()
    }

    /// Explicitly stop the executor (also called on drop).
    pub fn stop(&self) {
        self.executor.stop();
    }
}

impl<T: Executor> Drop for AutoStopHandle<T> {
    fn drop(&mut self) {
        self.executor.stop();
    }
}
