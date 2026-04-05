// SPDX-License-Identifier: GPL-2.0

//! HTTP version enum.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Http10,
    Http11,
}

impl Version {
    /// Parse from httparse version number (0 = HTTP/1.0, 1 = HTTP/1.1).
    pub fn from_httparse(v: u8) -> Self {
        match v {
            0 => Self::Http10,
            _ => Self::Http11,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Http10 => "HTTP/1.0",
            Self::Http11 => "HTTP/1.1",
        }
    }
}
