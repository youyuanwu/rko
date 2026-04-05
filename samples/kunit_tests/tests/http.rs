use rko_core::alloc::{Flags, KVec};
use rko_core::error::Error;
use rko_util::http::{Headers, Method, Request, Response, StatusCode, Version, header};

#[rko_core::rko_tests]
pub mod http_tests {
    use super::*;

    // --- Method ---

    #[test]
    fn method_from_str() {
        assert_eq!(Method::parse("GET"), Some(Method::Get));
        assert_eq!(Method::parse("POST"), Some(Method::Post));
        assert_eq!(Method::parse("DELETE"), Some(Method::Delete));
        assert_eq!(Method::parse("INVALID"), None);
    }

    #[test]
    fn method_roundtrip() {
        let m = Method::Put;
        assert_eq!(Method::parse(m.as_str()), Some(m));
    }

    // --- StatusCode ---

    #[test]
    fn status_code_basic() {
        let s = StatusCode::OK;
        assert_eq!(s.as_u16(), 200);
        assert!(s.is_success());
        assert!(!s.is_client_error());
    }

    #[test]
    fn status_code_categories() {
        assert!(StatusCode::CREATED.is_success());
        assert!(StatusCode::NOT_FOUND.is_client_error());
        assert!(StatusCode::INTERNAL_SERVER_ERROR.is_server_error());
        assert!(StatusCode::from_u16(301).is_redirect());
    }

    #[test]
    fn status_code_reason() {
        assert_eq!(StatusCode::OK.reason(), "OK");
        assert_eq!(StatusCode::NOT_FOUND.reason(), "Not Found");
        assert_eq!(StatusCode::BAD_REQUEST.reason(), "Bad Request");
    }

    // --- Version ---

    #[test]
    fn version_from_httparse() {
        assert_eq!(Version::from_httparse(0), Version::Http10);
        assert_eq!(Version::from_httparse(1), Version::Http11);
    }

    #[test]
    fn version_as_str() {
        assert_eq!(Version::Http11.as_str(), "HTTP/1.1");
        assert_eq!(Version::Http10.as_str(), "HTTP/1.0");
    }

    // --- Headers ---

    #[test]
    fn headers_insert_and_get() -> Result<(), Error> {
        let mut h = Headers::new();
        h.insert(b"Content-Type", b"text/plain")
            .map_err(|_| Error::ENOMEM)?;
        h.insert(b"X-Custom", b"value123")
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(h.get("Content-Type"), Some(&b"text/plain"[..]));
        assert_eq!(h.get("X-Custom"), Some(&b"value123"[..]));
        assert_eq!(h.get("Missing"), None);
        assert_eq!(h.len(), 2);
        Ok(())
    }

    #[test]
    fn headers_case_insensitive() -> Result<(), Error> {
        let mut h = Headers::new();
        h.insert(b"Content-Type", b"text/html")
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(h.get("content-type"), Some(&b"text/html"[..]));
        assert_eq!(h.get("CONTENT-TYPE"), Some(&b"text/html"[..]));
        assert_eq!(h.get("Content-type"), Some(&b"text/html"[..]));
        Ok(())
    }

    #[test]
    fn headers_content_length() -> Result<(), Error> {
        let mut h = Headers::new();
        h.insert(b"Content-Length", b"42")
            .map_err(|_| Error::ENOMEM)?;
        assert_eq!(h.content_length(), Some(42));

        let empty = Headers::new();
        assert_eq!(empty.content_length(), None);
        Ok(())
    }

    #[test]
    fn headers_connection_close() -> Result<(), Error> {
        let mut h = Headers::new();
        h.insert(b"Connection", b"close")
            .map_err(|_| Error::ENOMEM)?;
        assert!(h.is_connection_close());
        assert!(!h.is_connection_keepalive());

        let mut h2 = Headers::new();
        h2.insert(b"Connection", b"keep-alive")
            .map_err(|_| Error::ENOMEM)?;
        assert!(!h2.is_connection_close());
        assert!(h2.is_connection_keepalive());
        Ok(())
    }

