//! Async networking — `Future`-based TCP sockets.
//!
//! Provides async versions of [`TcpListener`] and [`TcpStream`] that
//! wrap the synchronous counterparts from [`crate::net`] with
//! [`SocketFuture`], converting `EAGAIN` into `Poll::Pending`.
//!
//! The key abstraction is [`SocketFuture`], which:
//! 1. Registers a wait-queue callback on the socket's wait queue.
//! 2. Stores the `Waker` in a separately-allocated [`CallbackState`].
//! 3. Converts non-blocking socket operations from `EAGAIN` → `Pending`.
//!
//! # Safety
//!
//! `SocketFuture` uses raw pointers and `unsafe` FFI. All safety
//! invariants are documented inline.

use core::future::Future;
use core::marker::PhantomPinned;
use core::pin::Pin;
use core::ptr::NonNull;
use core::task::{Context, Poll, Waker};

use crate::error::Error;
use crate::sync::NoWaitLock;
use crate::types::Opaque;

/// EAGAIN errno value for non-blocking operation.
const EAGAIN: i32 = -rko_sys::rko::err::EAGAIN;

// ---------------------------------------------------------------------------
// CallbackState
// ---------------------------------------------------------------------------

/// Shared state between [`SocketFuture::poll`] and [`wake_callback`].
///
/// Heap-allocated (via `KBox`) to decouple its aliasing from the
/// `SocketFuture` borrow — the callback accesses this through a raw
/// pointer stored in `wq_entry.private`, never through `&SocketFuture`.
///
/// Layout: `u32` (mask) + padding + `NoWaitLock<Option<Waker>>`.
#[allow(dead_code)]
struct CallbackState {
    /// Event mask — only events matching these bits trigger a wake.
    mask: u32,
    /// The waker to invoke, protected by a try-lock to avoid blocking
    /// in interrupt context.
    waker: NoWaitLock<Option<Waker>>,
}

// ---------------------------------------------------------------------------
// SocketFuture
// ---------------------------------------------------------------------------

/// A `Future` that wraps a non-blocking socket operation.
///
/// On first poll:
/// 1. Stores the `Waker` in [`CallbackState`].
/// 2. Initializes a `wait_queue_entry` and registers it on the socket's
///    wait queue.
///
/// On each poll, calls the `operation` closure. If the operation returns
/// `EAGAIN`, we return `Poll::Pending`. Otherwise, the result is returned.
///
/// # Pin guarantee
///
/// `PhantomPinned` prevents moves after pinning. The `wq_entry` field
/// is registered with the kernel and must not move.
#[allow(dead_code)]
struct SocketFuture<'a, Out, F: FnMut() -> Result<Out, Error> + Send + 'a> {
    /// Raw pointer to the kernel socket (for wait-queue registration).
    sock: *mut rko_sys::rko::net::socket,
    /// Whether we have registered on the socket's wait queue.
    is_queued: bool,
    /// Opaque kernel wait_queue_entry — pinned in place.
    wq_entry: Opaque<rko_sys::rko::pagemap::wait_queue_entry>,
    /// Pointer to the heap-allocated callback state.
    /// Owned by this SocketFuture; freed on drop.
    cb_state: Option<NonNull<CallbackState>>,
    /// The non-blocking operation to attempt on each poll.
    operation: F,
    /// Prevent Unpin — the wq_entry is registered with the kernel.
    _pin: PhantomPinned,
    /// Tie the lifetime to the socket borrow.
    _lifetime: core::marker::PhantomData<&'a ()>,
}

// SAFETY: The SocketFuture is Send if the operation closure is Send.
// The raw pointers (sock, cb_state) point to kernel-managed or
// heap-allocated data that is safe to transfer between threads.
unsafe impl<'a, Out, F: FnMut() -> Result<Out, Error> + Send + 'a> Send
    for SocketFuture<'a, Out, F>
{
}

impl<'a, Out, F: FnMut() -> Result<Out, Error> + Send + 'a> SocketFuture<'a, Out, F> {
    /// Create a new `SocketFuture`.
    ///
    /// `sock` is the raw kernel socket pointer.
    /// `mask` is the poll event mask (e.g., POLLIN).
    /// `operation` is the non-blocking operation to attempt.
    fn new(sock: *mut rko_sys::rko::net::socket, mask: u32, operation: F) -> Self {
        // Allocate CallbackState on the heap via KBox.
        let cb = CallbackState {
            mask,
            waker: NoWaitLock::new(None),
        };

        // Allocate via KBox.
        let cb_state = match crate::alloc::KBox::new(cb, crate::alloc::Flags::GFP_KERNEL) {
            Ok(boxed) => {
                let ptr = crate::alloc::KBox::into_raw(boxed);
                Some(ptr.cast::<CallbackState>())
            }
            Err(_) => None,
        };

        Self {
            sock,
            is_queued: false,
            wq_entry: Opaque::uninit(),
            cb_state,
            operation,
            _pin: PhantomPinned,
            _lifetime: core::marker::PhantomData,
        }
    }
}

