# Feature: Read-Only Filesystem (ROFS) Support

## Goal

Enable writing a simple read-only in-memory filesystem as an out-of-tree
kernel module using rko, similar to `rinux-rofs` in the lnx project.

## Overview

A ROFS module registers a filesystem type with the VFS, provides a
superblock with a root inode, and serves directory listings, lookups,
file reads, and symlink resolution. The module author implements a
trait with 4 callbacks; rko handles vtable wiring, registration, and
kernel object lifecycle.

```
User implements              rko-core provides              rko-sys binds
─────────────────           ──────────────────             ──────────────
trait RoFs {                Registration (RAII)            register_filesystem
  fill_super()              SuperBlock<T>                  iget_locked / iput
  read_dir()                INode<T>, NewINode             d_make_root
  lookup()                  LockedFolio                    kmap_local_folio
  read_folio()              module_ro_fs! macro            folio_mark_uptodate
}                           vtable generation              file_operations, ...
```

## Bindings (`rko-sys` partitions)

New partitions needed in `rko.toml`:

| Partition | Header | Key Symbols |
|-----------|--------|-------------|
| `rko.fs` | `linux/fs.h` | `struct super_block`, `struct inode`, `struct dentry`, `struct file_operations`, `struct inode_operations`, `struct super_operations`, `struct address_space_operations`, `struct file_system_type`, `iget_locked`, `iget_failed`, `ihold`, `iput`, `unlock_new_inode`, `generic_read_dir`, `generic_file_llseek`, `alloc_inode_sb` |
| `rko.fs_context` | `linux/fs_context.h` | `struct fs_context`, `struct fs_context_operations`, `get_tree_nodev`, `get_tree_bdev` |
| `rko.dcache` | `linux/dcache.h` | `d_make_root`, `d_splice_alias` |
| `rko.pagemap` | `linux/pagemap.h` | `struct folio`, `folio_pos`, `folio_size`, `folio_mark_uptodate`, `folio_unlock`, `mapping_set_large_folios` |
| `rko.highmem` | `linux/highmem.h` | `kmap_local_folio`, `kunmap_local` |

Some symbols are static inlines or macros and will need C wrappers in
`helpers.c` (e.g. `i_uid_write`, `i_gid_write`, `set_nlink`, `mkdev`,
`init_special_inode`).

## Wrappers (`rko-core` modules)

### `rko-core::fs` module tree

```
rko-core/src/fs/
├── mod.rs            # Type trait, DirEntryType, INodeType, re-exports
├── registration.rs   # Registration — RAII register/unregister_filesystem
├── super_block.rs    # SuperBlock<T>, NewSuperBlock (type-state init)
├── inode.rs          # INode<T>, NewINode, INodeParams
├── folio.rs          # Folio<T>, LockedFolio (kmap/kunmap + read/write)
└── dentry.rs         # d_make_root, d_splice_alias wrappers
```

### Core abstractions

#### `trait RoFs`

The module author's main interface:

```rust
pub trait RoFs: Sized + Send + Sync {
    /// Initialize superblock: set params, create root inode.
    fn fill_super(sb: &mut NewSuperBlock<'_, Self>) -> Result<(), Error>;

    /// Emit directory entries for the given inode.
    fn read_dir(sb: &SuperBlock<Self>, inode: &INode<Self>,
                ctx: &mut DirContext<'_>) -> Result<(), Error>;

    /// Look up a child name under a directory inode.
    fn lookup(sb: &SuperBlock<Self>, dir: &INode<Self>,
              name: &[u8]) -> Result<ARef<INode<Self>>, Error>;

    /// Read file content into a folio (page cache).
    fn read_folio(sb: &SuperBlock<Self>, inode: &INode<Self>,
                  folio: &mut LockedFolio<'_>) -> Result<(), Error>;
}
```

#### `Registration`

RAII wrapper around `register_filesystem` / `unregister_filesystem`.
Created by the `module_ro_fs!` macro in module init; dropped in exit.

#### `SuperBlock<T>` / `NewSuperBlock<T>`

- `NewSuperBlock` is a type-state builder: `NeedsInit → NeedsRoot → Done`
- `set_params(SuperParams)` sets block size, magic, flags
- `set_root(INode<T>)` creates root dentry via `d_make_root`
- `SuperBlock<T>` is the immutable handle passed to callbacks

#### `INode<T>` / `NewINode<T>`

