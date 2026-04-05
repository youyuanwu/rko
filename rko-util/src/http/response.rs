// SPDX-License-Identifier: GPL-2.0

//! HTTP response type and builder.

use rko_core::alloc::{AllocError, KVec};

use super::headers::Headers;
use super::status::StatusCode;
use super::version::Version;

/// HTTP response — used for both server (built) and client (parsed).
pub struct Response<B = KVec<u8>> {
    pub(crate) status: StatusCode,
    pub(crate) version: Version,
    pub(crate) headers: Headers,
    pub(crate) body: B,
}

impl<B> Response<B> {
    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn version(&self) -> Version {
        self.version
    }

    pub fn headers(&self) -> &Headers {
        &self.headers
    }

    pub fn body(&self) -> &B {
        &self.body
    }

    pub fn into_body(self) -> B {
        self.body
    }

    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

impl Response {
    /// Start building a response.
    pub fn builder() -> ResponseBuilder {
        ResponseBuilder::new()
    }
}

/// Builder for constructing HTTP responses.
pub struct ResponseBuilder {
    status: StatusCode,
    version: Version,
    headers: Headers,
    error: Option<AllocError>,
}

impl ResponseBuilder {
    fn new() -> Self {
        Self {
            status: StatusCode::OK,
            version: Version::Http11,
            headers: Headers::new(),
            error: None,
        }
    }

    pub fn status(mut self, s: StatusCode) -> Self {
        self.status = s;
        self
    }

    pub fn version(mut self, v: Version) -> Self {
        self.version = v;
        self
    }

    pub fn header(mut self, name: &str, value: &[u8]) -> Self {
        if let Err(e) = self.headers.insert(name.as_bytes(), value) {
            self.error = Some(e);
        }
        self
    }

    /// Finalize with a body. Consumes the builder.
    pub fn body(self, body: KVec<u8>) -> Result<Response, AllocError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        Ok(Response {
            status: self.status,
            version: self.version,
            headers: self.headers,
            body,
        })
    }
}
