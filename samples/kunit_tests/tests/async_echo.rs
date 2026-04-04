use rko_core::error::Error;
use rko_core::kasync::executor::Executor;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::net::TcpListener;
use rko_core::net::{Ipv4Addr, Namespace, SocketAddr};
use rko_core::workqueue;

#[rko_core::rko_tests]
pub mod async_echo_tests {
    use super::*;

    #[test]
    fn block_on_basic() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let val = exec.block_on(async { 42i32 })?;
        assert_eq!(val, 42);
        Ok(())
    }

    #[test]
    fn block_on_with_yield() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let val = exec.block_on(async {
            rko_core::kasync::yield_now().await;
            99i32
        })?;
        assert_eq!(val, 99);
        Ok(())
    }

    /// Test: bind to loopback (requires test.sh to bring up lo).
    #[test]
    fn bind_listener() -> Result<(), Error> {
        let addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19876);
        let ns = Namespace::init_ns();
        let _listener = rko_core::net::TcpListener::try_new(ns, &addr)?;
        Ok(())
    }

    /// Test: async accept + blocking connect via block_on.
    #[test]
    fn async_echo() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let exec2 = exec.clone();

        let addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19877);
        let ns = Namespace::init_ns();
        let listener = TcpListener::try_new(ns, &addr)?;

        let result = exec.block_on(async move {
            let connect_addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19877);
            let client_ns = Namespace::init_ns();
            exec2
                .as_arc_borrow()
                .spawn(async move {
                    rko_core::kasync::yield_now().await;
                    let stream = rko_core::net::TcpStream::connect(client_ns, &connect_addr)
                        .expect("connect failed");
                    stream.write_all(b"ping").expect("write failed");
                })
                .unwrap();

            let stream = listener.accept().await?;
            let mut buf = [0u8; 16];
            let n = stream.read(&mut buf).await?;
            Ok::<bool, Error>(&buf[..n] == b"ping")
        })?;

        assert!(result?);
        Ok(())
    }
}
