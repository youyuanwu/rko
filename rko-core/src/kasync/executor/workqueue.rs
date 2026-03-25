//! Workqueue-backed async executor.
//!
//! Provides [`WorkqueueExecutor`] — an [`Executor`](super::Executor)
//! implementation that runs `Future`-based tasks on a kernel workqueue.
//!
//! Each spawned future is wrapped in a [`Task`] that implements both
//! [`WorkItem`] (for workqueue integration) and [`ArcWake`] (for
//! `Future` polling). When a task's waker is invoked, the task is
//! re-enqueued on the workqueue for another poll cycle.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

use crate::alloc::KVec;
use crate::error::Error;
use crate::revocable::AsyncRevocable;
use crate::sync::{Arc, ArcBorrow, Mutex};
use crate::workqueue::{Queue, Work, WorkItem, impl_has_work};

use super::{ArcWake, AutoStopHandle, Executor, RevocableTask, ref_waker};

// ---------------------------------------------------------------------------
// Task<F>
// ---------------------------------------------------------------------------

/// A spawned async task wrapping a `Future`.
///
/// `Task<F>` bridges the workqueue system with the `Future` polling
/// mechanism:
/// - It implements [`WorkItem`] so the workqueue can call `run()`.
/// - It implements [`ArcWake`] so the `Waker` re-enqueues it.
/// - The future is wrapped in [`AsyncRevocable`] for safe cancellation.
///
/// # Layout
///
/// The `work` field must be at a known offset for `container_of` to
/// recover the `Task` pointer from the `work_struct` pointer that the
/// kernel passes to the callback.
#[repr(C)]
struct Task<F: Future + Send + 'static> {
    /// Workqueue linkage — must be pinned and initialized before enqueue.
    work: Work<Task<F>>,
    /// The future, wrapped for revocation.
    future: AsyncRevocable<F>,
    /// Back-reference to the owning executor.
    executor: Arc<WorkqueueExecutor>,
}

