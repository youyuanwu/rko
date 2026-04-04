use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU32, Ordering};
use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::sync::{Arc, Completion, UniqueArc};
use rko_core::workqueue::{self, Work, WorkItem};
use rko_core::{impl_has_work, static_lock_class};

// A simple work item that increments a shared counter.
struct CounterWork {
    work: Work<CounterWork>,
    counter: Arc<AtomicU32>,
}

impl_has_work! {
    impl HasWork<Self> for CounterWork { self.work }
}

impl WorkItem for CounterWork {
    type Pointer = Arc<CounterWork>;
    fn run(this: Arc<CounterWork>) {
        this.counter.fetch_add(1, Ordering::Release);
    }
}

impl CounterWork {
    fn new(counter: Arc<AtomicU32>) -> Result<Arc<Self>, Error> {
        let mut ua = UniqueArc::<CounterWork>::new_uninit(Flags::GFP_KERNEL)?;
        let ptr = ua.as_mut_ptr();
        unsafe {
            Work::<CounterWork>::init(
                core::ptr::addr_of_mut!((*ptr).work),
                c"CounterWork::work",
                static_lock_class!(),
            );
            core::ptr::addr_of_mut!((*ptr).counter).write(counter);
        }
        let ua = unsafe { ua.assume_init() };
        Ok(Arc::from(ua))
    }
}

// A work item that signals a completion when run.
struct SignalWork {
    work: Work<SignalWork>,
    comp: *mut Completion,
}

impl_has_work! {
    impl HasWork<Self> for SignalWork { self.work }
}

// SAFETY: comp points to a stack-allocated Completion that outlives this work.
unsafe impl Send for SignalWork {}
unsafe impl Sync for SignalWork {}

impl WorkItem for SignalWork {
    type Pointer = Arc<SignalWork>;
    fn run(this: Arc<SignalWork>) {
        unsafe { (*this.comp).complete() };
    }
}

impl SignalWork {
    fn new(comp: *mut Completion) -> Result<Arc<Self>, Error> {
        let mut ua = UniqueArc::<SignalWork>::new_uninit(Flags::GFP_KERNEL)?;
        let ptr = ua.as_mut_ptr();
        unsafe {
            Work::<SignalWork>::init(
                core::ptr::addr_of_mut!((*ptr).work),
                c"SignalWork::work",
                static_lock_class!(),
            );
            core::ptr::addr_of_mut!((*ptr).comp).write(comp);
        }
        let ua = unsafe { ua.assume_init() };
        Ok(Arc::from(ua))
    }
}

#[rko_core::rko_tests]
pub mod workqueue_tests {
    use super::*;

    #[test]
    fn system_queue_exists() {
        // Just verify we can get a reference to the system workqueue.
        let _q = workqueue::system();
    }

    #[test]
    fn enqueue_and_run() -> Result<(), Error> {
        let counter = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;
        let item = CounterWork::new(counter.clone())?;

        if workqueue::system().enqueue(item).is_err() {
            return Err(Error::EBUSY);
        }

        // Wait for the work item to execute via Completion.
        let mut comp = MaybeUninit::<Completion>::uninit();
        unsafe { Completion::init(comp.as_mut_ptr()) };
        let signal = SignalWork::new(comp.as_mut_ptr())?;
        if workqueue::system().enqueue(signal).is_err() {
            return Err(Error::EBUSY);
        }
        let comp = unsafe { comp.assume_init_mut() };
        comp.wait_timeout(5000);

        // Counter should have been incremented.
        assert!(counter.load(Ordering::Acquire) >= 1);
        Ok(())
    }

    #[test]
    fn multiple_work_items() -> Result<(), Error> {
        let counter = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;

        for _ in 0..5 {
            let item = CounterWork::new(counter.clone())?;
            let _ = workqueue::system().enqueue(item);
        }

        // Signal work to wait for all prior items to drain.
        let mut comp = MaybeUninit::<Completion>::uninit();
        unsafe { Completion::init(comp.as_mut_ptr()) };
        let signal = SignalWork::new(comp.as_mut_ptr())?;
        let _ = workqueue::system().enqueue(signal);
        let comp = unsafe { comp.assume_init_mut() };
        comp.wait_timeout(5000);

        assert_eq!(counter.load(Ordering::Acquire), 5);
        Ok(())
    }
}
