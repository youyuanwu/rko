// SPDX-License-Identifier: GPL-2.0

//! INode wrappers for filesystem implementations.

use core::marker::PhantomData;
use core::mem::{ManuallyDrop, MaybeUninit};
use core::ptr;

use crate::error::Error;
use crate::types::{ARef, AlwaysRefCounted, Lockable, Opaque};
use rko_sys::rko::{fs as bindings, helpers as bindings_h};

type Result<T = ()> = core::result::Result<T, Error>;

/// File type constants from `<linux/stat.h>`.
pub const S_IFDIR: u16 = 0o040000;
pub const S_IFREG: u16 = 0o100000;
pub const S_IFLNK: u16 = 0o120000;
pub const S_IFCHR: u16 = 0o020000;
pub const S_IFBLK: u16 = 0o060000;
pub const S_IFIFO: u16 = 0o010000;
pub const S_IFSOCK: u16 = 0o140000;

/// Wraps the kernel's `struct inode`.
///
/// # Invariants
///
/// Instances are always ref-counted via `ihold`/`iput`.
#[repr(transparent)]
pub struct INode<T: super::FileSystem>(Opaque<bindings::inode>, PhantomData<T>);

impl<T: super::FileSystem> INode<T> {
    /// Returns the inode number.
    pub fn ino(&self) -> u64 {
        // SAFETY: `i_ino` is immutable after creation.
        unsafe { (*self.0.get()).i_ino }
    }

    /// Returns the super-block that owns this inode.
    pub fn super_block(&self) -> *mut bindings::super_block {
        // SAFETY: `i_sb` is valid for the lifetime of the inode.
        unsafe { (*self.0.get()).i_sb }
    }

    /// Returns the per-inode data associated with this inode.
    ///
    /// # Safety
    ///
    /// The inode must have been allocated from the filesystem's slab cache
    /// and initialized via `NewINode::init`.
    pub unsafe fn data(&self) -> &T::INodeData {
        let inode_ptr = self.0.get();
        unsafe {
            let outer = container_of::<T::INodeData>(inode_ptr.cast());
            &*(*outer).data.as_ptr()
        }
    }

    /// Returns the file size in bytes (`i_size`).
    pub fn size(&self) -> super::Offset {
        unsafe { (*self.0.get()).i_size }
    }
}

// SAFETY: Ref-counted via ihold/iput.
unsafe impl<T: super::FileSystem> AlwaysRefCounted for INode<T> {
    fn inc_ref(&self) {
        // SAFETY: Shared reference implies non-zero refcount.
        unsafe { bindings::ihold(self.0.get()) };
    }

    unsafe fn dec_ref(obj: ptr::NonNull<Self>) {
        // SAFETY: Caller guarantees non-zero refcount.
        unsafe { bindings::iput(obj.cast().as_ptr()) }
    }
}

/// Marker type for the inode's read semaphore (`i_rwsem` in shared mode).
pub struct ReadSem;

// SAFETY: inode_lock_shared/inode_unlock_shared correctly acquire and
// release the inode's i_rwsem in shared (read) mode.
unsafe impl<T: super::FileSystem> Lockable<ReadSem> for INode<T> {
    fn raw_lock(&self) {
        unsafe { bindings::inode_lock_shared(self.0.get()) }
    }

    unsafe fn unlock(&self) {
        unsafe { bindings::inode_unlock_shared(self.0.get()) }
    }
}

/// Internal layout: per-inode user data followed by the kernel inode.
///
/// Allocated from the filesystem's slab cache. The kernel allocates
/// `sizeof(INodeWithData<T>)` bytes and the `inode` field sits at the
/// end, matching the `container_of` pattern.
#[repr(C)]
pub(crate) struct INodeWithData<T> {
    pub(crate) data: MaybeUninit<T>,
    pub(crate) inode: bindings::inode,
}

