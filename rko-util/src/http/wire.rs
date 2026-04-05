// SPDX-License-Identifier: GPL-2.0

//! HTTP request/response serialization (writing to stream).

use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;
use rko_core::kasync::net::TcpStream;
use rko_core::net::SocketAddr;

use super::header;
use super::request::Request;
use super::response::Response;

/// Write an HTTP request to the stream.
pub async fn write_request(stream: &TcpStream, req: &Request, host: &[u8]) -> Result<(), Error> {
    let mut buf = KVec::<u8>::with_capacity(512, Flags::GFP_KERNEL)?;

    // Request line: "GET /path HTTP/1.1\r\n"
    extend(&mut buf, req.method().as_str().as_bytes())?;
    extend(&mut buf, b" ")?;
    extend(&mut buf, req.path_bytes())?;
    extend(&mut buf, b" ")?;
    extend(&mut buf, req.version().as_str().as_bytes())?;
    extend(&mut buf, b"\r\n")?;

    // Host header (mandatory in HTTP/1.1)
    write_header(&mut buf, header::HOST.as_bytes(), host)?;

    // Content-Length if body present
    if !req.body().is_empty() {
        write_header_usize(
            &mut buf,
            header::CONTENT_LENGTH.as_bytes(),
            req.body().len(),
        )?;
    }

    // User headers
    for (name, value) in req.headers().iter() {
        write_header(&mut buf, name, value)?;
    }

    extend(&mut buf, b"\r\n")?;

    stream.write_all(&buf).await?;
    if !req.body().is_empty() {
        stream.write_all(req.body()).await?;
    }
    Ok(())
}

/// Write an HTTP response to the stream.
pub async fn write_response(stream: &TcpStream, resp: &Response) -> Result<(), Error> {
    let mut buf = KVec::<u8>::with_capacity(512, Flags::GFP_KERNEL)?;

    // Status line: "HTTP/1.1 200 OK\r\n"
    extend(&mut buf, resp.version().as_str().as_bytes())?;
    extend(&mut buf, b" ")?;
    write_u16(&mut buf, resp.status().as_u16())?;
    extend(&mut buf, b" ")?;
    extend(&mut buf, resp.status().reason().as_bytes())?;
    extend(&mut buf, b"\r\n")?;

    // Content-Length (always set)
    write_header_usize(
        &mut buf,
        header::CONTENT_LENGTH.as_bytes(),
        resp.body().len(),
    )?;

    // User headers
    for (name, value) in resp.headers().iter() {
        write_header(&mut buf, name, value)?;
    }

    extend(&mut buf, b"\r\n")?;

    stream.write_all(&buf).await?;
    if !resp.body().is_empty() {
        stream.write_all(resp.body()).await?;
    }
    Ok(())
}

/// Format a SocketAddr as a Host header value.
pub fn format_host(addr: &SocketAddr, buf: &mut KVec<u8>) -> Result<(), Error> {
    match addr {
        SocketAddr::V4(v4) => {
            let octets = v4.ip.0;
            write_u8(buf, octets[0])?;
            extend(buf, b".")?;
            write_u8(buf, octets[1])?;
            extend(buf, b".")?;
            write_u8(buf, octets[2])?;
            extend(buf, b".")?;
            write_u8(buf, octets[3])?;
            if v4.port != 80 {
                extend(buf, b":")?;
                write_u16(buf, v4.port)?;
            }
        }
        SocketAddr::V6(_v6) => {
            extend(buf, b"[::1]")?;
        }
    }
    Ok(())
}

// --- Internal helpers ---

fn extend(buf: &mut KVec<u8>, data: &[u8]) -> Result<(), Error> {
    buf.extend_from_slice(data, Flags::GFP_KERNEL)
        .map_err(|_| Error::ENOMEM)
}

fn write_header(buf: &mut KVec<u8>, name: &[u8], value: &[u8]) -> Result<(), Error> {
    extend(buf, name)?;
    extend(buf, b": ")?;
    extend(buf, value)?;
    extend(buf, b"\r\n")
}

fn write_header_usize(buf: &mut KVec<u8>, name: &[u8], value: usize) -> Result<(), Error> {
    let mut num_buf = [0u8; 20];
    let s = format_usize(value, &mut num_buf);
    write_header(buf, name, s)
}

fn write_u16(buf: &mut KVec<u8>, val: u16) -> Result<(), Error> {
    let mut num_buf = [0u8; 5];
    let s = format_usize(val as usize, &mut num_buf);
    extend(buf, s)
}

fn write_u8(buf: &mut KVec<u8>, val: u8) -> Result<(), Error> {
    let mut num_buf = [0u8; 3];
    let s = format_usize(val as usize, &mut num_buf);
    extend(buf, s)
}

/// Format a usize into a byte buffer, returning the written slice.
/// No-alloc integer formatting for no_std.
fn format_usize(mut val: usize, buf: &mut [u8]) -> &[u8] {
    if val == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut pos = buf.len();
    while val > 0 && pos > 0 {
        pos -= 1;
        buf[pos] = b'0' + (val % 10) as u8;
        val /= 10;
    }
    &buf[pos..]
}
