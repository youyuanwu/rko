# Feature: io_uring HTTP API — Userspace Control of Kernel HTTP Server

**Status**: ✅ Implemented — all commands work, userspace e2e test passes
in QEMU. TCP round-trip pending (needs loopback timing fix).

See: `rko-util/src/http/uring.rs`, `samples/http_uring/`,
`rko-core/src/io_uring.rs`, `rko-core/src/miscdevice.rs`,
`rko-test-uring/` (userspace test binary).

**Depends on**: [io_uring_cmd](io-uring-cmd.md) (✅ implemented),
[HTTP/1.1 library](../impl/http.md) (✅ implemented)

## Goal

Expose the in-kernel HTTP server (`rko_util::http`) to userspace
applications via `IORING_OP_URING_CMD` custom commands on a misc
device (`/dev/rko_http`). Userspace receives parsed HTTP requests and
sends responses through io_uring — the kernel handles TCP, HTTP
parsing, connection keep-alive, and error responses internally.

This is the Linux equivalent of the Windows HTTP.sys / HTTPApi model:

| Windows HTTP.sys | rko io_uring HTTP API |
|-----------------|----------------------|
| `HttpInitialize` | `open("/dev/rko_http")` + io_uring ring setup |
| `HttpCreateRequestQueue` | `HTTP_CMD_CREATE_QUEUE` |
| `HttpAddUrl` | `HTTP_CMD_ADD_URL` |
| `HttpReceiveHttpRequest` | `HTTP_CMD_RECV_REQUEST` (multishot) |
| `HttpSendHttpResponse` | `HTTP_CMD_SEND_RESPONSE` |
| `HttpCloseRequestQueue` | `HTTP_CMD_DESTROY_QUEUE` / close fd |

## Why

The existing `HttpServer` runs entirely in-kernel: the handler trait
is implemented by the kernel module itself. This works for kernel-only
services, but many use cases want **userspace logic** with
**kernel-mode HTTP performance**:

- Web applications that benefit from kernel TCP efficiency and
  zero-syscall HTTP parsing, but need userspace flexibility
- API gateways that route parsed requests to userspace handlers
- Microservices using io_uring's batched async model for high
  concurrency

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│                      Userspace                           │
│                                                          │
│  io_uring ring (IORING_SETUP_SQE128)                     │
│  ┌─────────────────────────────────────────────────────┐ │
│  │ Provided buffer ring (IORING_REGISTER_PBUF_RING)    │ │
│  │ ┌─────┐ ┌─────┐ ┌─────┐ ┌─────┐                    │ │
│  │ │buf 0│ │buf 1│ │buf 2│ │buf 3│ ...                 │ │
│  │ └─────┘ └─────┘ └─────┘ └─────┘                    │ │
│  └─────────────────────────────────────────────────────┘ │
│                                                          │
│  ┌─ SQE ───────────────┐  ┌─ SQE ────────────────────┐  │
│  │ RECV_REQUEST         │  │ SEND_RESPONSE            │  │
│  │ (multishot)          │  │ req_id=42, status=200    │  │
│  │ buf_group=0          │  │ addr → response buffer   │  │
│  └──────────┬──────────┘  └──────────┬───────────────┘  │
└─────────────┼────────────────────────┼───────────────────┘
              │ io_uring_enter()       │ io_uring_enter()
              ▼                        ▼
┌──────────────────────────────────────────────────────────┐
│           io_uring core (kernel)                         │
│  file->f_op->uring_cmd(io_uring_cmd, issue_flags)        │
└──────────────────────┬───────────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────────┐
│       rko HTTP module (.ko) — /dev/rko_http              │
│                                                          │
│  URL Router: "/api/*" → queue 1, "/admin/*" → queue 2    │
│                                                          │
│  HTTP Server (rko_util::http)                            │
│    TCP accept → parse full request → route → enqueue     │
│    → post CQE → wait for response SQE → write TCP        │
│                                                          │
│  Pending Requests: req_id → (connection, response_tx)    │
└──────────────────────────────────────────────────────────┘
```

Requests and responses are always complete (full headers + full body).
No streaming, no chunked transfer. The kernel buffers the entire
request before delivering to userspace, and userspace provides the
entire response in a single SQE.

## Command Protocol

All commands use `IORING_OP_URING_CMD` targeting the `/dev/rko_http`
fd. Payloads are in `sqe->cmd[]` (up to 80 bytes with SQE128).

### Opcodes

```c
enum rko_http_cmd_op {
    HTTP_CMD_SERVER_START    = 0,  // Start listening
    HTTP_CMD_SERVER_STOP     = 1,  // Stop listening
    HTTP_CMD_CREATE_QUEUE    = 2,  // Create request queue
    HTTP_CMD_DESTROY_QUEUE   = 3,  // Destroy request queue
    HTTP_CMD_ADD_URL         = 4,  // Register URL prefix
    HTTP_CMD_REMOVE_URL      = 5,  // Unregister URL prefix
    HTTP_CMD_RECV_REQUEST    = 6,  // Receive parsed request (multishot)
    HTTP_CMD_SEND_RESPONSE   = 7,  // Send complete response
};
```

### Data Structures (UAPI)

```c
// --- Command payloads (in sqe->cmd[]) ---

