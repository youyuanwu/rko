# Feature: `pr_info!` and friends — Rust `printk` macros

## Goal

Provide `pr_info!`, `pr_err!`, `pr_warn!`, etc. macros that accept Rust
`format_args!` syntax and print to the kernel log via `_printk`.

```rust
pr_info!("hello from {}\n", module_name);
```

## Design

### Kernel's `%pA` mechanism

The kernel's `vsprintf` (when `CONFIG_RUST=y`) supports a custom format
specifier `%pA`. When `_printk` encounters `%pA`, it calls the exported
Rust function `rust_fmt_argument(buf, end, ptr)` where `ptr` is a
`*const fmt::Arguments`. This callback renders the Rust format string
directly into `vsprintf`'s output buffer — no intermediate allocation.

### Architecture

```
pr_info!("x = {}\n", val)
        │
        ▼
print_macro!  ──►  format_args!("x = {}\n", val)
        │
        ▼
call_printk(format_string, args)
        │
        ▼
_printk("\x016%s: %pA\0", LOG_PREFIX, &args as *const c_void)
        │
        ▼  (kernel vsprintf sees %pA)
rust_fmt_argument(buf, end, ptr)
        │
        ▼
RawFormatter::write_fmt(*ptr)  ──►  bytes written into vsprintf buffer
```

### Components

| File | What |
|------|------|
| `rko-core/src/printk.rs` | `RawFormatter`, `call_printk`, `rust_fmt_argument`, format strings, `print_macro!` |
| `rko-core/src/lib.rs` | Re-export `pr_info!`, `pr_err!`, etc. |

### 1. `RawFormatter`

Wraps a `(*mut u8, *mut u8)` pair (cursor, end) and implements
`core::fmt::Write`. Writes bytes directly into the buffer without
allocation. Advances the cursor; stops silently at `end`.

```rust
pub struct RawFormatter {
    pos: *mut u8,
    end: *mut u8,
}

impl fmt::Write for RawFormatter {
    fn write_str(&mut self, s: &str) -> fmt::Result { ... }
}
```

### 2. `rust_fmt_argument`

Exported C function called by the kernel's `vsprintf` for `%pA`:

```rust
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_fmt_argument(
    buf: *mut c_char,
    end: *mut c_char,
    ptr: *const c_void,
) -> *mut c_char
```

Casts `ptr` to `&fmt::Arguments`, creates a `RawFormatter`, calls
`write_fmt`, returns the new cursor position.

### 3. Format strings

Pre-built `[u8; 10]` constants combining `KERN_*` prefix with `%s: %pA\0`:

```
KERN_INFO + "%s: %pA\0"  →  "\x016%s: %pA\0"
```

The `%s` is the module name (from `LOG_PREFIX` global). The `%pA` is the
Rust `fmt::Arguments` pointer.

For `KERN_CONT`, the format is just `"\x01c%pA\0"` (no module prefix).

### 4. `call_printk`

```rust
pub unsafe fn call_printk(
    format_string: &[u8; FORMAT_LEN],
    args: fmt::Arguments<'_>,
)
```

Reads the global `LOG_PREFIX` pointer and calls
`_printk(format_string.as_ptr(), LOG_PREFIX, &args as *const _ as *const c_void)`.

### 5. Log prefix

A global `static mut` pointer in `rko-core`, set once during module init:

```rust
static mut LOG_PREFIX: *const u8 = b"<unknown>\0".as_ptr();

/// Set the module log prefix for `pr_*!` macros.
///
/// # Safety
///
/// Must be called with a pointer to a `'static`, null-terminated byte
/// string. Must only be called from module init (single-threaded
/// context). The pointed-to data must remain valid for the lifetime of
/// the module.
pub unsafe fn set_log_prefix(prefix: &'static [u8]) {
    unsafe { LOG_PREFIX = prefix.as_ptr(); }
}
```

Modules call `unsafe { rko_core::set_log_prefix(b"hello\0") }` in
`init_module`. This avoids requiring a `__LOG_PREFIX` constant in scope
at every macro call site.

### 6. Public macros

Each macro delegates to `print_macro!`:

```rust
macro_rules! pr_info {
    ($($arg:tt)*) => {
        $crate::print_macro!($crate::printk::format_strings::INFO, false, $($arg)*)
    }
}
```

## Prerequisites

- `CONFIG_RUST=y` in the running kernel (for `%pA` support in `vsprintf`)
- `rust_fmt_argument` symbol must be exported (our module provides it)

## Scope

| In scope | Out of scope |
|----------|-------------|
| `pr_emerg!` through `pr_debug!`, `pr_cont!` | `module!` proc macro |
| `RawFormatter` + `rust_fmt_argument` | Dynamic debug (`pr_debug` gating) |
| `set_log_prefix` API | `dev_info!` (device-level logging) |

## Example usage

```rust
use rko_core::{pr_info, pr_err, printk};

unsafe extern "C" fn init_module() -> core::ffi::c_int {
    unsafe { printk::set_log_prefix(b"hello\0"); }
    pr_info!("module loaded, version {}\n", 1);
    0
}
```

Kernel log output:
```
[  12.345678] hello: module loaded, version 1
[  12.345679] hello: something went wrong: -22
```
