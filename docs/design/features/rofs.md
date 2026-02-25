# Feature: Read-Only Filesystem (ROFS) Support

## Goal

Enable writing a read-only in-memory filesystem as an out-of-tree kernel
module, following the lnx project's `rinux-fs` design.

## User API

A filesystem author implements the `fs::Type` trait and uses the
`module!` macro with `InPlaceModule`:

```rust
#![no_std]
use rko_core::prelude::*;
use rko_core::fs::{self, INode, NewSuperBlock, SuperBlock, LockedFolio, DirEntryType};

struct MyRoFs;

impl fs::Type for MyRoFs {
    type INodeData = ();

    const NAME: &'static CStr = c"rko_rofs";

    fn fill_super(sb: NewSuperBlock<'_, Self>) -> Result<&SuperBlock<Self>, Error> {
        // set block size, magic, create root inode
    }

    fn read_dir(
        inode: &INode<Self>,
        pos: i64,
        report: impl FnMut(&[u8], i64, u64, DirEntryType) -> bool,
    ) -> Result<i64, Error> {
        // emit directory entries via report callback
    }

    fn lookup(parent: &INode<Self>, name: &[u8]) -> Result<ARef<INode<Self>>, Error> {
        // find child inode by name
    }

    fn read_folio(inode: &INode<Self>, folio: LockedFolio<'_>) -> Result<(), Error> {
        // write file content into folio
    }
}

// Module uses InPlaceModule because Registration is pinned
#[pin_data]
struct RofsModule {
    #[pin]
    fs_reg: fs::Registration,
}

impl InPlaceModule for RofsModule {
    fn init() -> impl PinInit<Self, Error> {
        try_pin_init!(Self {
            fs_reg <- fs::Registration::new::<MyRoFs>(),
        })
    }
}

module! {
    type: RofsModule,
    name: "rko_rofs",
    license: "GPL",
    author: "rko",
    description: "Read-only filesystem",
}
```

## `fs::Type` trait

```rust
pub trait Type: Sized + Send + Sync {
    /// Per-inode user data.
    type INodeData: Send + Sync;

    /// Filesystem name (shown in /proc/filesystems).
    const NAME: &'static CStr;

    /// Initialize superblock and create root inode.
    fn fill_super(sb: NewSuperBlock<'_, Self>) -> Result<&SuperBlock<Self>, Error>;

    /// Emit directory entries. `report` returns false to stop.
    fn read_dir(
        inode: &INode<Self>,
        pos: i64,
        report: impl FnMut(&[u8], i64, u64, DirEntryType) -> bool,
    ) -> Result<i64, Error>;

    /// Look up a child by name under a directory inode.
    fn lookup(parent: &INode<Self>, name: &[u8]) -> Result<ARef<INode<Self>>, Error>;

    /// Read file content into a locked folio (page cache page).
    fn read_folio(inode: &INode<Self>, folio: LockedFolio<'_>) -> Result<(), Error>;
}
```

Follows the lnx `rinux-fs` design: 4 callbacks, `report` closure for
directory enumeration (wraps `dir_emit` internally), `ARef` for
ref-counted inodes.

## Core Abstractions

### `ARef<T>` — atomic ref-counted pointer

Wraps kernel ref-counting (`ihold`/`iput` for inodes). Implements
`AlwaysRefCounted` trait with `inc_ref()` / `dec_ref()`.

Prerequisite for ROFS — also useful for other kernel objects.

### `Registration` — RAII filesystem registration

```rust
pub struct Registration {
    fs: Opaque<file_system_type>,
    inode_cache: Option<MemCache>,
    _pin: PhantomPinned,   // must not move (kernel holds pointer)
}

impl Registration {
    pub fn new<T: Type>() -> impl PinInit<Self, Error>;
}
```

Pinned because the kernel stores a pointer to the `file_system_type`.
Uses `InPlaceModule` (Phase 2 of module macro) for storage.

On construction: populates `file_system_type` vtable, creates inode
slab cache, calls `register_filesystem`. On drop: `unregister_filesystem`.

### `SuperBlock<T>` / `NewSuperBlock<T>`

- `NewSuperBlock`: type-state builder (set params → set root → done)
- `SuperBlock<T>`: immutable handle passed to callbacks

