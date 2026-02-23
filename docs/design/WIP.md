# WIP: Kernel Module Init/Exit Bindings

Goal: produce the minimum `rko-sys` bindings needed to write an out-of-tree
kernel module with `module_init` / `module_exit`.

## Status: ✅ Complete

All steps done. `hello.ko` builds end-to-end (cargo + Kbuild).
Automated QEMU testing via `make test` / `ctest`.
See `samples/hello/` for the full working example.

## What Was Built

| Component | File | Description |
|---|---|---|
| Generator | `rko-sys-gen/src/lib.rs` | bnd-winmd → winmd → windows-bindgen pipeline |
| Partition config | `rko-sys-gen/rko.toml` | 2 partitions, kernel include paths, clang args |
| Types | `rko-sys/src/rko/types/mod.rs` | Generated: 119 typedefs, 10 structs, 3 constants |
| Errno | `rko-sys/src/rko/err/mod.rs` | Generated: 150 constants (EPERM..ENOGRACE) |
| Printk | `rko-core/src/printk.rs` | `_printk` extern, `KERN_*`, `pr_info!` macro family, `RawFormatter`, `rust_fmt_argument` |
| Module macros | `rko-core/src/module.rs` | `global_asm!`-based modinfo macros |
| Kbuild config | `samples/hello/Kbuild` | Module object declaration |
| Build wrapper | `samples/hello/Makefile` | cargo + `ld --whole-archive` + `make -C` + `test` target |
| Cargo config | `samples/hello/cargo-kernel.toml` | Kernel rustflags + `build-std` (passed via `--config`) |
| Sample module | `samples/hello/hello.rs` | init/exit + `pr_info!`, builds to `hello.ko` |
| Test scripts | `scripts/init.sh`, `make-initramfs.sh`, `run-qemu-test.sh` | QEMU-based automated testing |

## Crate Structure

| Crate | Purpose |
|---|---|
| `rko-sys` | Generated FFI bindings (types, errno) — `#![no_std]` |
| `rko-sys-gen` | Generator crate (bnd-winmd + windows-bindgen) |
| `rko-core` | Hand-written wrappers (printk, module macros) — `#![no_std]`, depends on `rko-sys` |

## Design Decisions

### What is generated (bnd-winmd partitions)

- **`rko.types`** — `linux/types.h`: kernel typedefs (`__u8`–`__u64`, `pid_t`, `gfp_t`, …), structs (`atomic_t`, `list_head`, `callback_head`, …)
- **`rko.err`** — `linux/errno.h`: all `E*` constants including kernel-internal (ERESTARTSYS, EPROBE_DEFER, …)

### What is hand-written (rko-core)

- **`printk.rs`** — `_printk` extern, `KERN_*` constants, `RawFormatter`, `rust_fmt_argument` (`%pA` callback), `call_printk`, `set_log_prefix`, `pr_info!` through `pr_cont!`
- **`module.rs`** — `.modinfo` section macros using `global_asm!`

### What the module author provides

- `init_module` / `cleanup_module` — `#[unsafe(no_mangle)] pub extern "C" fn` with `#[unsafe(link_section = ".init.text")]` / `.exit.text`
- Addressability markers — `#[used] #[unsafe(link_section = ".init.data")]` static fn pointers
- Panic handler — `#[panic_handler]` (upstream kernel uses `pr_emerg!` + `BUG()`)

## Known Bugs

See `docs/design/bugs/` for details:

| Bug | Severity | Status |
|---|---|---|
| `typedef _Bool bool` → recursive type alias | Build-breaking | ✅ Fixed (bnd-winmd 0.0.3) |
| `__int128` → `isize` (should be `i128`/`u128`) | Low | ✅ Fixed (bnd-winmd 0.0.3) |
| Function pointer struct field → `*mut isize` | Medium | Open — bnd-winmd |
| `phys_addr_t`/`dma_addr_t` → `u32` (missing autoconf.h) | Medium | ✅ Fixed (rko.toml config) |

## Next Steps

- Add `rko.module` partition (`struct module`) for device/filesystem registration
- Add `slab`, `fs` partitions for richer kernel API coverage
- Reduce `.ko` size by using kernel's pre-compiled core instead of build-std