struct rko_http_server_addr {
    __u32 addr;             // IPv4 (network byte order), 0 = INADDR_ANY
    __u16 port;             // Port (network byte order)
    __u16 reserved;
};

struct rko_http_create_queue {
    __u32 max_pending;      // Max queued requests (0 = default 256)
    __u32 reserved;
};

struct rko_http_url {
    __u32 queue_id;
    __u32 url_len;
    __u64 url_addr;         // Userspace pointer to URL prefix string
};

struct rko_http_recv_request {
    __u32 queue_id;
    __u32 buf_len;          // Size of userspace buffer
    __u64 buf_addr;         // Userspace buffer pointer (kernel writes here)
};

struct rko_http_send_response {
    __u64 req_id;           // From received request header
    __u16 status_code;      // HTTP status (200, 404, etc.)
    __u16 header_count;     // Number of headers in response buffer
    __u32 body_len;         // Response body length
    __u64 buf_addr;         // Userspace buffer pointer (headers + body)
};

// --- Request layout (kernel → userspace, in provided buffer) ---

struct rko_http_request_hdr {
    __u64 req_id;           // Unique request ID
    __u8  method;           // enum rko_http_method
    __u8  version;          // 10 = HTTP/1.0, 11 = HTTP/1.1
    __u16 header_count;
    __u32 path_len;
    __u32 body_len;
    __u32 total_len;        // Total bytes in buffer
    // Layout after this header:
    //   path bytes (path_len)
    //   header_entry[0..header_count]  (each: u16 name_len, u16 value_len, name, value)
    //   body bytes (body_len)
};

struct rko_http_header_entry {
    __u16 name_len;
    __u16 value_len;
    // followed by name bytes then value bytes
};

enum rko_http_method {
    RKO_HTTP_GET = 0, RKO_HTTP_HEAD = 1, RKO_HTTP_POST = 2,
    RKO_HTTP_PUT = 3, RKO_HTTP_DELETE = 4, RKO_HTTP_OPTIONS = 5,
    RKO_HTTP_PATCH = 6,
};

