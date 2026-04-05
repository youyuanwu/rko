// SPDX-License-Identifier: GPL-2.0

//! Typed HTTP status code.

/// HTTP status code (mirrors http::StatusCode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCode(u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const NO_CONTENT: Self = Self(204);
    pub const BAD_REQUEST: Self = Self(400);
    pub const NOT_FOUND: Self = Self(404);
    pub const METHOD_NOT_ALLOWED: Self = Self(405);
    pub const PAYLOAD_TOO_LARGE: Self = Self(413);
    pub const HEADER_TOO_LARGE: Self = Self(431);
    pub const INTERNAL_SERVER_ERROR: Self = Self(500);
    pub const GATEWAY_TIMEOUT: Self = Self(504);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);

    pub const fn from_u16(code: u16) -> Self {
        Self(code)
    }

    pub const fn as_u16(&self) -> u16 {
        self.0
    }

    /// Standard reason phrase for common status codes.
    pub const fn reason(&self) -> &'static str {
        match self.0 {
            200 => "OK",
            201 => "Created",
            204 => "No Content",
            301 => "Moved Permanently",
            302 => "Found",
            304 => "Not Modified",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            413 => "Payload Too Large",
            431 => "Request Header Fields Too Large",
            500 => "Internal Server Error",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            _ => "Unknown",
        }
    }

    pub const fn is_success(&self) -> bool {
        self.0 >= 200 && self.0 < 300
    }

    pub const fn is_redirect(&self) -> bool {
        self.0 >= 300 && self.0 < 400
    }

    pub const fn is_client_error(&self) -> bool {
        self.0 >= 400 && self.0 < 500
    }

    pub const fn is_server_error(&self) -> bool {
        self.0 >= 500 && self.0 < 600
    }
}