/// Computes the pointer to the containing `INodeWithData` from a pointer
/// to its `inode` field.
///
/// # Safety
///
/// `inode_ptr` must point to the `inode` field of a valid `INodeWithData<T>`.
unsafe fn container_of<T>(inode_ptr: *const u8) -> *const INodeWithData<T> {
    let offset = core::mem::offset_of!(INodeWithData<T>, inode);
    inode_ptr.wrapping_sub(offset).cast()
}

/// Type-safe wrapper for `*const inode_operations` bound to filesystem type `T`.
pub struct INodeOps<T: super::FileSystem>(*const bindings::inode_operations, PhantomData<T>);

impl<T: super::FileSystem> INodeOps<T> {
    /// Directory inode operations from the filesystem's Tables.
    pub fn dir(tables: &super::vtable::Tables<T>) -> Self {
        Self(tables.dir_inode_ops(), PhantomData)
    }

    /// Returns inode operations for inline symlinks (`i_link`).
    pub fn simple_symlink_inode() -> Self {
        Self(
            unsafe { bindings_h::rust_helper_simple_symlink_inode_operations() },
            PhantomData,
        )
    }

    /// Returns inode operations for page-cache symlinks.
    pub fn page_symlink_inode() -> Self {
        Self(
            unsafe { bindings_h::rust_helper_page_symlink_inode_operations() },
            PhantomData,
        )
    }

    pub(crate) fn as_ptr(&self) -> *const bindings::inode_operations {
        self.0
    }
}

/// Type-safe wrapper for `*const file_operations` bound to filesystem type `T`.
pub struct FileOps<T: super::FileSystem>(*const bindings::file_operations, PhantomData<T>);

impl<T: super::FileSystem> FileOps<T> {
    /// Directory file operations from the filesystem's Tables.
    pub fn dir(tables: &super::vtable::Tables<T>) -> Self {
        Self(tables.dir_file_ops(), PhantomData)
    }

    /// Regular file operations from the filesystem's Tables.
    pub fn regular(tables: &super::vtable::Tables<T>) -> Self {
        Self(tables.reg_file_ops(), PhantomData)
    }

    pub(crate) fn as_ptr(&self) -> *const bindings::file_operations {
        self.0
    }
}

/// Type-safe wrapper for `*const address_space_operations` bound to filesystem type `T`.
pub struct AopsOps<T: super::FileSystem>(*const bindings::address_space_operations, PhantomData<T>);

impl<T: super::FileSystem> AopsOps<T> {
    /// Creates from a raw pointer (e.g. from `RoAops::as_aops_ptr()`).
    ///
    /// # Safety
    ///
    /// The pointer must be valid for the `'static` lifetime.
    pub unsafe fn from_raw(ptr: *const bindings::address_space_operations) -> Self {
        Self(ptr, PhantomData)
    }

    pub(crate) fn as_ptr(&self) -> *const bindings::address_space_operations {
        self.0
    }
}

/// A locked, uninitialized inode returned by `SuperBlock::get_or_create_inode`.
///
/// Must be initialized via [`init`](NewINode::init) or it will be failed
/// via `iget_failed` on drop.
pub struct NewINode<T: super::FileSystem> {
    inner: ARef<INode<T>>,
    custom_iops: Option<*const bindings::inode_operations>,
    custom_fops: Option<*const bindings::file_operations>,
    custom_aops: Option<*const bindings::address_space_operations>,
}

impl<T: super::FileSystem> NewINode<T> {
    /// Creates a `NewINode` from a raw ARef. Used internally.
    pub(crate) fn new(inode: ARef<INode<T>>) -> Self {
        Self {
            inner: inode,
            custom_iops: None,
            custom_fops: None,
            custom_aops: None,
        }
    }

    /// Override inode operations for this inode.
    pub fn set_iops(&mut self, ops: INodeOps<T>) -> &mut Self {
        self.custom_iops = Some(ops.as_ptr());
        self
    }

