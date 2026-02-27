# Adding Partitions to rko.toml

Guide for adding new bnd-winmd partitions to reduce `[[inject_type]]`
entries and get real struct layouts instead of opaque blobs.

## Background

Each partition in `rko-sys-gen/rko.toml` tells bnd-winmd to parse a
kernel header and extract types from it. Types referenced by a partition
but not defined in any traversed header must be declared via
`[[inject_type]]` — an opaque blob with manually-specified size and
alignment.

Currently there are **11 partitions** and **23 inject_types**. The
`rko.fs` partition traverse list includes 32 headers (the primary
`linux/fs.h` plus 31 dependency headers) which extract most embedded
types directly. Many injected structs could become real types if
their defining header were added as a traverse entry.

## When to add a partition

Add a partition when:

- Multiple inject_types come from the same header (eliminates several
  at once)
- You need field access into a currently-opaque type
- The header is self-contained (few transitive dependencies)

Do NOT add a partition when:

- The header pulls in massive dependency trees (e.g. `linux/sched.h`
  pulls `task_struct` → hundreds of types)
- Only one inject_type would be eliminated
- The type is only used as a pointer (opaque blob is fine)

## How to add a partition

### Step 1: Check the dependency fan-out

Before adding a header, estimate how many new unresolved types it will
introduce. Run bnd-winmd in verbose mode after adding the partition:

```sh
cargo run -p rko-sys-gen 2>&1
```

If it fails with `unresolved type reference(s)`, each missing type must
be either:
- Added as a new `[[inject_type]]` (if opaque/pointer-only)
- Covered by adding yet another partition (if you need its fields)

A header is a **good candidate** if it adds ≤5 new inject_types.
A header is a **bad candidate** if it adds 20+.

### Step 2: Add the partition

```toml
[[partition]]
namespace = "rko.spinlock"
library = "kernel"
headers = ["linux/spinlock_types.h"]
traverse = ["linux/spinlock_types.h"]
```

Key fields:
- **namespace**: `rko.<name>`. Must be unique. Becomes a Rust module
  (`rko-sys/src/rko/<name>/mod.rs`).
- **headers**: The header(s) to parse with libclang. Usually one.
- **traverse**: Which headers' types to extract. Start with just the
  primary header. Add sub-headers only if types you need are defined
  there.

### Step 3: Run the generator

```sh
cargo run -p rko-sys-gen
```

Fix any unresolved types by adding `[[inject_type]]` entries or
expanding `traverse`.

### Step 4: Remove replaced inject_types

Delete `[[inject_type]]` entries for types now covered by the new
partition. But keep inject_types for types in the new partition's
namespace that come from non-traversed transitive headers.

### Step 5: Add the feature to rko-sys

In `rko-sys/Cargo.toml`, add the feature flag and update dependencies:

```toml
[features]
spinlock = []
fs = ["dcache", "fs_context", "types", "spinlock"]  # add dep
```

### Step 6: Verify

```sh
cargo run -p rko-sys-gen       # generate
cargo clippy --workspace       # compile
cmake --build build --target all  # build .ko modules
ctest --test-dir build         # QEMU tests
```

## Partition candidates

Types grouped by defining header, sorted by inject_type reduction.
"Pointer-only" means the type is only used via `*mut T` in our
partitions (field access not needed — lower priority).

### Already done

The following inject_types have been eliminated by adding traverse
entries to existing partitions. No new partitions were needed — all
headers are transitively included by `linux/fs.h`.

**Sync partition** (separate partition):

| Partition | Headers | Types extracted | Inject_types removed |
|-----------|---------|-----------------|---------------------|
| `rko.sync` | `spinlock_types.h`, `mutex_types.h`, `rwsem.h`, `lockref.h`, `refcount_types.h`, `osq_lock.h`, `spinlock_types_raw.h`, `qspinlock_types.h` | `spinlock`, `mutex`, `rw_semaphore`, `lockref`, `refcount_struct`, `raw_spinlock`, `qspinlock`, `optimistic_spin_queue` + typedefs | 5 (net: 54→49) |

**Easy batch** (added to `rko.fs` traverse): `linux/time64.h`,
`linux/delayed_call.h`, `linux/path.h`, `linux/rbtree_types.h`,
`linux/migrate_mode.h`, `linux/pid_types.h`, `linux/rw_hint.h`,
`linux/llist.h`, `linux/list_bl.h`, `linux/workqueue_types.h`,
`linux/lockdep_types.h`, `linux/timer_types.h`.
Removed 12 inject_types: `timespec64`, `delayed_call`, `path`,
`rb_root_cached`, `migrate_mode`, `pid_type`, `rw_hint`, `llist_node`,
`hlist_bl_head`, `work_struct`, `lock_class_key`, `hlist_bl_node`
(dcache). (net: 48→36)

