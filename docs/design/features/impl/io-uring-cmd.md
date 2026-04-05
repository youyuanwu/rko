# Feature: Custom io_uring Commands

**Status**: ✅ Implemented

See: `rko-core/src/io_uring.rs`, `rko-core/src/miscdevice.rs`,
`rko-sys/src/rko/io_uring/`, `samples/http_uring/`, `rko-test-uring/`.

## Goal

Enable Rust kernel modules to handle custom `IORING_OP_URING_CMD`
commands. A misc device's `file_operations.uring_cmd` dispatches to
a Rust trait method, giving userspace batched async access to
driver-specific operations through io_uring.

## Architecture

```
userspace (io-uring crate / kimojio)
  │  IORING_OP_URING_CMD + cmd[] payload
  ▼
io_uring core → file->f_op->uring_cmd(io_uring_cmd, issue_flags)
  │
  ▼
MiscdeviceVTable trampoline → recovers per-fd state from private_data
  │
  ▼
MiscDevice::uring_cmd(device, cmd, flags) → return result
```

## API

### Safe wrappers (`rko-core/src/io_uring.rs`)

```rust
pub struct IoUringCmd { /* *mut io_uring_cmd */ }

impl IoUringCmd {
    pub fn cmd_op(&self) -> u32;
    pub unsafe fn cmd_data<T: Sized>(&self) -> &T;  // sqe->cmd payload
    pub fn sqe_addr(&self) -> u64;                   // sqe->addr
    pub fn sqe_len(&self) -> u32;                    // sqe->len
    pub unsafe fn pdu<T: Sized>(&self) -> *mut T;    // 32-byte inline storage
    pub fn file(&self) -> *mut file;
    pub fn done(self, ret: i32, flags: IssueFlags);  // async completion only
    pub fn defer(self) -> IoUringCmdAsync;            // deferred async
    pub fn mark_cancelable(&self, flags: IssueFlags);
}

pub struct IoUringCmdAsync { /* must call done() */ }
pub struct IssueFlags(u32);
```

### Misc device (`rko-core/src/miscdevice.rs`)

```rust
pub struct MiscDeviceRegistration<T> { /* KBox<miscdevice> */ }
pub struct MiscDeviceOptions { pub name: &'static CStr }

#[vtable]
pub trait MiscDevice: Sized {
    type Ptr: ForeignOwnable + Send + Sync;
    fn open(misc: &MiscDeviceRegistration<Self>) -> Result<Self::Ptr, Error>;
    fn release(_device: Self::Ptr) { drop(_device); }
    fn uring_cmd(_device: ..., _cmd: IoUringCmd, _flags: IssueFlags) -> i32;
}
```

### Completion Model

**Critical**: For synchronous completion, return the result directly.
Do NOT call `io_uring_cmd_done()` — the kernel handles CQE posting.
Calling `done()` for sync causes double cleanup → NULL deref crash.

| Pattern | Return from `uring_cmd` |
|---------|-------------------------|
| **Synchronous** | result value directly |
| Async deferred | `cmd.defer()`, return `-EIOCBQUEUED`, call `async_cmd.done()` later |

## Bindings

- **Partition**: `rko.io_uring` traverses `cmd.h` + `io_uring_types.h` +
  `uapi/io_uring.h`. 6 inject_types cut cascade.
- **C helper**: Only `rust_helper_io_uring_cmd_done` (inline wrapper).
  All other fields accessed via generated struct bindings.
- **Feature**: `io_uring` in `rko-sys/Cargo.toml`.
- **Misc partition**: `rko.misc` for `miscdevice`, `misc_register`,
  `misc_deregister`, `MISC_DYNAMIC_MINOR`.

## Kernel Config

`CONFIG_IO_URING=y` and `CONFIG_KALLSYMS=y` in `CMakeLists.txt`.

Exported symbols: `__io_uring_cmd_done`, `io_uring_mshot_cmd_post_cqe`,
`io_uring_cmd_mark_cancelable`, `io_uring_cmd_buffer_select`,
`io_uring_cmd_import_fixed`, `misc_register`, `misc_deregister`.

## Future Work

- Filesystem vtable wiring (add `uring_cmd` to `fs/vtable.rs`)
- Standalone ping/echo sample (`samples/uring_cmd_test/`)
- `complete_in_task()` wrapper for task-context async completion

## References

- Kernel: `io_uring/uring_cmd.c`, `include/linux/io_uring/cmd.h`
- NVMe passthrough: `drivers/nvme/host/ioctl.c`
- FUSE over io_uring: [kernel docs](https://docs.kernel.org/next/filesystems/fuse-io-uring.html)
- Upstream miscdevice: [`rust/kernel/miscdevice.rs`](https://github.com/torvalds/linux/blob/master/rust/kernel/miscdevice.rs)
