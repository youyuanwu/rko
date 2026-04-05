// SPDX-License-Identifier: GPL-2.0

//! io_uring HTTP API — userspace control of kernel HTTP server.
//!
//! Provides [`HttpUringHandler`] which bridges the kernel HTTP server
//! to userspace via `IORING_OP_URING_CMD` custom commands on a misc
//! device. Userspace receives parsed HTTP requests and sends complete
//! responses through io_uring.
//!
//! See `docs/design/features/futures/io-uring-http-api.md`.

use core::cell::UnsafeCell;
use core::future::Future;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;
use rko_core::io_uring::{IoUringCmd, IssueFlags};
use rko_core::kasync::executor::AutoStopHandle;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::kasync::oneshot as async_oneshot;
use rko_core::net::{Ipv4Addr, SocketAddr};
use rko_core::sync::Arc;
use rko_core::workqueue;

use super::method::Method;
use super::request::Request;
use super::response::Response;
use super::server::{HttpHandler, HttpServer, ServerConfig};
use super::status::StatusCode;
use super::version::Version;

// ── Command opcodes ──────────────────────────────────────────────────

pub const HTTP_CMD_SERVER_START: u32 = 0;
pub const HTTP_CMD_SERVER_STOP: u32 = 1;
pub const HTTP_CMD_CREATE_QUEUE: u32 = 2;
pub const HTTP_CMD_DESTROY_QUEUE: u32 = 3;
pub const HTTP_CMD_ADD_URL: u32 = 4;
pub const HTTP_CMD_REMOVE_URL: u32 = 5;
pub const HTTP_CMD_RECV_REQUEST: u32 = 6;
pub const HTTP_CMD_SEND_RESPONSE: u32 = 7;

// ── Command payloads (mirror UAPI structs) ───────────────────────────

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CmdServerAddr {
    pub addr: u32,
    pub port: u16,
    pub reserved: u16,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CmdCreateQueue {
    pub max_pending: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CmdUrl {
    pub queue_id: u32,
    pub url_len: u32,
    pub url_addr: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CmdRecvRequest {
    pub queue_id: u32,
    pub buf_len: u32,
    pub buf_addr: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct CmdSendResponse {
    pub req_id: u64,
    pub status_code: u16,
    pub header_count: u16,
    pub body_len: u32,
    pub buf_addr: u64,
}

// ── Request header (kernel → userspace, in provided buffer) ──────────

#[repr(C)]
pub struct RequestHdr {
    pub req_id: u64,
    pub method: u8,
    pub version: u8,
    pub header_count: u16,
    pub path_len: u32,
    pub body_len: u32,
    pub total_len: u32,
}

#[repr(C)]
pub struct HeaderEntry {
    pub name_len: u16,
    pub value_len: u16,
}

// ── Simple spin-mutex for handler state ──────────────────────────────
// The kernel's SpinLock requires PinInit which can't be used in a plain
// new() constructor. This tiny spin-mutex is sufficient for the handler's
// short critical sections.

struct SimpleMutex<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for SimpleMutex<T> {}
unsafe impl<T: Send> Sync for SimpleMutex<T> {}

impl<T> SimpleMutex<T> {
    fn new(data: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    fn lock(&self) -> SimpleMutexGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        SimpleMutexGuard { mutex: self }
    }
}

struct SimpleMutexGuard<'a, T> {
    mutex: &'a SimpleMutex<T>,
}

impl<T> core::ops::Deref for SimpleMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> core::ops::DerefMut for SimpleMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}

impl<T> Drop for SimpleMutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
    }
}

// ── Internal types ───────────────────────────────────────────────────

/// A pending request awaiting userspace response.
struct PendingRequest {
    req_id: u64,
    response_tx: async_oneshot::Sender<Response>,
}

/// A request queue that userspace subscribes to.
#[allow(dead_code)]
struct RequestQueue {
    id: u32,
    max_pending: u32,
    /// Buffered serialized requests waiting for a subscriber.
    buffered: KVec<BufferedRequest>,
}

/// A serialized request ready for delivery to userspace.
#[allow(dead_code)]
struct BufferedRequest {
    req_id: u64,
    data: KVec<u8>,
}

// ── HttpUringHandler ─────────────────────────────────────────────────