impl<'a, Out, F: FnMut() -> Result<Out, Error> + Send + 'a> Future for SocketFuture<'a, Out, F> {
    type Output = Result<Out, Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: We only access fields through raw pointers and never
        // move the SocketFuture (PhantomPinned prevents Unpin).
        let this = unsafe { self.get_unchecked_mut() };

        // If CallbackState allocation failed, return error immediately.
        let cb_ptr = match this.cb_state {
            Some(ptr) => ptr,
            None => return Poll::Ready(Err(Error::ENOMEM)),
        };

        // Store/update the waker in CallbackState.
        // SAFETY: cb_ptr is valid — we allocated it and haven't freed it.
        unsafe {
            if let Some(mut guard) = (*cb_ptr.as_ptr()).waker.try_lock() {
                *guard = Some(cx.waker().clone());
            }
            // If try_lock fails, a wake is in progress — we'll be woken
            // again anyway (NoWaitLock contention = pending wake).
        }

        // Register on the socket's wait queue on first poll.
        if !this.is_queued {
            // SAFETY: wq_entry is owned by this SocketFuture and has not
            // been registered yet. The socket pointer is valid.
            unsafe {
                let wq_entry_ptr = Opaque::raw_get(&mut this.wq_entry as *mut _ as *mut _);

                // Initialize the wait queue entry with our callback.
                // SAFETY: Transmute is sound — extern "C" == extern "system"
                // on Linux, and the parameter types are ABI-compatible.
                #[allow(clippy::missing_transmute_annotations)]
                rko_sys::rko::helpers::rust_helper_init_waitqueue_func_entry(
                    wq_entry_ptr,
                    core::mem::transmute(wake_callback as *const ()),
                );

                // Store CallbackState pointer in the wq_entry's private field.
                rko_sys::rko::helpers::rust_helper_set_wq_entry_private(
                    wq_entry_ptr,
                    cb_ptr.as_ptr() as *mut core::ffi::c_void,
                );

                // Register on the socket's wait queue.
                // SAFETY: sock is a valid socket. We use a C helper to get
                // the wait queue head because socket_wq has
                // ____cacheline_aligned_in_smp which the Rust bindings
                // don't model (field offsets would be wrong).
                let wq_head = rko_sys::rko::helpers::rust_helper_sock_wq_head(this.sock);
                rko_sys::rko::fs::add_wait_queue(wq_head, wq_entry_ptr);
            }
            this.is_queued = true;
        }

        // Attempt the non-blocking operation.
        match (this.operation)() {
            Ok(val) => Poll::Ready(Ok(val)),
            Err(e) => {
                let errno = e.to_errno();
                if errno == EAGAIN {
                    // Operation would block — wait for the wake callback.
                    Poll::Pending
                } else {
                    // Real error.
                    Poll::Ready(Err(Error::from_errno(errno)))
                }
            }
        }
    }
}

impl<'a, Out, F: FnMut() -> Result<Out, Error> + Send + 'a> Drop for SocketFuture<'a, Out, F> {
    fn drop(&mut self) {
        // Remove from wait queue if registered.
        if self.is_queued {
            // SAFETY: The wq_entry was registered on the socket's wait
            // queue during poll. The socket and wq_entry are still valid.
            // Cast through raw pointer for cross-namespace type compat.
            unsafe {
                let wq_head = rko_sys::rko::helpers::rust_helper_sock_wq_head(self.sock);
                rko_sys::rko::fs::remove_wait_queue(
                    wq_head,
                    Opaque::raw_get(&mut self.wq_entry as *mut _ as *mut _),
                );
            }
        }

        // Free the CallbackState.
        if let Some(ptr) = self.cb_state.take() {
            // SAFETY: We allocated this via KBox and haven't freed it yet.
            unsafe {
                let _ = crate::alloc::KBox::<CallbackState>::from_raw(ptr);
            };
        }
    }
}

