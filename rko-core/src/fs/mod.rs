//! Filesystem abstractions.

use crate::error::Error;

mod folio;
mod inode;
mod registration;
mod sb;
pub mod vtable;

pub use folio::{Folio, LockedFolio};
pub use inode::{INode, INodeParams, INodeType, NewINode, Time};
pub use inode::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFREG, S_IFSOCK};
pub use registration::Registration;
pub use sb::{SuperBlock, SuperParams};

type Result<T = ()> = core::result::Result<T, Error>;

/// Filesystem type trait — implemented by each filesystem.
///
/// See `docs/design/features/rofs.md` for the full design.
pub trait Type: Sized + Send + Sync + 'static {
    /// Per-inode user data.
    type INodeData: Send + Sync;

    /// Filesystem name (shown in /proc/filesystems).
    const NAME: &'static core::ffi::CStr;

    /// Static operation tables for this filesystem type.
    const TABLES: &'static vtable::Tables<Self>;

    /// Populate the superblock.
    ///
    /// Called from `get_tree_nodev` → `fill_super`. Must set up the
    /// root inode and call `sb.set_root()`.
    fn fill_super(sb: &SuperBlock<Self>, tables: &vtable::Tables<Self>) -> Result<()>;

    /// Look up a child inode by name in a directory.
    ///
    /// Return the inode if found, or `None` for negative dentry.
    /// The filesystem should call `sb.iget()` and initialize the inode.
    fn lookup(
        parent: &INode<Self>,
        name: &[u8],
        tables: &vtable::Tables<Self>,
    ) -> Result<Option<crate::types::ARef<INode<Self>>>>;

    /// Iterate directory entries starting from `pos`.
    ///
    /// Call `emit` for each entry. Return `Ok(())` when done.
    /// `emit` returns `false` when the buffer is full — stop iterating.
    fn read_dir(
        inode: &INode<Self>,
        pos: &mut i64,
        emit: &mut dyn FnMut(&[u8], u64, u8) -> bool,
    ) -> Result<()>;

    /// Read a folio (page) for a regular file.
    ///
    /// Fill the folio with file content at the folio's file offset.
    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_>) -> Result<()>;

    /// Called after `kill_anon_super` to drop `s_fs_info` data.
    ///
    /// Default does nothing. Override if you store data via `set_fs_info`.
    fn kill_sb(_sb_ptr: *mut rko_sys::rko::fs::super_block) {}
}

/// Page size on x86_64. Used by folio page iteration.
pub(crate) const PAGE_SIZE: usize = 4096;
