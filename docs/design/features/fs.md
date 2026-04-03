# Feature: Filesystem Support

## Goal

Enable writing filesystems as out-of-tree Rust kernel modules. Read-only
support is complete; read-write is future work. Based on Wedson Almeida
Filho's VFS patch (`docs/patches/vfs.patch`, 50 commits).

## Patch Coverage

**50/50 VFS patch commits ported (100%).** All framework, architectural
patterns, utility types, and infrastructure are implemented.

### Ported features

| Category | Key items |
|----------|-----------|
| **Core FS framework** | `FileSystem`, `SuperBlock<T, New/Ready>`, `INode<T>`, `DEntry<T>`, `File<T>`, `Folio<S>`, `DirEmitter` |
| **Trait architecture** | Operations split (`inode::Operations`, `file::Operations`), `#[vtable]` with `HAS_*` consts |
| **Callbacks** | `fill_super`, `init_root`, `lookup`, `read_dir`, `read_folio`, `read_xattr`, `statfs`, `seek`, `read`, `read_iter`, `get_link` |
| **Safety patterns** | `Locked<&INode, ReadSem>` in lookup/read_dir, typed `sb.sb_data()` via `DataInner`, `container_of!` in Registration |
| **Block device** | bdev mount, `iomap::Operations` (begin/end), `RoAops` (read_folio, readahead, bmap, invalidate_folio, release_folio), `Mapper`, `BoundedMapper`, `BlockDevice` |
| **Symlinks** | Inline (`i_link`), page-cache, custom `get_link` with `CString` + `delayed_call` cleanup |
| **Special inodes** | Chr, Blk, Fifo, Sock via `init_special_inode` |
| **Types** | `LE<T>` (u8–u64, i8–i64), `FromBytes` + `#[derive(FromBytes)]`, `CString` + `ForeignOwnable`, `IoVecIter` |
| **Alloc** | `KBox`, `KVec`, `Flags`, `MemCache`, `NoFsGuard`, `Box<[T]>::new_uninit_slice`, `KVec::resize` |
| **Error handling** | 21 named constants, `from_result()`, `from_err_ptr()`, `to_err_ptr()` |
| **Proc macros** | `#[vtable]`, `#[derive(FromBytes)]` in `rko-macros` crate |
| **Pin init** | `Registration::pin_init()`, re-exported `pin_init!`/`try_pin_init!` from `pinned-init` crate |
| **Time** | `Time` (timestamps), `Ktime` (monotonic nanoseconds) |
| **User I/O** | `user::Writer` (copy_to_user), `user::Reader` (copy_from_user) |
| **Folio** | `Folio::map()`/`FolioMap` RAII, `PageCache<T>` state, `LockedFolio` |
| **Samples** | rofs_test (8 tests), tarfs (3 tests), ext2 (3 tests) — 14/14 QEMU |

## User API

A filesystem implements three traits and uses `module_fs!`:

```rust
#![no_std]
use rko_core::error::Error;
use rko_core::fs::{self, DirEmitter, INode, LockedFolio, Root, SuperBlock, Unhashed};
use rko_core::types::{ARef, Locked};

struct MyFs;
static TABLES: fs::vtable::Tables<MyFs> = fs::vtable::Tables::new();

#[rko_core::vtable]
impl fs::FileSystem for MyFs {
    type Data = ();
    type INodeData = ();
    const NAME: &'static core::ffi::CStr = c"myfs";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;

    fn fill_super(sb: &SuperBlock<Self, fs::sb::New>, _t: &fs::vtable::Tables<Self>)
        -> Result<(), Error> { Ok(()) }
    fn init_root(sb: &SuperBlock<Self>, _t: &fs::vtable::Tables<Self>)
        -> Result<Root<Self>, Error> { todo!() }
    fn read_folio(_i: &INode<Self>, _f: &mut LockedFolio<'_, fs::PageCache<Self>>)
        -> Result<(), Error> { todo!() }
}

#[rko_core::vtable]
impl fs::inode::Operations for MyFs {
    type FileSystem = Self;
    fn lookup(p: &Locked<'_, INode<Self>, fs::inode::ReadSem>, d: Unhashed<'_, Self>)
        -> Result<Option<ARef<fs::DEntry<Self>>>, Error> { todo!() }
}

#[rko_core::vtable]
impl fs::file::Operations for MyFs {
    type FileSystem = Self;
    fn read_dir(_f: &fs::File<Self>,
        _i: &Locked<'_, INode<Self>, fs::inode::ReadSem>,
        _e: &mut DirEmitter) -> Result<(), Error> { todo!() }
}

rko_core::module_fs! {
    type: MyFs, name: "myfs", license: "GPL",
    author: "rko", description: "My filesystem",
}
```

