//! Kernel printk facilities.
//!
//! Provides `_printk` FFI binding, `KERN_*` level constants,
//! `RawFormatter`, `rust_fmt_argument`, and the `pr_*!` macro family.

use core::ffi::{c_char, c_void};
use core::fmt;

// ---------------------------------------------------------------------------
// FFI
// ---------------------------------------------------------------------------

unsafe extern "C" {
    /// Kernel printk implementation.
    ///
    /// This is the actual symbol exported by the kernel. The `printk` name
    /// in C is a macro that expands to `_printk`.
    pub safe fn _printk(fmt: *const c_char, ...) -> core::ffi::c_int;
}

// ---------------------------------------------------------------------------
// KERN_* level constants
// ---------------------------------------------------------------------------

/// ASCII Start Of Header — prefix byte used by all `KERN_*` level strings.
pub const KERN_SOH: &[u8; 2] = b"\x01\0";
pub const KERN_SOH_ASCII: u8 = b'\x01';

/// System is unusable (level 0).
pub const KERN_EMERG: &[u8; 3] = b"\x010\0";
/// Action must be taken immediately (level 1).
pub const KERN_ALERT: &[u8; 3] = b"\x011\0";
/// Critical conditions (level 2).
pub const KERN_CRIT: &[u8; 3] = b"\x012\0";
/// Error conditions (level 3).
pub const KERN_ERR: &[u8; 3] = b"\x013\0";
/// Warning conditions (level 4).
pub const KERN_WARNING: &[u8; 3] = b"\x014\0";
/// Normal but significant condition (level 5).
pub const KERN_NOTICE: &[u8; 3] = b"\x015\0";
/// Informational (level 6).
pub const KERN_INFO: &[u8; 3] = b"\x016\0";
/// Debug-level messages (level 7).
pub const KERN_DEBUG: &[u8; 3] = b"\x017\0";
/// Default kernel loglevel.
pub const KERN_DEFAULT: &[u8; 1] = b"\0";
/// Continuation of a previous log line.
pub const KERN_CONT: &[u8; 3] = b"\x01c\0";

// ---------------------------------------------------------------------------
// Log prefix
// ---------------------------------------------------------------------------

static mut LOG_PREFIX: *const u8 = c"<unknown>".as_ptr().cast::<u8>();

/// Set the module log prefix for `pr_*!` macros.
///
/// # Safety
///
/// Must be called with a null-terminated `&'static [u8]`.
/// Must only be called from module init (single-threaded context).
pub unsafe fn set_log_prefix(prefix: &'static [u8]) {
    unsafe {
        LOG_PREFIX = prefix.as_ptr();
    }
}

// ---------------------------------------------------------------------------
// RawFormatter
// ---------------------------------------------------------------------------

/// Writes directly into a raw `[buf, end)` byte range.
///
/// Used by `rust_fmt_argument` to render `fmt::Arguments` into the
/// kernel's `vsprintf` output buffer without allocation.
pub struct RawFormatter {
    pos: *mut u8,
    end: *mut u8,
}

impl RawFormatter {
    /// Create a new `RawFormatter` from raw pointers.
    ///
    /// # Safety
    ///
    /// `buf` must be valid for writes up to `end`. If `buf >= end` no
    /// bytes will be written.
    pub unsafe fn from_ptrs(buf: *mut u8, end: *mut u8) -> Self {
        Self { pos: buf, end }
    }

    /// Current write position.
    pub fn pos(&self) -> *mut u8 {
        self.pos
    }
}