// --- Response buffer layout (userspace → kernel, at sqe->addr) ---
//   header_entry[0..header_count]
//   body bytes (body_len)
// header_count and body_len from rko_http_send_response
```

### Command Summary

| Command | SQE payload | CQE result | Notes |
|---------|------------|------------|-------|
| `SERVER_START` | `server_addr` | 0 or -errno | Starts TCP listener |
| `SERVER_STOP` | — | 0 | Graceful shutdown |
| `CREATE_QUEUE` | `create_queue` | queue_id (>0) | Returns queue ID |
| `DESTROY_QUEUE` | queue_id in cmd[0..4] | 0 | Sends 503 to pending |
| `ADD_URL` | `url` | 0 or -EEXIST | Longest-prefix routing |
| `REMOVE_URL` | `url` | 0 or -ENOENT | |
| `RECV_REQUEST` | `recv_request` (buf_addr+len) | total_bytes or -EAGAIN | Single-shot, copy_to_user |
| `SEND_RESPONSE` | `send_response` (buf_addr) | 0 or -ENOENT | copy_from_user |

### `RECV_REQUEST` — Single-Shot Request Delivery

Userspace provides a buffer pointer and length in the `cmd[]` payload.
If a request is queued, the kernel copies the serialized request into
the userspace buffer via `copy_to_user` and completes with the byte
count. If no request is available, completes with `-EAGAIN`.

```
SQE: cmd_op=6, cmd[]=rko_http_recv_request { queue_id, buf_len, buf_addr }
CQE: res=total_bytes (>0) or -EAGAIN (no request ready) or -ENOENT (bad queue)
```

Buffer layout at `buf_addr` on success:
```
[rko_http_request_hdr][path bytes][header entries...][body bytes]
```

Userspace polls by resubmitting `RECV_REQUEST` until a request arrives.
Future optimization: multishot mode with provided buffer rings
(eliminates resubmission overhead).

### `SEND_RESPONSE` — Single-Shot Response

Userspace provides the complete response (headers + body) at `buf_addr`
in the `cmd[]` payload. The kernel copies it via `copy_from_user`,
builds a `Response`, and sends it on the TCP connection.

```
SQE: cmd_op=7, cmd[]=rko_http_send_response { req_id, status, header_count, body_len, buf_addr }
CQE: res=0 or -ENOENT (req_id expired/unknown)
```

Buffer layout at `buf_addr`:
```
[header_entry 0][name][value][header_entry 1][name][value]...[body bytes]
```

## Kernel Implementation

### Bridge Handler

The kernel HTTP server uses a custom `HttpHandler` that bridges
parsed requests to io_uring queues:

```rust
struct UringBridgeHandler {
    handler: Arc<HttpUringHandler>,
}

impl HttpHandler for UringBridgeHandler {
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_ {
        async move {
            let req_id = self.handler.alloc_req_id();
            let queue_id = self.handler.route(req.path_bytes());
            let (tx, rx) = oneshot::channel();
            self.handler.enqueue(queue_id, req_id, req, tx);
            // Block until userspace responds or timeout
            rx.await.unwrap_or_else(|_| gateway_timeout())
        }
    }
}
```

### Internal State

```rust
pub struct HttpUringHandler {
    queues: Mutex<KVec<(u32, RequestQueue)>>,       // linear scan, < 256 queues
    routes: SpinLock<KVec<(KVec<u8>, u32)>>,       // sorted longest-first
    pending: Mutex<KVec<(u64, ResponseSender)>>,    // linear scan, < 10K pending
    next_req_id: AtomicU64,
    next_queue_id: AtomicU32,
}

struct RequestQueue {
    id: u32,
    subscribers: KVec<IoUringCmdAsync>,   // multishot waiters
    buffered: KVec<BufferedRequest>,       // requests waiting for subscriber
}
```

When a request arrives and a subscriber is waiting, the kernel
serializes the request into a provided buffer and posts a CQE
immediately. If no subscriber is waiting, the request is buffered
(up to `max_pending`).

When `SEND_RESPONSE` arrives, the kernel looks up the pending
request by `req_id`, deserializes the response buffer, and sends
the `Response` through the oneshot channel to unblock the
`HttpHandler::handle()` future.

## Userspace Example (C)

```c
int fd = open("/dev/rko_http", O_RDWR);
struct io_uring ring;
io_uring_queue_init_params(256, &ring,
    &(struct io_uring_params){ .flags = IORING_SETUP_SQE128 });

// Register 64 × 64KB provided buffers
struct io_uring_buf_ring *br = io_uring_setup_buf_ring(&ring, 64, 0, 0, &ret);
for (int i = 0; i < 64; i++)
    io_uring_buf_ring_add(br, malloc(65536), 65536, i, 63, i);
io_uring_buf_ring_advance(br, 64);

// Start server on :8080
submit_cmd(&ring, fd, HTTP_CMD_SERVER_START,
    &(struct rko_http_server_addr){ .port = htons(8080) });
wait_cqe(&ring);

// Create queue, add URL
int qid = submit_and_wait(&ring, fd, HTTP_CMD_CREATE_QUEUE,
    &(struct rko_http_create_queue){ .max_pending = 256 });
submit_and_wait(&ring, fd, HTTP_CMD_ADD_URL,
    &(struct rko_http_url){ .queue_id = qid, .url_len = 2,
                            .url_addr = (uintptr_t)"/*" });

// Subscribe (multishot)
struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
sqe->opcode = IORING_OP_URING_CMD;
sqe->fd = fd;
sqe->cmd_op = HTTP_CMD_RECV_REQUEST;
sqe->flags |= IOSQE_BUFFER_SELECT;
sqe->buf_group = 0;
memcpy(sqe->cmd, &(struct rko_http_recv_request){ .queue_id = qid },
       sizeof(struct rko_http_recv_request));