- Ref-counted (`ARef<INode<T>>`) — wraps `ihold` / `iput`
- `NewINode` from `iget_locked` — call `init(INodeParams)` to set
  mode, uid, gid, size, nlink, timestamps, then `unlock_new_inode`
- `INodeParams` bundles: type (Dir/Reg/Lnk), mode, size, nlink, times

#### `LockedFolio<'a>`

- Obtained in `read_folio` callback (kernel holds the lock)
- `write(offset, &[u8])` — `kmap_local_folio` + memcpy + `kunmap_local`
- `zero_out(offset, len)` — zero remaining page
- `mark_uptodate()` — `folio_mark_uptodate`
- `flush_dcache()` — `flush_dcache_folio`
- Dropped: `folio_unlock` (automatic)

#### `module_ro_fs!` macro

Generates:
- `init_module` / `cleanup_module` with filesystem registration
- `.modinfo` section entries (license, description, etc.)
- Static `file_system_type` with vtable pointing to generated shims
- Vtable shims for `file_operations`, `inode_operations`,
  `super_operations`, `address_space_operations` that dispatch to
  the `RoFs` trait methods

### Vtable wiring

The kernel expects C function pointer structs. rko generates static
vtables at compile time:

```
file_operations {
    .llseek     = generic_file_llseek,  // kernel builtin
    .read_iter  = generic_file_read_iter,
    .iterate_shared = rofs_read_dir_shim,  // → T::read_dir()
}

inode_operations {
    .lookup     = rofs_lookup_shim,     // → T::lookup()
    .get_link   = page_get_link,        // kernel builtin (symlinks)
}

address_space_operations {
    .read_folio = rofs_read_folio_shim, // → T::read_folio()
}

super_operations {
    .alloc_inode  = rofs_alloc_inode,   // custom slab cache
    .destroy_inode = rofs_destroy_inode,
    .statfs       = simple_statfs,      // kernel builtin
}
```

## C Helpers (`helpers.c`)

These kernel symbols are macros or static inlines — must be wrapped:

| Helper | Wraps | Used by |
|--------|-------|---------|
| `rko_i_uid_write` | `i_uid_write` | inode init |
| `rko_i_gid_write` | `i_gid_write` | inode init |
| `rko_set_nlink` | `set_nlink` | inode init |
| `rko_mkdev` | `mkdev` | special inode init |
| `rko_init_special_inode` | `init_special_inode` | device/fifo/socket inodes |
| `rko_mapping_set_large_folios` | `mapping_set_large_folios` | superblock init |

## Dependencies

| Existing | New |
|----------|-----|
| `rko.slab` (kmem_cache) | `rko.fs`, `rko.fs_context`, `rko.dcache`, `rko.pagemap`, `rko.highmem` |
| `rko.types` (kernel types) | `ARef<T>` (atomic refcount wrapper in rko-core) |
| `rko.gfp` (allocation flags) | C helpers in `helpers.c` |
| `KVec<T>` (may be used by fs data) | `module_ro_fs!` macro |

## Sample: `rofs_test`

A minimal test module (like `kvec_test`) that:
1. Registers a filesystem type `"rko_rofs"`
2. `fill_super`: creates root dir with 2 children (file + symlink)
3. `read_dir`: emits `.`, `..`, `test.txt`, `link.txt`
4. `lookup`: returns inode for `test.txt` (regular) or `link.txt` (symlink)
5. `read_folio`: returns `"hello\n"` for `test.txt`

QEMU test: mount, `cat`, `readlink`, verify output.

## Implementation Order

| Step | Description |
|------|-------------|
| 1 | Add `rko.fs` + `rko.dcache` + `rko.pagemap` + `rko.highmem` + `rko.fs_context` partitions to `rko.toml`, regenerate bindings |
| 2 | Add C helpers for inline/macro symbols, add `rko.helpers` partition |
| 3 | Implement `rko-core::fs::folio` — `LockedFolio` wrapper |
| 4 | Implement `rko-core::fs::inode` — `INode<T>`, `NewINode`, ref-counting |
| 5 | Implement `rko-core::fs::super_block` — type-state builder |
| 6 | Implement `rko-core::fs::registration` — RAII register/unregister |
| 7 | Implement `rko-core::fs::dentry` — root + alias helpers |
| 8 | Implement `trait RoFs` + vtable generation + `module_ro_fs!` macro |
| 9 | Create `samples/rofs_test` — minimal filesystem + QEMU test |
| 10 | Update docs |
