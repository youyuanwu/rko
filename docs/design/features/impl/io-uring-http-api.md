# Feature: io_uring HTTP API — Userspace Control of Kernel HTTP Server

**Status**: ✅ Implemented — full round-trip verified in QEMU.

See: `rko-util/src/http/uring.rs`, `samples/http_uring/`,
`rko-test-uring/`, `rko-core/src/io_uring.rs`, `rko-core/src/miscdevice.rs`.

**Depends on**: [io_uring_cmd](io-uring-cmd.md), [HTTP/1.1 library](http.md)

## Goal

Expose the in-kernel HTTP server (`rko_util::http`) to userspace via
`IORING_OP_URING_CMD` on a misc device (`/dev/rko_http`). The kernel
handles TCP, HTTP parsing, keep-alive, and connection management.
Userspace receives parsed requests and sends responses through io_uring.

Linux equivalent of Windows HTTP.sys / HTTPApi.

## Architecture

```
Userspace (io-uring crate)
  │  IORING_OP_URING_CMD + cmd[] payload
  ▼
io_uring core → MiscDevice::uring_cmd → HttpUringHandler::dispatch
  │
  ▼
Kernel HTTP Server (rko_util::http)
  TCP accept → parse → route by URL → enqueue → copy_to_user
  ← copy_from_user response ← write TCP
```

## Command Protocol

All commands use `IORING_OP_URING_CMD` on `/dev/rko_http` fd.
Payloads in `sqe->cmd[]` (80 bytes with SQE128).

| Opcode | Command | Payload | CQE result |
|--------|---------|---------|------------|
| 0 | `SERVER_START` | addr(4) + port(2) | 0 or -errno |
| 1 | `SERVER_STOP` | — | 0 |
| 2 | `CREATE_QUEUE` | max_pending(4) | queue_id (>0) |
| 3 | `DESTROY_QUEUE` | queue_id(4) | 0 or -ENOENT |
| 4 | `ADD_URL` | queue_id(4) + url_len(4) + url_addr(8) | 0 or -EEXIST |
| 5 | `REMOVE_URL` | queue_id(4) + url_len(4) + url_addr(8) | 0 or -ENOENT |
| 6 | `RECV_REQUEST` | queue_id(4) + buf_len(4) + buf_addr(8) | bytes or -EAGAIN |
| 7 | `SEND_RESPONSE` | req_id(8) + status(2) + hdr_count(2) + body_len(4) + buf_addr(8) | 0 or -ENOENT |

### Request buffer layout (kernel → userspace at buf_addr)

```
[RequestHdr: req_id(8) method(1) version(1) hdr_count(2) path_len(4) body_len(4) total_len(4)]
[path bytes]
[HeaderEntry: name_len(2) value_len(2) name value] × hdr_count
[body bytes]
```

### Response buffer layout (userspace → kernel at buf_addr)

```
[body bytes]  (header deserialization is future work)
```

## Key Design Decisions

### Synchronous uring_cmd return (no `cmd.done()`)

Return result directly from `uring_cmd`. Do NOT call
`io_uring_cmd_done()` — that's only for async deferred completion.
Calling it for sync causes double cleanup → NULL deref crash.

### Buffer pointers in cmd[] payload

`RECV_REQUEST` and `SEND_RESPONSE` carry buffer pointers in the
80-byte cmd[] payload (not sqe->addr). Compatible with any io_uring
library including kimojio.

### One-shot request/response (no streaming)

Full request buffered before delivery. Full response in one SQE.
Matches the HTTP library's Content-Length-only design.

### Kernel-side URL prefix routing

Longest-prefix match at parse time. Userspace registers prefixes
once via `ADD_URL`. No round-trip per request for routing.

## Implementation

### Kernel module (`samples/http_uring/`)

```rust
#[vtable]
impl MiscDevice for HttpDeviceState {
    type Ptr = Arc<Self>;
    fn open(...) -> Result<Arc<Self>, Error> { /* alloc handler */ }
    fn uring_cmd(device, cmd, flags) -> i32 {
        HttpUringHandler::dispatch(&device.handler, cmd, flags)
    }
}
```

### Handler (`rko-util/src/http/uring.rs`)

- `HttpUringHandler` — queues, routes, pending requests (all behind `SimpleMutex`)
- `UringBridgeHandler` — implements `HttpHandler`, serializes requests,
  bridges to async oneshot channel for response delivery
- `serialize_request()` — writes `RequestHdr` + path + headers + body into flat buffer

### Userspace test (`rko-test-uring/`)

Static musl binary using `io-uring` crate (v0.7). Verified flow:

1. Open `/dev/rko_http`
2. CREATE_QUEUE → queue_id=1
3. ADD_URL `/*`
4. RECV_REQUEST → EAGAIN (no requests)
5. SERVER_START :8080
6. TCP `GET /hello` → RECV_REQUEST → 66 bytes, method=GET, path=/hello
7. SEND_RESPONSE (200 + body) → TCP receives `HTTP/1.1 200 OK` + body
8. SERVER_STOP

## Primitives Built

| Component | Location | Lines |
|-----------|----------|-------|
| `IoUringCmd` / `IssueFlags` | `rko-core/src/io_uring.rs` | ~150 |
| `MiscDeviceRegistration` / `MiscDevice` | `rko-core/src/miscdevice.rs` | ~200 |
| `ForeignOwnable` for `Arc<T>` | `rko-core/src/sync/arc.rs` | ~30 |
| Oneshot (sync + async) | `rko-core/src/sync/oneshot.rs`, `kasync/oneshot.rs` | ~300 |
| `KVec::remove` / `swap_remove` | `rko-core/src/alloc/kvec.rs` | ~30 |
| `Queue::enqueue_delayed` | `rko-core/src/workqueue.rs` | ~15 |
| `HttpUringHandler` | `rko-util/src/http/uring.rs` | ~500 |

## Future Work

- Multishot RECV_REQUEST with provided buffer ring
- Response header deserialization from userspace buffer
- Request timeout (30s → auto 504 via delayed_work)
- Kernel TLS (kTLS) for HTTPS
- Response caching (serve without waking userspace)

## References

- [io_uring custom commands](io-uring-cmd.md)
- [HTTP/1.1 library](http.md)
- [kimojio](https://github.com/Azure/kimojio-rs) — async io_uring runtime with `uring_cmd`
- Windows HTTP.sys: [Microsoft Docs](https://learn.microsoft.com/en-us/windows/win32/http/http-server-api-start-page)
