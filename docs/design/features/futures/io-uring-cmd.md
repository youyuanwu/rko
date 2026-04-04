# Feature: Custom io_uring Commands

**Status**: 📋 Design — not yet implemented.

## Goal

Enable Rust kernel modules built with rko to handle custom `io_uring`
commands via `IORING_OP_URING_CMD`. A module registers a character or
misc device whose `file_operations.uring_cmd` callback dispatches to a
Rust trait method. This gives userspace high-performance, batched,
asynchronous access to driver-specific operations through io_uring.

## Background

The kernel's `IORING_OP_URING_CMD` mechanism lets any file descriptor
expose custom asynchronous commands to userspace. When a userspace
application submits an SQE with this opcode targeting a device fd, the
kernel invokes the fd's `file_operations.uring_cmd` callback with a
`struct io_uring_cmd`. The driver validates the command, performs work
(synchronously or asynchronously), and posts a CQE via
`io_uring_cmd_done()`.

**Kernel API** (from `include/linux/io_uring/cmd.h`):

```c
struct io_uring_cmd {
    struct file             *file;
    const struct io_uring_sqe *sqe;   // userspace-submitted entry
    u32                     cmd_op;
    u32                     flags;
    u8                      pdu[32];  // inline driver-private storage
};

// Extract typed command from SQE (compile-time size check)
io_uring_sqe_cmd(sqe, type)

// Use pdu[] as typed private data
io_uring_cmd_to_pdu(cmd, pdu_type)

// Completion
void io_uring_cmd_done(struct io_uring_cmd *cmd, s32 ret,
                       unsigned issue_flags);

// Async completion in task context
void io_uring_cmd_complete_in_task(struct io_uring_cmd *cmd,
                                   io_req_tw_func_t cb);

// Mark command as cancelable
void io_uring_cmd_mark_cancelable(struct io_uring_cmd *cmd,
                                  unsigned issue_flags);
```

**Reference implementations**: NVMe passthrough (`drivers/nvme/host/ioctl.c`),
FUSE over io_uring (`fs/fuse/dev_uring.c`), block async discard.

## Architecture

```
userspace (liburing)
  │  IORING_OP_URING_CMD + sqe->cmd payload
  ▼
io_uring core (io_uring/uring_cmd.c)
  │  file->f_op->uring_cmd(io_uring_cmd, issue_flags)
  ▼
rko vtable trampoline (rko-core/src/io_uring.rs)
  │  wraps raw pointers → safe Rust types
  ▼
<T as io_uring::Operations>::uring_cmd(&IoUringCmd, IssueFlags)
  │  driver logic: validate, dispatch, complete
  ▼
IoUringCmd::done(result) → io_uring_cmd_done()
```

### Layer breakdown

| Layer | Crate | Responsibility |
|-------|-------|----------------|
| **rko-sys** | `rko_sys::rko::io_uring` | FFI types: `io_uring_cmd`, `io_uring_sqe`, constants |
| **rko-sys helpers** | `helpers.{c,h}` | C wrappers for inline functions: `io_uring_cmd_done`, `io_uring_cmd_to_pdu`, etc. |
| **rko-core** | `rko_core::io_uring` | Safe `IoUringCmd` wrapper, `IssueFlags`, `Operations` trait, vtable trampoline |
| **Driver module** | user crate | Implements `io_uring::Operations`, registers device with `.uring_cmd` wired |

## Bindings (rko-sys)

### New partition: `rko.io_uring`

A new partition in `rko-sys-gen/rko.toml` to extract io_uring types:

```toml
[[partition]]
namespace = "rko.io_uring"
library = "kernel"
headers = ["linux/io_uring/cmd.h"]
traverse = [
  "linux/io_uring/cmd.h",
  "linux/io_uring_types.h",
  "uapi/linux/io_uring.h",
]
```

**Key types to extract**:
- `struct io_uring_cmd` — the command descriptor
- `struct io_uring_sqe` — submission queue entry (for `sqe->cmd` access)
- Constants: `IORING_URING_CMD_CANCELABLE`, `IORING_URING_CMD_REISSUE`

