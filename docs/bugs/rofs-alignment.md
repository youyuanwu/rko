# ROFS Alignment with lnx Reference

Comparing our rko ROFS with the working lnx implementation
(`/home/user1/code/lnx/crates/rinux-fs/src/fs.rs`).

## Critical Fixes (all resolved)

### 1. Inode slab cache (alloc_inode / destroy_inode) ✅

**Bug**: We used `iget_locked()` which called the kernel's default
`alloc_inode` — it allocates a bare `struct inode`. Our `container_of`
then read memory *before* the inode, which is unowned.

**Fix**: `Registration` creates a per-filesystem `kmem_cache` for
`INodeWithData<T>`. `alloc_inode_callback` allocates from the cache
and returns `&(*ptr).inode`. `destroy_inode_callback` calls
`drop_in_place` on user data (unless `is_bad_inode`) and frees to
cache. `inode_init_once` is passed to `kmem_cache_create` as the
slab constructor.

Files: `registration.rs`, `vtable.rs`

### 2. kill_sb doesn't clean up s_fs_info ✅

**Bug**: `kill_sb_callback` only called `kill_anon_super`.

**Fix**: Added `Type::kill_sb()` trait method (default no-op). The
trampoline calls it after `kill_anon_super`, giving the filesystem
a chance to drop data stored in `s_fs_info`.

Files: `mod.rs` (trait), `vtable.rs` (trampoline)

### 3. folio_end_read not called ✅

**Bug**: `read_folio` trampoline only unlocked the folio via
`LockedFolio::drop` (`folio_unlock`). It didn't signal I/O completion.

**Fix**: The trampoline now calls `folio_end_read(folio, success)`
after `T::read_folio` returns. The `LockedFolio` is `mem::forget`-ed
to prevent double-unlock. `folio_end_read` both marks the folio
uptodate (on success) and unlocks it.

Files: `vtable.rs` (read_folio_trampoline)

### 4. inode_operations ____cacheline_aligned padding ✅

**Bug**: Kernel `struct inode_operations` uses `____cacheline_aligned`
which pads it from 200 bytes (25 function pointers) to 256 bytes
(64-byte aligned). bnd-winmd generated a 200-byte struct. When
`Tables` embedded multiple `inode_operations` back-to-back, the
kernel read 256 bytes from each, bleeding 56 bytes into the next
struct — corrupting ops pointers and causing infinite spins in
`start_dir_add` / `__d_add`.

**Fix**: Added `[u8; INODE_OPS_PAD]` (56 bytes) after each
`inode_operations` in the `Tables` struct. This is a workaround;
the root cause is tracked in `bnd-winmd-cacheline-aligned.md`.

Files: `vtable.rs` (Tables struct)

## Important Fixes (all resolved)

### 5. mapping_set_large_folios for regular files ✅

**Missing**: lnx calls `mapping_set_large_folios(inode.i_mapping)`
for regular file inodes.

**Fix**: `NewINode::init` now calls `mapping_set_large_folios` for
`INodeType::Reg`.

Files: `inode.rs`

### 6. inode_init_once callback ✅

**Missing**: The `kmem_cache_create` call needed `inode_init_once` as
the constructor.

**Fix**: Implemented as part of fix #1 — `inode_init_once_callback::<T>`
is passed to `kmem_cache_create`.

Files: `registration.rs`

## Design Differences (lower priority, not yet addressed)

- **NewSuperBlock type-state**: lnx enforces init ordering at compile
  time (`NeedsInit` → `NeedsRoot`). We use a plain `&SuperBlock`.
- **from_result wrapper**: lnx wraps all callbacks in `from_result`
  for clean error conversion. We do inline match.