sqe->user_data = TAG_RECV;
io_uring_submit(&ring);

// Event loop
while (running) {
    struct io_uring_cqe *cqe;
    io_uring_wait_cqe(&ring, &cqe);

    if (cqe->user_data == TAG_RECV) {
        int buf_id = cqe->flags >> IORING_CQE_BUFFER_SHIFT;
        struct rko_http_request_hdr *hdr = buffers[buf_id];

        // Process request, build response into resp_buf
        char resp_buf[4096];
        int resp_len = build_json_response(resp_buf, hdr);

        // Send response
        sqe = io_uring_get_sqe(&ring);
        sqe->opcode = IORING_OP_URING_CMD;
        sqe->fd = fd;
        sqe->cmd_op = HTTP_CMD_SEND_RESPONSE;
        struct rko_http_send_response sr = {
            .req_id = hdr->req_id, .status_code = 200,
            .header_count = 1, .body_len = resp_len,
        };
        memcpy(sqe->cmd, &sr, sizeof(sr));
        sqe->addr = (uintptr_t)resp_buf;
        sqe->len = resp_len;
        io_uring_submit(&ring);

        // Return buffer to ring
        io_uring_buf_ring_add(br, buffers[buf_id], 65536, buf_id, 63, 0);
        io_uring_buf_ring_advance(br, 1);

        if (!(cqe->flags & IORING_CQE_F_MORE))
            resubmit_recv(&ring, fd, qid);
    }
    io_uring_cqe_seen(&ring, cqe);
}

