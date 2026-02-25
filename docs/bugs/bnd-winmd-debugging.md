# bnd-winmd: Debugging Improvements

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.3

## Problem

When `windows-bindgen` panics with `"type not found: rko.fs.kuid_t"`,
there is no way to find all such missing types without repeatedly
running the generator and hoping for different HashMap iteration order.
The panic fires on the **first** missing TypeDef hit, and which one
comes first is non-deterministic.

Diagnosing required building a separate tool to parse the `.winmd`
binary and diff TypeRef entries against TypeDef entries.

## Suggestions

### 1. Validate TypeRef ↔ TypeDef before calling windows-bindgen

After emitting the `.winmd`, bnd-winmd should read it back and check
that every `TypeRef` row has a corresponding `TypeDef` row. Report
**all** mismatches at once, not just the first.

The current `validate_type_references()` only catches `CType::Named`
with `resolved: None` that are missing from the registry. It misses:

- **Resolved typedefs** — `kuid_t` has `resolved: Some(struct kuid_t)`,
  so `collect_unresolved` skips it (`resolved.is_none()` is false). But
  `ctype_to_wintype` recurses into the resolved type and emits a TypeRef
  for `kuid_t` that has no matching TypeDef.
- **Types surfaced during codegen** — `vfsuid_t`, `mnt_idmap`, etc.
  appear in function signatures that bnd-winmd extracts but don't
  trigger the unresolved check because they have `resolved` values or
  come through typedef chains.

A post-emit winmd validation would catch both classes. The check is
straightforward: iterate the TypeRef table, look up each
`(namespace, name)` pair in the TypeDef table, report all misses.

```rust
fn validate_winmd(bytes: &[u8]) -> Vec<(String, String)> {
    // Parse winmd, collect TypeDef set, iterate TypeRef,
    // return (namespace, name) pairs with no matching TypeDef.
}
```

### 2. `--dry-run` / `--validate` mode

A CLI flag that runs the full pipeline (clang extraction → winmd emit)
but skips calling `windows-bindgen`. Instead it:

- Prints partition stats (structs, functions, typedefs, enums per
  partition)
- Runs the TypeRef ↔ TypeDef validation (#1 above)
- Reports all unresolved types grouped by partition
- Exits with non-zero if there are issues

This would give users a fast feedback loop without waiting for
windows-bindgen codegen.

### 3. Dump registry contents

A `--dump-registry` flag (or `RUST_LOG=debug` trace) that prints the
full `TypeRegistry` after extraction — every type name and its
namespace. This would immediately show which types are registered and
which are missing, without needing to read winmd internals.

```
TypeRegistry (342 entries):
  address_space → rko.fs
  address_space_operations → rko.fs
  callback_head → rko.types
  cred → rko.fs         (injected)
  dentry → rko.dcache
  ...
```

Injected types should be annotated so users can tell them apart from
extracted types.

### 4. Dump partition model

A `--dump-partitions` flag that prints the extracted model for each
partition before winmd emission:

```
Partition rko.fs (from linux/fs.h):
  structs: inode (1360 bytes), file (384 bytes), ...
  enums: inode_state_bits (3 variants), ...
  functions: register_filesystem, unregister_filesystem, ...
  typedefs: fmode_t → u32, ...
  injected: timespec64 (struct, 16 bytes), kuid_t (struct, 4 bytes), ...
```

This would help diagnose:
- Why a type isn't being extracted (not in traverse list)
- What types are being injected vs extracted
- Size mismatches between injected and real types

### 5. Better error from windows-bindgen

This is in `windows-bindgen` not `bnd-winmd`, but the panic at
`tables/field.rs:30` and `types/cpp_fn.rs:294` should be a structured
error listing **all** missing types, not a panic on the first one.
Could be raised upstream.

## Priority

| Suggestion | Impact | Effort |
|---|---|---|
| #1 Post-emit validation | High — catches all missing types in one run | Low |
| #2 `--dry-run` | Medium — fast feedback loop | Low |
| #3 Dump registry | Medium — immediate visibility | Trivial |
| #4 Dump partitions | Medium — full debugging | Low |
| #5 windows-bindgen error | High — but upstream | N/A |

\#1 alone would have saved hours of debugging in this project.