**Dependency**: Cross-partition references to `rko.fs::file` (for
`io_uring_cmd.file`). The `io_uring_sqe` struct is in UAPI headers and
may bring in many unrelated types — use `[[inject_type]]` if the
traverse cascade is too large.

### Alternative: inject_type only

If the `io_uring_sqe` dependency graph is too deep, inject
`io_uring_cmd` directly and access `sqe->cmd` through raw offsets via
C helpers. This avoids pulling hundreds of io_uring types that a
filesystem/device module does not need:

```toml
[[inject_type]]
name = "rko.io_uring.io_uring_cmd"
size = 88    # sizeof(struct io_uring_cmd) — verify with clang
align = 8
```

The `sqe->cmd` payload would be accessed through a C helper that
copies the bytes out, avoiding the need to bind `io_uring_sqe` at all.

### C helpers

New helpers in `rko-sys/src/helpers.{c,h}`:

```c
// helpers.h
#include <linux/io_uring/cmd.h>

void rust_helper_io_uring_cmd_done(struct io_uring_cmd *cmd,
                                   int ret, unsigned int issue_flags);
void rust_helper_io_uring_cmd_mark_cancelable(struct io_uring_cmd *cmd,
                                              unsigned int issue_flags);
void rust_helper_io_uring_cmd_complete_in_task(
    struct io_uring_cmd *cmd, io_req_tw_func_t cb);

// Access sqe->cmd as raw bytes (avoids binding io_uring_sqe)
const void *rust_helper_io_uring_cmd_sqe_cmd(struct io_uring_cmd *cmd);
u32 rust_helper_io_uring_cmd_op(struct io_uring_cmd *cmd);

// pdu access
void *rust_helper_io_uring_cmd_pdu(struct io_uring_cmd *cmd);

// helpers.c
void rust_helper_io_uring_cmd_done(struct io_uring_cmd *cmd,
                                   int ret, unsigned int issue_flags)
{
    io_uring_cmd_done(cmd, ret, issue_flags);
}

void rust_helper_io_uring_cmd_mark_cancelable(struct io_uring_cmd *cmd,
                                              unsigned int issue_flags)
{
    io_uring_cmd_mark_cancelable(cmd, issue_flags);
}

const void *rust_helper_io_uring_cmd_sqe_cmd(struct io_uring_cmd *cmd)
{
    return cmd->sqe->cmd;
}

u32 rust_helper_io_uring_cmd_op(struct io_uring_cmd *cmd)
{
    return cmd->cmd_op;
}

void *rust_helper_io_uring_cmd_pdu(struct io_uring_cmd *cmd)
{
    return cmd->pdu;
}
```

## Safe Rust API (rko-core)

### `rko-core/src/io_uring.rs`

```rust
use crate::error::Error;

/// Wraps `struct io_uring_cmd` with a safe interface.
///
/// # Invariants
///
/// The inner pointer is valid for the duration of the `uring_cmd`
/// callback. For async completion (`EIOCBQUEUED`), validity extends
/// until `done()` is called.
pub struct IoUringCmd {
    cmd: *mut bindings::io_uring_cmd,
}

impl IoUringCmd {
    /// The driver-defined command opcode (from `cmd_op`).
    pub fn cmd_op(&self) -> u32 { ... }

    /// Read the SQE command payload as a typed struct.
    ///
    /// # Safety
    ///
    /// `T` must match the layout the userspace application wrote into
    /// `sqe->cmd`. The caller must validate the contents.
    pub unsafe fn cmd_data<T: FromBytes>(&self) -> &T { ... }

    /// Access the 32-byte inline pdu for driver-private state.
    pub fn pdu<T: Sized>(&self) -> &mut T { ... }

    /// Complete the command synchronously.
    pub fn done(self, ret: i32, issue_flags: IssueFlags) { ... }

    /// Return EIOCBQUEUED — caller must call `done()` later.
    ///
    /// Consumes self, returns an `IoUringCmdAsync` that must be
    /// completed. Prevents double-completion at the type level.
    pub fn defer(self) -> IoUringCmdAsync { ... }

    /// Mark this command as cancelable.
    pub fn mark_cancelable(&self, issue_flags: IssueFlags) { ... }

    /// The raw `struct file *` this command targets.
    pub fn file(&self) -> &crate::fs::File<???> { ... }
}

/// Held when async completion is deferred. Must call `done()`.
pub struct IoUringCmdAsync {
    cmd: *mut bindings::io_uring_cmd,
}

impl IoUringCmdAsync {
    /// Complete the deferred command.
    pub fn done(self, ret: i32, issue_flags: IssueFlags) { ... }

    /// Complete in task context (schedules task_work).
    pub fn complete_in_task(self, cb: impl FnOnce(&IoUringCmd)) { ... }
}

/// Flags passed by the io_uring core to the uring_cmd callback.
#[derive(Copy, Clone)]
pub struct IssueFlags(u32);

impl IssueFlags {
    pub fn is_cancel(&self) -> bool { ... }
    pub fn is_nonblock(&self) -> bool { ... }
}
```

