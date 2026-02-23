# Feature: `KVec` — kernel-allocator-backed Vec

## Status: ✅ Implemented

All steps complete. `kvec_test.ko` builds and passes 7 sub-tests in QEMU.
See `samples/kvec_test/` for a working example.

## Goal

Provide a `Vec<T, A>` type in `rko-core` that uses kernel memory allocators
(`kmalloc`/`krealloc`/`kfree`) instead of the standard library allocator.
This is equivalent to the upstream kernel's `KVec<T>` from
`rust/kernel/alloc/kvec.rs`.

```rust
use rko_core::alloc::{KVec, Flags, AllocError};

let mut v = KVec::new();
v.push(42, Flags::GFP_KERNEL)?;
```

## Background

Kernel code cannot use `std::vec::Vec` because:
1. No standard allocator — kernel uses `kmalloc`/`kfree` with GFP flags
2. Allocation can fail — kernel must handle `ENOMEM` gracefully (no panic)
3. Context matters — GFP flags control whether allocation may sleep, use DMA, etc.

The upstream kernel (6.19) has a full `Vec<T, A>` parameterized over an
`Allocator` trait, with type aliases `KVec<T>` (kmalloc), `VVec<T>` (vmalloc),
`KVVec<T>` (kvmalloc fallback). We start with `KVec` only.

### Why not `alloc::Vec` with custom allocator?

Rust's `alloc::vec::Vec` supports custom allocators via the unstable
`allocator_api` feature. However, the standard `core::alloc::Allocator`
trait takes only a `Layout` — it has no concept of per-call GFP flags.
The kernel requires per-allocation flags (`GFP_KERNEL` for sleepable
contexts, `GFP_ATOMIC` for interrupt/spinlock contexts), so we need a
custom `Allocator` trait where every method takes `Flags`. This matches
the upstream kernel's design.

## Design

### Layered architecture

```
rko-sys/src/rko/
├── slab/mod.rs     — Generated: kfree, krealloc_node_align_noprof, 60 slab functions
├── gfp/mod.rs      — Generated: 29 ___GFP_*_BIT constants
├── helpers.c       — C wrappers for future macros/inlines (currently empty)
├── helpers.h       — Declarations for helpers.c (currently empty)

rko-core/src/alloc/
├── mod.rs          — Flags (bitflags), AllocError, Allocator trait, re-exports
├── allocator.rs    — Kmalloc impl (calls krealloc_node_align_noprof + kfree from rko.slab)
├── kvec.rs         — Vec<T, A> + KVec type alias + IntoIter
└── layout.rs       — array_layout helper (safe size calculations)
```

### Dependencies on rko-sys

The allocator uses these kernel FFI symbols from generated partitions:

| Symbol | Partition | Kind | Status |
|--------|-----------|------|--------|
| `___GFP_*_BIT` (29 constants) | `rko.gfp` | constants | ✅ Generated |
| `krealloc_node_align_noprof` | `rko.slab` | function | ✅ Generated |
| `kfree` | `rko.slab` | function | ✅ Generated |
| `GFP_KERNEL`, `GFP_ATOMIC` etc. | — | compound flags | Hand-written in `rko-core` (bitflags) |

The `rko.slab` partition (from `linux/slab.h`) extracts real exported
functions directly — `krealloc_node_align_noprof` is the actual kernel
function behind the `krealloc_node_align` macro. No C helper wrappers
are needed for the current allocator.

The `rko.gfp` partition generates the `___GFP_*_BIT` bit-position constants.
The high-level flags (`GFP_KERNEL = __GFP_RECLAIM | __GFP_IO | __GFP_FS`)
are composed in `rko-core` using the `bitflags` crate since they involve
complex C macros that bnd-winmd cannot extract.

### C helper wrappers (future use)

Kernel functions that are macros or static inlines cannot be called directly
from Rust. The `helpers.c` / `helpers.h` infrastructure is in place for
future use, following the pattern from the upstream kernel
(`linux/rust/helpers/*.c`) and the lnx project.

### Component 1: `Flags`

GFP allocation flags, matching the kernel's `gfp_t`. Uses the `bitflags`
crate (`no_std`-compatible) with values derived from the generated
`___GFP_*_BIT` constants in `rko-sys`:

```rust
use rko_sys::rko::gfp::*;

bitflags::bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct Flags: u32 {
        const __GFP_DMA            = 1 << ___GFP_DMA_BIT;
        const __GFP_HIGHMEM        = 1 << ___GFP_HIGHMEM_BIT;
        const __GFP_DMA32          = 1 << ___GFP_DMA32_BIT;
        const __GFP_HIGH           = 1 << ___GFP_HIGH_BIT;
        const __GFP_IO             = 1 << ___GFP_IO_BIT;
        const __GFP_FS             = 1 << ___GFP_FS_BIT;
        const __GFP_ZERO           = 1 << ___GFP_ZERO_BIT;
        const __GFP_DIRECT_RECLAIM = 1 << ___GFP_DIRECT_RECLAIM_BIT;
        const __GFP_KSWAPD_RECLAIM = 1 << ___GFP_KSWAPD_RECLAIM_BIT;
        const __GFP_NOWARN         = 1 << ___GFP_NOWARN_BIT;

        const __GFP_RECLAIM = Self::__GFP_DIRECT_RECLAIM.bits() | Self::__GFP_KSWAPD_RECLAIM.bits();

        const GFP_KERNEL = Self::__GFP_RECLAIM.bits() | Self::__GFP_IO.bits() | Self::__GFP_FS.bits();
        const GFP_ATOMIC = Self::__GFP_HIGH.bits() | Self::__GFP_KSWAPD_RECLAIM.bits();
        const GFP_NOWAIT = Self::__GFP_KSWAPD_RECLAIM.bits() | Self::__GFP_NOWARN.bits();
    }
}
```

