// SPDX-License-Identifier: GPL-2.0

//! Miscdevice support.
//!
//! Provides [`MiscDeviceRegistration`] and [`MiscDevice`] for creating
//! character devices via the kernel's `misc_register()` API.
//!
//! Ported from the upstream kernel Rust crate (`rust/kernel/miscdevice.rs`)
//! with a minimal subset: `open`, `release`, and `uring_cmd` only.
//! ioctl/mmap/read_iter/write_iter/show_fdinfo are omitted to reduce
//! dependencies.
//!
//! C headers: [`include/linux/miscdevice.h`]

use core::ffi::c_void;
use core::marker::PhantomData;

use rko_sys::rko::fs as fs_b;
use rko_sys::rko::misc as misc_b;

use crate::alloc::{Flags, KBox};
use crate::error::Error;
use crate::io_uring::{IoUringCmd, IssueFlags};
use crate::types::ForeignOwnable;

/// Options for creating a misc device.
#[derive(Copy, Clone)]
pub struct MiscDeviceOptions {
    /// The device name (appears as `/dev/<name>`).
    pub name: &'static core::ffi::CStr,
}

/// A registered miscdevice. Deregisters on drop.
///
/// # Invariants
///
/// `inner` is a heap-allocated `struct miscdevice` registered via
/// `misc_register()`. It must not be moved after registration (the
/// kernel stores list_head pointers into it). KBox ensures stable address.
pub struct MiscDeviceRegistration<T> {
    inner: KBox<misc_b::miscdevice>,
    _t: PhantomData<T>,
}

// SAFETY: misc_deregister can be called from any thread.
unsafe impl<T> Send for MiscDeviceRegistration<T> {}
// SAFETY: All &self methods are safe to call in parallel.
unsafe impl<T> Sync for MiscDeviceRegistration<T> {}

impl<T: MiscDevice> MiscDeviceRegistration<T> {
    /// Register a misc device.
    ///
    /// Returns a registration that deregisters on drop. The caller must
    /// keep it alive for the lifetime of the module.
    pub fn register(opts: MiscDeviceOptions) -> Result<Self, Error> {
        #[allow(clippy::field_reassign_with_default)]
        let dev = {
            let mut dev = misc_b::miscdevice::default();
            dev.minor = misc_b::MISC_DYNAMIC_MINOR;
            dev.name = opts.name.as_ptr().cast_mut();
            dev.fops = (&MiscdeviceVTable::<T>::VTABLE as *const fs_b::file_operations)
                .cast::<c_void>()
                .cast_mut();
            dev
        };

        // Heap-allocate so the address is stable after misc_register
        // (the kernel stores list_head pointers into the miscdevice).
        let inner = KBox::new(dev, Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;

        // SAFETY: inner is heap-allocated and will not move. We keep it
        // alive until drop calls misc_deregister.
        let ptr: *mut misc_b::miscdevice = &*inner as *const _ as *mut _;
        let ret = unsafe { misc_b::misc_register(ptr) };
        if ret < 0 {
            return Err(Error::from_errno(ret));
        }
        Ok(Self {
            inner,
            _t: PhantomData,
        })
    }

    /// Returns a raw pointer to the underlying `struct miscdevice`.
    pub fn as_raw(&self) -> *mut misc_b::miscdevice {
        &*self.inner as *const _ as *mut _
    }
}

impl<T> Drop for MiscDeviceRegistration<T> {
    fn drop(&mut self) {
        // SAFETY: The device was registered in `register()`.
        unsafe { misc_b::misc_deregister(&*self.inner as *const _ as *mut _) };
    }
}

/// Trait for misc device private data.
///
/// Implement this to define a misc device. Each open fd gets its own
/// instance of [`Self::Ptr`] as private data.
///
/// Only `open`, `release`, and `uring_cmd` are supported. For ioctl or
/// read/write, use a full character device instead.
#[crate::vtable]
pub trait MiscDevice: Sized {
    /// Pointer wrapper for per-fd private data.
    ///
    /// Common choices: `Arc<Self>` (shared), `KBox<Self>` (exclusive).
    type Ptr: ForeignOwnable + Send + Sync;

    /// Called when the device is opened. Return per-fd state.
    fn open(misc: &MiscDeviceRegistration<Self>) -> Result<Self::Ptr, Error>;

    /// Called when the fd is closed. Default: drop the pointer.
    fn release(_device: Self::Ptr) {
        // Default impl drops the pointer.
    }

