# bnd-winmd: unresolved type references from non-traversed headers

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.3  
**Status:** Resolved — using `[[inject_type]]`

## Summary

When a partition extracts a struct that contains an embedded field whose
type is defined in a header **not** in the traverse list, bnd-winmd
reports an "unresolved type reference" error — even though the type is
fully defined in clang's AST.

This is **not** a forward-declaration problem. The types are fully
`#include`-d and complete. The traverse filter prevents types from
non-traversed headers from entering the registry.

## Resolution

bnd-winmd v0.0.3 added `[[inject_type]]` support (see
`bnd-winmd-inject-types.md`). Each partition now uses a single-header
traverse with all dependent types declared as opaque injections in
`rko.toml`. This replaced ~190 traverse entries with ~40 inject entries
that are explicit and carry correct sizes.

### Two extraction limitations (still present in bnd-winmd)

1. **Bitfield enums** — `enum fs_value_type type:8;` in `fs_parameter`.
   Resolved via `[[inject_type]]` with `kind = "enum"`.

2. **Anonymous enums** — `enum { SYSCTL_TABLE_TYPE_DEFAULT, ... }` in
   `ctl_table_header`. Not needed for ROFS, so `sysctl.h` removed from
   traverse.

## Root cause

`should_emit()` filters entities by traverse file. A type like
`struct mutex` (defined in `mutex_types.h`) is never added to the
registry even when fully visible to clang. When `struct fs_context`
embeds `struct mutex uapi_mutex;`, the unresolved check fails.

The same pattern applies to all referenced types: typedefs (`kuid_t`,
`spinlock`, `rwlock_t`), structs (`super_block`, `task_struct`), and
enums (`migrate_mode`, `pid_type`).

## Multi-header wrapper bug

`PartitionConfig::wrapper_header()` writes `#include` directives with
relative paths that are invalid from `/tmp/`. Fix: canonicalize paths.
