//! Kernel task (thread) management.
//!
//! Provides a safe wrapper around `struct task_struct` and a `KTask` type
//! for spawning joinable kernel threads.
//!
//! The `current_raw`, `get_task_struct`, and `put_task_struct` helpers are
//! wired up via C helpers. `KTask::spawn` and `KTask::drop` require
//! `kthread_create_on_node`, `wake_up_process`, and `kthread_stop` which
//! are not yet in the rko-sys bindings (they need a new partition or helpers).

use crate::types::Opaque;

/// Wraps the kernel's `struct task_struct`.
///
/// # Invariants
///
/// All instances are valid tasks created by the C portion of the kernel.
/// Instances are always refcounted: a call to `get_task_struct` ensures
/// the allocation remains valid until the matching `put_task_struct`.
#[repr(transparent)]
pub struct Task(Opaque<rko_sys::rko::fs::task_struct>);

// SAFETY: By design the only way to access a `Task` is via `current!()` or via
// an `ARef<Task>` obtained through the `AlwaysRefCounted` impl. The task_struct
// fields we access are either immutable or synchronized by kernel code.
unsafe impl Send for Task {}
unsafe impl Sync for Task {}

/// The type of process identifiers (PIDs).
pub type Pid = rko_sys::rko::types::pid_t;

impl Task {
    /// Returns a raw pointer to the `task_struct` of the currently executing task.
    ///
    /// In the kernel this reads the per-CPU `current` variable. A proper
    /// implementation requires a C helper or inline assembly that is not
    /// yet wired up.
    ///
    /// # Safety
    ///
    /// The returned pointer is only valid while the caller remains in the
    /// same task context (i.e. no schedule / return to userspace).
    pub unsafe fn current_raw() -> *mut rko_sys::rko::fs::task_struct {
        // SAFETY: Returns the per-CPU `current` task_struct pointer via
        // the C helper which reads the kernel's `current` macro.
        // The returned pointer is only valid while the caller remains in
        // the same task context (i.e. no schedule / return to userspace).
        unsafe { rko_sys::rko::helpers::rust_helper_get_current() }
    }

    /// Returns the PID (thread-group-independent) of this task.
    pub fn pid(&self) -> Pid {
        // SAFETY: The Opaque wraps a valid task_struct for the lifetime of &self.
        // __task_pid_nr_ns(task, PIDTYPE_PID, NULL) is the exported version of
        // the inline task_pid_nr().
        unsafe {
            rko_sys::rko::fs::__task_pid_nr_ns(
                self.0.get(),
                rko_sys::rko::fs::PIDTYPE_PID,
                core::ptr::null_mut(),
            )
        }
    }
}

// SAFETY: `get_task_struct` and `put_task_struct` maintain the kernel
// refcount for `task_struct`, wired up via C helpers.
unsafe impl crate::types::AlwaysRefCounted for Task {
    fn inc_ref(&self) {
        // SAFETY: The task_struct is valid for the lifetime of &self.
        unsafe { rko_sys::rko::helpers::rust_helper_get_task_struct(self.0.get()) };
    }

    unsafe fn dec_ref(obj: core::ptr::NonNull<Self>) {
        // SAFETY: obj points to a valid Task whose task_struct has a
        // non-zero refcount.
        unsafe { rko_sys::rko::helpers::rust_helper_put_task_struct((*obj.as_ptr()).0.get()) };
    }
}

/// Returns a raw pointer to the current `task_struct`.
///
/// This is a stub that returns `null` until the C helper for reading
/// the per-CPU `current` pointer is wired up.
#[macro_export]
macro_rules! current {
    () => {
        // SAFETY: Wrapper around Task::current_raw().
        // The returned pointer must not outlive the current task context.
        unsafe { $crate::task::Task::current_raw() }
    };
}

/// A joinable kernel thread handle.
///
/// Owns a reference to the spawned `task_struct`. When dropped, the
/// thread is stopped via `kthread_stop`.
pub struct KTask {
    task: *mut rko_sys::rko::fs::task_struct,
}

// SAFETY: Kernel threads can be waited on / stopped from any context.
unsafe impl Send for KTask {}
unsafe impl Sync for KTask {}

impl KTask {
    /// Spawn a new kernel thread that executes `func`.
    ///
    /// # Stub
    ///
    /// This requires `kthread_create` + `wake_up_process` C helpers that
    /// are not yet in rko-sys. Returns `Err(EINVAL)` until wired up.
    pub fn spawn<F>(_func: F) -> Result<Self, crate::error::Error>
    where
        F: FnOnce() + Send + 'static,
    {
        // Requires kthread_create_on_node + wake_up_process, which are
        // not in rko-sys bindings (needs sched partition or C helpers).
        Err(crate::error::Error::EINVAL)
    }

    /// Returns the PID of the spawned thread.
    pub fn pid(&self) -> Pid {
        if self.task.is_null() {
            return 0;
        }
        // SAFETY: self.task is a valid task_struct while KTask is alive.
        unsafe {
            rko_sys::rko::fs::__task_pid_nr_ns(
                self.task,
                rko_sys::rko::fs::PIDTYPE_PID,
                core::ptr::null_mut(),
            )
        }
    }
}

impl Drop for KTask {
    fn drop(&mut self) {
        if !self.task.is_null() {
            // Requires kthread_stop (not in rko-sys bindings).
            // When added: kthread_stop(self.task); put_task_struct(self.task);
        }
    }
}