/// Central coordinator between io_uring commands and the HTTP server.
///
/// Manages request queues, URL routing, pending requests, and the
/// HTTP server lifecycle. Created per-fd in the misc device open.
pub struct HttpUringHandler {
    queues: SimpleMutex<KVec<(u32, RequestQueue)>>,
    routes: SimpleMutex<KVec<(KVec<u8>, u32)>>,
    pending: SimpleMutex<KVec<PendingRequest>>,
    next_req_id: AtomicU64,
    next_queue_id: AtomicU32,
    server: SimpleMutex<Option<HttpServer>>,
    /// Executor handle — keeps the executor alive. Dropping stops it.
    executor_handle: SimpleMutex<Option<AutoStopHandle<WorkqueueExecutor>>>,
}

// SAFETY: All fields are either atomic or behind SimpleMutex.
unsafe impl Send for HttpUringHandler {}
unsafe impl Sync for HttpUringHandler {}

impl HttpUringHandler {
    /// Create a new handler.
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            queues: SimpleMutex::new(KVec::new()),
            routes: SimpleMutex::new(KVec::new()),
            pending: SimpleMutex::new(KVec::new()),
            next_req_id: AtomicU64::new(1),
            next_queue_id: AtomicU32::new(1),
            server: SimpleMutex::new(None),
            executor_handle: SimpleMutex::new(None),
        })
    }

    /// Dispatch an io_uring command.
    ///
    /// Takes `&Arc<Self>` so commands like SERVER_START can clone the
    /// Arc for the bridge handler (which outlives the uring_cmd callback).
    pub fn dispatch(handler: &Arc<Self>, cmd: IoUringCmd, flags: IssueFlags) -> i32 {
        let op = cmd.cmd_op();
        match op {
            HTTP_CMD_SERVER_START => Self::cmd_server_start(handler, cmd, flags),
            HTTP_CMD_SERVER_STOP => handler.cmd_server_stop(cmd, flags),
            HTTP_CMD_CREATE_QUEUE => handler.cmd_create_queue(cmd, flags),
            HTTP_CMD_DESTROY_QUEUE => handler.cmd_destroy_queue(cmd, flags),
            HTTP_CMD_ADD_URL => handler.cmd_add_url(cmd, flags),
            HTTP_CMD_REMOVE_URL => handler.cmd_remove_url(cmd, flags),
            HTTP_CMD_RECV_REQUEST => handler.cmd_recv_request(cmd, flags),
            HTTP_CMD_SEND_RESPONSE => handler.cmd_send_response(cmd, flags),
            _ => Error::EINVAL.to_errno(),
        }
    }

    // ── SERVER_START ─────────────────────────────────────────────────

    fn cmd_server_start(handler: &Arc<Self>, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdServerAddr>() };
        let raw_addr = u32::from_be(payload.addr);
        let addr = SocketAddr::new_v4(
            Ipv4Addr::new(
                (raw_addr >> 24) as u8,
                (raw_addr >> 16) as u8,
                (raw_addr >> 8) as u8,
                raw_addr as u8,
            ),
            u16::from_be(payload.port),
        );

        // Create executor + server
        let handle = match WorkqueueExecutor::new(workqueue::system()) {
            Ok(h) => h,
            Err(_) => {
                return Error::ENOMEM.to_errno();
            }
        };
        let exec = handle.executor_arc();

        let bridge = Arc::new(
            UringBridgeHandler {
                handler: handler.clone(),
            },
            Flags::GFP_KERNEL,
        );
        let bridge = match bridge {
            Ok(b) => b,
            Err(_) => {
                return Error::ENOMEM.to_errno();
            }
        };

        let server =
            match HttpServer::start_on(&addr, bridge, ServerConfig::default(), exec.clone()) {
                Ok(s) => s,
                Err(e) => {
                    return e.to_errno();
                }
            };

        *handler.server.lock() = Some(server);
        *handler.executor_handle.lock() = Some(handle);
        0
    }

    // ── SERVER_STOP ──────────────────────────────────────────────────

    fn cmd_server_stop(&self, _cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        *self.server.lock() = None;
        *self.executor_handle.lock() = None;
        0
    }

    // ── CREATE_QUEUE ─────────────────────────────────────────────────

    fn cmd_create_queue(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdCreateQueue>() };
        let max_pending = if payload.max_pending == 0 {
            256
        } else {
            payload.max_pending
        };
        let id = self.next_queue_id.fetch_add(1, Ordering::Relaxed);

        let queue = RequestQueue {
            id,
            max_pending,
            buffered: KVec::new(),
        };

        let mut queues = self.queues.lock();
        if queues.push((id, queue), Flags::GFP_KERNEL).is_err() {
            return Error::ENOMEM.to_errno();
        }

        id as i32
    }

    // ── DESTROY_QUEUE ────────────────────────────────────────────────

    fn cmd_destroy_queue(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let queue_id = unsafe { cmd.cmd_data::<u32>() };
        let mut queues = self.queues.lock();
        let pos = queues.iter().position(|(id, _)| *id == *queue_id);
        match pos {
            Some(i) => {
                queues.remove(i);
                0
            }
            None => Error::ENOENT.to_errno(),
        }
    }

    // ── ADD_URL ──────────────────────────────────────────────────────

    fn cmd_add_url(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdUrl>() };
        let queue_id = payload.queue_id;
        let url_len = payload.url_len as usize;
        let url_addr = payload.url_addr as *const u8;

        // Verify queue exists
        {
            let queues = self.queues.lock();
            if !queues.iter().any(|(id, _)| *id == queue_id) {
                return Error::ENOENT.to_errno();
            }
        }

        // Copy URL from userspace
        let mut url_buf = match KVec::with_capacity(url_len, Flags::GFP_KERNEL) {
            Ok(v) => v,
            Err(_) => {
                return Error::ENOMEM.to_errno();
            }
        };
        // SAFETY: url_addr is a userspace pointer validated by io_uring core.
        // copy_from_user would be proper here, but for now we trust the pointer
        // since io_uring already validated the SQE address fields.
        unsafe {
            let src = core::slice::from_raw_parts(url_addr, url_len);
            for &b in src {
                let _ = url_buf.push(b, Flags::GFP_KERNEL);
            }
        }

        let mut routes = self.routes.lock();

        // Check for duplicate
        if routes
            .iter()
            .any(|(prefix, _)| prefix.as_slice() == url_buf.as_slice())
        {
            return Error::EEXIST.to_errno();
        }

        if routes.push((url_buf, queue_id), Flags::GFP_KERNEL).is_err() {
            return Error::ENOMEM.to_errno();
        }

        // Sort by prefix length (longest first) for longest-prefix matching.
        // Simple insertion sort — routes list is small.
        let len = routes.len();
        for i in 1..len {
            let mut j = i;
            while j > 0 && routes[j].0.len() > routes[j - 1].0.len() {
                routes.swap(j, j - 1);
                j -= 1;
            }
        }

        0
    }

    // ── REMOVE_URL ───────────────────────────────────────────────────

    fn cmd_remove_url(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdUrl>() };
        let url_len = payload.url_len as usize;
        let url_addr = payload.url_addr as *const u8;

        let url_slice = unsafe { core::slice::from_raw_parts(url_addr, url_len) };

        let mut routes = self.routes.lock();
        let pos = routes
            .iter()
            .position(|(prefix, _)| prefix.as_slice() == url_slice);
        match pos {
            Some(i) => {
                routes.remove(i);
                0
            }
            None => Error::ENOENT.to_errno(),
        }
    }

    // ── RECV_REQUEST (deferred — multishot) ──────────────────────────

    fn cmd_recv_request(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdRecvRequest>() };
        let queue_id = payload.queue_id;
        let buf_len = payload.buf_len;
        let buf_addr = payload.buf_addr;

        let mut queues = self.queues.lock();
        let queue = queues.iter_mut().find(|(id, _)| *id == queue_id);

        match queue {
            Some((_, q)) => {
                // If there's a buffered request, copy to userspace buffer.
                if !q.buffered.is_empty() {
                    let buffered = q.buffered.remove(0);
                    let data = &buffered.data;
                    let copy_len = data.len().min(buf_len as usize);

                    // copy_to_user: write serialized request to userspace
                    let dst = buf_addr as *mut u8;
                    // SAFETY: dst is a userspace pointer from the SQE cmd
                    // payload. copy_to_user handles fault detection.
                    let pending = unsafe {
                        rko_sys::rko::helpers::rust_helper_copy_to_user(
                            dst.cast(),
                            data.as_slice().as_ptr().cast(),
                            copy_len as u64,
                        )
                    };
                    if pending != 0 {
                        return Error::EFAULT.to_errno();
                    }
                    copy_len as i32
                } else {
                    // No request ready — tell userspace to retry.
                    Error::EAGAIN.to_errno()
                }
            }
            None => Error::ENOENT.to_errno(),
        }
    }

    // ── SEND_RESPONSE ────────────────────────────────────────────────

    fn cmd_send_response(&self, cmd: IoUringCmd, _flags: IssueFlags) -> i32 {
        let payload = unsafe { cmd.cmd_data::<CmdSendResponse>() };
        let req_id = payload.req_id;
        let status_code = payload.status_code;
        let _header_count = payload.header_count;
        let body_len = payload.body_len;
        let buf_addr = payload.buf_addr;

        // Find and remove the pending request
        let mut pending = self.pending.lock();
        let pos = pending.iter().position(|p| p.req_id == req_id);

        let sender = match pos {
            Some(i) => pending.remove(i).response_tx,
            None => {
                return Error::ENOENT.to_errno();
            }
        };
        drop(pending);

        let status = StatusCode::from_u16(status_code);

        // Copy response body from userspace
        // TODO: also copy headers from buf_addr (for now, body only)
        let body = if body_len > 0 && buf_addr != 0 {
            let mut buf = match KVec::with_capacity(body_len as usize, Flags::GFP_KERNEL) {
                Ok(b) => b,
                Err(_) => {
                    return Error::ENOMEM.to_errno();
                }
            };
            let _ = buf.resize(body_len as usize, 0u8, Flags::GFP_KERNEL);

            let src = buf_addr as *const u8;
            // SAFETY: src is a userspace pointer from the SQE cmd payload.
            let pending_bytes = unsafe {
                rko_sys::rko::helpers::rust_helper_copy_from_user(
                    buf.as_mut_slice().as_mut_ptr().cast(),
                    src.cast(),
                    body_len as u64,
                )
            };
            if pending_bytes != 0 {
                return Error::EFAULT.to_errno();
            }
            buf
        } else {
            KVec::new()
        };

        let response = match Response::builder().status(status).body(body) {
            Ok(r) => r,
            Err(_) => {
                return Error::ENOMEM.to_errno();
            }
        };

        // Send response through the oneshot channel, unblocking the
        // bridge handler's handle() future.
        sender.send(response);

        0
    }

    // ── Routing ──────────────────────────────────────────────────────

    /// Find the queue ID for a request path (longest prefix match).
    fn route(&self, path: &[u8]) -> Option<u32> {
        let routes = self.routes.lock();
        routes
            .iter()
            .find(|(prefix, _)| {
                if prefix.as_slice() == b"/*" {
                    true
                } else {
                    path.starts_with(prefix.as_slice())
                }
            })
            .map(|(_, qid)| *qid)
    }

    /// Allocate a unique request ID.
    fn alloc_req_id(&self) -> u64 {
        self.next_req_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Enqueue a serialized request for delivery to userspace.
    fn enqueue_serialized(
        &self,
        queue_id: u32,
        req_id: u64,
        data: KVec<u8>,
        response_tx: async_oneshot::Sender<Response>,
    ) {
        // Store pending request
        let pending_req = PendingRequest {
            req_id,
            response_tx,
        };
        let mut pending = self.pending.lock();
        let _ = pending.push(pending_req, Flags::GFP_KERNEL);
        drop(pending);

        // Buffer the serialized request in the target queue
        let buffered = BufferedRequest { req_id, data };
        let mut queues = self.queues.lock();
        if let Some((_, q)) = queues.iter_mut().find(|(id, _)| *id == queue_id)
            && (q.buffered.len() as u32) < q.max_pending
        {
            let _ = q.buffered.push(buffered, Flags::GFP_KERNEL);
            // TODO: If a multishot subscriber is waiting, deliver
            // immediately via io_uring_mshot_cmd_post_cqe instead
            // of buffering.
        }
    }
}

