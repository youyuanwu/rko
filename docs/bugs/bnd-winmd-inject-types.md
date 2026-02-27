# bnd-winmd: `[[inject_type]]` for user-declared types

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.4  
**Status:** Implemented

## Problem

bnd-winmd cannot extract certain C type patterns:

1. **Bitfield enums** — `enum fs_value_type type:8;` (✅ fixed in
   bnd `0ee989f`)
2. **Anonymous enums** — `enum { ... } type;` inside a struct
3. **Types from non-traversed headers** — embedded struct fields whose
   defining header isn't in the traverse list

## Solution

Top-level `[[inject_type]]` entries in the TOML config declare types
that are merged into partitions after clang extraction but before winmd
emission. See upstream docs:
[bnd/docs/design/InjectTypes.md](https://github.com/youyuanwu/bnd/blob/main/docs/design/InjectTypes.md)

### Usage in rko

`rko-sys-gen/rko.toml` uses 23 inject entries (down from 54 after
adding the `rko.sync` partition, bnd fixes, and expanding traverse
lists). Types fall into two categories:

- **Pointer-only / hard structs** (16) — `task_struct`, `folio`,
  `vm_area_struct`, `vm_area_desc`, `module`, `page`, `vm_fault`,
  `freeptr_t`, `wait_queue_entry`, `file_dedupe_range`, `uuid_t`,
  `mnt_idmap`, `key`, `user_struct`, `ctl_table_set`,
  `ctl_table_header`
- **Typedef / macro-generated structs** (7) — `kuid_t`, `kgid_t`,
  `vfsuid_t`, `vfsgid_t`, `rwlock_t`, `seqcount_spinlock`,
  `file_ref_t`

See `docs/design/Partitions.md` for how to add new partitions to
reduce the inject_type count further.

### Getting sizes

```sh
# Use clang to get sizeof/alignof (needs -include linux/kconfig.h)
clang --target=x86_64-linux-gnu -nostdinc -w \
  -Ilinux/arch/x86/include ... \
  -include linux/kconfig.h -include generated/autoconf.h \
  -D__KERNEL__ -DMODULE -D__BINDGEN__ \
  -S -emit-llvm -o - /tmp/check.c
# Parse @sz and @al from LLVM IR output
```

## Removal plan — completed

Most inject_types existed because their defining header was not in any
partition's traverse list. All headers below are already transitively
included by `linux/fs.h`, so adding them to traverse cost only
extraction time — no new compilation.

### ✅ Easy (12 types) — done

Added to `rko.fs` traverse: `linux/time64.h`, `linux/delayed_call.h`,
`linux/path.h`, `linux/rbtree_types.h`, `linux/migrate_mode.h`,
`linux/pid_types.h`, `linux/rw_hint.h`, `linux/llist.h`,
`linux/list_bl.h`, `linux/workqueue_types.h`, `linux/lockdep_types.h`.
Also `linux/timer_types.h` (dependency of `workqueue_types.h`).

Removed: `timespec64`, `delayed_call`, `path`, `rb_root_cached`,
`migrate_mode` (enum), `pid_type` (enum), `rw_hint` (enum),
`llist_node`, `hlist_bl_head`, `work_struct`, `lock_class_key`,
`hlist_bl_node` (moved to dcache traverse).

### ✅ Medium (15 types) — done

Added to `rko.fs` traverse: `linux/wait.h`, `linux/wait_bit.h`,
`linux/stat.h`, `linux/shrinker.h`, `linux/quota.h`, `linux/pid.h`,
`linux/mount.h`, `linux/percpu-rwsem.h`, `linux/maple_tree.h`,
`linux/list_lru.h`, `linux/xarray.h`.

Dependency headers also added: `linux/completion.h`,
`linux/rcu_sync.h`, `linux/percpu_counter.h`, `linux/swait.h`,
`linux/projid.h`.

Removed: `percpu_rw_semaphore`, `shrinker`, `list_lru`, `quota_info`,
`dquot`, `shrink_control`, `dquot_operations`, `quotactl_ops`,
`kstat`, `xarray`, `maple_tree`, `pid`, `wait_queue_head`,
`wait_bit_queue_entry`, `vfsmount`.

### Hard — attempted

| Header | Outcome |
|---|---|
| `linux/cred.h` | ✅ Extracted. Added `linux/cred.h` + `linux/capability.h` to fs traverse. Required 2 new pointer-only inject_types: `key`, `user_struct`. |
| `linux/user_namespace.h` | ✅ Extracted. Added to fs_context traverse with `linux/ns/ns_common_types.h` + `linux/list_nulls.h`. Required 2 new inject_types: `ctl_table_set`, `ctl_table_header` (anonymous enum in `sysctl.h` blocked full extraction). |
| `linux/mm_types.h` | ❌ Reverted. Cascades into arch types (`pgd_t`, `mm_context_t`, etc.) and `mm_struct` internals (`mm_mm_cid`, `uprobes_state`). Adding it creates more inject_types than it removes. |
| `linux/sched.h` | ❌ Not attempted. 42 includes, would cascade worse than mm_types.h. |
| `linux/module.h` | ❌ Attempted, 11 unresolved types. Too heavy. |

### Not extractable (11 types)

These are typedefs, arch-specific, or have no clear single header:

`kuid_t`, `kgid_t`, `vfsuid_t`, `vfsgid_t` (UID/GID wrapper
typedefs), `rwlock_t`, `seqcount_spinlock` (lock internals),
`file_ref_t`, `uuid_t`, `mnt_idmap`, `file_dedupe_range` (UAPI),
`vm_fault` (defined in `linux/mm.h`, 4000+ lines).

### Result

**54 → 23 inject_types** (57% reduction). The `rko.fs` partition
traverse list grew from 2 to 32 headers, `rko.fs_context` from 1
to 5. Extracted `cred` and `user_namespace` as real structs but at
the cost of 4 new pointer-only inject_types for their dependencies.