## Module Inventory

### `rko-macros/` (proc-macro crate)

| Macro | Description |
|-------|-------------|
| `#[vtable]` | On traits: `HAS_*` consts (`false` for defaults, `true` for required). On impls: overrides to `true`. |
| `#[derive(FromBytes)]` | Generates `unsafe impl FromBytes` for `#[repr(C)]` structs. |

### `rko-core/src/fs/`

| Module | Key Types |
|--------|-----------|
| `mod.rs` | `#[vtable] FileSystem`, `Stat`, `module_fs!`, `fs::mode` |
| `sb.rs` | `SuperBlock<T, New/Ready>`, `BlockDevice`, `SuperParams`, `DataInner`, `sb_data()` |
| `inode.rs` | `INode<T>`, `NewINode<T>`, `INodeOps<T>`, `FileOps<T>`, `AopsOps<T>`, `#[vtable] inode::Operations`, `GetLinkResult` |
| `dentry.rs` | `DEntry<T>`, `Unhashed<'a,T>`, `Root<T>` |
| `file.rs` | `File<T>`, `file::flags`, `#[vtable] file::Operations` (read_dir, seek, read, read_iter) |
| `dir.rs` | `DirEmitter`, `DirEntryType`, `Whence` (Start/Current/End/Data/Hole) |
| `folio.rs` | `Folio<S>`, `FolioMap`, `LockedFolio<'a, S>`, `PageCache<T>` |
| `mapper.rs` | `Mapper`, `MappedFolio`, `BoundedMapper` |
| `iomap.rs` | `#[vtable] iomap::Operations`, `Map<'a>`, `RoAops<T>` (read_folio, readahead, bmap, invalidate_folio, release_folio) |
| `vtable.rs` | `Tables<T>` — conditional vtable via `T::HAS_*` consts |
| `registration.rs` | `Registration`, `Registration::pin_init()` |

### `rko-core/src/`

| Module | Key Types |
|--------|-----------|
| `types/le.rs` | `LE<T>` (u8–u64, i8–i64), `FromBytes`, `LeInt` |
| `types/cstring.rs` | `CString` — heap NUL-terminated string, `ForeignOwnable` |
| `types/foreign_ownable.rs` | `ForeignOwnable` trait |
| `types/locked.rs` | `Locked<T,L>`, `Lockable`, `Locked::borrowed()` |
| `types/aref.rs` | `ARef<T>`, `AlwaysRefCounted` |
| `types/opaque.rs` | `Opaque<T>`, `ffi_init`, `try_ffi_init` |
| `error.rs` | 21 named constants, `from_result()`, `from_err_ptr()`, `to_err_ptr()` |
| `alloc/` | `KBox`, `KVec`, `Flags`, `MemCache`, `NoFsGuard`, `Box<[MaybeUninit<T>]>` |
| `time.rs` | `Time` (timestamps), `Ktime` (monotonic) |
| `user.rs` | `Writer` (copy_to_user), `Reader` (copy_from_user) |
| `iov.rs` | `IoVecIter` (scatter-gather via iov_iter) |
| `pin_init` | Re-exports: `pin_init!`, `try_pin_init!`, `PinInit`, `Init` |

### Samples

| Sample | Description | Tests |
|--------|-------------|-------|
| `rofs_test` | In-memory FS: read_iter, seek (SEEK_DATA/HOLE), get_link, FolioMap | 8/8 |
| `tarfs` | Block-device tar FS (indexed tar format) | 3/3 |
| `ext2` | Read-only ext2 (iomap, symlinks, special inodes, NoFsGuard) | 3/3 |

---

## Future Work

### Read-write support (Tier 2 — beyond VFS patch scope)