**Medium batch** (added to `rko.fs` traverse): `linux/wait.h`,
`linux/wait_bit.h`, `linux/stat.h`, `linux/shrinker.h`,
`linux/quota.h`, `linux/pid.h`, `linux/mount.h`,
`linux/percpu-rwsem.h`, `linux/maple_tree.h`, `linux/list_lru.h`,
`linux/xarray.h`. Also required dependency headers:
`linux/completion.h`, `linux/rcu_sync.h`, `linux/percpu_counter.h`,
`linux/swait.h`, `linux/projid.h`.
Removed 15 inject_types: `wait_queue_head`, `wait_bit_queue_entry`,
`kstat`, `shrinker`, `shrink_control`, `quota_info`, `dquot`,
`dquot_operations`, `quotactl_ops`, `pid`, `vfsmount`,
`percpu_rw_semaphore`, `maple_tree`, `list_lru`, `xarray`.
(net: 36→21)

**Hard batch** (mixed results):
- ✅ `linux/cred.h` → `cred` extracted (fs traverse, +
  `linux/capability.h`). Added 2 new inject_types: `key`,
  `user_struct`.
- ✅ `linux/user_namespace.h` → `user_namespace` extracted
  (fs_context traverse, + `linux/ns/ns_common_types.h`,
  `linux/list_nulls.h`). Added 2 new inject_types:
  `ctl_table_set`, `ctl_table_header`.
- ❌ `linux/mm_types.h` → cascades into arch types (`pgd_t`,
  `mm_context_t`). Creates more inject_types than it removes.
- ❌ `linux/module.h` → 11 unresolved types. Too heavy.
- ❌ `linux/sched.h` → not attempted (42 includes).

**Bitfield/enum fix** (bnd `0ee989f`): `fs_value_type` now extracted
via supplemental scan. (net: 49→48)

### Cannot be extracted (require `[[inject_type]]`)

| Type | Reason |
|------|--------|
| `kuid_t`, `kgid_t` | Typedefs to opaque structs in `<linux/uidgid.h>` |
| `vfsuid_t`, `vfsgid_t` | Wrappers around `kuid_t`/`kgid_t` |
| `seqcount_spinlock` | Generated by `SEQCOUNT_LOCKNAME` macro |
| `rwlock_t` | Defined via macros in spinlock headers |
| `file_ref_t` | `typedef atomic_long_t` |
| `uuid_t` | Defined in `linux/uuid.h` as `u8[16]` |
| `mnt_idmap` | Forward-declared, definition is in `fs/` (not headers) |
| `file_dedupe_range` | UAPI header, not kernel-internal |
| `vm_fault` | Defined in `linux/mm.h` (4000+ lines) |
| `wait_queue_entry` | Duplicate with pagemap partition |
| `folio`, `page`, `vm_area_struct`, `vm_area_desc`, `freeptr_t` | `mm_types.h` cascades too deeply |
| `task_struct` | `sched.h` cascades too deeply |
| `module` | `module.h` has 11+ unresolved deps |
| `key`, `user_struct` | Pointer-only deps of `cred` |
| `ctl_table_set`, `ctl_table_header` | Anonymous enum in `sysctl.h` blocks extraction |

## Getting sizes for inject_types

When adding inject_types for types the new partition exposes:

```c
// /tmp/sizes.c
#include <linux/kconfig.h>
#include <linux/THE_HEADER.h>
struct { long long s, a; } info[] = {
    { sizeof(struct TYPE), _Alignof(struct TYPE) },
};
```

```sh
clang --target=x86_64-linux-gnu -nostdinc \
  -include linux/kconfig.h -include generated/autoconf.h \
  -D__KERNEL__ -DMODULE \
  -I linux/arch/x86/include \
  -I linux_bin/arch/x86/include/generated \
  -I linux/include \
  -I linux_bin/include \
  -I linux/arch/x86/include/uapi \
  -I linux_bin/arch/x86/include/generated/uapi \
  -I linux/include/uapi \
  -I linux_bin/include/generated/uapi \
  -S -emit-llvm -o - /tmp/sizes.c | grep @info
```

## Example: the `rko.sync` partition

The sync partition demonstrates the full process. It extracts lock
primitives from 6 headers with 2 additional traverse-only headers:

```toml
[[partition]]
namespace = "rko.sync"
library = "kernel"
headers = [
    "linux/spinlock_types.h", "linux/mutex_types.h",
    "linux/rwsem.h", "linux/lockref.h",
    "linux/refcount_types.h", "linux/osq_lock.h",
]
traverse = [
    "asm-generic/qspinlock_types.h",
    "linux/spinlock_types_raw.h",
    "linux/spinlock_types.h",
    "linux/mutex_types.h",
    "linux/rwsem.h",
    "linux/lockref.h",
    "linux/refcount_types.h",
    "linux/osq_lock.h",
]
```

This eliminated 5 inject_types (`spinlock`, `mutex`, `rw_semaphore`,
`lockref`, `refcount_struct`) and extracted 18 real struct/union types
including `raw_spinlock`, `qspinlock`, and `arch_spinlock_t`.

Feature dependency in `rko-sys/Cargo.toml`:
```toml
sync = ["Foundation"]
fs = ["Foundation", "types", "sync"]
fs_context = ["Foundation", "sync"]
dcache = ["Foundation", "sync"]
```