### Completion model

| Pattern | API | Return from `uring_cmd` |
|---------|-----|-------------------------|
| Synchronous | `cmd.done(ret, flags)` | `Ok(())` |
| Async deferred | `let async_cmd = cmd.defer()` | `Err(Error::EIOCBQUEUED)` |
| Async task work | `async_cmd.complete_in_task(cb)` | (from deferred path) |
| Cancelable | `cmd.mark_cancelable(flags)` | check `flags.is_cancel()` |

The `defer()` → `IoUringCmdAsync` pattern uses Rust's ownership
system to prevent double-completion and ensure every deferred command
is eventually completed (via `#[must_use]` on `IoUringCmdAsync`).

### `io_uring::Operations` trait

```rust
/// Trait for handling custom io_uring commands on a device.
///
/// Implement on your module type and wire into file_operations.
#[crate::vtable]
pub trait Operations: Sized + Send + Sync + 'static {
    /// Handle a custom io_uring command.
    ///
    /// `cmd.cmd_op()` identifies the operation. Extract the payload
    /// with `cmd.cmd_data::<MyCmd>()`. Complete with `cmd.done()`
    /// for synchronous handling, or `cmd.defer()` for async.
    ///
    /// Return `Ok(())` after synchronous completion, or
    /// `Err(Error::EIOCBQUEUED)` after calling `cmd.defer()`.
    fn uring_cmd(cmd: IoUringCmd, flags: IssueFlags) -> Result<(), Error>;

    /// Poll for completion of an async io_uring command.
    ///
    /// Only needed for drivers that support `IORING_URING_CMD_POLLED`.
    /// Default: not implemented (returns EOPNOTSUPP).
    fn uring_cmd_iopoll(_cmd: &IoUringCmd) -> Result<i32, Error> {
        Err(Error::EOPNOTSUPP)
    }
}
```

### Vtable wiring

In `rko-core/src/fs/vtable.rs`, the `Tables<T>` constructor would
conditionally wire the trampoline based on a new `HAS_URING_CMD`
constant, following the existing pattern:

```rust
// In Tables::new(), for both dir_file_ops and reg_file_ops:
reg_file_ops: bindings::file_operations {
    // ... existing fields ...
    uring_cmd: if <T as io_uring::Operations>::HAS_URING_CMD {
        uring_cmd_trampoline::<T> as *mut isize
    } else {
        core::ptr::null_mut()
    },
    uring_cmd_iopoll: if <T as io_uring::Operations>::HAS_URING_CMD_IOPOLL {
        uring_cmd_iopoll_trampoline::<T> as *mut isize
    } else {
        core::ptr::null_mut()
    },
    ..const_default_file_operations()
},
```

### Trampoline

```rust
/// `file_operations::uring_cmd` → `<T as io_uring::Operations>::uring_cmd`.
unsafe extern "C" fn uring_cmd_trampoline<T: io_uring::Operations>(
    cmd: *mut bindings::io_uring_cmd,
    issue_flags: u32,
) -> i32 {
    let wrapper = IoUringCmd { cmd };
    let flags = IssueFlags(issue_flags);
    match T::uring_cmd(wrapper, flags) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}
```

## Design Decisions

### Filesystem-scoped vs standalone trait

**Decision**: `io_uring::Operations` is a **standalone trait**, not
embedded in `fs::file::Operations`.

