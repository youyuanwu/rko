//! Filesystem abstractions.

use crate::error::Error;
use crate::types::ForeignOwnable;

pub mod dentry;
pub mod dir;
pub mod file;
mod folio;
mod inode;
pub mod iomap;
pub mod mapper;
mod registration;
pub mod sb;
pub mod vtable;

pub use dentry::{DEntry, Root, Unhashed};
pub use dir::{DirEmitter, DirEntryType, Ino, Offset, Whence};
pub use file::File;
pub use folio::{Folio, LockedFolio};
pub use inode::{INode, INodeParams, INodeType, NewINode, ReadSem, Time};
pub use inode::{S_IFBLK, S_IFCHR, S_IFDIR, S_IFIFO, S_IFLNK, S_IFREG, S_IFSOCK};
pub use mapper::Mapper;
pub use registration::Registration;
pub use sb::{SuperBlock, SuperParams};

type Result<T = ()> = core::result::Result<T, Error>;

/// Filesystem trait — implemented by each filesystem.
///
/// See `docs/design/features/fs.md` for the full design.
pub trait FileSystem: Sized + Send + Sync + 'static {
    /// Per-superblock data. Stored in `s_fs_info` and automatically
    /// dropped when the superblock is destroyed.
    type Data: ForeignOwnable + Send + Sync;

    /// Per-inode user data.
    type INodeData: Send + Sync;

    /// Filesystem name (shown in /proc/filesystems).
    const NAME: &'static core::ffi::CStr;

    /// Static operation tables for this filesystem type.
    const TABLES: &'static vtable::Tables<Self>;

    /// How superblocks are keyed. Default: `Independent` (memory-backed).
    /// Set to `BlockDev` for block-device-backed filesystems.
    const SUPER_TYPE: sb::Type = sb::Type::Independent;

    /// Initialize the superblock.
    ///
    /// Called from `get_tree_nodev` → `fill_super`. Must set up the
    /// superblock parameters (block size, magic, etc.) and return the
    /// per-superblock data.
    fn fill_super(sb: &SuperBlock<Self>, tables: &vtable::Tables<Self>) -> Result<Self::Data>;

    /// Create and return the root dentry.
    ///
    /// Called after `fill_super` completes. Create the root inode,
    /// wrap it in `dentry::Root`, and return it.
    fn init_root(sb: &SuperBlock<Self>, tables: &vtable::Tables<Self>) -> Result<Root<Self>>;

    /// Look up a child inode by name in a directory.
    ///
    /// `dentry` is the unhashed dentry being looked up. Use
    /// `dentry.name()` to get the name. Bind the result by calling
    /// `dentry.splice_alias(Some(inode))` or `dentry.splice_alias(None)`
    /// for a negative dentry.
    fn lookup(
        parent: &INode<Self>,
        dentry: Unhashed<'_, Self>,
        tables: &vtable::Tables<Self>,
    ) -> Result<Option<crate::types::ARef<DEntry<Self>>>>;

    /// Iterate directory entries.
    ///
    /// `file` is the open directory file handle. Use `emitter.pos()`
    /// for the current position and `emitter.emit()` for each entry.
    /// `emit` returns `false` when the buffer is full — stop iterating.
    fn read_dir(file: &File<Self>, inode: &INode<Self>, emitter: &mut DirEmitter) -> Result<()>;

    /// Read a folio (page) for a regular file.
    ///
    /// Fill the folio with file content at the folio's file offset.
    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_>) -> Result<()>;

    /// Read an extended attribute.
    ///
    /// Returns the number of bytes written to `outbuf`. If `outbuf`
    /// is too small, returns the required size. The kernel passes both
    /// the dentry and inode because they may differ during permission
    /// checks.
    ///
    /// Default: returns `EOPNOTSUPP`.
    fn read_xattr(
        _dentry: &DEntry<Self>,
        _inode: &INode<Self>,
        _name: &core::ffi::CStr,
        _outbuf: &mut [u8],
    ) -> Result<usize> {
        Err(Error::new(-95)) // EOPNOTSUPP
    }

    /// Get filesystem statistics.
    ///
    /// Called by `statfs(2)`. Fill in the `Stat` struct with filesystem
    /// metadata (magic, block size, block count, etc.).
    ///
    /// Default: delegates to `simple_statfs`.
    fn statfs(_dentry: &DEntry<Self>) -> Result<Stat> {
        Err(Error::new(-38)) // ENOSYS — triggers fallback to simple_statfs
    }
}

/// Filesystem statistics — subset of `struct kstatfs`.
pub struct Stat {
    /// Filesystem magic number.
    pub magic: usize,
    /// Maximum filename length.
    pub namelen: isize,
    /// Block size.
    pub bsize: isize,
    /// Total number of files (inodes).
    pub files: u64,
    /// Total number of blocks.
    pub blocks: u64,
}

/// Page size on x86_64. Used by folio page iteration.
pub(crate) const PAGE_SIZE: usize = 4096;

/// Declare a kernel module that registers a single filesystem.
///
/// This eliminates the boilerplate of creating a module struct with a
/// pinned `Registration`. The given type must implement [`FileSystem`].
///
/// # Example
///
/// ```ignore
/// use rko_core::fs;
///
/// struct MyFs;
/// impl fs::FileSystem for MyFs { /* ... */ }
///
/// rko_core::module_fs! {
///     type: MyFs,
///     name: "my_fs",
///     license: "GPL",
///     author: "rko",
///     description: "My read-only filesystem",
/// }
/// ```
#[macro_export]
macro_rules! module_fs {
    (
        type: $type:ty,
        name: $name:literal,
        license: $license:literal,
        author: $author:literal,
        description: $desc:literal $(,)?
    ) => {
        struct __FsModule {
            _reg: ::core::pin::Pin<$crate::alloc::KBox<$crate::fs::Registration>>,
        }

        impl $crate::module::Module for __FsModule {
            fn init() -> ::core::result::Result<Self, $crate::error::Error> {
                $crate::pr_info!("module loaded\n");
                let mut reg = $crate::alloc::KBox::new(
                    $crate::fs::Registration::new_for::<$type>()?,
                    $crate::alloc::Flags::GFP_KERNEL,
                )
                .map_err(|_| $crate::error::Error::new(-12))?;
                // SAFETY: KBox is heap-allocated and stable.
                unsafe { ::core::pin::Pin::new_unchecked(&mut *reg).register()? };
                let pinned = $crate::alloc::KBox::into_pin(reg);
                Ok(__FsModule { _reg: pinned })
            }

            fn exit(&self) {
                $crate::pr_info!("module unloaded\n");
            }
        }

        $crate::module! {
            type: __FsModule,
            name: $name,
            license: $license,
            author: $author,
            description: $desc,
        }
    };
}
