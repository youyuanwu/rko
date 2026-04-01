# Feature: Filesystem Support

## Goal

Enable writing filesystems as out-of-tree Rust kernel modules, starting
with read-only support and extending toward full read-write VFS
coverage. Based on Wedson Almeida Filho's VFS patch series
(`docs/patches/vfs.patch` — 50 commits) from the upstream
Rust-for-Linux effort.

## User API

A filesystem author implements `fs::FileSystem` and uses `module_fs!`:

```rust
#![no_std]
use rko_core::error::Error;
use rko_core::fs::{self, DirEmitter, INode, LockedFolio, Root, SuperBlock, SuperParams};
use rko_core::types::ARef;

struct MyRoFs;
static TABLES: fs::vtable::Tables<MyRoFs> = fs::vtable::Tables::new();

impl fs::FileSystem for MyRoFs {
    type Data = ();
    type INodeData = ();
    const NAME: &'static core::ffi::CStr = c"my_rofs";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;

    fn fill_super(sb: &SuperBlock<Self>, _t: &fs::vtable::Tables<Self>)
        -> Result<(), Error> { /* set block size, magic */ Ok(()) }
    fn init_root(sb: &SuperBlock<Self>, t: &fs::vtable::Tables<Self>)
        -> Result<Root<Self>, Error> { /* create root inode */ }
    fn lookup(p: &INode<Self>, d: fs::Unhashed<'_, Self>, t: &fs::vtable::Tables<Self>)
        -> Result<Option<ARef<fs::DEntry<Self>>>, Error> { /* find child */ }
    fn read_dir(_f: &fs::File<Self>, i: &INode<Self>, e: &mut DirEmitter)
        -> Result<(), Error> { /* emit entries */ }
    fn read_folio(i: &INode<Self>, f: &mut LockedFolio<'_>)
        -> Result<(), Error> { /* fill page */ }
}

rko_core::module_fs! {
    type: MyRoFs, name: "my_rofs", license: "GPL",
    author: "rko", description: "My read-only filesystem",
}
```

## `fs::FileSystem` Trait

```rust
pub trait FileSystem: Sized + Send + Sync + 'static {
    type Data: ForeignOwnable + Send + Sync;  // per-superblock data
    type INodeData: Send + Sync;               // per-inode data
    const NAME: &'static CStr;
    const TABLES: &'static vtable::Tables<Self>;
    const SUPER_TYPE: sb::Type = sb::Type::Independent; // or BlockDev

    fn fill_super(sb: &SuperBlock<Self>, tables: &Tables<Self>) -> Result<Self::Data>;
    fn init_root(sb: &SuperBlock<Self>, tables: &Tables<Self>) -> Result<Root<Self>>;
    fn lookup(parent: &INode<Self>, dentry: Unhashed<'_, Self>, tables: &Tables<Self>)
        -> Result<Option<ARef<DEntry<Self>>>>;
    fn read_dir(file: &File<Self>, inode: &INode<Self>, emitter: &mut DirEmitter)
        -> Result<()>;
    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_>) -> Result<()>;
    fn read_xattr(d: &DEntry<Self>, i: &INode<Self>, name: &CStr, buf: &mut [u8])
        -> Result<usize> { Err(EOPNOTSUPP) }
    fn statfs(dentry: &DEntry<Self>) -> Result<Stat> { Err(ENOSYS) }
}
```

## Current Implementation

All Tier 1 (read-only path) features are implemented and tested.

### Core Abstractions (`rko-core/src/fs/`)

| Module | Types | Purpose |
|--------|-------|---------|
| `mod.rs` | `FileSystem` trait, `Stat`, `module_fs!` | Core trait + macro |
| `sb.rs` | `SuperBlock<T>`, `SuperParams`, `sb::Type` | Superblock wrapper, block device methods |
| `inode.rs` | `INode<T>`, `NewINode<T>`, `INodeParams`, `ReadSem` | Inode lifecycle, per-inode data, locking |
| `dentry.rs` | `DEntry<T>`, `Unhashed<'a,T>`, `Root<T>` | Dentry type-states for lookup |
| `file.rs` | `File<T>`, `file::flags` | Open file wrapper |
| `dir.rs` | `DirEmitter`, `DirEntryType`, `Whence`, `Ino`, `Offset` | Directory emission |
| `folio.rs` | `Folio`, `LockedFolio` | Page cache I/O |
| `mapper.rs` | `Mapper`, `MappedFolio` | Block device page-cache reader |
| `iomap.rs` | `iomap::Operations`, `Map<'a>`, `Type`, `map_flags` | Block I/O mapping |
| `vtable.rs` | `Tables<T>`, trampolines | C callback wiring |
| `registration.rs` | `Registration` | RAII register/unregister |