    /// Handle an io_uring custom command (`IORING_OP_URING_CMD`).
    ///
    /// `device` is the per-fd state from `open`. Use `cmd.cmd_op()` to
    /// identify the command, `cmd.cmd_data::<T>()` for the payload.
    ///
    /// **Synchronous completion**: return the result value directly
    /// (positive for success, negative errno for error). Do NOT call
    /// `cmd.done()` — the kernel handles CQE posting.
    ///
    /// **Asynchronous completion**: call `cmd.defer()` to get an
    /// `IoUringCmdAsync`, return `-EIOCBQUEUED`. Call `async_cmd.done()`
    /// later from another context.
    ///
    /// Default: returns -EOPNOTSUPP.
    fn uring_cmd(
        _device: <Self::Ptr as ForeignOwnable>::Borrowed<'_>,
        _cmd: IoUringCmd,
        _flags: IssueFlags,
    ) -> i32 {
        Error::EOPNOTSUPP.to_errno()
    }
}

/// Builds a `file_operations` vtable for a [`MiscDevice`] implementor.
struct MiscdeviceVTable<T: MiscDevice>(PhantomData<T>);

impl<T: MiscDevice> MiscdeviceVTable<T> {
    /// `file_operations::open` trampoline.
    ///
    /// # Safety
    ///
    /// `inode` and `file` must be valid pointers for a file being opened.
    /// The file must be associated with a `MiscDeviceRegistration<T>`.
    unsafe extern "C" fn open(_inode: *mut c_void, file: *mut fs_b::file) -> i32 {
        // misc_open() sets file->private_data to point to the miscdevice
        // struct before calling fops->open.
        // SAFETY: file is valid, private_data was set by misc_open.
        let misc_ptr = unsafe { (*file).private_data };
        let misc = unsafe { &*misc_ptr.cast::<MiscDeviceRegistration<T>>() };

        let ptr = match T::open(misc) {
            Ok(ptr) => ptr,
            Err(err) => return err.to_errno(),
        };

        // Store per-fd state. All future fops calls recover it from here.
        // SAFETY: file is valid and we're in the open path.
        unsafe { (*file).private_data = ptr.into_foreign().cast_mut().cast() };
        0
    }

    /// `file_operations::release` trampoline.
    ///
    /// # Safety
    ///
    /// `file` must be valid and associated with a `MiscDeviceRegistration<T>`.
    unsafe extern "C" fn release(_inode: *mut c_void, file: *mut fs_b::file) -> i32 {
        // SAFETY: The release call owns the private data.
        let private = unsafe { (*file).private_data };
        let ptr = unsafe { <T::Ptr as ForeignOwnable>::from_foreign(private.cast_const()) };
        T::release(ptr);
        0
    }

    /// `file_operations::uring_cmd` trampoline.
    ///
    /// # Safety
    ///
    /// `cmd` must be a valid `struct io_uring_cmd *`. The file associated
    /// with `cmd` must be a `MiscDeviceRegistration<T>` fd.
    unsafe extern "C" fn uring_cmd(
        cmd: *mut rko_sys::rko::io_uring::io_uring_cmd,
        issue_flags: u32,
    ) -> i32 {
        // SAFETY: cmd->file is valid, file->private_data was set in open.
        let file = unsafe { (*cmd).file };
        let private = unsafe { (*file).private_data };
        let device = unsafe { <T::Ptr as ForeignOwnable>::borrow(private.cast_const()) };
        // SAFETY: cmd is valid for the duration of this callback.
        let wrapper = unsafe { IoUringCmd::from_raw(cmd) };
        let flags = IssueFlags::from_raw(issue_flags);
        T::uring_cmd(device, wrapper, flags)
    }

    const VTABLE: fs_b::file_operations = {
        let mut ops = fs_b::file_operations {
            owner: core::ptr::null_mut(),
            fop_flags: 0,
            llseek: core::ptr::null_mut(),
            read: core::ptr::null_mut(),
            write: core::ptr::null_mut(),
            read_iter: core::ptr::null_mut(),
            write_iter: core::ptr::null_mut(),
            iopoll: core::ptr::null_mut(),
            iterate_shared: core::ptr::null_mut(),
            poll: core::ptr::null_mut(),
            unlocked_ioctl: core::ptr::null_mut(),
            compat_ioctl: core::ptr::null_mut(),
            mmap: core::ptr::null_mut(),
            open: core::ptr::null_mut(),
            flush: core::ptr::null_mut(),
            release: core::ptr::null_mut(),
            fsync: core::ptr::null_mut(),
            fasync: core::ptr::null_mut(),
            lock: core::ptr::null_mut(),
            get_unmapped_area: core::ptr::null_mut(),
            check_flags: core::ptr::null_mut(),
            flock: core::ptr::null_mut(),
            splice_write: core::ptr::null_mut(),
            splice_read: core::ptr::null_mut(),
            setlease: core::ptr::null_mut(),
            fallocate: core::ptr::null_mut(),
            show_fdinfo: core::ptr::null_mut(),
            copy_file_range: core::ptr::null_mut(),
            remap_file_range: core::ptr::null_mut(),
            fadvise: core::ptr::null_mut(),
            uring_cmd: core::ptr::null_mut(),
            uring_cmd_iopoll: core::ptr::null_mut(),
            mmap_prepare: core::ptr::null_mut(),
            splice_eof: core::ptr::null_mut(),
        };

        ops.open = Self::open as *mut isize;
        ops.release = Self::release as *mut isize;

        if T::HAS_URING_CMD {
            ops.uring_cmd = Self::uring_cmd as *mut isize;
        }

        ops
    };
}
