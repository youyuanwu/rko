# Bug: Function pointer struct fields emitted as `*mut isize`

## Summary

`struct callback_head` has a field `void (*func)(struct callback_head *)`,
which is a function pointer. The generated binding emits it as
`pub func: *mut isize` instead of a proper function pointer type.

## Observed

```rust
// rko-sys/src/rko/types/mod.rs (generated)
pub struct callback_head {
    pub next: *mut callback_head,
    pub func: *mut isize,           // wrong — should be a fn pointer
}
```

## Expected

```rust
pub struct callback_head {
    pub next: *mut callback_head,
    pub func: Option<unsafe extern "C" fn(*mut callback_head)>,
}
```

## Root Cause

The kernel header defines:

```c
// linux/types.h
struct callback_head {
    struct callback_head *next;
    void (*func)(struct callback_head *head);
};
```

bnd-winmd extracts function pointer types correctly for typedef-level
declarations (e.g., `rcu_callback_t` is correctly emitted as
`Option<unsafe extern "system" fn(...)>`). However, when a function
pointer appears as a **struct field** without a typedef, the extraction
falls back to an opaque pointer (`*mut isize`).

This is the same pattern noted in bnd's own WIP.md for function pointer
parameters — the winmd `*const isize` encoding is used as a placeholder
for opaque function pointers.

## Impact

Medium — `callback_head` (aliased as `rcu_head`) is used pervasively in
the kernel for RCU callbacks, timer callbacks, and work queue items. Code
using this struct must cast the `*mut isize` field to a function pointer,
which is error-prone.

## Suggested Fix

In `extract.rs`, when processing struct fields with
`TypeKind::FunctionProto` or `TypeKind::Pointer` → `TypeKind::FunctionProto`,
emit a proper delegate/function-pointer type instead of falling back to
`*mut isize`.