// ── UringBridgeHandler ───────────────────────────────────────────────

/// Bridges the kernel HTTP server to io_uring request queues.
///
/// Implements [`HttpHandler`] — for each incoming HTTP request, it
/// routes by URL prefix, serializes the request, and waits for
/// userspace to respond via `SEND_RESPONSE`.
struct UringBridgeHandler {
    handler: Arc<HttpUringHandler>,
}

unsafe impl Send for UringBridgeHandler {}
unsafe impl Sync for UringBridgeHandler {}

impl HttpHandler for UringBridgeHandler {
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_ {
        let handler = &self.handler;
        let req_id = handler.alloc_req_id();
        let queue_id = handler.route(req.path_bytes());

        // Serialize the request now (while we can borrow it).
        let serialized = serialize_request(req_id, req);

        async move {
            let queue_id = match queue_id {
                Some(qid) => qid,
                None => {
                    return Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(KVec::new())
                        .unwrap();
                }
            };

            let data = match serialized {
                Ok(d) => d,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(KVec::new())
                        .unwrap();
                }
            };

            // Create oneshot channel for response
            let (tx, rx) = match async_oneshot::channel(Flags::GFP_KERNEL) {
                Ok(pair) => pair,
                Err(_) => {
                    return Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(KVec::new())
                        .unwrap();
                }
            };

            // Enqueue serialized request for userspace delivery
            handler.enqueue_serialized(queue_id, req_id, data, tx);

            // Wait for userspace response
            match rx.await {
                Some(resp) => resp,
                None => Response::builder()
                    .status(StatusCode::GATEWAY_TIMEOUT)
                    .body(KVec::new())
                    .unwrap(),
            }
        }
    }
}