**Rationale**: `IORING_OP_URING_CMD` is not filesystem-specific. It
applies to any file descriptor — char devices, misc devices, block
devices. Making it standalone allows:
- Char/misc device modules to use it without the filesystem framework
- Filesystem modules to opt in via trait bound on `Tables<T>`
- Future `miscdevice::Operations` to also wire `uring_cmd`

For filesystem integration, `Tables<T>` adds a trait bound
`where T: io_uring::Operations` (or checks `HAS_URING_CMD` as a
`#[vtable]` optional method).

### `IoUringCmd` ownership for completion safety

**Decision**: `IoUringCmd::done()` consumes `self`.
`IoUringCmd::defer()` consumes `self` and returns `IoUringCmdAsync`.

**Rationale**: Calling `io_uring_cmd_done()` twice is a kernel bug
that corrupts the io_uring completion queue. By consuming the wrapper,
the Rust type system prevents double-completion at compile time. The
`#[must_use]` attribute on `IoUringCmdAsync` warns if a deferred
command is dropped without completion.

### `cmd_data<T>` requires `FromBytes`

**Decision**: Use rko's existing `#[derive(FromBytes)]` trait to
validate that the command struct is safe to interpret from raw bytes.

**Rationale**: The `sqe->cmd` payload is written by untrusted
userspace. `FromBytes` ensures the struct has no padding invariants or
invalid bit patterns. The driver must still validate field values,
but memory safety is guaranteed.

### C helper for `sqe->cmd` access (not direct struct binding)

**Decision**: Access `sqe->cmd` through a C helper that returns
`*const c_void` rather than binding `struct io_uring_sqe`.

**Rationale**: `io_uring_sqe` is a UAPI union with complex layout and
many conditional fields. Binding it would cascade into hundreds of
io_uring types that a device driver never needs. The helper approach
keeps the binding surface minimal — only `io_uring_cmd` and a few
functions.

### No `File<T>` in the trait signature

**Decision**: The `uring_cmd` callback receives `IoUringCmd` (which
can access the file via `.file()`), not a typed `&File<T>`.

**Rationale**: The io_uring command may not come from a filesystem
context. For standalone devices, there is no filesystem type parameter.
The raw file access is still available for drivers that need it.

## User API

### Kernel-side: device module with io_uring support

```rust
#![no_std]
use rko_core::error::Error;
use rko_core::io_uring::{self, IoUringCmd, IssueFlags};
use rko_core::types::FromBytes;

/// Command payload sent by userspace in sqe->cmd.
#[repr(C)]
#[derive(FromBytes)]
struct MyCmd {
    opcode: u32,
    addr: u64,
    len: u32,
}

const MY_OP_PING: u32 = 0;
const MY_OP_DO_WORK: u32 = 1;

struct MyDevice;

#[rko_core::vtable]
impl io_uring::Operations for MyDevice {
    fn uring_cmd(cmd: IoUringCmd, flags: IssueFlags) -> Result<(), Error> {
        if flags.is_cancel() {
            cmd.done(-1, flags); // or handle cancelation
            return Ok(());
        }

        // SAFETY: Userspace contract specifies MyCmd layout.
        let my_cmd = unsafe { cmd.cmd_data::<MyCmd>()? };

        match my_cmd.opcode {
            MY_OP_PING => {
                cmd.done(0, flags);
                Ok(())
            }
            MY_OP_DO_WORK => {
                // Synchronous work
                let result = do_work(my_cmd.addr, my_cmd.len);
                cmd.done(result, flags);
                Ok(())
            }
            _ => {
                cmd.done(Error::EINVAL.to_errno(), flags);
                Ok(())
            }
        }
    }
}
```

### User-side: liburing

```c
// Initialize ring with 128-byte SQE support
struct io_uring_params params = { .flags = IORING_SETUP_SQE128 };
io_uring_queue_init_params(32, &ring, &params);

// Open the device
int fd = open("/dev/mydevice", O_RDWR);

// Prepare command
struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
memset(sqe, 0, sizeof(*sqe));
sqe->opcode = IORING_OP_URING_CMD;
sqe->fd = fd;

struct my_cmd cmd = { .opcode = MY_OP_PING };
memcpy(sqe->cmd, &cmd, sizeof(cmd));

// Submit and wait
io_uring_submit(&ring);
struct io_uring_cqe *cqe;
io_uring_wait_cqe(&ring, &cqe);
printf("result: %d\n", cqe->res);
io_uring_cqe_seen(&ring, cqe);
```

