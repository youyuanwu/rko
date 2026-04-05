// SPDX-License-Identifier: GPL-2.0

//! Internal error type for HTTP parsing and handling.

use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;

use super::header;
use super::headers::Headers;
use super::response::Response;
use super::status::StatusCode;
use super::version::Version;

/// Errors during HTTP parsing/handling.
#[derive(Debug)]
pub enum HttpError {
    /// Malformed request line or headers → 400
    BadRequest,
    /// Headers exceed max_header_size → 431
    HeaderTooLarge,
    /// Body exceeds max_body_size → 413
    PayloadTooLarge,
    /// Client disconnected mid-request
    ConnectionClosed,
    /// Allocation or internal failure → 500
    Internal,
}

impl HttpError {
    /// Convert to an error Response for the server to send.
    pub fn to_response(&self) -> Response {
        let (status, body_bytes) = match self {
            Self::BadRequest => (StatusCode::BAD_REQUEST, &b"400 Bad Request\n"[..]),
            Self::HeaderTooLarge => (
                StatusCode::HEADER_TOO_LARGE,
                &b"431 Request Header Fields Too Large\n"[..],
            ),
            Self::PayloadTooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                &b"413 Payload Too Large\n"[..],
            ),
            Self::ConnectionClosed | Self::Internal => (
                StatusCode::INTERNAL_SERVER_ERROR,
                &b"500 Internal Server Error\n"[..],
            ),
        };

        let body = KVec::from_slice(body_bytes, Flags::GFP_KERNEL).unwrap_or_else(|_| KVec::new());
        let mut headers = Headers::new();
        let _ = headers.insert(header::CONTENT_TYPE.as_bytes(), b"text/plain");
        let _ = headers.insert(header::CONNECTION.as_bytes(), b"close");

        Response {
            status,
            version: Version::Http11,
            headers,
            body,
        }
    }

    /// Convert to rko Error for client callers.
    pub fn to_error(&self) -> Error {
        match self {
            Self::BadRequest | Self::HeaderTooLarge => Error::EINVAL,
            Self::PayloadTooLarge => Error::E2BIG,
            Self::ConnectionClosed => Error::ECONNRESET,
            Self::Internal => Error::ENOMEM,
        }
    }
}
