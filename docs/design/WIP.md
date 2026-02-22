# WIP: Kernel Module Init/Exit Bindings

Goal: produce the minimum `rko-sys` bindings needed to write an out-of-tree
kernel module with `module_init` / `module_exit`.

## Status: ✅ Complete

All steps done. Workspace builds and passes clippy with zero warnings.
See `samples/hello/hello.rs` for a working minimal module.

## What Was Built

| Component | File | Description |
|---|---|---|
| Generator | `rko-sys-gen/src/lib.rs` | bnd-winmd → winmd → windows-bindgen pipeline |
| Partition config | `rko-sys-gen/rko.toml` | 2 partitions, kernel include paths, clang args |
| Types | `rko-sys/src/rko/types/mod.rs` | Generated: 119 typedefs, 10 structs, 3 constants |
| Errno | `rko-sys/src/rko/err/mod.rs` | Generated: 150 constants (EPERM..ENOGRACE) |
| Printk | `rko-sys/src/printk.rs` | Hand-written: `_printk` extern + KERN_* constants |
| Module macros | `rko-sys/src/module.rs` | Hand-written: `module_license!`, `module_author!`, `module_description!` |
| Sample | `samples/hello/hello.rs` | Minimal init/exit module, `#![no_std]`, clippy-clean |

## Design Decisions

### What is generated (bnd-winmd partitions)

- **`rko.types`** — `linux/types.h`: kernel typedefs (`__u8`–`__u64`, `pid_t`, `gfp_t`, …), structs (`atomic_t`, `list_head`, `callback_head`, …)
- **`rko.err`** — `linux/errno.h`: all `E*` constants including kernel-internal (ERESTARTSYS, EPROBE_DEFER, …)

### What is hand-written

- **`printk.rs`** — `_printk` is variadic (auto-skipped by bnd-winmd), `KERN_*` are string-literal macros
- **`module.rs`** — `.modinfo` section macros are pure Rust `#[unsafe(link_section)]` constructs

### What the module author provides

- `init_module` / `cleanup_module` — `#[unsafe(no_mangle)] pub extern "C" fn` with `#[unsafe(link_section = ".init.text")]` / `.exit.text`
- Addressability markers — `#[used] #[unsafe(link_section = ".init.data")]` static fn pointers
- Panic handler — `#[panic_handler]` (upstream kernel uses `pr_emerg!` + `BUG()`)

### Upstream kernel Rust printk reference

See `linux/rust/kernel/print.rs`. The kernel uses a three-layer approach:
1. bindgen extracts `_printk` (variadic) + `KERN_*` (`&[u8; 3]`)
2. `kernel::print::call_printk()` builds format strings like `"\x016%s: %pA\0"` using `%pA` + `rust_fmt_argument` callback
3. `pr_info!` / `pr_err!` macros wrap `call_printk`

For rko-sys, we use raw `_printk` with c-string literals: `printk::_printk(c"\x016hello\n".as_ptr())`

## Known Bugs

See `docs/design/bugs/` for details:

| Bug | Severity | Fix location |
|---|---|---|
| `typedef _Bool bool` → recursive type alias | Build-breaking | bnd-winmd |
| `__int128` → `isize` (should be `i128`/`u128`) | Low | bnd-winmd |
| Function pointer struct field → `*mut isize` | Medium | bnd-winmd |
| `phys_addr_t`/`dma_addr_t` → `u32` (missing autoconf.h) | ✅ Fixed | rko.toml config |

## Resolved Questions

1. **`-isystem` clang builtins** — Not needed for types/errno. Resolve at runtime for future partitions.
2. **`-include autoconf.h`** — ✅ Fixed. Added `-include generated/autoconf.h` to `clang_args` in `rko.toml`. Resolves via the `../linux_bin/include` search path.
3. **`library = "kernel"`** — No effect on current output (zero functions). Will revisit for function-bearing partitions.
4. **Macro constant extraction** — ✅ Confirmed working. All 150 errno constants extracted correctly.

## Next Steps

- Fix autoconf.h injection in generator for correct `CONFIG_*`-dependent types
- Add `rko.module` partition (`struct module`) for device/filesystem registration
- Add `slab`, `fs` partitions for richer kernel API coverage
- Kbuild integration to compile `samples/hello` into an actual `.ko`
