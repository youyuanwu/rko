# Feature: `KVec` — kernel-allocator-backed Vec

## Status: ✅ Implemented

See `rko-core/src/alloc/` for implementation, `samples/kvec_test/` for tests.

## Design Decisions

- **Custom `Allocator` trait** (not `core::alloc::Allocator`): every
  method takes `Flags` for per-allocation GFP flags (`GFP_KERNEL`, etc.)
- **No C helpers needed**: `rko.slab` partition extracts
  `krealloc_node_align_noprof` and `kfree` directly
- **GFP flags**: bit constants from `rko.gfp`, composed via `bitflags`
- **No panics on OOM**: all allocating methods return `Result`

## API

```rust
use rko_core::alloc::{KVec, Flags};

let mut v = KVec::new();
v.push(42, Flags::GFP_KERNEL)?;
v.extend_from_slice(&[1, 2, 3], Flags::GFP_KERNEL)?;
for x in v { /* ... */ }    // IntoIterator
```

## Future

- `VVec<T>` (vmalloc), `KVVec<T>` (kvmalloc fallback), `KBox<T>`
