# WIP: rko Development Status

## Status: ✅ Core Complete

`hello.ko` and `kvec_test.ko` build end-to-end and pass QEMU tests.

## Crate Structure

| Crate | Purpose |
|---|---|
| `rko-sys` | Generated FFI bindings (types, errno, slab, gfp) — `#![no_std]` |
| `rko-sys-gen` | Generator: kernel headers → bnd-winmd → Rust FFI |
| `rko-core` | Wrappers: `Module` trait, `module!` macro, `Error`, `pr_info!`, `KVec<T>` |

## Generated partitions (rko-sys)

| Partition | Header | Contents |
|---|---|---|
| `rko.types` | `linux/types.h` | 119 typedefs, 10 structs |
| `rko.err` | `linux/errno.h` | 150 errno constants |
| `rko.slab` | `linux/slab.h` | 60 slab functions (kfree, krealloc, …) |
| `rko.gfp` | `linux/gfp_types.h` | 29 GFP bit-position constants |

## Hand-written (rko-core)

| Module | Key items |
|---|---|
| `module.rs` | `Module` trait, `module!` macro, modinfo macros |
| `error.rs` | `Error` type (wraps errno) |
| `prelude.rs` | Re-exports for module authors |
| `printk.rs` | `_printk` extern, `pr_info!` macros, `RawFormatter` |
| `alloc/` | `Flags`, `Allocator`, `Kmalloc`, `KVec<T>` |

## Known Bugs

See `docs/design/bugs/` for details.

## Next Steps

- `InPlaceModule` + `pin-init` for pinned module state (Phase 2)
- VFS bindings + ROFS filesystem support
- `KBox<T>`, `VVec<T>` allocator variants
- Reduce `.ko` size (kernel's pre-compiled core)
