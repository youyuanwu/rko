# Bug: `phys_addr_t` / `dma_addr_t` emitted as `u32` despite 64-bit config

## Summary

`phys_addr_t` and `dma_addr_t` are emitted as `u32` when they should be
`u64` on x86_64 with `CONFIG_PHYS_ADDR_T_64BIT=1`.

## Observed

```rust
// rko-sys/src/rko/types/mod.rs (generated)
pub type dma_addr_t = u32;
pub type phys_addr_t = u32;
pub type resource_size_t = phys_addr_t;
```

## Expected

```rust
pub type dma_addr_t = u64;
pub type phys_addr_t = u64;
pub type resource_size_t = phys_addr_t;
```

## Root Cause

The kernel header uses `#ifdef CONFIG_PHYS_ADDR_T_64BIT`:

```c
// linux/types.h
#ifdef CONFIG_PHYS_ADDR_T_64BIT
typedef u64 dma_addr_t;
#else
typedef u32 dma_addr_t;
#endif
```

This config macro is defined in `linux_bin/include/generated/autoconf.h`:

```c
#define CONFIG_PHYS_ADDR_T_64BIT 1
```

The `rko.toml` clang args currently include `-D__KERNEL__` and `-DMODULE`
but do **not** pass `-include .../autoconf.h`. Without this, libclang
does not see `CONFIG_PHYS_ADDR_T_64BIT` and takes the `#else` (32-bit)
branch.

## Impact

High — using `u32` for DMA/physical addresses on a 64-bit kernel is
silently wrong and would cause truncation bugs at runtime.

## Status: ✅ Fixed

Added `-include generated/autoconf.h` to `clang_args` in both partitions
of `rko-sys-gen/rko.toml`. Since `../linux_bin/include` is already in
`include_paths`, clang's `-include` resolves through the search path.

Re-generated output now correctly produces:
```rust
pub type dma_addr_t = u64;
pub type phys_addr_t = u64;
```