### Foundation Types (`rko-core/src/types/`)

| Module | Types | Purpose |
|--------|-------|---------|
| `foreign_ownable.rs` | `ForeignOwnable` | Store Rust data in C `void*` fields |
| `locked.rs` | `Locked<T,L>`, `Lockable` | Generic RAII lock guard |
| `le.rs` | `LE<T>`, `FromBytes` | Little-endian on-disk type parsing |
| `aref.rs` | `ARef<T>`, `AlwaysRefCounted` | Kernel ref-counted pointers |

### Bindings (`rko-sys`)

| Partition | Key Symbols |
|-----------|-------------|
| `rko.fs` | `super_block`, `inode`, `file`, `file_system_type`, vtable structs |
| `rko.fs_context` | `fs_context`, `get_tree_nodev`, `get_tree_bdev` |
| `rko.dcache` | `dentry`, `d_make_root`, `d_splice_alias`, `dget`, `dput` |
| `rko.pagemap` | `folio`, `read_cache_folio` |
| `rko.statfs` | `kstatfs` |
| `rko.xattr` | `xattr_handler` |
| `rko.iomap` | `iomap`, `iomap_ops`, `iomap_read_folio` |
| `rko.helpers` | C wrappers for inline kernel functions |

### Samples

| Sample | Description | QEMU Test |
|--------|-------------|-----------|
| `rofs_test` | In-memory read-only FS (1 file) | 5/5 pass |
| `tarfs` | Block-device-backed read-only FS (indexed tar) | 3/3 pass |

---

## Roadmap — Future Work

### Internal Quality (no new features)

- **Split callbacks into separate traits** — move `lookup` to
  `inode::Operations`, `read_dir`/`read`/`seek` to `file::Operations`,
  `read_folio` to `address_space::Operations`
- **Remove `const TABLES`** — internalize vtable construction
- **`SuperBlock<T, State>` typestate** — `sb::New` vs `sb::Ready`
- **`#[vtable]` attribute macro** — auto-generate `HAS_*` constants

### Tier 2 — Read-Write Filesystem Support

| Feature | Operations |
|---------|-----------|
| File write | `write_iter`, `fsync` |
| Inode mutation | `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `symlink`, `link`, `setattr` |
| Page writeback | `write_folio`, `writepages` |
| Xattr write | `set_xattr`, `remove_xattr`, `list_xattr` |
| Superblock sync | `sync_fs`, `write_super` |

### Tier 3 — Advanced Features

| Feature | Notes |
|---------|-------|
| mmap | `generic_file_mmap`, `vm_area_struct` wrapper |
| File locking | `posix_lock_file`, `flock` |
| ioctl | `unlocked_ioctl`, `_IOC` macro equivalents |
| Poll / epoll | `poll_table_struct`, `poll_wait` |
| POSIX ACLs | `posix_acl`, `get_inode_acl` |
| Symlink `get_link` | `page_get_link`, `simple_get_link` |
| Readahead | `readahead_control`, batch `read_folio` |

### Tier 4 — Sample Filesystems

| Sample | Requires | Status |
|--------|----------|--------|
| tarfs | Tier 1 | Done |
| ramfs | Tier 2 | Not started |
| ext2-ro | Tier 1 | Not started |
| simplefs | Tier 2 + mmap + symlinks | Not started |

## References

- VFS patch: `docs/patches/vfs.patch` (Wedson Almeida Filho, 50 commits)
- Kernel Rust VFS: `linux/rust/kernel/fs/` (upstream in-tree)
- rko bindings guide: `docs/guides/adding-bindings.md`
