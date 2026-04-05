// SPDX-License-Identifier: GPL-2.0

//! HTTP request/response parsing (reading from stream).

use rko_core::alloc::{Flags, KVec};
use rko_core::kasync::net::TcpStream;

use super::buf_reader::BufReader;
use super::error::HttpError;
use super::headers::Headers;
use super::method::Method;
use super::request::Request;
use super::response::Response;
use super::server::ServerConfig;
use super::status::StatusCode;
use super::version::Version;

/// Parse an HTTP request from a buffered stream (server-side).
pub async fn parse_request(
    reader: &mut BufReader,
    stream: &TcpStream,
    config: &ServerConfig,
) -> Result<Request, HttpError> {
    loop {
        reader
            .fill(stream)
            .await
            .map_err(|_| HttpError::ConnectionClosed)?;

        let mut raw_headers = [httparse::EMPTY_HEADER; 64];
        let mut req = httparse::Request::new(&mut raw_headers);

        match req.parse(reader.data()) {
            Ok(httparse::Status::Complete(body_offset)) => {
                let method =
                    Method::parse(req.method.unwrap_or("")).ok_or(HttpError::BadRequest)?;
                let path_vec =
                    KVec::from_slice(req.path.unwrap_or("/").as_bytes(), Flags::GFP_KERNEL)
                        .map_err(|_| HttpError::Internal)?;
                let version = Version::from_httparse(req.version.unwrap_or(1));

                let mut headers = Headers::new();
                for h in req.headers.iter() {
                    headers
                        .insert(h.name.as_bytes(), h.value)
                        .map_err(|_| HttpError::Internal)?;
                }

                let content_length = headers.content_length().unwrap_or(0);
                if content_length > config.max_body_size {
                    return Err(HttpError::PayloadTooLarge);
                }

                // Done borrowing reader.data() — now we can mutate
                reader.consume(body_offset);
                let body = read_body(reader, stream, content_length, config.max_body_size).await?;

                return Ok(Request {
                    method,
                    path: path_vec,
                    version,
                    headers,
                    body,
                });
            }
            Ok(httparse::Status::Partial) => {
                if reader.data().len() >= config.max_header_size {
                    return Err(HttpError::HeaderTooLarge);
                }
                continue;
            }
            Err(_) => return Err(HttpError::BadRequest),
        }
    }
}

/// Parse an HTTP response from a buffered stream (client-side).
pub async fn parse_response(
    reader: &mut BufReader,
    stream: &TcpStream,
    max_header_size: usize,
    max_body_size: usize,
) -> Result<Response, HttpError> {
    loop {
        reader
            .fill(stream)
            .await
            .map_err(|_| HttpError::ConnectionClosed)?;

        let mut raw_headers = [httparse::EMPTY_HEADER; 64];
        let mut resp = httparse::Response::new(&mut raw_headers);

        match resp.parse(reader.data()) {
            Ok(httparse::Status::Complete(body_offset)) => {
                let status = StatusCode::from_u16(resp.code.unwrap_or(0));
                let version = Version::from_httparse(resp.version.unwrap_or(1));

                let mut headers = Headers::new();
                for h in resp.headers.iter() {
                    headers
                        .insert(h.name.as_bytes(), h.value)
                        .map_err(|_| HttpError::Internal)?;
                }

                let content_length = headers.content_length().unwrap_or(0);

                // Done borrowing reader.data() — now we can mutate
                reader.consume(body_offset);
                let body = read_body(reader, stream, content_length, max_body_size).await?;

                return Ok(Response {
                    status,
                    version,
                    headers,
                    body,
                });
            }
            Ok(httparse::Status::Partial) => {
                if reader.data().len() >= max_header_size {
                    return Err(HttpError::HeaderTooLarge);
                }
                continue;
            }
            Err(_) => return Err(HttpError::BadRequest),
        }
    }
}

/// Read exactly `content_length` bytes of body from the stream.
async fn read_body(
    reader: &mut BufReader,
    stream: &TcpStream,
    content_length: usize,
    max_body_size: usize,
) -> Result<KVec<u8>, HttpError> {
    if content_length == 0 {
        return Ok(KVec::new());
    }
    if content_length > max_body_size {
        return Err(HttpError::PayloadTooLarge);
    }

    let mut body =
        KVec::with_capacity(content_length, Flags::GFP_KERNEL).map_err(|_| HttpError::Internal)?;

    // Use data already buffered in reader
    let buffered = reader.data();
    let from_buf = core::cmp::min(buffered.len(), content_length);
    body.extend_from_slice(&buffered[..from_buf], Flags::GFP_KERNEL)
        .map_err(|_| HttpError::Internal)?;
    reader.consume(from_buf);

    // Read remaining directly from stream
    let mut remaining = content_length - from_buf;
    while remaining > 0 {
        let cur_len = body.len();
        body.resize(cur_len + remaining, 0, Flags::GFP_KERNEL)
            .map_err(|_| HttpError::Internal)?;
        let n = stream
            .read(&mut body.as_mut_slice()[cur_len..])
            .await
            .map_err(|_| HttpError::ConnectionClosed)?;
        if n == 0 {
            return Err(HttpError::ConnectionClosed);
        }
        body.resize(cur_len + n, 0, Flags::GFP_KERNEL)
            .map_err(|_| HttpError::Internal)?;
        remaining -= n;
    }

    Ok(body)
}