### `INode<T>` / `NewINode<T>`

- `ARef<INode<T>>` — ref-counted, wraps `ihold`/`iput`
- `NewINode` from `iget_locked` → `init(INodeParams)` → `unlock_new_inode`
- `INodeParams`: type (Dir/Reg/Lnk), mode, size, nlink, timestamps
- `INode<T>` stores `T::INodeData` for user-defined per-inode state

### `LockedFolio<'a>`

- `write(offset, &[u8])` — `kmap_local_folio` + memcpy + `kunmap_local`
- `zero_out(offset, len)` — zero remaining page space
- `mark_uptodate()` / `flush_dcache()`
- Auto-unlocks on drop

### Vtable wiring

The `Registration` populates kernel vtable structs at init:

| Vtable | Key fields | Source |
|--------|-----------|--------|
| `file_operations` | `llseek`, `read_iter`, `iterate_shared` | kernel builtins + `T::read_dir` shim |
| `inode_operations` | `lookup` | `T::lookup` shim |
| `address_space_operations` | `read_folio` | `T::read_folio` shim |
| `super_operations` | `alloc_inode`, `destroy_inode`, `statfs` | slab cache + `simple_statfs` |

## Bindings needed (`rko-sys`)

| Partition | Header | Key Symbols |
|-----------|--------|-------------|
| `rko.fs` | `linux/fs.h` | `super_block`, `inode`, `dentry`, `file_system_type`, `file_operations`, `inode_operations`, `super_operations`, `address_space_operations`, `iget_locked`, `iput`, `ihold`, `alloc_inode_sb`, `unlock_new_inode` |
| `rko.fs_context` | `linux/fs_context.h` | `fs_context`, `fs_context_operations`, `get_tree_nodev` |
| `rko.dcache` | `linux/dcache.h` | `d_make_root`, `d_splice_alias` |
| `rko.pagemap` | `linux/pagemap.h` | `folio`, `folio_pos`, `folio_size`, `folio_mark_uptodate`, `folio_unlock` |
| `rko.highmem` | `linux/highmem.h` | `kmap_local_folio`, `kunmap_local` |

### C helpers needed (macros/inlines)

| Helper | Wraps | Used by |
|--------|-------|---------|
| `rko_i_uid_write` | `i_uid_write` | inode init |
| `rko_i_gid_write` | `i_gid_write` | inode init |
| `rko_set_nlink` | `set_nlink` | inode init |
| `rko_dir_emit` | `dir_emit` | read_dir callback |
| `rko_mapping_set_large_folios` | `mapping_set_large_folios` | superblock init |

Note: `folio_pos`, `folio_size` etc. may also be inlines — verify during
partition generation, add to helpers if bnd-winmd can't extract them.

## Prerequisites

| Dependency | Status |
|-----------|--------|
| `InPlaceModule` + `pin-init` (module macro Phase 2) | Planned |
| `KBox<T>` (heap-pinned module storage) | Planned |
| `ARef<T>` / `AlwaysRefCounted` trait | New |
| `rko.slab` (kmem_cache for inode cache) | ✅ Done |
| `Error` type | ✅ Done |

## Sample: `rofs_test`

Minimal test module:
1. Registers filesystem `"rko_rofs"`
2. Root dir with 2 children: `test.txt` (regular), `link.txt` (symlink)
3. `cat test.txt` → `"hello\n"`, `readlink link.txt` → `"test.txt"`

QEMU test: mount filesystem, verify file contents.

## Implementation Order

| Step | Description |
|------|-------------|
| 1 | Phase 2 module macro: `InPlaceModule`, `pin-init`, `KBox` |
| 2 | `ARef<T>` / `AlwaysRefCounted` in rko-core |
| 3 | Add fs/dcache/pagemap/highmem/fs_context partitions, regenerate bindings |
| 4 | Add C helpers for inline/macro symbols |
| 5 | `fs::LockedFolio` wrapper |
| 6 | `fs::INode<T>`, `NewINode`, inode slab cache |
| 7 | `fs::SuperBlock<T>`, `NewSuperBlock` type-state builder |
| 8 | `fs::Registration` (pinned, RAII) |
| 9 | `fs::Type` trait + vtable wiring |
| 10 | `samples/rofs_test` + QEMU test |
