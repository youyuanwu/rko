//! Kernel module that exercises workqueue APIs from rko-core.

#![no_std]

use rko_core::alloc::Flags;
use rko_core::prelude::*;
use rko_core::sync::{Arc, UniqueArc};
use rko_core::workqueue::{self, Work, WorkItem};

// Re-import the macros from the crate root (where #[macro_export] places them).
use rko_core::{impl_has_work, static_lock_class};

struct WorkqueueTest;

/// A simple work item that prints a message when executed.
struct MyWork {
    work: Work<MyWork>,
}

impl_has_work! {
    impl HasWork<Self> for MyWork { self.work }
}

impl WorkItem for MyWork {
    type Pointer = Arc<MyWork>;

    fn run(_this: Arc<MyWork>) {
        pr_info!("work executed\n");
    }
}

impl MyWork {
    /// Allocate and initialize a new `Arc<MyWork>`.
    fn new() -> Result<Arc<Self>, Error> {
        let mut ua = UniqueArc::<MyWork>::new_uninit(Flags::GFP_KERNEL)?;
        let ptr = ua.as_mut_ptr();

        // SAFETY: `ptr` is valid for writes (UniqueArc owns the allocation).
        // The Work field is properly initialized via `Work::init`.
        unsafe {
            Work::<MyWork>::init(
                core::ptr::addr_of_mut!((*ptr).work),
                c"MyWork::work",
                static_lock_class!(),
            );
        }

        // SAFETY: All fields have been initialized above.
        let ua = unsafe { ua.assume_init() };
        Ok(Arc::from(ua))
    }
}

impl Module for WorkqueueTest {
    fn init() -> Result<Self, Error> {
        let item = MyWork::new()?;

        // Enqueue the work item on the system workqueue.
        if workqueue::system().enqueue(item).is_err() {
            pr_warn!("failed to enqueue work item\n");
            return Err(Error::EBUSY);
        }

        pr_info!("module loaded\n");
        Ok(WorkqueueTest)
    }

    fn exit(&self) {
        pr_info!("module unloaded\n");
    }
}

module! {
    type: WorkqueueTest,
    name: "workqueue_test",
    license: "GPL",
    author: "rko",
    description: "Workqueue test kernel module using rko",
}
