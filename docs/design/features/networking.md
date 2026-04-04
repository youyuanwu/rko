# Feature: Kernel Networking Support

**Status**: Implemented — all 6 QEMU tests pass.

## Goal

Enable writing kernel modules that use TCP networking (both synchronous
and async) as out-of-tree modules. Based on the experimental patch
`docs/patches/netdev0x17.diff` from the upstream Rust-for-Linux effort.

Four layers:

1. **Synchronous TCP** — `net::TcpListener`, `net::TcpStream`
2. **Workqueue abstraction** — `workqueue::Queue`, `Work<T,ID>`
3. **Async executor** — `kasync::executor::Executor` trait + workqueue backend
4. **Async TCP** — `kasync::net::TcpListener`/`TcpStream` with Futures

## Architecture

```
kasync::net         — async TCP wrappers (SocketFuture)
kasync::executor    — Executor trait, workqueue backend, AutoStopHandle
net                 — sync TCP, addressing, Namespace
workqueue           — Queue, Work<T,ID>, WorkItem (ported from kernel crate)
sync                — Arc, Mutex, SpinLock, CondVar, NoWaitLock (ported)
infra               — AsyncRevocable, unsafe_list, task::KTask
rko-sys             — FFI partitions (rko.net, rko.workqueue, rko.poll)
```

Sync primitives and the workqueue module are ported from the in-tree
Linux kernel Rust crate (`linux/rust/kernel/sync/`, `workqueue.rs`).
Socket API, async executor, and `SocketFuture` are new.

## Key Design Decisions

### Kernel-style `Arc<T>` with `CoerceUnsized`

Ported from `linux/rust/kernel/sync/arc.rs`. Handles unsized
`from_raw` via `Layout::for_value` + `Layout::extend` — no
`ptr_metadata` feature needed. With `CoerceUnsized` (via
`RUSTC_BOOTSTRAP`), the executor stores `Arc<dyn RevocableTask>`
directly, matching the upstream patch design.

### `SocketFuture` — separate `CallbackState` allocation

The upstream patch has `wake_callback` create overlapping `&T` /
`&mut T` references (UB under Stacked Borrows). We extract shared
fields into a heap-allocated `CallbackState` (~24 bytes):

```rust
struct CallbackState {
    mask: u32,
    waker: NoWaitLock<Option<Waker>>,
}
```

The callback reads `CallbackState` from `wq_entry.private` — never
references `SocketFuture`. `remove_wait_queue` holds the wait queue
spinlock, serializing with in-flight callbacks — safe to free after
it returns. When `UnsafePinned` (RFC 3467) stabilizes, the allocation
can be eliminated.

### `NoWaitLock<T>` — try-only lock with contention tracking

`AtomicU8` states: 0=unlocked, 1=locked, 2=locked+contended.
`try_lock` uses `Acquire`, `unlock` uses `Release`. The contention
flag enables lost-wakeup prevention: if `wake_callback` cannot acquire
the lock, `set_waker` detects contention and re-polls.

### `AsyncRevocable<T>` — atomic usage-counted revocation

`AtomicU32` with bit 31 as revoked flag. `try_access` uses CAS with
holding across await points.

### Socket timeouts (mandatory)

`TcpListener::try_new` sets `SO_KEEPALIVE` (`TCP_KEEPIDLE=30`,
`TCP_KEEPINTVL=10`, `TCP_KEEPCNT=3`) and `SO_RCVTIMEO`/`SO_SNDTIMEO`
(30s). `SO_REUSEADDR` is always set. Without timeouts, a silent TCP
peer causes indefinite blocking.

### `write_all` zero-length guard

Returns `Err(ECONNRESET)` on `Ok(0)` instead of looping forever.

## Module Lifetime & Shutdown

`rmmod` frees module code pages. Any dangling reference causes a
kernel panic.

**Rules**:

1. **All long-lived resources in the module struct** — dropped during
   `rmmod` via `Drop`/`PinnedDrop`.

2. **`detach()` requires `spawn_with_module()`** — increments module
   refcount so `rmmod` returns `EBUSY` while threads run.

3. **`stop()` is the shutdown barrier** — sets stopped flag, revokes
   all tasks, `cancel_work_sync` on each.

4. **Explicit `PinnedDrop` for ordered teardown** — executor must stop
   before listener is released. Never rely on field declaration order:

```rust
#[pinned_drop]
impl PinnedDrop for NetModule {
    fn drop(self: Pin<&mut Self>) {
        self.executor.stop();  // 1. stop tasks
        // 2. listener dropped implicitly — safe
    }
}
```

**Executor `stop()` pattern** — peek with `front()`, drop lock before
`revoke()` (kernel Mutex is not recursive), `flush()` via
`cancel_work_sync`:

```rust
fn stop(&self) {
    self.inner.lock().stopped = true;
    loop {
        let guard = self.inner.lock();
        let task = match guard.tasks.front() {
            Some(t) => t.clone(),
            None => break,
        };
        drop(guard);
        task.revoke();
        task.flush();
    }
}
```

## Bindings & C Helpers

Three rko-sys partitions: `rko.net` (124 functions), `rko.workqueue`
(65 functions), `rko.poll` (13 constants). 3 inject_types needed.

26 C helpers in `rko-sys/src/helpers.{c,h}` wrapping inline kernel
functions (mutex, spinlock, RCU, lockdep, waitqueue, task, net
namespace, workqueue init, `wq_entry.private` access).

UAPI address types (`sockaddr_in`, `in_addr`, etc.) are `#[repr(C)]`
structs in `rko-core/src/net/addr.rs` — bnd-winmd cannot extract types
with anonymous enums.

## Feature Gating

`rko-core` has **no Cargo features** — all modules compile
unconditionally. `rko-sys` uses features to select FFI partitions.

### Unstable Rust features (`RUSTC_BOOTSTRAP=1`)

Set in `.cargo/config.toml` and `samples/cargo-kernel.toml`.

**Adopted**: `coerce_unsized`, `dispatch_from_dyn`, `unsize`,
`arbitrary_self_types` — all used by the in-tree kernel crate.

**Not adopted**: `unsafe_pinned` (API unstable), `ptr_metadata`
(not needed), `allocator_api` (not needed).

Toolchain pinned in `rust-toolchain.toml` (1.94.0).

## Future Work

- **UDP support** — same kernel socket API, straightforward extension
- **IPv6 QEMU testing** — types support it, test infra needs config
- **`timeout()` combinator** — async deadline futures
- **`Namespace::current()`** — task namespace instead of `init_net`
- **`UnsafePinned` migration** — eliminate `CallbackState` allocation

## Async testing

`block_on()`, `Completion`, `TcpStream::connect()`, and async
`TcpStream::connect()` were added to support in-kernel TCP echo tests
via `#[rko_tests]`. See `docs/design/features/test-framework.md`.

## Samples

| Sample | Description | Test |
|--------|-------------|------|
| `workqueue_test` | Enqueues work item, logs execution | QEMU pass |
| `tcp_echo` | Sync echo server via WorkItem on workqueue | QEMU pass |
| `async_echo` | Async echo server with WorkqueueExecutor | QEMU pass |
| `kunit_tests` | 56 unit + integration tests including async TCP echo | QEMU pass |

## References

- Upstream patch: `docs/patches/netdev0x17.diff`
- Bindings guide: `docs/guides/adding-bindings.md`
- Kernel Rust crate: `linux/rust/kernel/` (sync, workqueue, task)
- Test framework: `docs/design/features/test-framework.md`
