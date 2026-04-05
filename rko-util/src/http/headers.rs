// SPDX-License-Identifier: GPL-2.0

//! HTTP header storage with case-insensitive name lookup.

use rko_core::alloc::{AllocError, Flags, KVec};

use super::header;

/// Header storage — KVec with case-insensitive lookup.
///
/// Backed by a flat list. Linear scan is fine for typical HTTP
/// header counts (<30).
pub struct Headers(KVec<(KVec<u8>, KVec<u8>)>);

impl Default for Headers {
    fn default() -> Self {
        Self::new()
    }
}

impl Headers {
    pub fn new() -> Self {
        Self(KVec::new())
    }

    /// Get the first value for a header name (case-insensitive).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        let name_bytes = name.as_bytes();
        for (n, v) in self.0.as_slice() {
            if header::eq_ignore_ascii_case(n, name_bytes) {
                return Some(v);
            }
        }
        None
    }

    /// Insert a header (appends; does not replace existing).
    pub fn insert(&mut self, name: &[u8], value: &[u8]) -> Result<(), AllocError> {
        let n = KVec::from_slice(name, Flags::GFP_KERNEL)?;
        let v = KVec::from_slice(value, Flags::GFP_KERNEL)?;
        self.0.push((n, v), Flags::GFP_KERNEL)
    }

    /// Iterate over all headers as (name, value) byte slices.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.0
            .as_slice()
            .iter()
            .map(|(n, v)| (n.as_slice(), v.as_slice()))
    }

    /// Number of headers.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True if no headers.
    pub fn is_empty(&self) -> bool {
        self.0.len() == 0
    }

    /// Parse Content-Length value from headers.
    pub fn content_length(&self) -> Option<usize> {
        let val = self.get(header::CONTENT_LENGTH)?;
        core::str::from_utf8(val).ok()?.parse().ok()
    }

    /// True if "Connection: close" is set (case-insensitive value).
    pub fn is_connection_close(&self) -> bool {
        self.get(header::CONNECTION)
            .map(|v| header::eq_ignore_ascii_case(v, b"close"))
            .unwrap_or(false)
    }

    /// True if "Connection: keep-alive" is set.
    pub fn is_connection_keepalive(&self) -> bool {
        self.get(header::CONNECTION)
            .map(|v| header::eq_ignore_ascii_case(v, b"keep-alive"))
            .unwrap_or(false)
    }
}
