# Feature: `pr_info!` and friends

## Status: ✅ Implemented

See `rko-core/src/printk.rs` for implementation.

## Design

Uses the kernel's `%pA` format specifier: `_printk` calls back into
`rust_fmt_argument` (exported by our module) which renders
`core::fmt::Arguments` directly into `vsprintf`'s output buffer via
`RawFormatter` — no intermediate allocation.

```
pr_info!("x = {}\n", val)
  → _printk("\x016%s: %pA\0", LOG_PREFIX, &args)
    → rust_fmt_argument(buf, end, &args)
      → RawFormatter::write_fmt(args)
```

Log prefix is set once during init via `set_log_prefix()` (now handled
automatically by the `module!` macro).

Requires `CONFIG_RUST=y` in the running kernel for `%pA` support.