    /// Override file operations for this inode.
    pub fn set_fops(&mut self, ops: FileOps<T>) -> &mut Self {
        self.custom_fops = Some(ops.as_ptr());
        self
    }

    /// Override address space operations for this inode.
    pub fn set_aops(&mut self, ops: AopsOps<T>) -> &mut Self {
        self.custom_aops = Some(ops.as_ptr());
        self
    }

    /// Initializes the inode with the given parameters.
    ///
    /// Sets inode metadata, ops tables, and user data. On success, returns
    /// an owned ref-counted `ARef<INode<T>>`.
    pub fn init(
        self,
        params: INodeParams<T::INodeData>,
        tables: &super::vtable::Tables<T>,
    ) -> Result<ARef<INode<T>>> {
        let inode_ptr = self.inner.0.get();
        let outer = unsafe { &mut *container_of_mut::<T::INodeData>(inode_ptr.cast()) };

        // Always write data first — drop expects it initialized.
        let _ = outer.data.write(params.value);

        let inode = &mut outer.inode;

        let type_bits = match params.typ {
            INodeType::Dir => S_IFDIR,
            INodeType::Reg => S_IFREG,
            INodeType::Lnk(_) => S_IFLNK,
            INodeType::Chr(_, _) => S_IFCHR,
            INodeType::Blk(_, _) => S_IFBLK,
            INodeType::Fifo => S_IFIFO,
            INodeType::Sock => S_IFSOCK,
        };

        inode.i_mode = (params.mode & 0o777) | type_bits;
        inode.i_size = params.size;
        inode.i_blocks = params.blocks;

        inode.i_atime_sec = params.atime.secs as i64;
        inode.i_atime_nsec = params.atime.nsecs;
        inode.i_mtime_sec = params.mtime.secs as i64;
        inode.i_mtime_nsec = params.mtime.nsecs;
        inode.i_ctime_sec = params.ctime.secs as i64;
        inode.i_ctime_nsec = params.ctime.nsecs;

        unsafe {
            bindings_h::rust_helper_set_nlink(inode, params.nlink);
            bindings_h::rust_helper_i_uid_write(inode, params.uid);
            bindings_h::rust_helper_i_gid_write(inode, params.gid);

            // Set ops tables: use custom overrides if set, else defaults from Tables.
            match params.typ {
                INodeType::Dir => {
                    let iops = self
                        .custom_iops
                        .unwrap_or(&tables.dir_inode_ops as *const _);
                    inode.i_op = iops as *mut _;
                    let fops = self.custom_fops.unwrap_or(&tables.dir_file_ops as *const _);
                    bindings_h::rust_helper_inode_set_fop(inode, fops);
                }
                INodeType::Reg => {
                    let iops = self
                        .custom_iops
                        .unwrap_or(&tables.reg_inode_ops as *const _);
                    inode.i_op = iops as *mut _;
                    let fops = self.custom_fops.unwrap_or(&tables.reg_file_ops as *const _);
                    bindings_h::rust_helper_inode_set_fop(inode, fops);
                    if let Some(aops) = self.custom_aops {
                        bindings_h::rust_helper_inode_set_aops(inode, aops);
                    } else {
                        bindings_h::rust_helper_inode_set_aops(inode, &tables.reg_aops);
                    }
                    bindings_h::rust_helper_mapping_set_large_folios(ptr::addr_of_mut!(
                        inode.i_data
                    ));
                }
                INodeType::Lnk(target) => {
                    let iops = self.custom_iops.unwrap_or_else(|| {
                        if target.is_some() {
                            bindings_h::rust_helper_simple_symlink_inode_operations()
                        } else {
                            bindings_h::rust_helper_page_symlink_inode_operations()
                        }
                    });
                    inode.i_op = iops as *mut _;
                    // For page-based symlinks, prevent highmem usage.
                    if target.is_none() {
                        bindings::inode_nohighmem(inode);
                    }
                    if let Some(s) = target {
                        inode.inode__anon_4.i_link = s.as_ptr() as *mut i8;
                    }
                }
                INodeType::Chr(major, minor) => {
                    let dev =
                        bindings_h::rust_helper_MKDEV(major, minor & bindings_h::RKO_MINORMASK);
                    bindings::init_special_inode(inode, S_IFCHR, dev);
                }
                INodeType::Blk(major, minor) => {
                    let dev =
                        bindings_h::rust_helper_MKDEV(major, minor & bindings_h::RKO_MINORMASK);
                    bindings::init_special_inode(inode, S_IFBLK, dev);
                }
                INodeType::Fifo => {
                    bindings::init_special_inode(inode, S_IFIFO, 0);
                }
                INodeType::Sock => {
                    bindings::init_special_inode(inode, S_IFSOCK, 0);
                }
            }

            bindings::unlock_new_inode(inode);
        }

        // Prevent Drop from calling iget_failed, extract the ARef.
        let me = ManuallyDrop::new(self);
        Ok(unsafe { (&me.inner as *const ARef<INode<T>>).read() })
    }
}

