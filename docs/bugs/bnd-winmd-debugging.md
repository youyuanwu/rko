# bnd-winmd: Debugging Improvements

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.4

## Implemented

### Pre-emit TypeRef validation (was #1)

bnd-winmd 0.0.3+ validates TypeRef ↔ TypeDef before calling
windows-bindgen. Reports **all** unresolved types in one run:

```
1 unresolved type reference(s) found:
  • `raw_spinlock` — referenced in field `rlock` of struct `spinlock__anon_0` (partition `rko.sync`)
```

This catches the classes of missing types that previously required
repeated runs to discover.

## Open Feature Requests

### 1. Silent extraction failure warning

When a partition extracts 0 types, emit a warning. This catches
misconfigured traverse lists or header path issues that currently
produce no output and no error:

```
WARN  partition rko.sync extracted 0 structs, 0 functions — check headers/traverse paths
```

The multi-header wrapper path bug (now fixed) silently produced empty
partitions. A warning would have caught it immediately.

### 2. `--dry-run` / `--validate` mode

A CLI flag that runs clang extraction + winmd emit but skips
`windows-bindgen` codegen. It would:

- Print partition stats (structs, functions, typedefs, enums)
- Run the TypeRef ↔ TypeDef validation
- Report all unresolved types grouped by partition
- Exit with non-zero if there are issues

This gives a fast feedback loop without waiting for codegen.

### 3. Struct size validation against clang

After extraction, optionally compare `clang_Type_getSizeOf()` for each
extracted struct against the sum of field sizes in the generated Rust
struct. Report mismatches:

```
WARN  struct inode: clang sizeof=592, generated fields sum=544 (48 byte gap)
```

This would have caught the anonymous union bug (missing 48 bytes in
`struct inode`) and the `____cacheline_aligned` bug (200 vs 256 bytes
in `inode_operations`) before any runtime crash.

### 4. Dump registry contents

A `RUST_LOG=debug` trace that prints the full `TypeRegistry` after
extraction — every type name, its namespace, and whether it was
extracted or injected:

```
TypeRegistry (342 entries):
  address_space → rko.fs (extracted)
  cred → rko.fs (injected, 184 bytes)
  spinlock → rko.sync (extracted)
  ...
```

### 5. Inject_type size cross-check

When an `[[inject_type]]` struct is also extractable from headers
(e.g. it's in a traverse file), compare the injected size against
clang's `sizeof`. Report mismatches:

```
WARN  inject_type spinlock: declared size=4, clang sizeof=4 ✓
WARN  inject_type rw_semaphore: declared size=40, clang sizeof=48 ✗
```

This would catch stale inject_type sizes after kernel upgrades.

### 6. Duplicate type reporting

When multiple partitions extract the same type, report which partition
wins and which is dropped. Currently this is `WARN` level but only
shows when `RUST_LOG=warn`:

```
WARN  dropping duplicate struct spinlock: canonical=rko.fs, duplicate=rko.sync
```

Suggest making this visible by default when duplicates exist, so users
know to remove the inject_type.

## Priority

| Feature | Impact | Effort |
|---|---|---|
| #1 Silent extraction warning | High — catches misconfig immediately | Trivial |
| #2 `--dry-run` | Medium — fast feedback loop | Low |
| #3 Struct size validation | High — catches layout bugs pre-crash | Medium |
| #4 Dump registry | Medium — debugging visibility | Trivial |
| #5 Inject size cross-check | Medium — catches stale sizes | Low |
| #6 Duplicate reporting | Low — convenience | Trivial |

\#3 (struct size validation) would have saved the most debugging time
in this project — the `inode` offset mismatch and `inode_operations`
padding issue both caused kernel crashes that took hours to diagnose.
