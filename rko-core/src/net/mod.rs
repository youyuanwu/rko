//! Networking abstractions.
//!
//! Provides safe wrappers around kernel socket APIs for synchronous
//! TCP networking in kernel modules.

mod addr;
mod namespace;
mod tcp;

pub use addr::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};
pub use namespace::Namespace;
pub use tcp::{TcpListener, TcpStream};

/// Address family constants (not extracted by bnd-winmd).
pub const AF_INET: i32 = 2;
pub const AF_INET6: i32 = 10;
/// Socket type for stream (TCP) sockets.
pub const SOCK_STREAM: i32 = 1;
/// IP protocol number for TCP.
pub const IPPROTO_TCP: i32 = 6;
/// Socket-level option level.
pub const SOL_SOCKET: i32 = 1;
/// Socket option: allow address reuse.
pub const SO_REUSEADDR: i32 = 2;
/// Socket option: enable keep-alive probes.
pub const SO_KEEPALIVE: i32 = 9;
