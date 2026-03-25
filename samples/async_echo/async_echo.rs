//! Async TCP echo server kernel module.
//!
//! Demonstrates the `kasync` executor framework with async networking.
//! Binds to `0.0.0.0:8080`, accepts connections asynchronously via
//! `kasync::net::TcpListener`, and echoes data back on each stream.
//!
//! The accept loop and echo handlers run as spawned tasks on the
//! workqueue executor — no kernel threads needed.

#![no_std]

use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::executor::{AutoStopHandle, Executor};
use rko_core::kasync::net::{TcpListener, TcpStream};
use rko_core::net::{Ipv4Addr, Namespace, SocketAddr};
use rko_core::prelude::*;
use rko_core::workqueue;

const LISTEN_PORT: u16 = 8080;
const BUF_SIZE: usize = 4096;

/// Echo a single connection: read → write back, until EOF or error.
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

/// Accept loop: accepts connections and echoes each one inline.
async fn accept_loop(listener: TcpListener) {
    while let Ok(stream) = listener.accept().await {
        echo_client(stream).await;
    }
}

struct AsyncEcho {
    _handle: AutoStopHandle<WorkqueueExecutor>,
}

impl Module for AsyncEcho {
    fn init() -> Result<Self, Error> {
        let addr = SocketAddr::new_v4(Ipv4Addr::ANY, LISTEN_PORT);
        let ns = Namespace::init_ns();
        let listener = TcpListener::try_new(ns, &addr)?;

        let handle = WorkqueueExecutor::new(workqueue::system())?;

        pr_info!("async echo: listening on 0.0.0.0:{}\n", LISTEN_PORT);

        // Spawn the accept loop — listener is moved into the future.
        handle.executor().spawn(accept_loop(listener))?;

        Ok(AsyncEcho { _handle: handle })
    }

    fn exit(&self) {
        pr_info!("async echo: module unloaded\n");
    }
}

module! {
    type: AsyncEcho,
    name: "async_echo",
    license: "GPL",
    author: "rko",
    description: "Async TCP echo server using rko kasync framework",
}