| Feature | Key operations |
|---------|---------------|
| File write | `write_iter`, `fsync` |
| Inode mutation | `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `symlink`, `link`, `setattr` |
| Page writeback | `write_folio`, `writepages` |
| Xattr write | `set_xattr`, `remove_xattr`, `list_xattr` |
| Superblock sync | `sync_fs`, `write_super` |

### Nice-to-have

- `address_space::Operations` as a separate trait (currently via Tables/RoAops)
- Remove `const TABLES` requirement
- `UnspecifiedFS` erased filesystem type
- `PageOffset` newtype
- `block::Device` full module (currently minimal wrapper in sb.rs)

### Known limitations

- **Symlink targets via `INodeType::Lnk` are `&'static [u8]`** — for
  runtime targets, use `get_link` with `CString` instead.
- **`HAS_READ`/`HAS_READ_ITER` is per-filesystem, not per-inode** — when
  overridden, ALL regular files use the custom path. Use per-inode dispatch
  if mixed behavior is needed.

---

## Design Differences from VFS Patch

The rko-core implementation covers all 50 patch commits but diverges in
some API design choices. This section documents intentional differences
and the one incomplete area.

### Intentional differences (🟢 better or equivalent)

| Area | VFS Patch | rko-core | Rationale |
|------|-----------|----------|-----------|
| **iomap RoAops** | `read_folio` only | + `readahead`, `bmap`, `invalidate_folio`, `release_folio` | More complete; enables kernel readahead and proper folio lifecycle |
| **`read_iter`** | Not in patch — only legacy `read` | Both `read` and `read_iter` with priority logic | Forward-looking; supports scatter-gather and async I/O |
| **CString storage** | `Box<[u8]>` (std alloc) | `KBox<[u8]>` (kernel allocator) | Correct for out-of-tree modules without std |
| **`get_link` return** | `Either<CString, &'a CStr>` | `GetLinkResult::Owned(CString) / Borrowed(&CStr)` | Same semantics; avoids `Either` dependency |
| **Registration pin** | `InPlaceModule` trait | `Registration::pin_init()` + `KBox::pin_init` | Single-call, no unsafe at the macro site |
| **kill_sb ordering** | kill_*_super first, then free data | Same | Matched (was a bug we fixed) |

### Equivalent but different style (🟡)

| Area | VFS Patch | rko-core | Notes |
|------|-----------|----------|-------|
| **`fill_super`** | `(&mut SuperBlock<New>, Option<Mapper>) → Result` | `(&SuperBlock<New>, &Tables) → Result<Data>` | `&` vs `&mut`; extra Tables param; returns Data directly instead of storing via separate call |
| **`init_root`** | `(&SuperBlock) → Result<Root>` | `(&SuperBlock, &Tables) → Result<Root>` | Extra Tables param for convenience — samples access ops tables during root init |
| **`read_folio`** | `(Option<&File>, Locked<&Folio<PageCache>>)` | `(&INode, &mut LockedFolio<PageCache>)` | Patch passes optional File + folio ownership; ours passes inode + mutable borrow |
| **`Locked` type** | `Locked<T: Deref, M>` wrapping a deref-able | `Locked<'a, T: Lockable<L>, L>` with lifetime + `borrowed()` | Patch wraps `&INode` in Locked; ours wraps `INode` and borrows via `Deref`. `borrowed()` is extra for VFS-held locks. |
| **`sb.data()` return** | `<T::Data as ForeignOwnable>::Borrowed<'_>` via GAT | `unsafe fn sb_data() → &DataInner::Inner` via helper trait | Both type-safe; ours uses `DataInner` trait instead of GAT |
| **`const TABLES`** | Not in patch (vtable auto-generated by `#[vtable]` on trait) | Required — `const TABLES: &'static Tables<Self>` | Patch's vtable is embedded in the type system; ours is explicit |
| **Registration layout** | No `container_of` — different wiring | `#[repr(C)]` + `container_of!(s_type, Registration, fs_type)` | Our approach requires stable layout; patch wires differently |

### Previously incomplete — now fixed

| Area | Fix |
|------|-----|
| **ForeignOwnable GAT** | Added `type Borrowed<'a>` GAT. `CString::borrow() → &'a CStr`, `KBox<T>::borrow() → &'a T`, `()::borrow() → ()`. No more panics. |

## References

- VFS patch: `docs/patches/vfs.patch` (Wedson Almeida Filho, 50 commits)
- Kernel Rust VFS: `linux/rust/kernel/fs/` (upstream in-tree)
- rko bindings guide: `docs/guides/adding-bindings.md`