close(fd);
```

## Design Decisions

### One-shot request and response (no streaming)

Requests are fully buffered in kernel before delivery. Responses are
fully provided by userspace in one SQE. This matches the existing
`rko_util::http` design (no chunked transfer, `Content-Length`
required). Simplifies the protocol to two commands for the hot path
(`RECV_REQUEST` + `SEND_RESPONSE`).

### Provided buffers for request delivery

Kernel picks a free buffer from a pre-registered pool per request.
No per-request buffer address needed in the SQE. Enables multishot:
one subscription SQE, kernel selects buffer and posts CQE per request.

### Multishot for request reception

One SQE produces many CQEs — no resubmission overhead per request.
`IORING_CQE_F_MORE` signals more coming; absence means resubmit.

### Misc device (`/dev/rko_http`)

Simpler than full character device — auto minor number, only needs
`open`, `release`, `uring_cmd` in `file_operations`.

### Request timeout (30s → automatic 504)

Prevents resource leaks if userspace crashes. The TCP connection and
kernel memory must be freed even without a response.

### Kernel-side URL prefix routing

Routes at parse time with zero userspace round-trips. Longest prefix
match. Userspace registers prefixes once via `ADD_URL`.

### Synchronous uring_cmd completion (no `cmd.done()`)

**Critical**: For synchronous command handling, the `uring_cmd`
callback must return the result value directly. Do NOT call
`io_uring_cmd_done()` — the kernel handles CQE posting and cleanup
after the callback returns. Calling `io_uring_cmd_done()` for sync
completion causes double cleanup → NULL pointer crash in
`io_req_uring_cleanup` (accessing freed `async_data`).

`io_uring_cmd_done()` / `IoUringCmdAsync::done()` is ONLY for
deferred async completion (return `-EIOCBQUEUED` from the callback,
complete later from another context).

### Buffer pointers in cmd[] payload (not sqe->addr)

`RECV_REQUEST` and `SEND_RESPONSE` carry buffer pointers inside the
80-byte `cmd[]` payload, not in `sqe->addr`. This is because
kimojio's `uring_cmd()` API only sets `cmd_op` and `cmd[0..80]` —
it doesn't expose `sqe->addr`. Encoding pointers in cmd[] is
compatible with any io_uring userspace library.

## Primitives Gap Analysis

All prerequisites are implemented. Summary of what was built:

### ✅ Implemented (this work)

| Primitive | Location | Lines |
|-----------|----------|-------|
| `rko_core::io_uring` | `rko-core/src/io_uring.rs` | ~170 |
| `rko_core::miscdevice` | `rko-core/src/miscdevice.rs` | ~220 |
| `ForeignOwnable` for `Arc<T>` | `rko-core/src/sync/arc.rs` | ~30 |
| Oneshot channel (sync) | `rko-core/src/sync/oneshot.rs` | ~150 |
| Oneshot channel (async) | `rko-core/src/kasync/oneshot.rs` | ~150 |
| `KVec::remove` / `swap_remove` | `rko-core/src/alloc/kvec.rs` | ~30 |
| `Queue::enqueue_delayed` | `rko-core/src/workqueue.rs` | ~15 |
| `HttpUringHandler` | `rko-util/src/http/uring.rs` | ~500 |
| `http_uring` sample module | `samples/http_uring/` | ~70 |

### ✅ Bindings added

| Partition | Types | Notes |
|-----------|-------|-------|
| `rko.io_uring` | `io_uring_cmd`, `io_uring_sqe`, + 2185 lines | 6 inject_types for cascade-cutting |
| `rko.misc` | `miscdevice`, `misc_register`, `misc_deregister`, `MISC_DYNAMIC_MINOR` | 1 inject_type (`attribute_group`) |

### ✅ C helpers added

| Helper | Wraps |
|--------|-------|
| `rust_helper_io_uring_cmd_done` | Inline `io_uring_cmd_done()` (only helper needed — all other fields accessed via generated struct bindings) |
| `rust_helper_queue_delayed_work` | Inline `queue_delayed_work()` |

### ✅ Kernel config

- `CONFIG_IO_URING=y` enabled in `CMakeLists.txt`
- All 8 io_uring symbols confirmed in Module.symvers

### ⚠️ Remaining work (future optimization)

| Item | Notes |
|------|-------|
| Provided buffer ring (multishot) | `io_uring_cmd_buffer_select` exported ✅ but single-shot `copy_to_user` used. Multishot eliminates resubmission overhead. |
| Response header deserialization | `SEND_RESPONSE` copies body ✅ but headers not yet parsed from buffer (only status code used). |
| Request timeout (30s → 504) | `queue_delayed_work` helper ready ✅ but not wired into pending request expiration. |
| TCP round-trip test | Commands work ✅ but TCP connect to server gets "Connection refused" — needs loopback timing or async test pattern. |

## Implementation Plan

### Phase 1: Prerequisites (rko-core) ✅ Done

1. ✅ `rko_core::io_uring` — partition, C helpers, safe API
2. ✅ `rko_core::miscdevice` — ported from upstream, with `uring_cmd`
3. ✅ `queue_delayed_work` C helper + `Queue::enqueue_delayed()`
4. ✅ Oneshot channel — sync (`rko_core::sync::oneshot`) + async
   (`rko_core::kasync::oneshot`)
5. ✅ `KVec::remove()` / `swap_remove()` (replaces VecDeque)
6. ✅ `ForeignOwnable` for `Arc<T>`
7. ✅ `CONFIG_IO_URING=y` + all symbols verified

### Phase 2: Device + Queue Management ✅ Done

1. ✅ `rko-util/src/http/uring.rs` — `HttpUringHandler` with all
   8 commands dispatched
2. ✅ `samples/http_uring/` — kernel module, `/dev/rko_http`,
   tested insmod/rmmod in QEMU

### Phase 3: Request/Response Round-Trip ✅ Partially Done

1. ✅ `RECV_REQUEST` copies serialized request to userspace via `copy_to_user`
2. ✅ `SEND_RESPONSE` copies response body from userspace via `copy_from_user`
3. ✅ Userspace test binary (`rko-test-uring/`, io-uring crate, musl static)
4. ✅ All 8 commands verified in QEMU e2e test
5. ⚠️ TCP round-trip not yet working (server starts but TCP connect refused — timing issue)
6. TODO: Wire request timeout (30s → 504)
7. TODO: Multishot provided buffer ring (optimization)

### Userspace Test: rko-test-uring ✅ Done

Uses the `io-uring` crate (v0.7, same base as kimojio) for direct
`IORING_OP_URING_CMD` submission. Statically linked via musl for
QEMU initramfs.

**Crate**: `rko-test-uring/` in root workspace, binary `http_uring_test`.

**Build**: `cargo build -p rko-test-uring --release --target x86_64-unknown-linux-musl`

**Test sequence** (all verified ✅):
1. Open `/dev/rko_http`
2. `CREATE_QUEUE` → returns queue_id=1
3. `ADD_URL "/*"` → returns 0
4. `RECV_REQUEST` → returns -11 (EAGAIN, no requests buffered)
5. `SERVER_START :8080` → returns 0
6. TCP connect → Connection refused (timing — future fix)
7. `SERVER_STOP` → returns 0

**CMake integration**: `http_uring_test_bin` target builds musl binary,
copies to `samples/http_uring/build/test_bin/`, included in initramfs
by `run-module-test.sh`.

## Test Coverage

| Suite | Tests | What's covered |
|-------|-------|----------------|
| `oneshot_tests` (sync) | 6 | send→recv, sender dropped, timeout, KVec body |
| `async_oneshot_tests` | 3 | spawn+send→await, sender dropped, KVec body |
| `foreign_ownable_tests` | 4 | Arc round-trip, borrow, multiple borrows, unit |
| `kvec_tests` (new) | 5 | remove middle/first/last, swap_remove middle/last |
| `http_uring` module | 3 | insmod, all commands via userspace test, rmmod |
| `http_uring_test` (userspace) | 7 | open, CREATE_QUEUE, ADD_URL, RECV_REQUEST, SERVER_START, SERVER_STOP, close |

Total new tests: **28**. All pass in QEMU.

## Miscdevice Porting Plan

Port from the upstream kernel Rust crate
([`rust/kernel/miscdevice.rs`](https://github.com/torvalds/linux/blob/master/rust/kernel/miscdevice.rs),
~310 lines). We port a **minimal subset** — only `open` + `release` +
`uring_cmd` — skipping ioctl/mmap/read_iter/write_iter/show_fdinfo to
minimize dependencies.

### Upstream Source

| File | Description |
|------|-------------|
| `rust/kernel/miscdevice.rs` | `MiscDeviceRegistration<T>`, `MiscDevice` trait, `MiscdeviceVTable<T>` |
| `samples/rust/rust_misc_device.rs` | Sample: ioctl-based misc device with per-fd mutex state |

### What We Port (Minimal)

```rust
// rko-core/src/miscdevice.rs