impl<T: super::FileSystem> Drop for NewINode<T> {
    fn drop(&mut self) {
        // SAFETY: The inode was never successfully initialized.
        unsafe { bindings::iget_failed(self.inner.0.get()) };
    }
}

/// Mutable version of container_of.
pub(crate) unsafe fn container_of_mut<T>(inode_ptr: *mut u8) -> *mut INodeWithData<T> {
    let offset = core::mem::offset_of!(INodeWithData<T>, inode);
    inode_ptr.wrapping_sub(offset).cast()
}

/// The type of an inode.
pub enum INodeType {
    /// Directory.
    Dir,
    /// Regular file.
    Reg,
    /// Symbolic link.
    ///
    /// `None` — page-based symlink (target stored in page cache).
    /// `Some(target)` — inline symlink (target stored in `i_link`).
    Lnk(Option<&'static [u8]>),
    /// Character device (major, minor).
    Chr(u32, u32),
    /// Block device (major, minor).
    Blk(u32, u32),
    /// Named pipe (FIFO).
    Fifo,
    /// Unix domain socket.
    Sock,
}

/// Time specification for inode timestamps.
#[derive(Copy, Clone)]
pub struct Time {
    /// Seconds since the Unix epoch.
    pub secs: u64,
    /// Nanoseconds within the second.
    pub nsecs: u32,
}

/// Parameters for initializing a new inode.
pub struct INodeParams<T> {
    /// Access mode (lower 9 bits: rwxrwxrwx).
    pub mode: u16,
    /// Inode type.
    pub typ: INodeType,
    /// Content size in bytes.
    pub size: i64,
    /// Number of 512-byte blocks.
    pub blocks: u64,
    /// Hard link count.
    pub nlink: u32,
    /// Owner user ID.
    pub uid: u32,
    /// Owner group ID.
    pub gid: u32,
    /// Creation time.
    pub ctime: Time,
    /// Modification time.
    pub mtime: Time,
    /// Access time.
    pub atime: Time,
    /// Per-inode user data.
    pub value: T,
}

/// Inode operation callbacks for directories.
///
/// Implement this trait on your filesystem type to provide custom
/// `lookup` behavior.
pub trait Operations: Sized + Send + Sync + 'static {
    /// The filesystem type these operations belong to.
    type FileSystem: super::FileSystem;

    /// Look up a child inode by name in a directory.
    ///
    /// Use `dentry.name()` to get the name being looked up.
    /// Call `dentry.splice_alias(Some(inode))` to bind a found inode,
    /// or `dentry.splice_alias(None)` for a negative dentry.
    fn lookup(
        parent: &INode<Self::FileSystem>,
        dentry: super::Unhashed<'_, Self::FileSystem>,
    ) -> super::Result<Option<crate::types::ARef<super::DEntry<Self::FileSystem>>>>;
}
