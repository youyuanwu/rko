// SPDX-License-Identifier: GPL-2.0

//! Safe wrappers for io_uring custom commands (`IORING_OP_URING_CMD`).
//!
//! Provides [`IoUringCmd`] and [`IoUringCmdAsync`] for handling custom
//! io_uring commands in kernel modules. The ownership model prevents
//! double-completion at compile time.
//!
//! See `docs/design/features/futures/io-uring-cmd.md` for design details.
// UPSTREAM_REF: include/linux/io_uring/cmd.h

use rko_sys::rko::helpers as h;
use rko_sys::rko::io_uring as uring_b;

/// Wraps `struct io_uring_cmd` with a safe interface.
///
/// Created by the vtable trampoline when the kernel dispatches a
/// `IORING_OP_URING_CMD` to the device's `file_operations.uring_cmd`.
///
/// # Invariants
///
/// The inner pointer is valid for the duration of the `uring_cmd`
/// callback. For async completion (via [`defer`](Self::defer)),
/// validity extends until [`IoUringCmdAsync::done`] is called.
pub struct IoUringCmd {
    cmd: *mut uring_b::io_uring_cmd,
}

// SAFETY: io_uring_cmd is passed between threads by the io_uring core.
unsafe impl Send for IoUringCmd {}

impl IoUringCmd {
    /// Create from a raw pointer.
    ///
    /// # Safety
    ///
    /// `cmd` must be a valid `struct io_uring_cmd *` from the kernel's
    /// uring_cmd callback.
    pub unsafe fn from_raw(cmd: *mut uring_b::io_uring_cmd) -> Self {
        Self { cmd }
    }

    /// The raw pointer to the underlying `struct io_uring_cmd`.
    pub fn as_raw(&self) -> *mut uring_b::io_uring_cmd {
        self.cmd
    }

    /// The driver-defined command opcode (from `cmd_op`).
    pub fn cmd_op(&self) -> u32 {
        // SAFETY: cmd is valid for the callback duration.
        unsafe { (*self.cmd).cmd_op }
    }

    /// Read the SQE command payload as a typed struct.
    ///
    /// The payload is in `sqe->cmd[]` (up to 80 bytes with SQE128).
    ///
    /// # Safety
    ///
    /// `T` must match the layout userspace wrote into `sqe->cmd`.
    /// `T` should be `#[repr(C)]` and ideally `#[derive(FromBytes)]`.
    /// The caller must validate field values — this only ensures
    /// memory safety via size check.
    pub unsafe fn cmd_data<T: Sized>(&self) -> &T {
        // SAFETY: cmd and sqe are valid. The cmd[] flexible array in the
        // SQE extends up to 80 bytes in SQE128.
        let ptr = unsafe { (*(*self.cmd).sqe).io_uring_sqe__anon_5.cmd.as_ptr() };
        debug_assert!(core::mem::size_of::<T>() <= 80);
        unsafe { &*ptr.cast::<T>() }
    }

    /// The `addr` field from the SQE (typically a userspace buffer pointer).
    pub fn sqe_addr(&self) -> u64 {
        // SAFETY: cmd and cmd.sqe are valid for the callback duration.
        unsafe { (*(*self.cmd).sqe).io_uring_sqe__anon_1.addr }
    }

    /// The `len` field from the SQE (typically a buffer length).
    pub fn sqe_len(&self) -> u32 {
        // SAFETY: cmd and cmd.sqe are valid for the callback duration.
        unsafe { (*(*self.cmd).sqe).len }
    }

    /// Access the 32-byte inline pdu for driver-private state.
    ///
    /// The pdu is inline storage within `struct io_uring_cmd` that
    /// the driver can use freely (e.g., to stash async state).
    ///
    /// # Safety
    ///
    /// The caller must ensure no other references to the pdu exist.
    pub unsafe fn pdu<T: Sized>(&self) -> *mut T {
        debug_assert!(core::mem::size_of::<T>() <= 32);
        unsafe { (*self.cmd).pdu.as_ptr() as *mut T }
    }

    /// Complete the command synchronously.
    ///
    /// Consumes `self` — prevents double-completion at compile time.
    /// `ret` is the result value posted in the CQE.
    pub fn done(self, ret: i32, issue_flags: IssueFlags) {
        unsafe { h::rust_helper_io_uring_cmd_done(self.cmd, ret, issue_flags.0) };
    }

    /// Defer completion — returns an [`IoUringCmdAsync`] that must be
    /// completed later.
    ///
    /// The caller should return `-EIOCBQUEUED` from the uring_cmd callback.
    /// The `IoUringCmdAsync` must eventually call [`done`](IoUringCmdAsync::done)
    /// to post the CQE and free kernel resources.
    #[must_use = "deferred command must be completed via IoUringCmdAsync::done()"]
    pub fn defer(self) -> IoUringCmdAsync {
        IoUringCmdAsync { cmd: self.cmd }
    }

    /// Mark this command as cancelable.
    ///
    /// After marking, the io_uring core may cancel the command by calling
    /// the uring_cmd callback again with `flags.is_cancel() == true`.
    pub fn mark_cancelable(&self, issue_flags: IssueFlags) {
        unsafe {
            uring_b::io_uring_cmd_mark_cancelable(self.cmd, issue_flags.0);
        }
    }

    /// The raw `struct file *` this command targets.
    pub fn file(&self) -> *mut rko_sys::rko::fs::file {
        unsafe { (*self.cmd).file }
    }
}

/// Held when async completion is deferred via [`IoUringCmd::defer`].
///
/// Must call [`done`](Self::done) to complete the command and post
/// the CQE. Dropping without calling `done` is a bug — the `#[must_use]`
/// attribute on `IoUringCmd::defer()` warns about this.
pub struct IoUringCmdAsync {
    cmd: *mut uring_b::io_uring_cmd,
}

// SAFETY: Deferred commands can be completed from any thread.
unsafe impl Send for IoUringCmdAsync {}
unsafe impl Sync for IoUringCmdAsync {}

impl IoUringCmdAsync {
    /// Complete the deferred command.
    ///
    /// Posts a CQE with `ret` as the result. Consumes self.
    pub fn done(self, ret: i32, issue_flags: IssueFlags) {
        unsafe { h::rust_helper_io_uring_cmd_done(self.cmd, ret, issue_flags.0) };
    }

    /// The raw pointer to the underlying `struct io_uring_cmd`.
    pub fn as_raw(&self) -> *mut uring_b::io_uring_cmd {
        self.cmd
    }

    /// Access the 32-byte inline pdu.
    ///
    /// # Safety
    ///
    /// The caller must ensure no other references to the pdu exist.
    pub unsafe fn pdu<T: Sized>(&self) -> *mut T {
        debug_assert!(core::mem::size_of::<T>() <= 32);
        unsafe { (*self.cmd).pdu.as_ptr() as *mut T }
    }
}

/// Flags passed by the io_uring core to the uring_cmd callback.
///
/// These indicate the execution context and constraints on the handler.
#[derive(Copy, Clone, Debug)]
pub struct IssueFlags(pub(crate) u32);

impl IssueFlags {
    /// Create from raw flags value.
    pub fn from_raw(flags: u32) -> Self {
        Self(flags)
    }

    /// The raw flags value.
    pub fn raw(self) -> u32 {
        self.0
    }
}
