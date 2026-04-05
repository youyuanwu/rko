//! Kernel async runtime.
//!
//! Provides an executor framework ([`executor`]) and async networking
//! ([`net`]) for running `Future`-based tasks inside the kernel.

pub mod executor;
pub mod net;
pub mod oneshot;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

/// Yields the current async task, allowing other tasks to run.
///
/// The first poll returns `Pending` (after scheduling a wake-up); the
/// second poll returns `Ready(())`.
pub async fn yield_now() {
    struct YieldNow(bool);
    impl Future for YieldNow {
        type Output = ();
        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
    YieldNow(false).await
}
