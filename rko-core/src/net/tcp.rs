//! Synchronous TCP socket wrappers.
//!
//! Provides `TcpListener` and `TcpStream` wrapping the kernel's
//! synchronous socket API (`sock_create_kern`, `kernel_bind`,
//! `kernel_listen`, `kernel_accept`, `kernel_sendmsg`, `kernel_recvmsg`).

use core::ptr;

use crate::error::Error;
use crate::net::addr::SockaddrStorage;
use crate::net::{IPPROTO_TCP, SOCK_STREAM};

use super::SocketAddr;
use super::namespace::Namespace;

/// `O_NONBLOCK` flag for non-blocking accept (not in net bindings).
const O_NONBLOCK: i32 = 2048;

/// A listening TCP socket.
///
/// Created via [`TcpListener::try_new`], which binds and listens on the
/// given address. Use [`accept`](TcpListener::accept) to accept incoming
/// connections.
pub struct TcpListener {
    sock: *mut rko_sys::rko::net::socket,
}

// SAFETY: Kernel sockets are usable from any thread.
unsafe impl Send for TcpListener {}
unsafe impl Sync for TcpListener {}

/// A connected TCP stream.
///
/// Obtained from [`TcpListener::accept`]. Provides synchronous
/// [`read`](TcpStream::read), [`write`](TcpStream::write), and
/// [`write_all`](TcpStream::write_all) operations.
pub struct TcpStream {
    sock: *mut rko_sys::rko::net::socket,
}

// SAFETY: Kernel sockets are usable from any thread.
unsafe impl Send for TcpStream {}
unsafe impl Sync for TcpStream {}

/// Convert a negative kernel error code to `Result`.
fn check_err(ret: i32) -> Result<i32, Error> {
    if ret < 0 {
        Err(Error::from_errno(ret))
    } else {
        Ok(ret)
    }
}

impl TcpListener {
    /// Return the raw kernel socket pointer.
    ///
    /// Used by the async layer to register wait-queue callbacks.
    pub(crate) fn as_raw_sock(&self) -> *mut rko_sys::rko::net::socket {
        self.sock
    }

    /// Create a new TCP listener bound to `addr` in network namespace `ns`.
    ///
    /// Sets `SO_REUSEADDR` and `SO_KEEPALIVE` socket options when the
    /// necessary setsockopt bindings are available (see NOTE below).
    pub fn try_new(ns: &Namespace, addr: &SocketAddr) -> Result<Self, Error> {
        let mut sock: *mut rko_sys::rko::net::socket = ptr::null_mut();

        // SAFETY: `ns.as_ptr()` is a valid `struct net *`, `sock` is a valid out-pointer.
        let ret = unsafe {
            rko_sys::rko::net::sock_create_kern(
                ns.as_ptr(),
                addr.family(),
                SOCK_STREAM,
                IPPROTO_TCP,
                &mut sock,
            )
        };
        check_err(ret)?;

        // NOTE: SO_REUSEADDR and SO_KEEPALIVE socket options require
        // `do_sock_setsockopt` or `kernel_setsockopt`, which are not
        // exported in the current rko-sys bindings. These options are
        // desirable but not required for basic operation. Add a C helper
        // wrapping `do_sock_setsockopt` when kernel socket-option support
        // is added to the net partition.

        // Bind to the requested address.
        let (mut storage, addrlen) = addr.to_raw();
        // SAFETY: `sock` is a valid socket from sock_create_kern.
        // We cast &mut SockaddrStorage to *mut sockaddr_unsized because
        // kernel_bind reads `addrlen` bytes from the pointer regardless
        // of the Rust type — the C function expects `struct sockaddr *`.
        let ret = unsafe {
            rko_sys::rko::net::kernel_bind(
                sock,
                &mut storage as *mut SockaddrStorage as *mut rko_sys::rko::net::sockaddr_unsized,
                addrlen,
            )
        };
        if ret < 0 {
            // SAFETY: sock is valid — release on error.
            unsafe { rko_sys::rko::net::sock_release(sock) };
            return Err(Error::from_errno(ret));
        }

        // Listen with the kernel default backlog.
        // SAFETY: sock is a valid, bound socket.
        let ret = unsafe { rko_sys::rko::net::kernel_listen(sock, rko_sys::rko::net::SOMAXCONN) };
        if ret < 0 {
            unsafe { rko_sys::rko::net::sock_release(sock) };
            return Err(Error::from_errno(ret));
        }

        Ok(TcpListener { sock })
    }