    #[test]
    fn headers_iter() -> Result<(), Error> {
        let mut h = Headers::new();
        h.insert(b"A", b"1").map_err(|_| Error::ENOMEM)?;
        h.insert(b"B", b"2").map_err(|_| Error::ENOMEM)?;

        let pairs: KVec<(&[u8], &[u8])> = {
            let mut v = KVec::new();
            for (n, val) in h.iter() {
                v.push((n, val), Flags::GFP_KERNEL).unwrap();
            }
            v
        };
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[0].0, b"A");
        assert_eq!(pairs[1].1, b"2");
        Ok(())
    }

    // --- header::eq_ignore_ascii_case ---

    #[test]
    fn header_eq_ignore_case() {
        assert!(header::eq_ignore_ascii_case(
            b"Content-Type",
            b"content-type"
        ));
        assert!(header::eq_ignore_ascii_case(b"HOST", b"host"));
        assert!(!header::eq_ignore_ascii_case(b"Host", b"Content"));
        assert!(!header::eq_ignore_ascii_case(b"a", b"ab"));
    }

    // --- Request builder ---

    #[test]
    fn request_builder_basic() -> Result<(), Error> {
        let req = Request::builder()
            .method(Method::Get)
            .path("/api/test")
            .body(KVec::new())
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(req.method(), Method::Get);
        assert_eq!(req.path(), "/api/test");
        assert_eq!(req.version(), Version::Http11);
        assert!(req.body().is_empty());
        Ok(())
    }

    #[test]
    fn request_builder_with_headers_and_body() -> Result<(), Error> {
        let body = KVec::from_slice(b"hello", Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        let req = Request::builder()
            .method(Method::Post)
            .path("/submit")
            .header(header::CONTENT_TYPE, b"text/plain")
            .header(header::ACCEPT, b"*/*")
            .body(body)
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(req.method(), Method::Post);
        assert_eq!(req.path(), "/submit");
        assert_eq!(req.headers().len(), 2);
        assert_eq!(
            req.headers().get(header::CONTENT_TYPE),
            Some(&b"text/plain"[..])
        );
        assert_eq!(req.body().as_slice(), b"hello");
        Ok(())
    }

    #[test]
    fn request_get_convenience() -> Result<(), Error> {
        let req = Request::get("/health").map_err(|_| Error::ENOMEM)?;
        assert_eq!(req.method(), Method::Get);
        assert_eq!(req.path(), "/health");
        assert!(req.body().is_empty());
        Ok(())
    }

    #[test]
    fn request_post_convenience() -> Result<(), Error> {
        let req = Request::post("/data", b"body", "application/json").map_err(|_| Error::ENOMEM)?;
        assert_eq!(req.method(), Method::Post);
        assert_eq!(req.path(), "/data");
        assert_eq!(req.body().as_slice(), b"body");
        assert_eq!(
            req.headers().get(header::CONTENT_TYPE),
            Some(&b"application/json"[..])
        );
        Ok(())
    }

    #[test]
    fn request_default_path() -> Result<(), Error> {
        let req = Request::builder()
            .method(Method::Get)
            .body(KVec::new())
            .map_err(|_| Error::ENOMEM)?;
        assert_eq!(req.path(), "/");
        Ok(())
    }

    // --- Response builder ---

    #[test]
    fn response_builder_basic() -> Result<(), Error> {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(KVec::new())
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.is_success());
        assert_eq!(resp.version(), Version::Http11);
        assert!(resp.body().is_empty());
        Ok(())
    }

    #[test]
    fn response_builder_with_body() -> Result<(), Error> {
        let body = KVec::from_slice(b"Not Found", Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        let resp = Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, b"text/plain")
            .body(body)
            .map_err(|_| Error::ENOMEM)?;

        assert_eq!(resp.status().as_u16(), 404);
        assert!(resp.status().is_client_error());
        assert_eq!(resp.body().as_slice(), b"Not Found");
        assert_eq!(resp.headers().len(), 1);
        Ok(())
    }

    #[test]
    fn response_into_body() -> Result<(), Error> {
        let body_data = b"payload";
        let body = KVec::from_slice(body_data, Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(body)
            .map_err(|_| Error::ENOMEM)?;

        let extracted = resp.into_body();
        assert_eq!(extracted.as_slice(), b"payload");
        Ok(())
    }

    // --- KVec::from_slice (prerequisite) ---

    #[test]
    fn kvec_from_slice() -> Result<(), Error> {
        let v = KVec::from_slice(&[1u8, 2, 3, 4], Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        assert_eq!(v.len(), 4);
        assert_eq!(v[0], 1);
        assert_eq!(v[3], 4);
        Ok(())
    }

    #[test]
    fn kvec_from_slice_empty() -> Result<(), Error> {
        let v = KVec::<u8>::from_slice(&[], Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        assert_eq!(v.len(), 0);
        Ok(())
    }
}
