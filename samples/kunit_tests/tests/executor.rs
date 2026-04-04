use core::sync::atomic::{AtomicU32, Ordering};
use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::kasync::executor::Executor;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::sync::Arc;
use rko_core::workqueue;

#[rko_core::rko_tests]
pub mod executor_tests {
    use super::*;

    #[test]
    fn spawn_runs() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let counter = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;
        let c = counter.clone();

        handle.executor().spawn(async move {
            c.fetch_add(1, Ordering::Release);
        })?;

        // block_on a trivial future to wait for the spawned task.
        let exec = handle.executor_arc();
        exec.block_on(async {
            rko_core::kasync::yield_now().await;
            rko_core::kasync::yield_now().await;
        })?;

        assert!(counter.load(Ordering::Acquire) >= 1);
        Ok(())
    }

    #[test]
    fn block_on_returns_value() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let val = exec.block_on(async { 42i32 })?;
        assert_eq!(val, 42);
        Ok(())
    }

    #[test]
    fn block_on_with_multiple_yields() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();

        let val = exec.block_on(async {
            let mut sum = 0i32;
            for i in 0..5 {
                rko_core::kasync::yield_now().await;
                sum += i;
            }
            sum
        })?;

        assert_eq!(val, 10); // 0+1+2+3+4
        Ok(())
    }

    #[test]
    fn block_on_with_spawn() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let exec2 = exec.clone();

        let counter = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;
        let c = counter.clone();

        let val = exec.block_on(async move {
            // Spawn a side task that increments the counter.
            exec2
                .as_arc_borrow()
                .spawn(async move {
                    c.fetch_add(10, Ordering::Release);
                })
                .unwrap();

            // Yield to let the spawned task run.
            rko_core::kasync::yield_now().await;
            rko_core::kasync::yield_now().await;

            counter.load(Ordering::Acquire)
        })?;

        assert_eq!(val, 10);
        Ok(())
    }

    /// Two tasks communicate via a shared atomic: producer writes,
    /// consumer reads. Tests cooperative scheduling on the workqueue.
    #[test]
    fn two_tasks_producer_consumer() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let exec2 = exec.clone();

        let shared = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;
        let producer_val = shared.clone();
        let consumer_val = shared.clone();

        let result = exec.block_on(async move {
            // Producer: yields, then writes a value.
            exec2
                .as_arc_borrow()
                .spawn(async move {
                    rko_core::kasync::yield_now().await;
                    producer_val.store(42, Ordering::Release);
                })
                .unwrap();

            // Consumer: spin-yield until producer writes.
            let mut attempts = 0u32;
            loop {
                rko_core::kasync::yield_now().await;
                let v = consumer_val.load(Ordering::Acquire);
                if v == 42 {
                    return Ok::<u32, Error>(v);
                }
                attempts += 1;
                if attempts > 100 {
                    return Err(Error::EBUSY);
                }
            }
        })?;

        let val = result?;
        assert_eq!(val, 42);
        Ok(())
    }

    #[test]
    fn stop_cancels_tasks() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let counter = Arc::new(AtomicU32::new(0), Flags::GFP_KERNEL)?;
        let c = counter.clone();

        // Spawn a task that loops with yields.
        handle.executor().spawn(async move {
            loop {
                rko_core::kasync::yield_now().await;
                c.fetch_add(1, Ordering::Release);
            }
        })?;

        // Let it run a bit, then stop.
        let exec = handle.executor_arc();
        exec.block_on(async {
            rko_core::kasync::yield_now().await;
        })?;

        handle.stop();

        // Counter should have been incremented at least once.
        let val = counter.load(Ordering::Acquire);
        assert!(val >= 1);
        Ok(())
    }
}