impl fmt::Write for RawFormatter {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let len = bytes.len();
        let avail = (self.end as usize).saturating_sub(self.pos as usize);
        let to_write = if len < avail { len } else { avail };
        if to_write > 0 {
            // SAFETY: Caller of `from_ptrs` guarantees the range is valid.
            unsafe {
                core::ptr::copy_nonoverlapping(bytes.as_ptr(), self.pos, to_write);
            }
            self.pos = self.pos.wrapping_add(to_write);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// rust_fmt_argument — called by kernel vsprintf for %pA
// ---------------------------------------------------------------------------

/// Callback invoked by the kernel's `vsprintf` when it encounters `%pA`.
///
/// # Safety
///
/// Called from C with valid `buf`/`end` pointers and `ptr` pointing to
/// a `fmt::Arguments`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_fmt_argument(
    buf: *mut c_char,
    end: *mut c_char,
    ptr: *const c_void,
) -> *mut c_char {
    use fmt::Write;
    // SAFETY: The C contract guarantees that `buf` is valid if it's less than `end`.
    let mut w = unsafe { RawFormatter::from_ptrs(buf.cast(), end.cast()) };
    // SAFETY: The kernel passes a valid `&fmt::Arguments` via `ptr`.
    let _ = w.write_fmt(unsafe { *ptr.cast::<fmt::Arguments<'_>>() });
    w.pos().cast()
}

// ---------------------------------------------------------------------------
// Format strings
// ---------------------------------------------------------------------------

/// Pre-built format strings for `_printk`.
pub mod format_strings {
    /// Length of each format string.
    pub const LENGTH: usize = 10;

    const fn generate(is_cont: bool, prefix: &[u8; 3]) -> [u8; LENGTH] {
        assert!(prefix[0] == b'\x01');
        if is_cont {
            assert!(prefix[1] == b'c');
        } else {
            assert!(prefix[1] >= b'0' && prefix[1] <= b'7');
        }
        assert!(prefix[2] == b'\x00');

        let suffix: &[u8; LENGTH - 2] = if is_cont {
            b"%pA\0\0\0\0\0"
        } else {
            b"%s: %pA\0"
        };

        [
            prefix[0], prefix[1], suffix[0], suffix[1], suffix[2], suffix[3], suffix[4], suffix[5],
            suffix[6], suffix[7],
        ]
    }

    pub static EMERG: [u8; LENGTH] = generate(false, super::KERN_EMERG);
    pub static ALERT: [u8; LENGTH] = generate(false, super::KERN_ALERT);
    pub static CRIT: [u8; LENGTH] = generate(false, super::KERN_CRIT);
    pub static ERR: [u8; LENGTH] = generate(false, super::KERN_ERR);
    pub static WARNING: [u8; LENGTH] = generate(false, super::KERN_WARNING);
    pub static NOTICE: [u8; LENGTH] = generate(false, super::KERN_NOTICE);
    pub static INFO: [u8; LENGTH] = generate(false, super::KERN_INFO);
    pub static DEBUG: [u8; LENGTH] = generate(false, super::KERN_DEBUG);
    pub static CONT: [u8; LENGTH] = generate(true, super::KERN_CONT);
}

// ---------------------------------------------------------------------------
// call_printk
// ---------------------------------------------------------------------------

/// Calls `_printk` with a format string, the module log prefix, and
/// Rust `fmt::Arguments`.
///
/// # Safety
///
/// `format_string` must be one of the constants in [`format_strings`].
pub unsafe fn call_printk(format_string: &[u8; format_strings::LENGTH], args: fmt::Arguments<'_>) {
    unsafe {
        _printk(
            format_string.as_ptr().cast::<c_char>(),
            LOG_PREFIX,
            core::ptr::from_ref(&args).cast::<c_void>(),
        );
    }
}

/// Calls `_printk` for `KERN_CONT` (no module prefix).
pub fn call_printk_cont(args: fmt::Arguments<'_>) {
    _printk(
        format_strings::CONT.as_ptr().cast::<c_char>(),
        core::ptr::from_ref(&args).cast::<c_void>(),
    );
}

// ---------------------------------------------------------------------------
// print_macro! — internal dispatcher
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[macro_export]
macro_rules! print_macro {
    ($format_string:path, false, $($arg:tt)+) => {
        match format_args!($($arg)+) {
            args => unsafe {
                $crate::printk::call_printk(&$format_string, args);
            }
        }
    };
    ($format_string:path, true, $($arg:tt)+) => {
        $crate::printk::call_printk_cont(format_args!($($arg)+));
    };
}

// ---------------------------------------------------------------------------
// pr_*! public macros
// ---------------------------------------------------------------------------

/// Prints an emergency-level message (level 0).
#[macro_export]
macro_rules! pr_emerg {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::EMERG, false, $($arg)*)
    }
}

/// Prints an alert-level message (level 1).
#[macro_export]
macro_rules! pr_alert {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::ALERT, false, $($arg)*)
    }
}

/// Prints a critical-level message (level 2).
#[macro_export]
macro_rules! pr_crit {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::CRIT, false, $($arg)*)
    }
}

/// Prints an error-level message (level 3).
#[macro_export]
macro_rules! pr_err {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::ERR, false, $($arg)*)
    }
}

/// Prints a warning-level message (level 4).
#[macro_export]
macro_rules! pr_warn {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::WARNING, false, $($arg)*)
    }
}

/// Prints a notice-level message (level 5).
#[macro_export]
macro_rules! pr_notice {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::NOTICE, false, $($arg)*)
    }
}

/// Prints an info-level message (level 6).
#[macro_export]
macro_rules! pr_info {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::INFO, false, $($arg)*)
    }
}

/// Prints a debug-level message (level 7).
#[macro_export]
macro_rules! pr_debug {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            $crate::print_macro!($crate::printk::format_strings::DEBUG, false, $($arg)*)
        }
    }
}

/// Continues a previous log message in the same line.
#[macro_export]
macro_rules! pr_cont {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::CONT, true, $($arg)*)
    }
}
