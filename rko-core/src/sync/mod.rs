//! Kernel synchronization primitives.
//!
//! Ported from the in-tree Linux kernel Rust crate (`linux/rust/kernel/sync/`).
// UPSTREAM_REF: linux/rust/kernel/sync.rs, linux/rust/kernel/sync/

mod arc;
pub mod completion;
pub mod condvar;
pub mod lock;
pub mod nowaitlock;
pub mod rcu;
mod refcount;

pub use arc::{Arc, ArcBorrow, UniqueArc};
pub use completion::Completion;
pub use condvar::CondVar;
pub use lock::mutex::{Mutex, MutexGuard, new_mutex};
pub use lock::spinlock::{SpinLock, SpinLockGuard, new_spinlock};
pub use lock::{Backend, Guard, Lock};
pub use nowaitlock::{NoWaitLock, NoWaitLockGuard};
pub use refcount::Refcount;

use crate::types::Opaque;

/// A lock class key for lockdep.
///
/// Wraps `struct lock_class_key`. Without lockdep C helpers this is
/// currently a placeholder — the key is never registered with lockdep.
// UPSTREAM_REF: linux/rust/kernel/sync.rs LockClassKey
#[repr(transparent)]
pub struct LockClassKey(Opaque<rko_sys::rko::fs::lock_class_key>);

// SAFETY: lock_class_key is designed for concurrent use by lockdep.
unsafe impl Send for LockClassKey {}
// SAFETY: lock_class_key is designed for concurrent use by lockdep.
unsafe impl Sync for LockClassKey {}

impl LockClassKey {
    /// Create a new uninitialized lock class key for use in a `static`.
    ///
    /// # Safety
    ///
    /// The resulting key must be stored in a `static` and never dropped
    /// (its destructor must never run). This is usually done via the
    /// [`static_lock_class!`] macro.
    pub const unsafe fn new_static() -> Self {
        LockClassKey(Opaque::uninit())
    }

    /// Return a raw pointer to the inner `lock_class_key`.
    pub fn as_ptr(&self) -> *mut rko_sys::rko::fs::lock_class_key {
        self.0.get()
    }
}

/// Create a `static` [`LockClassKey`] and return a reference to it.
///
/// Each call-site gets its own unique key (one per `static`).
// UPSTREAM_REF: linux/rust/kernel/sync.rs static_lock_class!
#[macro_export]
macro_rules! static_lock_class {
    () => {{
        static CLASS: $crate::sync::LockClassKey =
            // SAFETY: Stored in a static, never dropped.
            unsafe { $crate::sync::LockClassKey::new_static() };
        &CLASS
    }};
}
pub use static_lock_class;
