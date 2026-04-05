use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;
use rko_core::kasync::executor::workqueue::WorkqueueExecutor;
use rko_core::net::{Ipv4Addr, SocketAddr};
use rko_core::sync::Arc;
use rko_core::workqueue;
use rko_util::http::{
    HttpHandler, HttpServer, Method, Request, Response, ServerConfig, StatusCode, header,
};

struct TestHandler;

impl HttpHandler for TestHandler {
    fn handle(&self, req: &Request) -> impl core::future::Future<Output = Response> + Send + '_ {
        let method = req.method();
        let path =
            KVec::from_slice(req.path_bytes(), Flags::GFP_KERNEL).unwrap_or_else(|_| KVec::new());
        let body_copy = KVec::from_slice(req.body().as_slice(), Flags::GFP_KERNEL)
            .unwrap_or_else(|_| KVec::new());
        async move {
            match (method, path.as_slice()) {
                (Method::Get, b"/hello") => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, b"text/plain")
                    .body(
                        KVec::from_slice(b"Hello from kernel HTTP!\n", Flags::GFP_KERNEL)
                            .unwrap_or_else(|_| KVec::new()),
                    )
                    .unwrap(),
                (Method::Post, b"/echo") => Response::builder()
                    .status(StatusCode::OK)
                    .header(header::CONTENT_TYPE, b"application/octet-stream")
                    .body(body_copy)
                    .unwrap(),
                (Method::Get, b"/status") => Response::builder()
                    .status(StatusCode::NO_CONTENT)
                    .body(KVec::new())
                    .unwrap(),
                _ => Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(
                        KVec::from_slice(b"404\n", Flags::GFP_KERNEL)
                            .unwrap_or_else(|_| KVec::new()),
                    )
                    .unwrap(),
            }
        }
    }
}

/// Send a raw HTTP request, return the raw response bytes.
fn http_request(
    exec: &Arc<WorkqueueExecutor>,
    addr: &SocketAddr,
    raw: &'static [u8],
) -> Result<KVec<u8>, Error> {
    let a = *addr;
    exec.block_on(async move {
        rko_core::kasync::yield_now().await;
        let ns = rko_core::net::Namespace::init_ns();
        let s = rko_core::net::TcpStream::connect(ns, &a)?;
        s.write_all(raw)?;
        let mut buf = [0u8; 4096];
        let n = s.read(&mut buf)?;
        KVec::from_slice(&buf[..n], Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)
    })?
}

#[rko_core::rko_tests]
pub mod http_integration_tests {
    use super::*;

    /// One server, multiple sequential requests via block_on.
    #[test]
    fn http_server_requests() -> Result<(), Error> {
        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19900);
        let handler = Arc::new(TestHandler, Flags::GFP_KERNEL)?;
        let _server = HttpServer::start_on(&addr, handler, ServerConfig::default(), exec.clone())?;

        // GET /hello → 200
        let resp = http_request(
            &exec,
            &addr,
            b"GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )?;
        let resp_str = core::str::from_utf8(&resp).unwrap_or("");
        assert!(resp_str.contains("200 OK"));
        assert!(resp_str.contains("Hello from kernel HTTP!"));

        // POST /echo → 200 with echoed body
        let resp = http_request(&exec, &addr,
            b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 12\r\nConnection: close\r\n\r\nkernel data!")?;
        let resp_str = core::str::from_utf8(&resp).unwrap_or("");
        assert!(resp_str.contains("200 OK"));
        assert!(resp_str.contains("kernel data!"));

        // GET /nope → 404
        let resp = http_request(
            &exec,
            &addr,
            b"GET /nope HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )?;
        let resp_str = core::str::from_utf8(&resp).unwrap_or("");
        assert!(resp_str.contains("404"));

        // GET /status → 204
        let resp = http_request(
            &exec,
            &addr,
            b"GET /status HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )?;
        let resp_str = core::str::from_utf8(&resp).unwrap_or("");
        assert!(resp_str.contains("204"));

        Ok(())
    }

    /// Test client codepath: write_request() → parse_response().
    /// Uses sync connect + async write/read (same as HttpClient::send
    /// internally, but avoids the async connect deadlock on 1-CPU).
    #[test]
    fn http_client_get() -> Result<(), Error> {
        use rko_util::http::{BufReader, parse_response, write_request};

        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19910);
        let handler = Arc::new(TestHandler, Flags::GFP_KERNEL)?;
        let _server = HttpServer::start_on(&addr, handler, ServerConfig::default(), exec.clone())?;

        let result = exec.block_on(async move {
            rko_core::kasync::yield_now().await;

            // Sync connect, wrap as async stream
            let ns = rko_core::net::Namespace::init_ns();
            let sync_s = rko_core::net::TcpStream::connect(ns, &addr)?;
            let stream = rko_core::kasync::net::TcpStream::new(sync_s);

            // Client write (tests write_request)
            let req = Request::get("/hello").map_err(|_| Error::ENOMEM)?;
            write_request(&stream, &req, b"127.0.0.1:19910").await?;

            // Client read (tests parse_response)
            let mut reader = BufReader::new(8192).map_err(|_| Error::ENOMEM)?;
            let resp = parse_response(&mut reader, &stream, 8192, 1024 * 1024)
                .await
                .map_err(|e| e.to_error())?;

            Ok::<(u16, KVec<u8>), Error>((resp.status().as_u16(), resp.into_body()))
        })?;

        let (status, body) = result?;
        assert_eq!(status, 200);
        assert_eq!(body.as_slice(), b"Hello from kernel HTTP!\n");
        Ok(())
    }

    /// Test client POST — write_request with body + parse_response.
    #[test]
    fn http_client_post() -> Result<(), Error> {
        use rko_util::http::{BufReader, parse_response, write_request};

        let handle = WorkqueueExecutor::new(workqueue::system())?;
        let exec = handle.executor_arc();
        let addr = SocketAddr::new_v4(Ipv4Addr::LOCALHOST, 19911);
        let handler = Arc::new(TestHandler, Flags::GFP_KERNEL)?;
        let _server = HttpServer::start_on(&addr, handler, ServerConfig::default(), exec.clone())?;

        let result = exec.block_on(async move {
            rko_core::kasync::yield_now().await;
            let ns = rko_core::net::Namespace::init_ns();
            let sync_s = rko_core::net::TcpStream::connect(ns, &addr)?;
            let stream = rko_core::kasync::net::TcpStream::new(sync_s);

            let req = Request::post("/echo", b"test payload", "application/octet-stream")
                .map_err(|_| Error::ENOMEM)?;
            write_request(&stream, &req, b"127.0.0.1:19911").await?;

            let mut reader = BufReader::new(8192).map_err(|_| Error::ENOMEM)?;
            let resp = parse_response(&mut reader, &stream, 8192, 1024 * 1024)
                .await
                .map_err(|e| e.to_error())?;

            Ok::<(u16, KVec<u8>), Error>((resp.status().as_u16(), resp.into_body()))
        })?;

        let (status, body) = result?;
        assert_eq!(status, 200);
        assert_eq!(body.as_slice(), b"test payload");
        Ok(())
    }
}