    /// Accept an incoming connection.
    ///
    /// If `block` is `true`, this call blocks until a connection arrives.
    /// If `false`, it returns immediately with an error if no connection
    /// is pending.
    pub fn accept(&self, block: bool) -> Result<TcpStream, Error> {
        let flags = if block { 0 } else { O_NONBLOCK };
        let mut new_sock: *mut rko_sys::rko::net::socket = ptr::null_mut();

        // SAFETY: self.sock is a valid listening socket.
        let ret = unsafe { rko_sys::rko::net::kernel_accept(self.sock, &mut new_sock, flags) };
        check_err(ret)?;

        Ok(TcpStream { sock: new_sock })
    }
}

impl Drop for TcpListener {
    fn drop(&mut self) {
        if !self.sock.is_null() {
            // SAFETY: we own the socket.
            unsafe { rko_sys::rko::net::sock_release(self.sock) };
        }
    }
}

impl TcpStream {
    /// Return the raw kernel socket pointer.
    ///
    /// Used by the async layer to register wait-queue callbacks.
    pub(crate) fn as_raw_sock(&self) -> *mut rko_sys::rko::net::socket {
        self.sock
    }

    /// Read data from the stream into `buf`.
    ///
    /// Returns the number of bytes read, or an error. Returns `Ok(0)` at
    /// EOF / connection close.
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, Error> {
        let mut kvec = rko_sys::rko::net::kvec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len() as u64,
        };
        let mut msg = rko_sys::rko::net::msghdr::default();

        // SAFETY: self.sock is valid; kvec and msg are stack-allocated and valid.
        let ret = unsafe {
            rko_sys::rko::net::kernel_recvmsg(
                self.sock,
                &mut msg,
                &mut kvec,
                1,
                buf.len() as u64,
                0,
            )
        };
        check_err(ret).map(|n| n as usize)
    }

    /// Write data from `buf` to the stream.
    ///
    /// Returns the number of bytes written (may be less than `buf.len()`).
    pub fn write(&self, buf: &[u8]) -> Result<usize, Error> {
        let mut kvec = rko_sys::rko::net::kvec {
            iov_base: buf.as_ptr() as *mut _,
            iov_len: buf.len() as u64,
        };
        let mut msg = rko_sys::rko::net::msghdr {
            msg_flags: rko_sys::rko::net::MSG_NOSIGNAL as u32,
            ..rko_sys::rko::net::msghdr::default()
        };

        // SAFETY: self.sock is valid; kvec and msg are stack-allocated and valid.
        let ret = unsafe {
            rko_sys::rko::net::kernel_sendmsg(self.sock, &mut msg, &mut kvec, 1, buf.len() as u64)
        };
        check_err(ret).map(|n| n as usize)
    }

    /// Write all of `buf` to the stream, retrying short writes.
    ///
    /// Returns `Ok(())` on success or an error if the connection is lost.
    pub fn write_all(&self, buf: &[u8]) -> Result<(), Error> {
        let mut offset = 0usize;
        while offset < buf.len() {
            let n = self.write(&buf[offset..])?;
            if n == 0 {
                return Err(Error::ECONNRESET);
            }
            offset += n;
        }
        Ok(())
    }
}

impl Drop for TcpStream {
    fn drop(&mut self) {
        if !self.sock.is_null() {
            // SAFETY: we own the socket.
            unsafe { rko_sys::rko::net::sock_release(self.sock) };
        }
    }
}
