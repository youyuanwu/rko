# Feature: Filesystem Support

## Goal

Enable writing filesystems as out-of-tree Rust kernel modules. Read-only
support is complete; read-write is future work. Based on Wedson Almeida
Filho's VFS patch (`docs/patches/vfs.patch`, 50 commits).

## User API

A filesystem implements three traits and uses `module_fs!`:

```rust
#![no_std]
use rko_core::error::Error;
use rko_core::fs::{self, DirEmitter, INode, LockedFolio, Root, SuperBlock, Unhashed};
use rko_core::types::ARef;

struct MyFs;
static TABLES: fs::vtable::Tables<MyFs> = fs::vtable::Tables::new();

impl fs::FileSystem for MyFs {
    type Data = ();
    type INodeData = ();
    const NAME: &'static core::ffi::CStr = c"myfs";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;

    fn fill_super(sb: &SuperBlock<Self, fs::sb::New>, _t: &fs::vtable::Tables<Self>)
        -> Result<(), Error> { /* set block size, magic */ Ok(()) }
    fn init_root(sb: &SuperBlock<Self>, _t: &fs::vtable::Tables<Self>)
        -> Result<Root<Self>, Error> { /* create root inode */ todo!() }
    fn read_folio(_i: &INode<Self>, _f: &mut LockedFolio<'_, fs::PageCache<Self>>)
        -> Result<(), Error> { /* fill page */ todo!() }
}

impl fs::inode::Operations for MyFs {
    type FileSystem = Self;
    fn lookup(p: &INode<Self>, d: Unhashed<'_, Self>)
        -> Result<Option<ARef<fs::DEntry<Self>>>, Error> { /* find child */ todo!() }
}

impl fs::file::Operations for MyFs {
    type FileSystem = Self;
    fn read_dir(_f: &fs::File<Self>, _i: &INode<Self>, _e: &mut DirEmitter)
        -> Result<(), Error> { /* emit entries */ todo!() }
}

rko_core::module_fs! {
    type: MyFs, name: "myfs", license: "GPL",
    author: "rko", description: "My filesystem",
}
```

## Traits

```rust
// Core filesystem — superblock lifecycle, read_folio, xattr, statfs
pub trait FileSystem: inode::Operations<FileSystem = Self>
                    + file::Operations<FileSystem = Self>
                    + Sized + Send + Sync + 'static
{
    type Data: ForeignOwnable + Send + Sync;
    type INodeData: Send + Sync;
    const NAME: &'static CStr;
    const TABLES: &'static vtable::Tables<Self>;
    const SUPER_TYPE: sb::Type = sb::Type::Independent;

    fn fill_super(sb: &SuperBlock<Self, sb::New>, t: &Tables<Self>) -> Result<Self::Data>;
    fn init_root(sb: &SuperBlock<Self>, t: &Tables<Self>) -> Result<Root<Self>>;
    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_, PageCache<Self>>) -> Result;
    fn read_xattr(...) -> Result<usize> { Err(Error::EOPNOTSUPP) }
    fn statfs(...) -> Result<Stat> { Err(Error::ENOSYS) }
}

// Directory inode operations
pub trait inode::Operations: Sized + Send + Sync + 'static {
    type FileSystem: FileSystem;
    fn lookup(parent: &INode<Self::FileSystem>, dentry: Unhashed<'_, Self::FileSystem>)
        -> Result<Option<ARef<DEntry<Self::FileSystem>>>>;
}

// Directory file operations
pub trait file::Operations: Sized + Send + Sync + 'static {
    type FileSystem: FileSystem;
    fn read_dir(file: &File<Self::FileSystem>, inode: &INode<Self::FileSystem>,
                emitter: &mut DirEmitter) -> Result;
}

// Block I/O mapping
pub trait iomap::Operations {
    type FileSystem: FileSystem;
    fn begin(inode: &INode<..>, pos, length, flags, map, srcmap) -> Result;
}
```

## Module Inventory

### `rko-core/src/fs/`

| Module | Key Types |
|--------|-----------|
| `mod.rs` | `FileSystem` trait, `Stat`, `module_fs!`, `fs::mode` |
| `sb.rs` | `SuperBlock<T, New/Ready>`, `BlockDevice`, `SuperParams` |
| `inode.rs` | `INode<T>`, `NewINode<T>`, `INodeOps<T>`, `FileOps<T>`, `AopsOps<T>`, `inode::Operations` |
| `dentry.rs` | `DEntry<T>`, `Unhashed<'a,T>`, `Root<T>` |
| `file.rs` | `File<T>`, `file::flags`, `file::Operations` |
| `dir.rs` | `DirEmitter`, `DirEntryType`, `Whence` (incl. Data/Hole) |
| `folio.rs` | `Folio<S>`, `LockedFolio<'a, S>`, `PageCache<T>` |
| `mapper.rs` | `Mapper`, `MappedFolio` |
| `iomap.rs` | `iomap::Operations`, `Map<'a>`, `RoAops<T>` |
| `vtable.rs` | `Tables<T>` with public ops accessors |
| `registration.rs` | `Registration` |

