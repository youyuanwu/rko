use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::kasync::executor::Executor;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::oneshot as async_oneshot;
use rko_core::workqueue;

#[rko_core::rko_tests]
pub mod async_oneshot_tests {
    use super::*;

    #[test]
    fn async_send_then_await() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();

        let (tx, rx) = async_oneshot::channel::<u32>(Flags::GFP_KERNEL)?;

        exec.as_arc_borrow().spawn(async move {
            tx.send(42);
        })?;

        let result = exec.block_on(rx)?;
        assert_eq!(result, Some(42));
        Ok(())
    }

    #[test]
    fn async_sender_dropped() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();

        let (tx, rx) = async_oneshot::channel::<u32>(Flags::GFP_KERNEL)?;

        exec.as_arc_borrow().spawn(async move {
            drop(tx);
        })?;

        let result = exec.block_on(rx)?;
        assert_eq!(result, None);
        Ok(())
    }

    #[test]
    fn async_send_kvec() -> Result<(), Error> {
        use rko_core::alloc::KVec;

        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();

        let (tx, rx) = async_oneshot::channel::<KVec<u8>>(Flags::GFP_KERNEL)?;

        exec.as_arc_borrow().spawn(async move {
            let data = KVec::from_slice(b"async hello", Flags::GFP_KERNEL).unwrap();
            tx.send(data);
        })?;

        let result = exec.block_on(rx)?;
        let val = result.unwrap();
        assert_eq!(val.as_slice(), b"async hello");
        Ok(())
    }
}
