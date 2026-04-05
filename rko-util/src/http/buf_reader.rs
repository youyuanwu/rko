// SPDX-License-Identifier: GPL-2.0

//! Buffered reader over an async TcpStream.

use rko_core::alloc::{AllocError, Flags, KVec};
use rko_core::error::Error;
use rko_core::kasync::net::TcpStream;

/// Buffered reader that accumulates data from a TcpStream.
///
/// Used by both server (parse_request) and client (parse_response)
/// to handle partial HTTP reads.
pub struct BufReader {
    buf: KVec<u8>,
    pos: usize,
    len: usize,
}

impl BufReader {
    pub fn new(capacity: usize) -> Result<Self, AllocError> {
        let mut buf = KVec::with_capacity(capacity, Flags::GFP_KERNEL)?;
        buf.resize(capacity, 0, Flags::GFP_KERNEL)?;
        Ok(Self {
            buf,
            pos: 0,
            len: 0,
        })
    }

    /// Read more data from the stream into the buffer.
    pub async fn fill(&mut self, stream: &TcpStream) -> Result<usize, Error> {
        // Compact: move unconsumed data to front
        if self.pos > 0 {
            let remaining = self.len - self.pos;
            // SAFETY: src and dst may overlap; use copy_within via slice
            self.buf.as_mut_slice()[..].copy_within(self.pos..self.len, 0);
            self.len = remaining;
            self.pos = 0;
        }

        if self.len >= self.buf.len() {
            return Err(Error::E2BIG);
        }

        let n = stream
            .read(&mut self.buf.as_mut_slice()[self.len..])
            .await?;
        if n == 0 {
            return Err(Error::ECONNRESET);
        }
        self.len += n;
        Ok(n)
    }

    /// Current unconsumed data.
    pub fn data(&self) -> &[u8] {
        &self.buf.as_slice()[self.pos..self.len]
    }

    /// Mark `n` bytes as consumed.
    pub fn consume(&mut self, n: usize) {
        self.pos += n;
    }

    /// Reset for next request on a keep-alive connection.
    pub fn reset(&mut self) {
        // Compact any leftover data (pipelined request start)
        if self.pos < self.len {
            let remaining = self.len - self.pos;
            self.buf.as_mut_slice()[..].copy_within(self.pos..self.len, 0);
            self.len = remaining;
        } else {
            self.len = 0;
        }
        self.pos = 0;
    }
}
