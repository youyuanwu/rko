# WIP: rko Development Status

## Status: ✅ Core Complete, VFS Bindings In Progress

`hello.ko` and `kvec_test.ko` build end-to-end and pass QEMU tests.
VFS partition bindings generated; ROFS implementation next.

## Crate Structure

| Crate | Purpose |
|---|---|
| `rko-sys` | Generated FFI bindings — `#![no_std]` |
| `rko-sys-gen` | Generator: kernel headers → bnd-winmd → Rust FFI |
| `rko-core` | Wrappers: `Module` trait, `module!` macro, `Error`, `pr_info!`, `KVec<T>`, types |

## Generated partitions (rko-sys)

| Partition | Header | Contents |
|---|---|---|
| `rko.types` | `linux/types.h` | Fundamental typedefs |
| `rko.err` | `linux/errno.h` | Errno constants |
| `rko.slab` | `linux/slab.h` | Slab allocator functions |
| `rko.gfp` | `linux/gfp_types.h` | GFP flag constants |
| `rko.fs` | `linux/fs.h` | VFS core (inode, file, super_block, ops) |
| `rko.fs_context` | `linux/fs_context.h` | Mount context |
| `rko.dcache` | `linux/dcache.h` | Dentry cache |
| `rko.pagemap` | `linux/pagemap.h` | Page/folio cache |
| `rko.highmem` | `linux/highmem.h` | High memory / kmap |
| `rko.helpers` | `helpers.h` | C wrappers for inline functions |

Each partition uses minimal traverse (one header) with `[[inject_type]]`
for dependent types. See `docs/bugs/bnd-winmd-inject-types.md`.

## Hand-written (rko-core)

| Module | Key items |
|---|---|
| `module.rs` | `Module` trait, `module!` macro, modinfo macros |
| `error.rs` | `Error` type (wraps errno) |
| `prelude.rs` | Re-exports for module authors |
| `printk.rs` | `_printk` extern, `pr_info!` macros, `RawFormatter` |
| `alloc/` | `Flags`, `Allocator`, `Kmalloc`, `KVec<T>` |
| `types/` | `ARef<T>`, `AlwaysRefCounted`, `Opaque<T>` |

## Next Steps

- ROFS filesystem implementation (folio, inode, super, registration)
- `InPlaceModule` + `pin-init` for pinned module state
- `KBox<T>`, `VVec<T>` allocator variants
