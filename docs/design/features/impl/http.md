# Feature: In-Kernel HTTP/1.1 Library (Server + Client)

**Status**: ✅ Implemented — 23 unit tests + 3 integration tests passing in QEMU.

See `rko-util/src/http/`, `samples/kunit_tests/tests/http.rs`,
`samples/kunit_tests/tests/http_integration.rs`.

## Goal

HTTP/1.1 server and client in the `rko-util` crate, built on
rko-core's `kasync::net::TcpStream`. Parses requests/responses
with `httparse` (no_std, zero-alloc, MIT-licensed). Uses `KVec`/`KBox`
for kernel allocation.

## Crate Structure

```
rko-util/
├── Cargo.toml          — rko-core + httparse (default-features = false)
└── src/
    ├── lib.rs           — #![no_std], pub mod http
    └── http/
        ├── mod.rs       — pub exports
        ├── method.rs    — Method enum
        ├── status.rs    — StatusCode (typed wrapper, constants, reason())
        ├── version.rs   — Version enum (Http10, Http11)
        ├── header.rs    — header::CONTENT_TYPE etc., eq_ignore_ascii_case
        ├── headers.rs   — Headers (KVec-backed, case-insensitive get)
        ├── request.rs   — Request<B>, RequestBuilder
        ├── response.rs  — Response<B>, ResponseBuilder
        ├── buf_reader.rs — BufReader (async TcpStream buffered reads)
        ├── wire.rs      — write_request(), write_response(), format_host()
        ├── parse.rs     — parse_request(), parse_response(), read_body()
        ├── server.rs    — HttpHandler, HttpServer, ServerConfig
        ├── client.rs    — HttpClient::send(), send_on()
        └── error.rs     — HttpError with to_response() / to_error()
```

## API Overview

### Core types (http crate style)

```rust
// Unified types — same for server (parsed) and client (built)
Request<B = KVec<u8>>    // method, path, version, headers, body
Response<B = KVec<u8>>   // status, version, headers, body

// Builders
Request::builder().method(Method::Get).path("/api").header("Accept", b"*/*").body(KVec::new())?;
Response::builder().status(StatusCode::OK).header("Content-Type", b"text/plain").body(body)?;

// Convenience
Request::get("/path")?;
Request::post("/path", b"body", "application/json")?;

// Primitives
Method       — Get, Head, Post, Put, Delete, Options, Patch
StatusCode   — typed u16 with constants (OK, NOT_FOUND, ...) and reason()
Version      — Http10, Http11
Headers      — KVec-backed, case-insensitive get, content_length(), is_connection_close()
header::*    — CONTENT_TYPE, HOST, CONNECTION, etc.
```

### Server

```rust
// Handler trait — struct impl or closure
trait HttpHandler: Send + Sync + 'static {
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_;
}

// Start server (creates its own executor)
let server = HttpServer::start(&addr, handler, ServerConfig::default())?;

// Start on caller's executor (for tests / shared workqueue)
let server = HttpServer::start_on(&addr, handler, config, executor)?;
```

Server handles keep-alive, Content-Length, error responses (400/413/431/500),
and connection close automatically.

### Client

```rust
// One-shot (opens TCP connection, sends, reads, closes)
let resp = HttpClient::send(&addr, Request::get("/api")?).await?;

// Connection reuse
let resp = HttpClient::send_on(&stream, req).await?;
```

### Low-level (public)

```rust
// For tests or custom protocols
parse_request(&mut reader, &stream, &config).await?;
parse_response(&mut reader, &stream, max_hdr, max_body).await?;
write_request(&stream, &req, host).await?;
write_response(&stream, &resp).await?;
BufReader::new(capacity)?;
```

## User API: Example Module