## Implementation Plan

### Phase 1: Bindings and helpers

1. Evaluate `io_uring_sqe` / `io_uring_cmd` dependency graph via
   `rko-sys-gen` test run — decide between new partition and
   inject_type approach
2. Add C helpers to `helpers.{c,h}` for `io_uring_cmd_done`,
   `io_uring_cmd_mark_cancelable`, sqe/pdu access
3. Regenerate bindings: `cargo run -p rko-sys-gen -- rko-sys-gen/rko.toml`
4. Add `io_uring` feature to `rko-sys/Cargo.toml`
5. Verify: `cargo check -p rko-sys --features io_uring`

### Phase 2: Safe wrappers

1. Create `rko-core/src/io_uring.rs` with `IoUringCmd`,
   `IoUringCmdAsync`, `IssueFlags`, `Operations` trait
2. Add `pub mod io_uring;` to `rko-core/src/lib.rs`
3. Wire trampoline in `vtable.rs` for filesystem integration
4. Verify: `cargo check -p rko-core`

### Phase 3: Sample and test

1. Create `samples/uring_cmd_test/` — misc device that handles
   a simple ping/echo command
2. Write QEMU test: userspace C program uses liburing to send
   commands and verify CQE results
3. Add to `kunit_tests` if applicable
4. CMake target: `cmake --build build --target uring_cmd_test_ko_test`

### Phase 4: Async completion

1. Implement `defer()` / `IoUringCmdAsync` path
2. Implement `complete_in_task()` wrapper
3. Sample: async command that defers completion to a workqueue,
   then calls `done()` from the work callback
4. Test cancelation via `IORING_ASYNC_CANCEL`

## Open Questions

1. **Partition vs inject**: How large is the `io_uring_sqe` type
   cascade? Needs empirical test with `rko-sys-gen`. If manageable
   (< 50 types), a proper partition is cleaner.

2. **Misc device framework**: rko currently only has `module_fs!`
   for filesystem modules. A `module_misc!` or char device
   registration framework would be needed for non-filesystem io_uring
   command handlers. This is separable work.

3. **`File<T>` typing**: For filesystem-backed io_uring commands,
   should `IoUringCmd` expose a typed `file()` accessor? This
   requires the trait to carry a filesystem type parameter, which
   conflicts with standalone device usage.

4. **`CONFIG_IO_URING`**: The kernel API is gated behind
   `CONFIG_IO_URING=y`. Need to verify this is enabled in the rko
   kernel config (`scripts/configure_linux`).

5. **Symbol export**: Verify that `io_uring_cmd_done` and related
   functions are in `linux_bin/Module.symvers`. If they are inline-only,
   C helpers are mandatory (not optional).

## Future Work

- **Multishot commands**: `io_uring_mshot_cmd_post_cqe()` for
  commands that produce multiple CQEs (e.g., event streams)
- **Fixed buffers**: `io_uring_cmd_import_fixed()` for pre-registered
  userspace buffers (zero-copy)
- **32-byte CQE**: `io_uring_cmd_done32()` for returning extra
  data in the completion entry
- **io_uring as async executor backend**: Replace or complement the
  workqueue executor with an io_uring-based executor for
  `kasync::executor` (mentioned in networking spec)

## References

- Kernel source: `linux/io_uring/uring_cmd.c`, `include/linux/io_uring/cmd.h`
- NVMe passthrough: `drivers/nvme/host/ioctl.c`
- FUSE over io_uring: `fs/fuse/dev_uring.c`, [kernel docs](https://docs.kernel.org/next/filesystems/fuse-io-uring.html)
- LWN — async block ops: https://lwn.net/Articles/989332/
- liburing headers: `src/include/liburing/io_uring.h`
- Bindings guide: `docs/guides/adding-bindings.md`
- Filesystem design: `docs/design/features/fs.md`
- Networking design: `docs/design/features/networking.md`