/// Wake callback for the kernel wait queue.
///
/// Called by the kernel when an event matching our mask occurs on the
/// socket. Reads [`CallbackState`] from `wq_entry.private` and wakes
/// the stored `Waker`.
///
/// # Safety
///
/// - `wq_entry` must point to a valid `wait_queue_entry` whose `private`
///   field contains a pointer to a live `CallbackState`.
/// - This function runs in interrupt/softirq context — it must not block.
///   `NoWaitLock::try_lock` ensures lock-free access.
#[allow(dead_code)]
unsafe extern "C" fn wake_callback(
    wq_entry: *mut rko_sys::rko::pagemap::wait_queue_entry,
    _mode: core::ffi::c_uint,
    _flags: core::ffi::c_int,
    key: *mut core::ffi::c_void,
) -> core::ffi::c_int {
    // SAFETY: wq_entry is a valid wait_queue_entry registered by us.
    // key encodes the event mask from the kernel.
    let event_mask = key as u32;
    let cb: *const CallbackState =
        unsafe { rko_sys::rko::helpers::rust_helper_get_wq_entry_private(wq_entry) }
            as *const CallbackState;

    if cb.is_null() {
        return 0;
    }

    // Only wake if the event matches our interest mask.
    // SAFETY: cb points to a live CallbackState allocated by SocketFuture.
    unsafe {
        if event_mask & (*cb).mask == 0 {
            return 0;
        }
        if let Some(guard) = (*cb).waker.try_lock()
            && let Some(ref w) = *guard
        {
            let cloned = w.clone();
            drop(guard);
            cloned.wake();
            return 1;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// Async TcpListener
// ---------------------------------------------------------------------------

/// An async TCP listener wrapping [`crate::net::TcpListener`].
///
/// Provides `accept()` returning a `Future` that yields connections.
pub struct TcpListener {
    /// The underlying synchronous listener.
    inner: crate::net::TcpListener,
}

impl TcpListener {
    /// Create a new async `TcpListener` from a synchronous one.
    pub fn new(listener: crate::net::TcpListener) -> Self {
        Self { inner: listener }
    }

    /// Create and bind a new async TCP listener.
    pub fn try_new(
        ns: &crate::net::Namespace,
        addr: &crate::net::SocketAddr,
    ) -> Result<Self, Error> {
        let listener = crate::net::TcpListener::try_new(ns, addr)?;
        Ok(Self::new(listener))
    }

    /// Accept an incoming connection asynchronously.
    ///
    /// Returns a `Future` that resolves to a [`TcpStream`] when a
    /// connection is available.
    pub fn accept(&self) -> impl Future<Output = Result<TcpStream, Error>> + Send + '_ {
        // We need access to the raw socket pointer for SocketFuture.
        // SAFETY: The socket pointer is valid for the lifetime of self.inner.
        let sock = self.inner.as_raw_sock();

        SocketFuture::new(sock, rko_sys::rko::poll::POLLIN as u32, move || {
            // Non-blocking accept.
            let stream = self.inner.accept(false)?;
            Ok(TcpStream { inner: stream })
        })
    }
}

// ---------------------------------------------------------------------------
// Async TcpStream
// ---------------------------------------------------------------------------

/// An async TCP stream wrapping [`crate::net::TcpStream`].
///
/// Provides `read()`, `write()`, and `write_all()` as `Future`s.
pub struct TcpStream {
    /// The underlying synchronous stream.
    inner: crate::net::TcpStream,
}

impl TcpStream {
    /// Create a new async `TcpStream` from a synchronous one.
    pub fn new(stream: crate::net::TcpStream) -> Self {
        Self { inner: stream }
    }

    /// Read data asynchronously.
    ///
    /// The returned future resolves when data is available or an error
    /// occurs.
    pub fn read<'a>(
        &'a self,
        buf: &'a mut [u8],
    ) -> impl Future<Output = Result<usize, Error>> + Send + 'a {
        let sock = self.inner.as_raw_sock();

        SocketFuture::new(sock, rko_sys::rko::poll::POLLIN as u32, move || {
            self.inner.read(buf)
        })
    }

    /// Write data asynchronously.
    ///
    /// The returned future resolves when some bytes have been written
    /// or an error occurs.
    pub fn write<'a>(
        &'a self,
        buf: &'a [u8],
    ) -> impl Future<Output = Result<usize, Error>> + Send + 'a {
        let sock = self.inner.as_raw_sock();

        SocketFuture::new(sock, rko_sys::rko::poll::POLLOUT as u32, move || {
            self.inner.write(buf)
        })
    }

    /// Write all data asynchronously.
    ///
    /// Retries short writes until all bytes are written.
    pub async fn write_all(&self, buf: &[u8]) -> Result<(), Error> {
        let mut offset = 0;
        while offset < buf.len() {
            let n = self.write(&buf[offset..]).await?;
            if n == 0 {
                return Err(Error::ECONNRESET);
            }
            offset += n;
        }
        Ok(())
    }
}
