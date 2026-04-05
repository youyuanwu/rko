// SPDX-License-Identifier: GPL-2.0

//! HTTP request type and builder.

use rko_core::alloc::{AllocError, Flags, KVec};

use super::header;
use super::headers::Headers;
use super::method::Method;
use super::version::Version;

/// HTTP request — used for both server (parsed) and client (built).
pub struct Request<B = KVec<u8>> {
    pub(crate) method: Method,
    pub(crate) path: KVec<u8>,
    pub(crate) version: Version,
    pub(crate) headers: Headers,
    pub(crate) body: B,
}

impl<B> Request<B> {
    pub fn method(&self) -> Method {
        self.method
    }

    pub fn path(&self) -> &str {
        core::str::from_utf8(&self.path).unwrap_or("/")
    }

    pub fn path_bytes(&self) -> &[u8] {
        &self.path
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
}

impl Request {
    /// Start building a request.
    pub fn builder() -> RequestBuilder {
        RequestBuilder::new()
    }

    /// Shorthand for a GET request.
    pub fn get(path: &str) -> Result<Self, AllocError> {
        Request::builder()
            .method(Method::Get)
            .path(path)
            .body(KVec::new())
    }

    /// Shorthand for a POST request with body.
    pub fn post(path: &str, body: &[u8], content_type: &str) -> Result<Self, AllocError> {
        Request::builder()
            .method(Method::Post)
            .path(path)
            .header(header::CONTENT_TYPE, content_type.as_bytes())
            .body(KVec::from_slice(body, Flags::GFP_KERNEL)?)
    }
}

/// Builder for constructing HTTP requests.
pub struct RequestBuilder {
    method: Method,
    path: Option<KVec<u8>>,
    version: Version,
    headers: Headers,
    error: Option<AllocError>,
}

impl RequestBuilder {
    fn new() -> Self {
        Self {
            method: Method::Get,
            path: None,
            version: Version::Http11,
            headers: Headers::new(),
            error: None,
        }
    }

    pub fn method(mut self, m: Method) -> Self {
        self.method = m;
        self
    }

    pub fn path(mut self, p: &str) -> Self {
        match KVec::from_slice(p.as_bytes(), Flags::GFP_KERNEL) {
            Ok(v) => self.path = Some(v),
            Err(e) => self.error = Some(e),
        }
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
    pub fn body(self, body: KVec<u8>) -> Result<Request, AllocError> {
        if let Some(e) = self.error {
            return Err(e);
        }
        let path = match self.path {
            Some(p) => p,
            None => KVec::from_slice(b"/", Flags::GFP_KERNEL)?,
        };
        Ok(Request {
            method: self.method,
            path,
            version: self.version,
            headers: self.headers,
            body,
        })
    }
}
