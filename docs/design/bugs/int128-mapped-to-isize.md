# ✅ Fixed (bnd-winmd 0.0.3): `__int128` types emitted as `isize`

## Summary

`__s128` and `__u128` (kernel `__int128` typedefs) are emitted as
`pub type __s128 = isize;` (64-bit) instead of a 128-bit type.

## Observed

```rust
// rko-sys/src/rko/types/mod.rs (generated)
pub type __s128 = isize;   // wrong — isize is 64 bits on x86_64
pub type __u128 = isize;
```

## Expected

These should map to Rust's `i128` / `u128`:

```rust
pub type __s128 = i128;
pub type __u128 = u128;
```

## Root Cause

The kernel headers define:

```c
// uapi/linux/types.h
typedef __signed__ __int128 __s128 __attribute__((aligned(16)));
typedef unsigned __int128 __u128 __attribute__((aligned(16)));
```

libclang reports the canonical type as `Int128` / `UInt128`. bnd-winmd's
`ctype_from_clang()` in `extract.rs` likely doesn't have a mapping for
`TypeKind::Int128` / `TypeKind::UInt128` and falls through to a default
that picks `isize` (pointer-sized integer).

## Impact

Low — `__s128` / `__u128` are rarely used in kernel APIs. They exist
mainly for internal 128-bit arithmetic helpers.

## Suggested Fix

In `extract.rs`, add `TypeKind::Int128 => CType::I128` and
`TypeKind::UInt128 => CType::U128` mappings. If `CType` doesn't have
128-bit variants, add them to `model.rs` and map to
`ELEMENT_TYPE_I8`/`ELEMENT_TYPE_U8` with a 16-byte size in the winmd
emission (or use a `[u8; 16]` struct workaround).

## Resolution

Fixed in bnd-winmd 0.0.3. The `__s128`, `__u128`, `s128`, and `u128`
typedefs are now suppressed entirely rather than emitting incorrect
`isize` mappings.
