use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::sync::oneshot;

#[rko_core::rko_tests]
pub mod oneshot_tests {
    use super::*;

    #[test]
    fn send_then_recv() -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<u32>(Flags::GFP_KERNEL)?;
        tx.send(42);
        let val = rx.recv();
        assert_eq!(val, Some(42));
        Ok(())
    }

    #[test]
    fn sender_dropped_recv_returns_none() -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<u32>(Flags::GFP_KERNEL)?;
        drop(tx);
        let val = rx.recv();
        assert_eq!(val, None);
        Ok(())
    }

    #[test]
    fn recv_timeout_after_send() -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<u64>(Flags::GFP_KERNEL)?;
        tx.send(99);
        let val = rx.recv_timeout(1000);
        assert_eq!(val, Some(99));
        Ok(())
    }

    #[test]
    fn recv_timeout_sender_dropped() -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<u64>(Flags::GFP_KERNEL)?;
        drop(tx);
        let val = rx.recv_timeout(1000);
        assert_eq!(val, None);
        Ok(())
    }

    #[test]
    fn receiver_dropped_without_recv() -> Result<(), Error> {
        let (tx, rx) = oneshot::channel::<u32>(Flags::GFP_KERNEL)?;
        drop(rx);
        // Sender can still send — it just has no effect.
        tx.send(10);
        Ok(())
    }

    #[test]
    fn send_kvec_body() -> Result<(), Error> {
        use rko_core::alloc::KVec;
        let (tx, rx) = oneshot::channel::<KVec<u8>>(Flags::GFP_KERNEL)?;
        let body = KVec::from_slice(b"hello oneshot", Flags::GFP_KERNEL)?;
        tx.send(body);
        let val = rx.recv().unwrap();
        assert_eq!(val.as_slice(), b"hello oneshot");
        Ok(())
    }
}
