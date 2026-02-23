# WIP: Kernel Module Init/Exit Bindings

Goal: produce the minimum `rko-sys` bindings needed to write an out-of-tree
kernel module with `module_init` / `module_exit`.

## Status: ✅ Complete

All steps done. `hello.ko` and `kvec_test.ko` build end-to-end (cargo + Kbuild).
Automated QEMU testing via CMake (`cmake --build build`, `ctest --test-dir build`).
See `samples/hello/` and `samples/kvec_test/` for working examples.

## What Was Built

| Component | File | Description |
|---|---|---|
| Generator | `rko-sys-gen/src/lib.rs` | bnd-winmd → winmd → windows-bindgen pipeline |
| Partition config | `rko-sys-gen/rko.toml` | 4 partitions (types, err, slab, gfp), kernel include paths, clang args |
| Types | `rko-sys/src/rko/types/mod.rs` | Generated: 119 typedefs, 10 structs, 3 constants |
| Errno | `rko-sys/src/rko/err/mod.rs` | Generated: 150 constants (EPERM..ENOGRACE) |
| Slab | `rko-sys/src/rko/slab/mod.rs` | Generated: 60 slab functions (kfree, krealloc_node_align_noprof, …) + constants/types |
| GFP bits | `rko-sys/src/rko/gfp/mod.rs` | Generated: 29 `___GFP_*_BIT` bit-position constants |
| Printk | `rko-core/src/printk.rs` | `_printk` extern, `KERN_*`, `pr_info!` macro family, `RawFormatter`, `rust_fmt_argument` |
| Module macros | `rko-core/src/module.rs` | `global_asm!`-based modinfo macros |
| Alloc | `rko-core/src/alloc/` | `Flags` (bitflags), `AllocError`, `Allocator` trait, `Kmalloc`, `Vec<T,A>`, `KVec<T>` |
| Build system | `cmake/kernel_module.cmake` | `add_kernel_module()` — cargo + ld + Kbuild + QEMU test |
| Kernel config | `samples/cargo-kernel.toml` | Shared kernel rustflags + `build-std` (passed via `--config`) |
| Test runner | `scripts/run-module-test.sh` | All-in-one QEMU test (initramfs + run + check) |
| Hello sample | `samples/hello/hello.rs` | init/exit + `pr_info!`, builds to `hello.ko` |
| KVec test | `samples/kvec_test/` | 7-test KVec exercise module, builds to `kvec_test.ko` |
| C helpers | `rko-sys/src/helpers.c`, `helpers.h` | Infrastructure for future macro/inline wrappers (currently empty) |

## Crate Structure

| Crate | Purpose |
|---|---|
| `rko-sys` | Generated FFI bindings (types, errno, slab, gfp) — `#![no_std]` |
| `rko-sys-gen` | Generator crate (bnd-winmd + windows-bindgen) |
| `rko-core` | Hand-written wrappers (printk, module, alloc) — `#![no_std]`, depends on `rko-sys` + `bitflags` |

## Design Decisions

### What is generated (bnd-winmd partitions)

- **`rko.types`** — `linux/types.h`: kernel typedefs (`__u8`–`__u64`, `pid_t`, `gfp_t`, …), structs (`atomic_t`, `list_head`, `callback_head`, …)
- **`rko.err`** — `linux/errno.h`: all `E*` constants including kernel-internal (ERESTARTSYS, EPROBE_DEFER, …)
- **`rko.slab`** — `linux/slab.h`: 60 slab allocator functions (`kfree`, `krealloc_node_align_noprof`, `kmalloc_noprof`, …), types (`kmem_cache_args`, …), constants
- **`rko.gfp`** — `linux/gfp_types.h`: 29 `___GFP_*_BIT` bit-position constants

### What is hand-written (rko-core)

- **`printk.rs`** — `_printk` extern, `KERN_*` constants, `RawFormatter`, `rust_fmt_argument` (`%pA` callback), `call_printk`, `set_log_prefix`, `pr_info!` through `pr_cont!`
- **`module.rs`** — `.modinfo` section macros using `global_asm!`
- **`alloc/`** — `Flags` (bitflags over generated `___GFP_*_BIT`), `AllocError`, `Allocator` trait, `Kmalloc` (calls generated `krealloc_node_align_noprof` + `kfree`), `Vec<T, A>`, `KVec<T>`

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
- Add `VVec<T>` (vmalloc) and `KVVec<T>` (kvmalloc) allocator variants
- Add `KBox<T>` — single-element kernel-allocated box
- Reduce `.ko` size by using kernel's pre-compiled core instead of build-std