```rust
struct MyApi;

impl HttpHandler for MyApi {
    fn handle(&self, req: &Request) -> impl Future<Output = Response> + Send + '_ {
        let method = req.method();
        let path = KVec::from_slice(req.path_bytes(), Flags::GFP_KERNEL)
            .unwrap_or_else(|_| KVec::new());
        async move {
            match (method, path.as_slice()) {
                (Method::Get, b"/") => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, b"text/plain")
                    .body(KVec::from_slice(b"hello\n", Flags::GFP_KERNEL).unwrap())
                    .unwrap(),
                _ => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(KVec::new()).unwrap(),
            }
        }
    }
}

// In Module::init():
let addr = SocketAddr::new_v4(Ipv4Addr::ANY, 8080);
let handler = Arc::new(MyApi, Flags::GFP_KERNEL)?;
let server = HttpServer::start(&addr, handler, ServerConfig::default())?;
```

## Design Decisions

### Unified `Request<B>` / `Response<B>`

Same type for server (parsed) and client (built), following the `http`
crate. Headers always owned (`KVec`). Zero-copy borrowing from parse
buffer deliberately abandoned — copying ~2 KB headers adds ~200 ns,
negligible vs network I/O.

### `httparse` for parsing, hand-written serialization

`httparse` (MIT, no_std, fuzz-tested) handles request and response
parsing edge cases. Serialization is trivial (status line + headers +
body) — no crate needed.

### `HttpServer::start()` vs `start_on()`

`start()` creates its own `WorkqueueExecutor` — simple for production.
`start_on()` takes a caller-provided executor — required for tests on
single-CPU QEMU where `block_on` + separate executor deadlocks the
system workqueue.

### No `tower::Service`

`&self` + RPITIT is simpler, naturally concurrent, no `poll_ready`
needed (workqueue handles backpressure), no `Box<dyn Future>`. Blanket
`impl HttpHandler for Fn` provides closure ergonomics.

### `Namespace::init_ns()` used internally

Server and client use root network namespace by default. Not exposed
in the API — eliminates a parameter most users never change.

### No TLS, no chunked encoding

Plain HTTP only. `Content-Length` always set (body fully buffered).
Both are future work items.

## Test Coverage

| Test suite | Count | Codepaths |
|-----------|-------|-----------|
| `http_tests` (unit) | 23 | Method, StatusCode, Version, Headers, Request/Response builders, header utils, KVec::from_slice |
| `http_integration_tests` | 3 | `http_server_requests` — server parse + respond (GET 200, POST echo, 404, 204); `http_client_get/post` — write_request + parse_response round-trip |

**Not tested** (single-CPU limitation): `HttpClient::send()` (async
connect), `HttpClient::send_on()`, `HttpServer::start()`. These
require multi-CPU or a different test harness.

## Known Limitations

- **Single-CPU deadlock**: `HttpClient::send()` uses async connect
  which deadlocks in `block_on` on 1-CPU QEMU. Tests use sync connect
  + async write/read as workaround.
- **No chunked Transfer-Encoding** — body must have Content-Length
- **No request/response streaming** — full body buffered in KVec
- **No timeouts** — keep-alive idle timeout and request read timeout
  are future work

## Future Work

- **Chunked Transfer-Encoding** for both request and response bodies
- **Streaming responses** via async iterator + chunked encoding
- **Client connection pool** — idle connections keyed by (addr, port)
- **Client retry with backoff** on connection errors / 5xx
- **Timeouts** — keep-alive idle, request read, client total
- **WebSocket upgrade** (101 Switching Protocols)
- **Routing** — path-based dispatch with parameter extraction
  (`rko_util::http::router`)
- **JSON** — `serde_json` no_std + alloc (`rko_util::json`)
- **Compression** — gzip/deflate via kernel `crypto_comp` API

## References

- `httparse`: [GitHub](https://github.com/seanmonstar/httparse),
  [docs.rs](https://docs.rs/httparse), MIT license
- rko async TCP: `rko-core/src/kasync/net/mod.rs`
- rko executor: `rko-core/src/kasync/executor/workqueue.rs`
- rko alloc: `rko-core/src/alloc/` (KVec, KBox, Flags)
- Async echo sample: `samples/async_echo/async_echo.rs`
- Networking design: `docs/design/features/networking.md`
- Test framework: `docs/design/features/test-framework.md`