### `rko-core/src/types/`

| Module | Key Types |
|--------|-----------|
| `le.rs` | `LE<T>` (read+write), `FromBytes`, `derive_readable_from_bytes!` |
| `foreign_ownable.rs` | `ForeignOwnable` |
| `locked.rs` | `Locked<T,L>`, `Lockable` |
| `aref.rs` | `ARef<T>`, `AlwaysRefCounted` |

### `rko-core/src/error.rs`

20 named error constants, `from_result()`, `from_err_ptr()`, `to_err_ptr()`.

### `rko-core/src/alloc/`

`KBox`, `KVec`, `Flags` (GFP_KERNEL, GFP_NOFS, GFP_ATOMIC).

### Samples

| Sample | Description | Tests |
|--------|-------------|-------|
| `rofs_test` | In-memory read-only FS (1 file) | 5/5 |
| `tarfs` | Block-device tar FS (indexed tar format) | 3/3 |
| `ext2` | Read-only ext2 (real disk, iomap, symlinks, special inodes) | 3/3 |

---

## Future Work

### Safety hardening

- **`Locked<&INode<T>, ReadSem>` in callbacks** — `lookup` and `read_dir`
  receive bare `&INode` today. The VFS holds `i_rwsem` during these calls;
  wrapping the inode in `Locked` proves this at compile time and gates
  APIs like `for_each_page()` behind the lock.
- **Typed `sb.data()`** — current `data<D>()` lets callers specify the
  wrong type. Replace with `data() -> <T::Data as ForeignOwnable>::Borrowed<'_>`
  using the generic associated type from `ForeignOwnable`.
- **iomap `ro_aops` completeness** — wire `readahead`, `bmap`,
  `invalidate_folio`, `release_folio` into `RoAops`. Missing `readahead`
  forces synchronous single-page I/O on every fault.

### Proc macro infrastructure

- **`#[vtable]` macro** — auto-generate `HAS_*` consts for each trait
  method, conditionally populate vtable entries. Requires a separate
  `rko-macros` proc-macro crate.
- **pin_init** — `InPlaceModule`, `try_pin_init!`, `#[pin_data(PinnedDrop)]`
  for safer Registration lifecycle. Also requires proc-macro crate.

### Read-only feature gaps (from VFS patch)

- **`get_link` callback** — custom symlink resolution via
  `inode::Operations::get_link` returning `Either<CString, &CStr>`,
  with `set_delayed_call` for deferred cleanup.
- **`file::Operations::read`** + `user::Writer` — custom read path
  with `copy_to_user`. Needed for pseudo-files or non-page-cache reads.
- **Folio mapping** — `Folio::map()`/`map_owned()`, `MapGuard` RAII,
  `test_uptodate()`, `test_highmem()`, `Lockable` impl.
- **Range-bounded Mapper** — `split_at()`, `cap_len()`, mapper passed
  to `fill_super` as `Option<inode::Mapper>`.

### Read-write support (Tier 2 — beyond VFS patch scope)

| Feature | Key operations |
|---------|---------------|
| File write | `write_iter`, `fsync` |
| Inode mutation | `create`, `mkdir`, `unlink`, `rmdir`, `rename`, `symlink`, `link`, `setattr` |
| Page writeback | `write_folio`, `writepages` |
| Xattr write | `set_xattr`, `remove_xattr`, `list_xattr` |
| Superblock sync | `sync_fs`, `write_super` |

### Nice-to-have

- `address_space::Operations` as separate trait
- `MemCache` safe wrapper (currently inline in `Registration`)
- Remove `const TABLES` requirement
- `CString` utilities, `UnspecifiedFS`, `PageOffset`
- LE signed types (`i16`, `i32`, `i64`)
- `memalloc_nofs(cb)` scoped allocation context

### Known limitations

- **Symlink targets are `&'static [u8]`** — heap-allocated symlink
  targets (`CString`) not supported. `destroy_inode` does not free
  `i_link`. Change `Lnk` to `Lnk(Option<CString>)` and add cleanup
  if runtime-constructed targets are needed.
- **No readahead** — iomap `ro_aops` only wires `read_folio`. Files
  are read one page at a time until `readahead` callback is added.

## References

- VFS patch: `docs/patches/vfs.patch` (Wedson Almeida Filho, 50 commits)
- Kernel Rust VFS: `linux/rust/kernel/fs/` (upstream in-tree)
- rko bindings guide: `docs/guides/adding-bindings.md`

