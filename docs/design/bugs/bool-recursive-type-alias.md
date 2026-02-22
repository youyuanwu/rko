# Bug: `typedef _Bool bool` produces recursive type alias

## Summary

When the kernel header `linux/types.h` contains `typedef _Bool bool;`,
bnd-winmd + windows-bindgen emits `pub type bool = bool;` which is a
recursive type alias and fails to compile.

## Observed

```rust
// rko-sys/src/rko/types/mod.rs (generated)
pub type bool = bool;
```

```
error[E0391]: cycle detected when expanding type alias `rko::types::bool`
```

## Expected

The typedef should either be suppressed entirely (since `_Bool` maps
directly to Rust's `bool` primitive) or emitted with the primitive:

```rust
// Option A: suppress — no output for `typedef _Bool bool`
// Option B: emit with explicit primitive
pub type bool = core::primitive::bool;
```

## Root Cause

The C kernel header defines:

```c
// linux/types.h
typedef _Bool bool;
```

libclang reports `_Bool` → `Bool` type kind, which bnd-winmd maps to
Rust `bool`. The typedef name is also `bool`, producing the self-referential
alias `pub type bool = bool;`.

## Impact

Build-breaking — the generated code does not compile without manual
patching.

## Suggested Fix

In the winmd emission or post-processing, detect when a typedef name
collides with its underlying Rust primitive type name (`bool`, `i8`,
`u8`, `i16`, `u16`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`, `isize`,
`usize`) and either:

1. Skip emitting the typedef entirely, or
2. Qualify the RHS with `core::primitive::` prefix.

Option 1 is simpler and matches how the kernel's own bindgen handles it
(bindgen does not emit `type bool = bool;`).
