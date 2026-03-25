//! Synchronous TCP echo server kernel module.
//!
//! Binds to `0.0.0.0:8080` and echoes back any data received on each
//! accepted connection. The accept loop runs as a workqueue work item
//! so module init returns immediately.

#![no_std]

use core::sync::atomic::{AtomicBool, Ordering};

use rko_core::alloc::Flags;
use rko_core::net::{Ipv4Addr, Namespace, SocketAddr, TcpListener, TcpStream};
use rko_core::prelude::*;
use rko_core::static_lock_class;
use rko_core::sync::{Arc, UniqueArc};
use rko_core::workqueue::{self, Work, WorkItem, impl_has_work};

const LISTEN_PORT: u16 = 8080;
const BUF_SIZE: usize = 4096;

/// The server struct holds the listener and an embedded Work field so
/// it can be enqueued on a workqueue as an `Arc<EchoServer>`.
#[repr(C)]
struct EchoServer {
    listener: TcpListener,
    stop: AtomicBool,
    work: Work<EchoServer>,
}

impl_has_work! {
    impl HasWork<EchoServer> for EchoServer { self.work }
}

impl WorkItem for EchoServer {
    type Pointer = Arc<EchoServer>;

    fn run(this: Arc<EchoServer>) {
        pr_info!("tcp_echo: accept loop running\n");
        while !this.stop.load(Ordering::Relaxed) {
            match this.listener.accept(true) {
                Ok(stream) => Self::echo_client(&stream),
                Err(_) => break,
            }
        }
        pr_info!("tcp_echo: accept loop exited\n");
    }
}

impl EchoServer {
    fn echo_client(stream: &TcpStream) {
        let mut buf = [0u8; BUF_SIZE];
        loop {
            let n = match stream.read(&mut buf) {
                Ok(0) => return,
                Ok(n) => n,
                Err(_) => return,
            };
            if stream.write_all(&buf[..n]).is_err() {
                return;
            }
        }
    }
}

struct TcpEcho {
    server: Arc<EchoServer>,
}

impl Module for TcpEcho {
    fn init() -> Result<Self, Error> {
        let addr = SocketAddr::new_v4(Ipv4Addr::ANY, LISTEN_PORT);
        let ns = Namespace::init_ns();
        let listener = TcpListener::try_new(ns, &addr)?;

        pr_info!("tcp_echo: listening on 0.0.0.0:{}\n", LISTEN_PORT);

        // Allocate EchoServer with embedded Work field using UniqueArc.
        let mut ua = UniqueArc::<EchoServer>::new_uninit(Flags::GFP_KERNEL)?;
        let ptr = ua.as_mut_ptr();
        // SAFETY: ptr is valid for writes (UniqueArc owns the allocation).
        unsafe {
            core::ptr::addr_of_mut!((*ptr).listener).write(listener);
            core::ptr::addr_of_mut!((*ptr).stop).write(AtomicBool::new(false));
            Work::<EchoServer>::init(
                core::ptr::addr_of_mut!((*ptr).work),
                c"EchoServer::work",
                static_lock_class!(),
            );
        }
        // SAFETY: All fields initialized above.
        let ua = unsafe { ua.assume_init() };
        let server = Arc::from(ua);

        // Enqueue the accept loop on the system workqueue.
        // Arc ownership is transferred to the workqueue — the clone
        // in `self.server` keeps the EchoServer alive.
        let _ = workqueue::system().enqueue(server.clone());

        Ok(TcpEcho { server })
    }

    fn exit(&self) {
        // Signal the accept loop to stop.
        self.server.stop.store(true, Ordering::Relaxed);
        // Dropping TcpEcho drops our Arc<EchoServer>. The listener's
        // socket is released when the last Arc drops (after the work
        // item completes), which unblocks any pending accept().
        pr_info!("tcp_echo: module unloaded\n");
    }
}

module! {
    type: TcpEcho,
    name: "tcp_echo",
    license: "GPL",
    author: "rko",
    description: "Synchronous TCP echo server using rko networking",
}