/// Options for creating a misc device.
pub struct MiscDeviceOptions {
    pub name: &'static CStr,
}

/// A registered miscdevice. Deregisters on drop.
pub struct MiscDeviceRegistration<T> { /* Opaque<miscdevice> */ }

/// Trait for misc device private data.
#[vtable]
pub trait MiscDevice: Sized {
    /// Pointer wrapper for per-fd private data.
    type Ptr: ForeignOwnable + Send + Sync;

    /// Called on open. Return per-fd state.
    fn open(file: &File, misc: &MiscDeviceRegistration<Self>) -> Result<Self::Ptr>;

    /// Called on release.
    fn release(device: Self::Ptr, file: &File) { drop(device); }

    /// Handle io_uring custom command. (rko addition — not in upstream)
    fn uring_cmd(
        device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        cmd: IoUringCmd,
        flags: IssueFlags,
    ) -> Result<(), Error> {
        cmd.done(Error::EOPNOTSUPP.to_errno(), flags);
        Ok(())
    }
}
```

### What We Skip

| Upstream feature | Why skip |
|-----------------|----------|
| `ioctl` / `compat_ioctl` | Not needed for io_uring HTTP API |
| `mmap` | Not needed |
| `read_iter` / `write_iter` | Not needed — all I/O via uring_cmd |
| `show_fdinfo` | Debug feature, add later |
| `Device` type | Requires device model bindings; use raw pointer initially |
| `Kiocb`, `IovIter*`, `VmaNew`, `SeqFile` | Dependencies of skipped features |

### New Dependencies to Port

#### 1. `ForeignOwnable` trait (~80 lines)

Upstream: `rust/kernel/types.rs`. Converts between Rust owned types
and raw `*mut c_void` for storing in `file->private_data`.

```rust
/// Types that can be stored as `void *` in C structures.
pub trait ForeignOwnable: Sized {
    /// The borrowed form of this type.
    type Borrowed<'a>;

    /// Convert to a raw pointer (transfers ownership to C).
    fn into_foreign(self) -> *mut core::ffi::c_void;

    /// Borrow from a raw pointer (does NOT take ownership).
    unsafe fn borrow<'a>(ptr: *const core::ffi::c_void) -> Self::Borrowed<'a>;

