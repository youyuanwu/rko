// SPDX-License-Identifier: GPL-2.0

//! HTTP/1.1 server and client library for kernel modules.
//!
//! Built on `rko_core::kasync::net::TcpStream` with `httparse` for
//! zero-allocation request/response parsing.

mod buf_reader;
mod client;
mod error;
pub mod header;
mod headers;
mod method;
mod parse;
mod request;
mod response;
mod server;
mod status;
mod version;
mod wire;

pub use buf_reader::BufReader;
pub use client::HttpClient;
pub use error::HttpError;
pub use headers::Headers;
pub use method::Method;
pub use parse::{parse_request, parse_response};
pub use request::{Request, RequestBuilder};
pub use response::{Response, ResponseBuilder};
pub use server::{HttpHandler, HttpServer, ServerConfig};
pub use status::StatusCode;
pub use version::Version;
pub use wire::{write_request, write_response};