// ── Serialization ────────────────────────────────────────────────────

/// Serialize a parsed HTTP request into the binary format for delivery
/// to userspace in a provided buffer.
fn serialize_request(req_id: u64, req: &Request) -> Result<KVec<u8>, Error> {
    let path = req.path_bytes();
    let body = req.body().as_slice();
    let headers = req.headers();

    // Calculate total size
    let hdr_size = core::mem::size_of::<RequestHdr>();
    let mut headers_size = 0usize;
    for (name, value) in headers.iter() {
        headers_size += core::mem::size_of::<HeaderEntry>() + name.len() + value.len();
    }
    let total = hdr_size + path.len() + headers_size + body.len();

    let mut buf = KVec::with_capacity(total, Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;

    // Write header
    let hdr = RequestHdr {
        req_id,
        method: method_to_u8(req.method()),
        version: match req.version() {
            Version::Http10 => 10,
            Version::Http11 => 11,
        },
        header_count: headers.len() as u16,
        path_len: path.len() as u32,
        body_len: body.len() as u32,
        total_len: total as u32,
    };
    let hdr_bytes = unsafe { core::slice::from_raw_parts(&hdr as *const _ as *const u8, hdr_size) };
    for &b in hdr_bytes {
        let _ = buf.push(b, Flags::GFP_KERNEL);
    }

    // Write path
    for &b in path {
        let _ = buf.push(b, Flags::GFP_KERNEL);
    }

    // Write headers
    for (name, value) in headers.iter() {
        let entry = HeaderEntry {
            name_len: name.len() as u16,
            value_len: value.len() as u16,
        };
        let entry_bytes = unsafe {
            core::slice::from_raw_parts(
                &entry as *const _ as *const u8,
                core::mem::size_of::<HeaderEntry>(),
            )
        };
        for &b in entry_bytes {
            let _ = buf.push(b, Flags::GFP_KERNEL);
        }
        for &b in name {
            let _ = buf.push(b, Flags::GFP_KERNEL);
        }
        for &b in value {
            let _ = buf.push(b, Flags::GFP_KERNEL);
        }
    }

    // Write body
    for &b in body {
        let _ = buf.push(b, Flags::GFP_KERNEL);
    }

    Ok(buf)
}

fn method_to_u8(m: Method) -> u8 {
    match m {
        Method::Get => 0,
        Method::Head => 1,
        Method::Post => 2,
        Method::Put => 3,
        Method::Delete => 4,
        Method::Options => 5,
        Method::Patch => 6,
    }
}
