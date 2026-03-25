//! IP address and socket address types.
//!
//! These are hand-written `#[repr(C)]` structs because the kernel UAPI types
//! (`sockaddr_in`, `in_addr`, etc.) use anonymous unions/enums that
//! bnd-winmd cannot extract.

use super::{AF_INET, AF_INET6};

/// An IPv4 address (4 bytes, network order).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv4Addr(pub [u8; 4]);

impl Ipv4Addr {
    /// The unspecified address `0.0.0.0`.
    pub const ANY: Self = Ipv4Addr([0, 0, 0, 0]);
    /// The loopback address `127.0.0.1`.
    pub const LOCALHOST: Self = Ipv4Addr([127, 0, 0, 1]);

    /// Create a new IPv4 address from four octets.
    pub const fn new(a: u8, b: u8, c: u8, d: u8) -> Self {
        Ipv4Addr([a, b, c, d])
    }

    /// Return the four octets as a `u32` in network byte order.
    pub const fn to_bits(self) -> u32 {
        u32::from_be_bytes(self.0)
    }
}

/// An IPv6 address (16 bytes, network order).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ipv6Addr(pub [u8; 16]);

impl Ipv6Addr {
    /// The unspecified address `::`.
    pub const ANY: Self = Ipv6Addr([0; 16]);
    /// The loopback address `::1`.
    pub const LOCALHOST: Self = Ipv6Addr([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);

    /// Create an IPv6 address from 16 octets.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Ipv6Addr(bytes)
    }
}

/// An IPv4 socket address: IP + port.
#[derive(Clone, Copy, Debug)]
pub struct SocketAddrV4 {
    /// The IPv4 address.
    pub ip: Ipv4Addr,
    /// The port number (host byte order — converted to network order in FFI).
    pub port: u16,
}

/// An IPv6 socket address: IP + port + flow info + scope id.
#[derive(Clone, Copy, Debug)]
pub struct SocketAddrV6 {
    /// The IPv6 address.
    pub ip: Ipv6Addr,
    /// The port number (host byte order).
    pub port: u16,
    /// IPv6 flow information.
    pub flowinfo: u32,
    /// Scope ID.
    pub scope_id: u32,
}

/// A socket address (IPv4 or IPv6).
#[derive(Clone, Copy, Debug)]
pub enum SocketAddr {
    V4(SocketAddrV4),
    V6(SocketAddrV6),
}

impl SocketAddr {
    /// Create an IPv4 socket address.
    pub const fn new_v4(ip: Ipv4Addr, port: u16) -> Self {
        SocketAddr::V4(SocketAddrV4 { ip, port })
    }

    /// Create an IPv6 socket address.
    pub const fn new_v6(ip: Ipv6Addr, port: u16) -> Self {
        SocketAddr::V6(SocketAddrV6 {
            ip,
            port,
            flowinfo: 0,
            scope_id: 0,
        })
    }

    /// Return the address family (`AF_INET` or `AF_INET6`).
    pub const fn family(&self) -> i32 {
        match self {
            SocketAddr::V4(_) => AF_INET,
            SocketAddr::V6(_) => AF_INET6,
        }
    }

    /// Return the port number.
    pub const fn port(&self) -> u16 {
        match self {
            SocketAddr::V4(v4) => v4.port,
            SocketAddr::V6(v6) => v6.port,
        }
    }
}

// ---- FFI sockaddr structs for kernel_bind / kernel_connect ----

/// `struct sockaddr_in` — matches the kernel C layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SockaddrIn {
    pub sin_family: u16,
    pub sin_port: u16, // network byte order
    pub sin_addr: u32, // network byte order
    pub sin_zero: [u8; 8],
}

/// `struct sockaddr_in6` — matches the kernel C layout.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SockaddrIn6 {
    pub sin6_family: u16,
    pub sin6_port: u16,     // network byte order
    pub sin6_flowinfo: u32, // network byte order
    pub sin6_addr: [u8; 16],
    pub sin6_scope_id: u32,
}

/// Enough storage for either `sockaddr_in` or `sockaddr_in6`.
#[repr(C)]
pub(crate) union SockaddrStorage {
    pub v4: SockaddrIn,
    pub v6: SockaddrIn6,
}

impl SocketAddr {
    /// Write this address into a `SockaddrStorage` and return the length
    /// in bytes (suitable for the `addrlen` parameter of `kernel_bind`).
    pub(crate) fn to_raw(self) -> (SockaddrStorage, i32) {
        match self {
            SocketAddr::V4(v4) => {
                let sa = SockaddrIn {
                    sin_family: AF_INET as u16,
                    sin_port: v4.port.to_be(),
                    sin_addr: v4.ip.to_bits().to_be(),
                    sin_zero: [0u8; 8],
                };
                (
                    SockaddrStorage { v4: sa },
                    core::mem::size_of::<SockaddrIn>() as i32,
                )
            }
            SocketAddr::V6(v6) => {
                let sa = SockaddrIn6 {
                    sin6_family: AF_INET6 as u16,
                    sin6_port: v6.port.to_be(),
                    sin6_flowinfo: v6.flowinfo.to_be(),
                    sin6_addr: v6.ip.0,
                    sin6_scope_id: v6.scope_id,
                };
                (
                    SockaddrStorage { v6: sa },
                    core::mem::size_of::<SockaddrIn6>() as i32,
                )
            }
        }
    }
}
