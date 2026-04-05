// SPDX-License-Identifier: GPL-2.0

//! Well-known HTTP header name constants.

pub const HOST: &str = "Host";
pub const CONTENT_TYPE: &str = "Content-Type";
pub const CONTENT_LENGTH: &str = "Content-Length";
pub const CONNECTION: &str = "Connection";
pub const TRANSFER_ENCODING: &str = "Transfer-Encoding";
pub const ACCEPT: &str = "Accept";
pub const USER_AGENT: &str = "User-Agent";
pub const AUTHORIZATION: &str = "Authorization";
pub const LOCATION: &str = "Location";

/// Case-insensitive ASCII comparison for header names.
pub fn eq_ignore_ascii_case(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(x, y)| x.eq_ignore_ascii_case(y))
}