    /// Reconstruct from a raw pointer (takes back ownership).
    unsafe fn from_foreign(ptr: *mut core::ffi::c_void) -> Self;
}

// Impl for KBox<T>:
// into_foreign → KBox::into_raw().cast()
// borrow       → &*ptr.cast::<T>()
// from_foreign → KBox::from_raw(ptr.cast())

// Impl for Pin<KBox<T>>:
// Same as KBox but wraps in Pin

// Impl for Arc<T>:
// into_foreign → Arc::into_raw().cast()
// borrow       → ArcBorrow from raw
// from_foreign → Arc::from_raw(ptr.cast())
```

#### 2. `fs::File` wrapper (thin, ~30 lines)

Minimal wrapper for `struct file *` — just enough to pass to
`open`/`release`. Not the filesystem `File<T>` in `rko-core/src/fs/`.

```rust
/// Wrapper for `struct file` from file_operations callbacks.
pub struct File {
    ptr: *mut bindings::file,
}

impl File {
    pub(crate) unsafe fn from_raw_file(ptr: *mut bindings::file) -> &File {
        &*(ptr as *const File)
    }
}
```

#### 3. `struct miscdevice` binding

Either inject_type in rko-sys-gen or add a new `rko.misc` partition:

```toml
# Option A: inject_type (minimal, avoids cascade)
[[inject_type]]
name = "rko.misc.miscdevice"
size = 112     # sizeof(struct miscdevice) — verify
align = 8

# Option B: new partition (cleaner, gets MISC_DYNAMIC_MINOR constant)
[[partition]]
namespace = "rko.misc"
library = "kernel"
headers = ["linux/miscdevice.h"]
traverse = ["linux/miscdevice.h"]
```

#### 4. C helpers

```c
// helpers.h
#include <linux/miscdevice.h>
int rust_helper_misc_register(struct miscdevice *misc);
void rust_helper_misc_deregister(struct miscdevice *misc);

// helpers.c
int rust_helper_misc_register(struct miscdevice *misc)
{
    return misc_register(misc);
}

void rust_helper_misc_deregister(struct miscdevice *misc)
{
    misc_deregister(misc);
}
```

### Vtable Construction

The key difference from upstream: we add `uring_cmd` to the
`file_operations` vtable. The upstream `MiscdeviceVTable` builds
`file_operations` with conditional fields; we follow the same pattern:

```rust
const VTABLE: bindings::file_operations = bindings::file_operations {
    open: Some(Self::open),
    release: Some(Self::release),
    // rko addition — not in upstream miscdevice.rs
    uring_cmd: if T::HAS_URING_CMD {
        Some(Self::uring_cmd_trampoline)
    } else {
        core::ptr::null_mut()  // NULL = not supported
    },
    ..zeroed()
};

unsafe extern "C" fn uring_cmd_trampoline(
    cmd: *mut bindings::io_uring_cmd,
    issue_flags: u32,
) -> i32 {
    // Recover per-fd private data from cmd->file->private_data
    let file = (*cmd).file;
    let private = (*file).private_data;
    let device = <T::Ptr as ForeignOwnable>::borrow(private);
    let wrapper = IoUringCmd::from_raw(cmd);
    let flags = IssueFlags(issue_flags);
    match T::uring_cmd(device, wrapper, flags) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}
```

### Porting Effort Summary

| Item | Lines | Depends on |
|------|-------|-----------|
| `ForeignOwnable` trait + impls | ~80 | Nothing |
| `fs::File` thin wrapper | ~30 | `rko-sys` bindings (already have `struct file`) |
| `struct miscdevice` binding | ~10 | rko-sys-gen inject or partition |
| C helpers (`misc_register`/`deregister`) | ~15 | helpers.{c,h} |
| `MiscDeviceRegistration<T>` | ~60 | `ForeignOwnable`, bindings |
| `MiscDevice` trait + vtable | ~100 | `ForeignOwnable`, `IoUringCmd`, `File` |
| **Total** | **~300** | |

### Sample Usage (io_uring HTTP device)

```rust
use rko_core::miscdevice::{MiscDevice, MiscDeviceOptions, MiscDeviceRegistration};
use rko_core::io_uring::{IoUringCmd, IssueFlags};

struct HttpDeviceState {
    handler: Arc<HttpUringHandler>,
}

