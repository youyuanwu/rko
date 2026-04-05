// SPDX-License-Identifier: GPL-2.0

//! HTTP client — send requests, receive responses.

use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;
use rko_core::kasync::net::TcpStream;
use rko_core::net::{Namespace, SocketAddr};

use super::buf_reader::BufReader;
use super::parse::parse_response;
use super::request::Request;
use super::response::Response;
use super::wire::{format_host, write_request};

/// HTTP/1.1 client for making requests from kernel modules.
pub struct HttpClient;

impl HttpClient {
    /// Send a request and get a response. Opens a new TCP connection.
    ///
    /// Uses `Namespace::init_ns()` (root network namespace) internally.
    pub async fn send(addr: &SocketAddr, req: Request) -> Result<Response, Error> {
        let ns = Namespace::init_ns();
        let stream = TcpStream::connect(ns, addr).await?;
        Self::send_on_inner(&stream, &req, Some(addr)).await
    }

    /// Send a request over an existing TCP stream (connection reuse).
    ///
    /// The caller manages the stream lifecycle. The `Host` header
    /// must already be set in the request (or will be omitted).
    pub async fn send_on(stream: &TcpStream, req: Request) -> Result<Response, Error> {
        Self::send_on_inner(stream, &req, None).await
    }

    async fn send_on_inner(
        stream: &TcpStream,
        req: &Request,
        addr: Option<&SocketAddr>,
    ) -> Result<Response, Error> {
        // Format Host header from address if provided
        let host_buf = if let Some(addr) = addr {
            let mut buf = KVec::with_capacity(32, Flags::GFP_KERNEL)?;
            format_host(addr, &mut buf)?;
            buf
        } else {
            // Use Host from request headers, or empty
            match req.headers().get("Host") {
                Some(h) => KVec::from_slice(h, Flags::GFP_KERNEL)?,
                None => KVec::new(),
            }
        };

        write_request(stream, req, &host_buf)
            .await
            .map_err(|_| Error::EIO)?;

        let mut reader = BufReader::new(8192).map_err(|_| Error::ENOMEM)?;
        parse_response(&mut reader, stream, 8192, 1024 * 1024)
            .await
            .map_err(|e| e.to_error())
    }
}