### Component 2: `Allocator` trait

```rust
pub unsafe trait Allocator {
    unsafe fn realloc(
        ptr: Option<NonNull<u8>>,
        layout: Layout,
        old_layout: Layout,
        flags: Flags,
    ) -> Result<NonNull<[u8]>, AllocError>;

    unsafe fn free(ptr: NonNull<u8>, layout: Layout);
}
```

### Component 3: `Kmalloc`

```rust
pub struct Kmalloc;

unsafe impl Allocator for Kmalloc {
    unsafe fn realloc(...) -> Result<NonNull<[u8]>, AllocError> {
        // Calls rust_helper_krealloc_node_align(ptr, size, align, flags, NUMA_NO_NODE)
    }

    unsafe fn free(ptr: NonNull<u8>, _layout: Layout) {
        // kfree accepts NULL safely — no null check needed
        kfree(ptr.as_ptr().cast());
    }
}
```

### Component 4: `Vec<T, A>`

Core struct matching upstream:

```rust
pub struct Vec<T, A: Allocator> {
    ptr: NonNull<T>,
    len: usize,
    capacity: usize,
    _alloc: PhantomData<A>,
}

pub type KVec<T> = Vec<T, Kmalloc>;
```

**Key API** (all take `Flags` parameter for allocations):

| Method | Description |
|--------|-------------|
| `new()` | Empty vec (no allocation) |
| `with_capacity(cap, flags)` | Pre-allocate capacity |
| `push(val, flags)` | Append, grow if needed |
| `pop()` | Remove last |
| `len()` / `capacity()` / `is_empty()` | Getters |
| `as_slice()` / `as_mut_slice()` | Borrow as slice |
| `extend_from_slice(s, flags)` | Bulk append (Clone) |
| `clear()` | Remove all (without dealloc) |
| `truncate(len)` | Shorten |
| `Deref<Target=[T]>` | Slice coercion |
| `Drop` | Calls `A::free` |
| `IntoIterator` | Consuming iteration |

**Growth strategy**: double capacity (minimum 1), matching upstream kernel.

**Error handling**: `push`, `with_capacity`, `extend_from_slice` return
`Result<_, AllocError>`. No panics on OOM.

### Component 5: `AllocError`

```rust
#[derive(Copy, Clone, Debug)]
pub struct AllocError;
```

Can be converted to the kernel error code `-ENOMEM` when we add
error type support.

## Implementation Plan

All steps complete:

| Step | Description | Status |
|------|-------------|--------|
| 1 | Add `rko.slab` partition to `rko.toml` — generates `kfree` + `krealloc_node_align_noprof` | ✅ |
| 2 | Add `bitflags` dependency to `rko-core/Cargo.toml` | ✅ |
| 3 | Implement `Flags`, `AllocError`, `Allocator` trait in `rko-core/src/alloc/mod.rs` | ✅ |
| 4 | Implement `Kmalloc` in `rko-core/src/alloc/allocator.rs` | ✅ |
| 5 | Implement `Vec<T, A>` + `IntoIter` in `rko-core/src/alloc/kvec.rs` | ✅ |
| 6 | Add `KVec` type alias + `layout.rs` helper | ✅ |
| 7 | Create `samples/kvec_test/` module with 7 sub-tests | ✅ |
| 8 | Test in QEMU — all tests pass | ✅ |

**Design change**: C helper wrappers turned out to be unnecessary — the
`rko.slab` partition extracts `krealloc_node_align_noprof` (the real
function behind the `krealloc_node_align` macro) and `kfree` directly
from `linux/slab.h`. The helpers infrastructure is kept empty for future
macros/static-inlines that do need C wrappers.

## Scope

| In scope | Out of scope |
|----------|-------------|
| `KVec<T>` (Kmalloc-backed) | `VVec<T>` (Vmalloc) |
| `push`, `pop`, `with_capacity`, `extend_from_slice` | `KBox<T>` |
| `GFP_KERNEL`, `GFP_ATOMIC` flags | Full GFP flag set |
| `Deref`/`Drop`/`IntoIterator` | NUMA node selection |
| `kvec!` macro | `try_reserve` / `reserve_exact` |

## Example usage

```rust
use rko_core::alloc::{KVec, Flags, AllocError};

fn create_numbers(n: usize) -> Result<KVec<i32>, AllocError> {
    let mut v = KVec::with_capacity(n, Flags::GFP_KERNEL)?;
    for i in 0..n {
        v.push(i as i32, Flags::GFP_KERNEL)?;
    }
    Ok(v)
}
```
