// SPDX-License-Identifier: GPL-2.0

//! HTTP server — accept loop, handler trait, connection management.

use core::future::Future;

use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::executor::{AutoStopHandle, Executor};
use rko_core::kasync::net::TcpStream;
use rko_core::net::{Namespace, SocketAddr};
use rko_core::sync::Arc;
use rko_core::workqueue;

use super::buf_reader::BufReader;
use super::error::HttpError;
use super::parse::parse_request;
use super::request::Request;
use super::response::Response;
use super::version::Version;
use super::wire::write_response;

/// Implement this to handle HTTP requests in your kernel module.
pub trait HttpHandler: Send + Sync + 'static {
    /// Handle a parsed HTTP request and return a response.
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_;
}

/// Blanket impl: async closures are handlers.
///
/// Note: the closure's returned future must be `'static` with respect
/// to the request (it cannot borrow from `req`). For handlers that
/// need to borrow request data, use a struct impl instead.
impl<F, Fut> HttpHandler for F
where
    F: Fn(&Request) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send + 'static,
{
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_ {
        (self)(req)
    }
}

/// Server configuration.
pub struct ServerConfig {
    /// Maximum request header size in bytes (default: 8192).
    pub max_header_size: usize,
    /// Maximum request body size in bytes (default: 1 MB).
    pub max_body_size: usize,
    /// Maximum concurrent connections (default: 64).
    pub max_connections: usize,
    /// Keep-alive timeout in seconds (default: 30).
    pub keepalive_timeout_secs: u32,
    /// Maximum headers per request (default: 64).
    pub max_headers: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            max_header_size: 8192,
            max_body_size: 1024 * 1024,
            max_connections: 64,
            keepalive_timeout_secs: 30,
            max_headers: 64,
        }
    }
}

/// An HTTP/1.1 server.
///
/// Spawns an accept loop on a workqueue executor. Automatically
/// stops all connections when dropped (if using the internal executor).
pub struct HttpServer {
    /// Owned executor — created by `start()`, dropped on server drop.
    /// None when using `start_on()` (caller owns the executor).
    _handle: Option<AutoStopHandle<WorkqueueExecutor>>,
}

impl HttpServer {
    /// Start serving HTTP on `addr` with the given handler.
    ///
    /// Creates its own `WorkqueueExecutor`. Uses `Namespace::init_ns()`.
    /// Returns immediately — connections are handled asynchronously.
    pub fn start<H: HttpHandler>(
        addr: &SocketAddr,
        handler: Arc<H>,
        config: ServerConfig,
    ) -> Result<Self, Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        Self::start_inner(addr, handler, config, exec)?;
        Ok(Self {
            _handle: Some(handle),
        })
    }

    /// Start serving on a caller-provided executor.
    ///
    /// The caller owns the executor lifetime. The server spawns its
    /// accept loop as a task on the provided executor. Useful when
    /// the caller needs to share the executor with other tasks
    /// (e.g., tests, or a module that also runs client requests).
    pub fn start_on<H: HttpHandler>(
        addr: &SocketAddr,
        handler: Arc<H>,
        config: ServerConfig,
        executor: Arc<WorkqueueExecutor>,
    ) -> Result<Self, Error> {
        Self::start_inner(addr, handler, config, executor)?;
        Ok(Self { _handle: None })
    }

    fn start_inner<H: HttpHandler>(
        addr: &SocketAddr,
        handler: Arc<H>,
        config: ServerConfig,
        exec: Arc<WorkqueueExecutor>,
    ) -> Result<(), Error> {
        let ns = Namespace::init_ns();
        let listener = rko_core::net::TcpListener::try_new(ns, addr)?;
        let async_listener = rko_core::kasync::net::TcpListener::new(listener);
        let config = Arc::new(config, Flags::GFP_KERNEL)?;

        let exec2 = exec.clone();
        exec.as_arc_borrow().spawn(async move {
            accept_loop(async_listener, handler, config, exec2).await;
        })?;
        Ok(())
    }
}

/// Accept loop — spawns a task per connection.
async fn accept_loop<H: HttpHandler>(
    listener: rko_core::kasync::net::TcpListener,
    handler: Arc<H>,
    config: Arc<ServerConfig>,
    executor: Arc<WorkqueueExecutor>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok(s) => s,
            Err(_) => return,
        };

        let h = handler.clone();
        let c = config.clone();
        #[allow(clippy::explicit_auto_deref)]
        let _ = executor.as_arc_borrow().spawn(async move {
            handle_connection(stream, &*h, &*c).await;
        });
    }
}

/// Handle one TCP connection with keep-alive support.
async fn handle_connection<H: HttpHandler>(stream: TcpStream, handler: &H, config: &ServerConfig) {
    let mut reader = match BufReader::new(config.max_header_size) {
        Ok(r) => r,
        Err(_) => return,
    };

    loop {
        let request = match parse_request(&mut reader, &stream, config).await {
            Ok(req) => req,
            Err(HttpError::ConnectionClosed) => return,
            Err(e) => {
                let _ = write_response(&stream, &e.to_response()).await;
                return;
            }
        };

        let keepalive = match request.version() {
            Version::Http11 => !request.headers().is_connection_close(),
            Version::Http10 => request.headers().is_connection_keepalive(),
        };

        let response = handler.handle(&request).await;

        if write_response(&stream, &response).await.is_err() {
            return;
        }

        if !keepalive {
            return;
        }

        reader.reset();
    }
}