// HasWork implementation — tells the workqueue system where the Work
// field lives inside Task<F>.
impl_has_work! {
    impl{F: Future + Send + 'static} HasWork<Task<F>> for Task<F> { self.work }
}

// SAFETY: Task fields are Send + Sync (AsyncRevocable<F: Send> is
// Send+Sync, Arc<WorkqueueExecutor> is Send+Sync, Work is Send+Sync).
unsafe impl<F: Future + Send + 'static> Send for Task<F> {}
unsafe impl<F: Future + Send + 'static> Sync for Task<F> {}

// ---------------------------------------------------------------------------
// WorkItem implementation — the workqueue calls this
// ---------------------------------------------------------------------------

impl<F: Future<Output = ()> + Send + 'static> WorkItem for Task<F> {
    type Pointer = Arc<Self>;

    fn run(this: Arc<Self>) {
        // Try to access the future through the revocable wrapper.
        // If revoked, the task is cancelled — just drop the Arc.
        let Some(future_guard) = this.future.try_access() else {
            return;
        };

        // Create a waker that will re-enqueue this task.
        let waker = ref_waker(&this);
        let mut cx = Context::from_waker(&waker);

        // SAFETY: The future inside AsyncRevocable is pinned because:
        // 1. Task is allocated via Arc (heap, stable address).
        // 2. We never move the future out of the AsyncRevocable.
        // 3. Only one guard (and thus one poll) exists at a time —
        //    workqueue dispatches are serialized per work item.
        let future_pin = unsafe { Pin::new_unchecked(&mut *future_guard.as_mut_ptr()) };

        match future_pin.poll(&mut cx) {
            Poll::Ready(()) => {
                // Future completed — revoke to release resources.
                drop(future_guard);
                this.future.revoke();
            }
            Poll::Pending => {
                // The waker will re-enqueue when progress is possible.
                // Drop the guard before the Arc so the !Send guard
                // doesn't cross an await boundary.
                drop(future_guard);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ArcWake implementation — the Waker re-enqueues the task
// ---------------------------------------------------------------------------

impl<F: Future<Output = ()> + Send + 'static> ArcWake for Task<F> {
    fn wake(self: Arc<Self>) {
        // Enqueue the task's work item on the executor's workqueue.
        // If already enqueued, that's fine — we get it back in the Err.
        let _ = self.executor.queue.enqueue(self);
    }
}

// ---------------------------------------------------------------------------
// RevocableTask implementation
// ---------------------------------------------------------------------------

impl<F: Future<Output = ()> + Send + 'static> RevocableTask for Task<F> {
    fn revoke(&self) {
        self.future.revoke();
    }

    fn flush(&self) {
        // SAFETY: Work is #[repr(transparent)] over Opaque<work_struct>,
        // so addr_of!(self.work) can be cast directly to *mut work_struct.
        // cancel_work_sync waits for in-flight workqueue execution to
        // complete, ensuring no callback is running when it returns.
        unsafe {
            rko_sys::rko::workqueue::cancel_work_sync(
                core::ptr::addr_of!(self.work)
                    .cast::<rko_sys::rko::workqueue::work_struct>()
                    .cast_mut(),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// WorkqueueExecutor
// ---------------------------------------------------------------------------

/// Inner state protected by the executor's mutex.
struct ExecutorInner {
    /// When `true`, no new tasks can be spawned.
    stopped: bool,
    /// Active task handles for stop() iteration.
    tasks: KVec<Arc<dyn RevocableTask>>,
}

/// An [`Executor`] backed by a kernel workqueue.
///
/// Each spawned future becomes a [`Task`] that is polled on the
/// workqueue. The executor tracks all live tasks so that [`stop`]
/// can revoke and flush them during module teardown.
pub struct WorkqueueExecutor {
    /// The backing workqueue.
    queue: &'static Queue,
    /// Protected state (stopped flag + task list).
    inner: Mutex<ExecutorInner>,
}

// SAFETY: All fields are Send+Sync.
unsafe impl Send for WorkqueueExecutor {}
unsafe impl Sync for WorkqueueExecutor {}

impl WorkqueueExecutor {
    /// Create a new workqueue executor backed by `queue`.
    ///
    /// Returns an [`AutoStopHandle`] wrapping the executor.
    pub fn new(queue: &'static Queue) -> Result<AutoStopHandle<Self>, Error> {
        use crate::sync::UniqueArc;

        let inner = ExecutorInner {
            stopped: false,
            tasks: KVec::new(),
        };

        // Allocate via UniqueArc to pin-initialize the Mutex in place.
        let mut uninit =
            UniqueArc::<WorkqueueExecutor>::new_uninit(crate::alloc::Flags::GFP_KERNEL)
                .map_err(|_| Error::ENOMEM)?;

        // SAFETY: We initialize all fields before calling assume_init.
        unsafe {
            let ptr = uninit.as_mut_ptr();
            core::ptr::addr_of_mut!((*ptr).queue).write(queue);

            // Initialize the Mutex in place. The backend init is currently
            // a stub, so this is safe even without the C helper.
            let mutex_slot = core::ptr::addr_of_mut!((*ptr).inner);
            // Write the data into the UnsafeCell inside the Lock.
            // Lock layout: { data: UnsafeCell<T>, state: Opaque<B::State>, _pin }
            // We use the PinInit returned by Mutex::new via raw pointer init.
            use pinned_init::PinInit;
            let init = crate::sync::Mutex::new(inner, c"wq_executor", crate::static_lock_class!());
            init.__pinned_init(mutex_slot)
                .unwrap_or_else(|e: core::convert::Infallible| match e {});
        }

        // SAFETY: All fields initialized above.
        let executor = unsafe { uninit.assume_init() };
        let arc: Arc<Self> = executor.into();

        Ok(AutoStopHandle::new(arc))
    }

    /// Internal spawn helper — takes a concrete future type `F`.
    fn spawn_inner<F>(self: ArcBorrow<'_, Self>, future: F) -> Result<Arc<dyn RevocableTask>, Error>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        let executor_arc: Arc<Self> = self.into();

        // Check that we haven't been stopped.
        {
            let guard = self.inner.lock();
            if guard.stopped {
                return Err(Error::EBUSY);
            }
        }

        // Allocate the Task via UniqueArc for in-place Work initialization.
        let mut task_uninit =
            crate::sync::UniqueArc::<Task<F>>::new_uninit(crate::alloc::Flags::GFP_KERNEL)
                .map_err(|_| Error::ENOMEM)?;

        // Initialize fields in place.
        // SAFETY: The pointer targets valid memory inside the UniqueArc
        // allocation, and we will pin it before any use.
        unsafe {
            let task_ptr = task_uninit.as_mut_ptr();
            Work::<Task<F>>::init(
                core::ptr::addr_of_mut!((*task_ptr).work),
                c"kasync_task",
                crate::static_lock_class!(),
            );
            core::ptr::addr_of_mut!((*task_ptr).future).write(AsyncRevocable::new(future));
            core::ptr::addr_of_mut!((*task_ptr).executor).write(executor_arc);
        }

        // SAFETY: All fields have been initialized above.
        let task = unsafe { task_uninit.assume_init() };
        let task_arc: Arc<Task<F>> = task.into();

        // Register with the executor's task list.
        let revocable: Arc<dyn RevocableTask> = task_arc.clone();
        {
            let mut guard = self.inner.lock();
            if guard.stopped {
                revocable.revoke();
                return Err(Error::EBUSY);
            }
            guard
                .tasks
                .push(revocable.clone(), crate::alloc::Flags::GFP_KERNEL)
                .map_err(|_| Error::ENOMEM)?;
        }

        // Enqueue the first poll.
        let _ = self.queue.enqueue(task_arc);

        Ok(revocable)
    }
}

impl Executor for WorkqueueExecutor {
    fn spawn(
        self: ArcBorrow<'_, Self>,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> Result<Arc<dyn RevocableTask>, Error> {
        self.spawn_inner(future)
    }

    fn stop(&self) {
        // Set the stopped flag.
        let tasks = {
            let mut guard = self.inner.lock();
            guard.stopped = true;
            // Take ownership of the task list.
            core::mem::take(&mut guard.tasks)
        };

        // Revoke and flush all tasks.
        //
        // Design doc specifies: front() + drop lock + revoke + flush.
        // Since we took the Vec above (under lock), we iterate without
        // holding the lock — revoke() doesn't need the executor lock.
        for task in tasks {
            task.revoke();
            task.flush();
        }
    }
}