#[rko_core::vtable]
impl MiscDevice for HttpDeviceState {
    type Ptr = Arc<Self>;

    fn open(_file: &File, _misc: &MiscDeviceRegistration<Self>) -> Result<Arc<Self>> {
        Arc::new(HttpDeviceState {
            handler: Arc::new(HttpUringHandler::new()?, Flags::GFP_KERNEL)?,
        }, Flags::GFP_KERNEL)
    }

    fn uring_cmd(
        device: ArcBorrow<'_, Self>,
        cmd: IoUringCmd,
        flags: IssueFlags,
    ) -> Result<(), Error> {
        match cmd.cmd_op() {
            HTTP_CMD_SERVER_START => device.handler.server_start(cmd, flags),
            HTTP_CMD_CREATE_QUEUE => device.handler.create_queue(cmd, flags),
            HTTP_CMD_RECV_REQUEST => device.handler.recv_request(cmd, flags),
            HTTP_CMD_SEND_RESPONSE => device.handler.send_response(cmd, flags),
            // ...
            _ => { cmd.done(-libc::EINVAL, flags); Ok(()) }
        }
    }
}
```

## Open Questions

1. **Per-fd vs shared server** — should multiple openers share one
   TCP listener, or does each fd get its own?

2. **Capability gating** — should `open()` require `CAP_NET_BIND_SERVICE`
   for ports < 1024?

**Resolved**:
- ✅ `CONFIG_IO_URING` — enabled in `CMakeLists.txt`, kernel rebuilt.
  8 io_uring symbols now in `Module.symvers`.
- ✅ `io_uring_mshot_cmd_post_cqe` — confirmed exported (`EXPORT_SYMBOL_GPL`)
- ✅ Provided buffer ring access — `io_uring_cmd_buffer_select` is
  exported (`EXPORT_SYMBOL_GPL`). Custom `uring_cmd` handlers CAN
  pick from the provided buffer ring.
- ✅ `misc_register` / `misc_deregister` — confirmed in `Module.symvers`
- ✅ `rko.io_uring` partition — generated successfully. Traverses
  `cmd.h` + `io_uring_types.h` + `uapi/io_uring.h`. 6 opaque
  inject_types cut the cascade. `io_uring_cmd` struct with correct
  fields (`file`, `sqe`, `cmd_op`, `flags`, `pdu`). 2185 lines.
- ✅ Trait signature — http-api's `MiscDevice::uring_cmd` takes
  per-fd `device` state, io-uring-cmd's `Operations::uring_cmd` is
  static. No conflict — the miscdevice trampoline bridges them by
  recovering device from `file->private_data`.

### Exported io_uring Symbols

All `EXPORT_SYMBOL_GPL` from `vmlinux`:

| Symbol | Usage |
|--------|-------|
| `__io_uring_cmd_done` | Completion (sync and async) |
| `io_uring_mshot_cmd_post_cqe` | Multishot CQE for RECV_REQUEST |
| `io_uring_cmd_buffer_select` | Pick from provided buffer ring |
| `io_uring_cmd_mark_cancelable` | Cancelation support |
| `io_uring_cmd_import_fixed` | Fixed buffer zero-copy (future) |
| `io_uring_cmd_import_fixed_vec` | Vectored fixed buffer (future) |
| `__io_uring_cmd_do_in_task` | Task-context completion |
| `io_uring_cmd_sock` | Socket helper |

## Future Work

- Kernel TLS (kTLS) for HTTPS
- Response caching in kernel (serve without waking userspace)
- Sendfile for static file serving
- Multi-process queue sharing
- eBPF request filtering

## References

- [io_uring custom commands design](io-uring-cmd.md)
- [HTTP/1.1 library implementation](../impl/http.md)
- kimojio: [GitHub](https://github.com/Azure/kimojio-rs), [docs.rs](https://docs.rs/kimojio) — userspace async io_uring runtime with `uring_cmd` support
- Windows HTTP.sys: [Microsoft Docs](https://learn.microsoft.com/en-us/windows/win32/http/http-server-api-start-page)
- FUSE over io_uring: [LWN](https://lwn.net/Articles/1066182/)
- io_uring provided buffers: [man page](https://man.archlinux.org/man/io_uring_provided_buffers.7.en)
- io_uring multishot: [man page](https://man.archlinux.org/man/io_uring_multishot.7.en)
