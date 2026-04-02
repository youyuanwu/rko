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

## Roadmap — Gap Analysis vs VFS Patch

Full gap analysis against `docs/patches/vfs.patch` (50 commits). Items grouped
by architectural impact, not patch order.

### Architecture — Operations Split & vtable Macro

The patch splits callbacks into separate traits with a `#[vtable]` attribute
macro that auto-generates `HAS_*` consts for conditional vtable population.
This is the foundational change everything else builds on.

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `#[vtable]` attribute macro | macros/ | Missing — we hardwire all callbacks |
| `file::Operations` trait (`seek`, `read`, `read_dir`) | fs/file.rs | On monolithic `FileSystem` |
| `inode::Operations` trait (`lookup`, `get_link`) | fs/inode.rs | On monolithic `FileSystem` |
| `address_space::Operations` trait (`read_folio`) | fs/address_space.rs | On monolithic `FileSystem` |
| `Ops<T>` wrapper types (`file::Ops`, `inode::Ops`, `address_space::Ops`) | fs/*.rs | We use `Tables<T>` fields directly |
| Conditional vtable population (`HAS_SEEK`, etc.) | patches 32–38 | We always wire all ops |
| `file::Ops::generic_ro_file()` — returns `&generic_ro_fops` | fs/file.rs | We wire `generic_file_read_iter` manually |
| Remove `const TABLES` requirement | — | Internalize vtable construction |

### Architecture — SuperBlock Typestate

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `SuperBlock<T, S>` with `S = New \| Ready` | fs/sb.rs | No typestate parameter |
| `sb::New` restricts to `set_magic()`, `min_blocksize()` only | fs/sb.rs | All methods always available |
| `sb::Ready` + `DataInited` guards `data()` access | fs/sb.rs | `data()` always available (unsafe) |
| `SuperBlock::rdonly()` | fs/sb.rs | Missing |

### Architecture — Folio Type States

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `Folio<S>` with `S = Unspecified \| PageCache<T>` | folio.rs | No type parameter |
| `Folio<PageCache<T>>::inode()` — get owning inode | folio.rs | We pass inode separately |
| `Folio::map()` / `map_owned()` — RAII mapped page access | folio.rs | No read-mapping (only `write`/`zero_out`) |
| `MapGuard<'a>` / `Mapped<'a, S>` — unmap-on-drop | folio.rs | Missing |
| `Folio::test_highmem()`, `test_uptodate()` | folio.rs | Missing |
| `Lockable` impl for `Folio<S>` | folio.rs | LockedFolio unlocks on drop but doesn't use Lockable |

### Functional — Symlink Support

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `inode::Type::Lnk(Option<CString>)` — inline symlink target | fs/inode.rs | `Lnk` has no data |
| `i_link` population in inode init | fs/inode.rs | Missing |
| `destroy_inode` frees `i_link` via `CString::from_foreign` | fs/inode.rs | Missing |
| `inode::Ops::simple_symlink_inode()` | fs/inode.rs | Missing |
| `inode::Ops::page_symlink_inode()` | fs/inode.rs | We wire `page_get_link` but no builder |
| `inode_nohighmem()` call for page-based symlinks | fs/inode.rs | Missing |
| `inode::Operations::get_link` callback | fs/inode.rs | Missing |
| `set_delayed_call` C helper | helpers.c | Missing |

### Functional — Special Inodes

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `init_special_inode()` for Chr/Blk/Fifo/Sock | fs/inode.rs | We set mode bits only — **bug** |
| `MKDEV()` C helper | helpers.c | Missing |

### Functional — File Read & User I/O

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `file::Operations::read` callback | fs/file.rs | Missing |
| `user::Writer` — `copy_to_user` wrapper | user.rs | Missing |

### Functional — Mapper Enhancements

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| Range-bounded `inode::Mapper` with `split_at()`, `cap_len()` | fs/inode.rs | Our Mapper is simpler |
| Mapper passed to `fill_super` as `Option<inode::Mapper>` | fs.rs | fill_super doesn't receive mapper |
| `INode::mapped_folio()` / `for_each_page()` | fs/inode.rs | On Mapper, not INode |

### Infrastructure — pin_init

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `InPlaceModule` trait — pinned module init | lib.rs | `Module::init()` returns `Self` on stack |
| `Opaque::try_ffi_init` | types.rs | Not used |
| `#[pin_data(PinnedDrop)]` / `try_pin_init!` macros | macros/ | Manual `Pin::new_unchecked` |

### Infrastructure — Allocation & Types

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `GFP_NOFS` allocation flag | alloc.rs | Hardcoded GFP_KERNEL bits |
| `memalloc_nofs(cb)` — scoped nofs context | fs.rs | Missing |
| `Vec::resize` / `resize_with` with alloc flags | vec_ext.rs | Missing |
| `Box::new_uninit_slice` / `new_slice` with alloc flags | box_ext.rs | Missing |
| `MemCache` — safe `kmem_cache` wrapper | mem_cache.rs | Inline cache mgmt in Registration |
| `derive_readable_from_bytes!` macro | types.rs | Manual `unsafe impl FromBytes` |
| `LE<T>`: `From<T>`, `to_le()`/`to_cpu()`, signed types | types.rs | Read-only, unsigned only |
| `CString` backed by `Box<[u8]>`, `ForeignOwnable`, `TryFrom` impls | str.rs | No CString in rko-core |
| `block::Device` typed wrapper with `inode()` | block.rs | Raw `*mut c_void` from `bdev_raw()` |
| Named error codes (`ESTALE`, `EUCLEAN`, `ENODATA`, `EOPNOTSUPP`) | error.rs | We use `Error::new(-errno)` |

### Infrastructure — Misc

| Item | Patch | rko-core Status |
|------|-------|-----------------|
| `kernel::file::File` — general-purpose (non-FS) file with `fget()` | file.rs | Our `fs::File<T>` is FS-specific only |
| `BadFdError` — niche error for null-pointer optimization | file.rs | Missing |
| `UnspecifiedFS` — dummy FS type for unparameterized types | fs.rs | Always need concrete type param |
| `PageOffset` type alias (`pgoff_t`) | fs.rs | Missing |
| `from_result()` / `from_err_ptr()` error helpers | error.rs | Manual per-trampoline |
| `Whence::Data`, `Whence::Hole` variants | fs/file.rs | Only Set/Current/End |
| `file::generic_seek()` free function | fs/file.rs | Wired directly as trampoline |
| `DirEntryType::TryFrom<u32>` and `From<inode::Type>` | fs/file.rs | Missing |
| `container_of!` macro (safe `wrapping_sub` version) | lib.rs | Direct `offset_of!` + `sub()` |

### Implementation Priority

1. **`init_special_inode()`** — bug fix, Chr/Blk/Fifo/Sock inodes broken without it
2. **Symlink support** — enables ext2-ro sample, core read-only functionality
3. **ext2-ro sample** — validates abstractions against a real on-disk format
4. **Operations split + `#[vtable]`** — enables per-inode-type callback selection
5. **SuperBlock typestate** — compile-time safety for `data()` access
6. **Folio type states** — enables `inode()` on page-cache folios
7. **File read + user::Writer** — enables custom read implementations
8. **pin_init infra** — safer Registration lifecycle
9. **Alloc extensions** — GFP_NOFS, Vec::resize, MemCache

### Next Milestone: ext2-ro

Port the VFS patch's `fs/rust-ext2/` (551 lines ext2.rs + 178 lines defs.rs)
as `samples/ext2/`. This is the highest-value next target because it:

- Exercises iomap, Mapper, LE<T>/FromBytes against a **real on-disk format**
  (block groups, inode tables, indirect blocks, directory hash)
- Forces implementation of **symlink support** (#7.1–7.8) and
  **`init_special_inode()`** (#8.1) — both are prerequisite gaps
- Demonstrates rko can mount and read actual ext2 disk images

**Prerequisites** (implement before or during ext2-ro):

| Gap | Items |
|-----|-------|
| `init_special_inode()` + `MKDEV()` | #8.1, #8.2 |
| Symlink: `Lnk(Option<CString>)`, `i_link`, `destroy_inode` cleanup | #7.1–7.3 |
| Symlink: `page_symlink_inode()`, `inode_nohighmem()` | #7.5–7.6 |
| `DirEntryType::From<inode::Type>` | #11.8 |

**Deferred** (not needed for ext2-ro but valuable later):

- ramfs, simplefs — require Tier 2 write support (large effort, no patch reference)
- Operations split + `#[vtable]` — architectural improvement, not blocking ext2-ro

### Sample Filesystems

| Sample | Description | Status |
|--------|-------------|--------|
| rofs_test | In-memory read-only FS (5 QEMU tests) | Done |
| tarfs | Block-device tar FS (3 QEMU tests) | Done |
| ext2-ro | Read-only ext2 — real on-disk format (ported from VFS patch) | Next |

## References

- VFS patch: `docs/patches/vfs.patch` (Wedson Almeida Filho, 50 commits)
- Kernel Rust VFS: `linux/rust/kernel/fs/` (upstream in-tree)
- rko bindings guide: `docs/guides/adding-bindings.md`
