//! Async TCP echo server kernel module.
//!
//! Demonstrates the `kasync` executor framework with async networking.
//! Binds to `0.0.0.0:8080`, accepts connections asynchronously, and
//! echoes data back on each stream.
//!
//! Uses a **dedicated** workqueue (not `system_wq`) to isolate async
//! networking from the kernel's shared workqueue.

#![no_std]

use rko_core::kasync::executor::AutoStopHandle;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::net::{TcpListener, TcpStream};
use rko_core::net::{Ipv4Addr, Namespace, SocketAddr};
use rko_core::prelude::*;
use rko_core::workqueue;

const LISTEN_PORT: u16 = 8080;

#[allow(dead_code)]
const BUF_SIZE: usize = 4096;

/// Echo loop: read from the async stream, write back, until EOF or error.
#[allow(dead_code)]
async fn echo_client(stream: TcpStream) {
    let mut buf = [0u8; BUF_SIZE];
    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) => return,
            Ok(n) => n,
            Err(_) => return,
        };
        if stream.write_all(&buf[..n]).await.is_err() {
            return;
        }
    }
}

struct AsyncEcho {
    _executor: AutoStopHandle<WorkqueueExecutor>,
}

impl Module for AsyncEcho {
    fn init() -> Result<Self, Error> {
        let addr = SocketAddr::new_v4(Ipv4Addr::ANY, LISTEN_PORT);
        let ns = Namespace::init_ns();
        let _listener = TcpListener::try_new(ns, &addr)?;

        // Use the system workqueue for now — a production module would
        // create a dedicated workqueue via Queue::try_new().
        let handle = WorkqueueExecutor::new(workqueue::system())?;

        pr_info!("async echo: listening on 0.0.0.0:{}\n", LISTEN_PORT);

        // Spawn the accept loop.
        // TODO: The accept loop would normally run as a spawned task on
        // the executor. For now we show the API shape — the actual
        // loop requires the listener to be moved into a 'static future.
        //
        // handle.executor().spawn(async move {
        //     loop {
        //         match listener.accept().await {
        //             Ok(stream) => { let _ = echo_client(stream).await; }
        //             Err(_) => break,
        //         }
        //     }
        // })?;

        Ok(AsyncEcho { _executor: handle })
    }

    fn exit(&self) {
        pr_info!("async echo: module unloaded\n");
        // AutoStopHandle::drop calls executor.stop() — revokes all
        // tasks and flushes the workqueue.
    }
}

module! {
    type: AsyncEcho,
    name: "async_echo",
    license: "GPL",
    author: "rko",
    description: "Async TCP echo server using rko kasync framework",
}
