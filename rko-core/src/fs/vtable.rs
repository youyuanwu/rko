// SPDX-License-Identifier: GPL-2.0

//! C callback trampolines and vtable wiring for filesystem types.
//!
//! This module provides `Tables<T: FileSystem>` which holds the static
//! operation tables (super_operations, inode_operations, file_operations,
//! address_space_operations) with C-ABI trampolines that dispatch to
//! the Rust `FileSystem` trait methods.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

use rko_sys::rko::{
    dcache as dcache_b, fs as bindings, fs_context as fc_bindings, helpers as bindings_h,
    slab as slab_b, statfs as statfs_b, xattr as xattr_b,
};

use super::inode::INodeWithData;
use super::registration::Registration;
use super::sb::SuperBlock;
use super::{FileSystem, LockedFolio};

/// Static operation tables for a filesystem type `T`.
///
/// The tables are constructed once per type and their pointers are stored
/// in the kernel's inode/superblock/file structures.
#[repr(C)]
pub struct Tables<T: FileSystem> {
    pub(crate) super_ops: bindings::super_operations,
    pub(crate) dir_inode_ops: bindings::inode_operations,
    pub(crate) dir_file_ops: bindings::file_operations,
    pub(crate) reg_inode_ops: bindings::inode_operations,
    pub(crate) reg_file_ops: bindings::file_operations,
    pub(crate) reg_aops: bindings::address_space_operations,
    pub(crate) symlink_inode_ops: bindings::inode_operations,
    pub(crate) xattr_handler: xattr_b::xattr_handler,
    /// NULL-terminated array of xattr handler pointers. Set on s_xattr.
    pub(crate) xattr_handlers: [*const xattr_b::xattr_handler; 2],
    _marker: core::marker::PhantomData<T>,
}

// SAFETY: Tables contains only function pointers (as *mut isize) which are
// inherently thread-safe since they point to static code.
unsafe impl<T: FileSystem> Send for Tables<T> {}
unsafe impl<T: FileSystem> Sync for Tables<T> {}

impl<T: FileSystem> Default for Tables<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: FileSystem> Tables<T> {
    /// Builds the operation tables for filesystem type `T`.
    pub const fn new() -> Self {
        Self {
            super_ops: bindings::super_operations {
                alloc_inode: if size_of::<T::INodeData>() != 0 {
                    alloc_inode_callback::<T> as *mut isize
                } else {
                    core::ptr::null_mut()
                },
                destroy_inode: if size_of::<T::INodeData>() != 0 {
                    destroy_inode_callback::<T> as *mut isize
                } else {
                    core::ptr::null_mut()
                },
                statfs: if T::HAS_STATFS {
                    statfs_trampoline::<T> as *mut isize
                } else {
                    simple_statfs_trampoline as *mut isize
                },
                ..const_default_super_operations()
            },
            dir_inode_ops: bindings::inode_operations {
                lookup: lookup_trampoline::<T> as *mut isize,
                ..const_default_inode_operations()
            },
            dir_file_ops: bindings::file_operations {
                read: generic_read_dir_trampoline as *mut isize,
                iterate_shared: iterate_shared_trampoline::<T> as *mut isize,
                llseek: generic_file_llseek_trampoline as *mut isize,
                ..const_default_file_operations()
            },
            reg_inode_ops: const_default_inode_operations(),
            reg_file_ops: bindings::file_operations {
                llseek: if <T as super::file::Operations>::HAS_SEEK {
                    seek_trampoline::<T> as *mut isize
                } else {
                    generic_file_llseek_trampoline as *mut isize
                },
                // Priority: read_iter > read > generic_file_read_iter.
                // read_iter and read are mutually exclusive in the kernel.
                read: if <T as super::file::Operations>::HAS_READ_ITER {
                    core::ptr::null_mut()
                } else if <T as super::file::Operations>::HAS_READ {
                    file_read_trampoline::<T> as *mut isize
                } else {
                    core::ptr::null_mut()
                },
                read_iter: if <T as super::file::Operations>::HAS_READ_ITER {
                    custom_read_iter_trampoline::<T> as *mut isize
                } else if <T as super::file::Operations>::HAS_READ {
                    core::ptr::null_mut()
                } else {
                    read_iter_trampoline as *mut isize
                },
                ..const_default_file_operations()
            },
            reg_aops: bindings::address_space_operations {
                read_folio: read_folio_trampoline::<T> as *mut isize,
                ..const_default_address_space_operations()
            },
            symlink_inode_ops: if <T as super::inode::Operations>::HAS_GET_LINK {
                bindings::inode_operations {
                    get_link: get_link_trampoline::<T> as *mut isize,
                    ..const_default_inode_operations()
                }
            } else {
                const_default_inode_operations()
            },
            xattr_handler: xattr_b::xattr_handler {
                name: core::ptr::null_mut(),
                prefix: c"".as_ptr().cast_mut(),
                flags: 0,
                list: core::ptr::null_mut(),
                get: if T::HAS_READ_XATTR {
                    xattr_get_trampoline::<T> as *mut isize
                } else {
                    core::ptr::null_mut()
                },
                set: core::ptr::null_mut(),
            },
            // Initialized properly after construction since we can't
            // self-reference in const fn. See Tables::with_xattr_ptrs().
            xattr_handlers: [core::ptr::null(), core::ptr::null()],
            _marker: core::marker::PhantomData,
        }
    }

    /// Returns a pointer to the directory inode operations.
    pub fn dir_inode_ops(&self) -> *const bindings::inode_operations {
        &self.dir_inode_ops
    }

    /// Returns a pointer to the directory file operations.
    pub fn dir_file_ops(&self) -> *const bindings::file_operations {
        &self.dir_file_ops
    }

    /// Returns a pointer to the regular file operations.
    pub fn reg_file_ops(&self) -> *const bindings::file_operations {
        &self.reg_file_ops
    }

    /// Returns a pointer to the custom symlink inode operations.
    ///
    /// Only meaningful when `T::HAS_GET_LINK` is true.
    pub fn symlink_inode_ops(&self) -> *const bindings::inode_operations {
        &self.symlink_inode_ops
    }
}

// --- Const default helpers (zeroed, since Default isn't const) ---

const fn const_default_super_operations() -> bindings::super_operations {
    // SAFETY: All-zero is valid for super_operations (null function pointers).
    unsafe { core::mem::zeroed() }
}

const fn const_default_inode_operations() -> bindings::inode_operations {
    unsafe { core::mem::zeroed() }
}

const fn const_default_file_operations() -> bindings::file_operations {
    unsafe { core::mem::zeroed() }
}

const fn const_default_address_space_operations() -> bindings::address_space_operations {
    unsafe { core::mem::zeroed() }
}

// --- Trampolines ---

/// `super_operations::alloc_inode` — allocates from the slab cache.
unsafe extern "C" fn alloc_inode_callback<T: FileSystem>(
    sb: *mut bindings::super_block,
) -> *mut bindings::inode {
    let super_type = unsafe { (*sb).s_type };
    // SAFETY: super_type points to the fs_type field of a Registration.
    let reg = unsafe { crate::container_of!(super_type, Registration, fs_type) };
    let cache = unsafe { (*reg).inode_cache() };

    let gfp = super::registration::GFP_KERNEL;
    let obj = unsafe { bindings_h::rust_helper_alloc_inode_sb(sb, cache, gfp) };
    if obj.is_null() {
        return ptr::null_mut();
    }
    let outer = obj.cast::<INodeWithData<T::INodeData>>();
    unsafe { ptr::addr_of_mut!((*outer).inode) }
}

/// `super_operations::destroy_inode` — drops INodeData, frees to slab cache.
unsafe extern "C" fn destroy_inode_callback<T: FileSystem>(inode: *mut bindings::inode) {
    let is_bad = unsafe { bindings_h::rust_helper_is_bad_inode(inode) };

    let super_type = unsafe { (*(*inode).i_sb).s_type };
    // SAFETY: super_type points to the fs_type field of a Registration.
    let reg = unsafe { crate::container_of!(super_type, Registration, fs_type) };
    let cache = unsafe { (*reg).inode_cache() };

    let outer = unsafe { super::inode::container_of_mut::<T::INodeData>(inode.cast()) };

    if !is_bad {
        // Drop the user data.
        unsafe { ptr::drop_in_place((*outer).data.as_mut_ptr()) };
    }

    unsafe { slab_b::kmem_cache_free(cache, outer.cast()) };
}

/// `super_operations::statfs` → `T::statfs`.
///
/// Only wired when `T::HAS_STATFS` is true. Otherwise `simple_statfs`
/// is used directly.
unsafe extern "C" fn statfs_trampoline<T: FileSystem>(
    dentry: *mut dcache_b::dentry,
    buf: *mut c_void,
) -> i32 {
    let dentry_ref = unsafe { super::dentry::DEntry::<T>::from_raw(dentry) };
    match T::statfs(dentry_ref) {
        Ok(s) => {
            let kst = buf.cast::<statfs_b::kstatfs>();
            unsafe {
                (*kst).f_type = s.magic as i64;
                (*kst).f_bsize = s.bsize as i64;
                (*kst).f_blocks = s.blocks;
                (*kst).f_files = s.files;
                (*kst).f_namelen = s.namelen as i64;
            }
            0
        }
        Err(e) => e.to_errno(),
    }
}

/// `super_operations::statfs` → `simple_statfs` (kernel default).
///
/// Used when `T::HAS_STATFS` is false.
unsafe extern "C" fn simple_statfs_trampoline(
    dentry: *mut dcache_b::dentry,
    buf: *mut c_void,
) -> i32 {
    unsafe { bindings::simple_statfs(dentry, buf) }
}

/// `inode_operations::lookup` → `<T as inode::Operations>::lookup`.
///
/// The filesystem calls `Unhashed::splice_alias` to bind the dentry.
/// This trampoline just returns the result to the VFS.
unsafe extern "C" fn lookup_trampoline<T: FileSystem>(
    dir: *mut bindings::inode,
    dentry: *mut dcache_b::dentry,
    _flags: u32,
) -> *mut dcache_b::dentry {
    let inode_ref = unsafe { &*(dir as *const super::INode<T>) };
    // SAFETY: The VFS holds i_rwsem in shared mode during lookup.
    let parent = unsafe { crate::types::Locked::borrowed(inode_ref) };
    let unhashed = unsafe { super::Unhashed::<T>::from_raw(dentry) };

    crate::error::to_err_ptr(
        match <T as super::inode::Operations>::lookup(&parent, unhashed) {
            Ok(Some(aref)) => Ok(crate::types::ARef::into_raw(aref).as_ptr().cast()),
            Ok(None) => Ok(core::ptr::null_mut()),
            Err(e) => Err(e),
        },
    )
}

/// `inode_operations::get_link` → `<T as inode::Operations>::get_link`.
///
/// For `Owned(CString)`, registers a `delayed_call` to drop the string
/// when the kernel is done. For `Borrowed(&CStr)`, returns the pointer
/// directly.
unsafe extern "C" fn get_link_trampoline<T: FileSystem>(
    dentry_ptr: *mut dcache_b::dentry,
    inode_ptr: *mut bindings::inode,
    delayed_call: *mut bindings::delayed_call,
) -> *const i8 {
    /// Cleanup callback for heap-allocated CString targets.
    unsafe extern "C" fn drop_cstring(ptr: *mut c_void) {
        unsafe { <crate::types::CString as crate::types::ForeignOwnable>::from_foreign(ptr) };
    }

    let dentry = if dentry_ptr.is_null() {
        None
    } else {
        Some(unsafe { super::dentry::DEntry::<T>::from_raw(dentry_ptr) })
    };
    let inode = unsafe { &*(inode_ptr as *const super::INode<T>) };

    match <T as super::inode::Operations>::get_link(dentry, inode) {
        Err(e) => crate::error::to_err_ptr::<i8>(Err(e)).cast(),
        Ok(super::inode::GetLinkResult::Borrowed(cstr)) => cstr.as_ptr(),
        Ok(super::inode::GetLinkResult::Owned(cstring)) => {
            let ptr = crate::types::ForeignOwnable::into_foreign(cstring);
            unsafe {
                bindings_h::rust_helper_set_delayed_call(
                    delayed_call,
                    drop_cstring as *mut isize,
                    ptr as *mut c_void,
                )
            };
            ptr.cast()
        }
    }
}

/// `file_operations::iterate_shared` → `<T as file::Operations>::read_dir`.
unsafe extern "C" fn iterate_shared_trampoline<T: FileSystem>(
    file_ptr: *mut bindings::file,
    ctx: *mut bindings::dir_context,
) -> i32 {
    crate::error::from_result(|| {
        let file = unsafe { super::File::<T>::from_raw(file_ptr) };
        let inode = unsafe { bindings_h::rust_helper_file_inode(file_ptr) };
        let inode_ref = unsafe { &*(inode as *const super::INode<T>) };
        // SAFETY: The VFS holds i_rwsem in shared mode during iterate_shared.
        let locked = unsafe { crate::types::Locked::borrowed(inode_ref) };
        let emitter = unsafe { super::DirEmitter::from_raw(ctx) };
        <T as super::file::Operations>::read_dir(file, &locked, emitter)
    })
}

/// `file_operations::read` → `generic_read_dir` (kernel-provided).
unsafe extern "C" fn generic_read_dir_trampoline(
    file: *mut bindings::file,
    buf: *mut i8,
    size: usize,
    pos: *mut i64,
) -> isize {
    unsafe { bindings::generic_read_dir(file, buf, size as u64, pos) as isize }
}

/// `file_operations::llseek` → `generic_file_llseek`.
unsafe extern "C" fn generic_file_llseek_trampoline(
    file: *mut bindings::file,
    offset: i64,
    whence: i32,
) -> i64 {
    unsafe { bindings::generic_file_llseek(file, offset, whence) }
}

/// `file_operations::llseek` → `<T as file::Operations>::seek`.
///
/// Only wired when `T::HAS_SEEK` is true. Falls back to
/// `generic_file_llseek` when the Rust implementation returns EINVAL,
/// so filesystems only need to handle custom whence values (e.g.,
/// SEEK_DATA/SEEK_HOLE) and can delegate standard seeks automatically.
unsafe extern "C" fn seek_trampoline<T: FileSystem>(
    file: *mut bindings::file,
    offset: i64,
    whence: i32,
) -> i64 {
    let file_ref = unsafe { super::File::<T>::from_raw(file) };
    let w = match super::Whence::from_raw(whence) {
        Some(w) => w,
        None => return crate::error::Error::EINVAL.to_errno() as i64,
    };
    match <T as super::file::Operations>::seek(file_ref, offset, w) {
        Ok(pos) => pos,
        Err(e) => {
            let errno = e.to_errno();
            if errno == crate::error::Error::EINVAL.to_errno() {
                // Fall back to generic_file_llseek for standard whences.
                unsafe { bindings::generic_file_llseek(file, offset, whence) }
            } else {
                errno as i64
            }
        }
    }
}

/// `file_operations::read` → `<T as file::Operations>::read`.
///
/// Only wired when `T::HAS_READ` is true. Creates a `user::Writer` from
/// the raw userspace pointer and dispatches to the Rust callback.
unsafe extern "C" fn file_read_trampoline<T: FileSystem>(
    file_ptr: *mut bindings::file,
    buf: *mut i8,
    len: usize,
    offset: *mut i64,
) -> isize {
    let file = unsafe { super::File::<T>::from_raw(file_ptr) };
    let mut writer = unsafe { crate::user::Writer::new(buf.cast(), len) };
    let off = unsafe { &mut *offset };
    match <T as super::file::Operations>::read(file, &mut writer, off) {
        Ok(n) => n as isize,
        Err(e) => e.to_errno() as isize,
    }
}

/// `file_operations::read_iter` → `generic_file_read_iter` via helper.
unsafe extern "C" fn read_iter_trampoline(iocb: *mut bindings::kiocb, iter: *mut c_void) -> isize {
    unsafe { bindings_h::rust_helper_generic_file_read_iter(iocb, iter.cast()) as isize }
}

/// `file_operations::read_iter` → `<T as file::Operations>::read_iter`.
///
/// Only wired when `T::HAS_READ_ITER` is true. Creates an `IoVecIter`
/// from the raw iov_iter and extracts the file offset from the kiocb.
unsafe extern "C" fn custom_read_iter_trampoline<T: FileSystem>(
    iocb: *mut bindings::kiocb,
    iter: *mut c_void,
) -> isize {
    let file_ptr = unsafe { (*iocb).ki_filp };
    let file = unsafe { super::File::<T>::from_raw(file_ptr) };
    let offset = unsafe { (*iocb).ki_pos };
    let mut iov = unsafe { crate::iov::IoVecIter::from_raw(iter.cast()) };
    match <T as super::file::Operations>::read_iter(file, &mut iov, offset) {
        Ok(n) => {
            // Update ki_pos to reflect bytes read.
            unsafe { (*iocb).ki_pos += n as i64 };
            n as isize
        }
        Err(e) => e.to_errno() as isize,
    }
}

/// `address_space_operations::read_folio` → `T::read_folio`.
unsafe extern "C" fn read_folio_trampoline<T: FileSystem>(
    file: *mut bindings::file,
    folio: *mut c_void,
) -> i32 {
    // Create a PageCache-typed locked folio — the inode is accessible
    // via folio.inode() through mapping->host.
    let mut locked_folio =
        unsafe { LockedFolio::<super::folio::PageCache<T>>::from_raw(folio.cast()) };

    // Also pass the inode directly for backward compatibility.
    let inode = if !file.is_null() {
        unsafe { bindings_h::rust_helper_file_inode(file) }
    } else {
        // Readahead path: get inode from the folio's mapping->host.
        locked_folio.inode() as *const super::INode<T> as *mut bindings::inode
    };

    let inode_ref = unsafe { &*(inode as *const super::INode<T>) };

    let ret = match T::read_folio(inode_ref, &mut locked_folio) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    };

    // folio_end_read marks uptodate (on success) and unlocks.
    // Prevent LockedFolio::drop from double-unlocking.
    core::mem::forget(locked_folio);
    unsafe { bindings_h::rust_helper_folio_end_read(folio.cast(), ret == 0) };

    ret
}

/// `xattr_handler::get` → `T::read_xattr`.
unsafe extern "C" fn xattr_get_trampoline<T: FileSystem>(
    _handler: *const xattr_b::xattr_handler,
    dentry_ptr: *mut dcache_b::dentry,
    inode_ptr: *mut bindings::inode,
    name: *const i8,
    buffer: *mut c_void,
    size: usize,
) -> i32 {
    let dentry = unsafe { super::dentry::DEntry::<T>::from_raw(dentry_ptr) };
    let inode = unsafe { &*(inode_ptr as *const super::INode<T>) };
    let name = unsafe { core::ffi::CStr::from_ptr(name) };

    if buffer.is_null() || size == 0 {
        // Size query — call with empty buffer.
        match T::read_xattr(dentry, inode, name, &mut []) {
            Ok(n) => n as i32,
            Err(e) => e.to_errno(),
        }
    } else {
        let buf = unsafe { core::slice::from_raw_parts_mut(buffer.cast::<u8>(), size) };
        match T::read_xattr(dentry, inode, name, buf) {
            Ok(n) => n as i32,
            Err(e) => e.to_errno(),
        }
    }
}

// --- fs_context callbacks ---

/// Returns the `fs_context_operations` for type `T`.
///
/// Used by the filesystem module to build a `Registration`.
pub const fn fs_context_ops<T: FileSystem>() -> fc_bindings::fs_context_operations {
    fc_bindings::fs_context_operations {
        get_tree: get_tree_callback::<T> as *mut isize,
        free: core::ptr::null_mut(),
        dup: core::ptr::null_mut(),
        parse_param: core::ptr::null_mut(),
        parse_monolithic: core::ptr::null_mut(),
        reconfigure: core::ptr::null_mut(),
    }
}

/// Generic `init_fs_context` callback for filesystem type `T`.
///
/// Sets `fc->ops` to a static `fs_context_operations` table.
/// The caller must store the ops table (from `fs_context_ops::<T>()`)
/// in a static and pass its address via the Registration.
///
/// # Safety
///
/// `fc` must be a valid pointer to a kernel `fs_context`.
pub unsafe extern "C" fn init_fs_context_callback<T: FileSystem>(
    fc: *mut fc_bindings::fs_context,
) -> i32 {
    // The registration stores the ctx_ops. The init_fs_context
    // function registered in file_system_type is this trampoline.
    // We access the ctx_ops via the registration pointer stored
    // by the Registration::register method.
    //
    // For now, we rely on the Registration storing ctx_ops inline
    // and the caller setting fc->ops in their init_fs_context.
    // This is a placeholder — real wiring happens in Registration.
    let _ = fc;
    0
}

/// `fs_context_operations::get_tree` callback.
///
/// Calls `get_tree_nodev` or `get_tree_bdev` based on `T::SUPER_TYPE`.
unsafe extern "C" fn get_tree_callback<T: FileSystem>(fc: *mut fc_bindings::fs_context) -> i32 {
    match T::SUPER_TYPE {
        super::sb::Type::Independent => unsafe {
            fc_bindings::get_tree_nodev(fc, fill_super_callback::<T> as *mut isize)
        },
        super::sb::Type::BlockDev => unsafe {
            fc_bindings::get_tree_bdev(fc, fill_super_callback::<T> as *mut isize)
        },
    }
}

/// `fill_super` callback passed to `get_tree_nodev`.
unsafe extern "C" fn fill_super_callback<T: FileSystem>(
    sb: *mut bindings::super_block,
    _fc: *mut fc_bindings::fs_context,
) -> i32 {
    // fill_super gets a New-state superblock.
    let sb_new = unsafe { SuperBlock::<T, super::sb::New>::from_raw_new(sb) };

    // Set s_op and s_xattr from the tables.
    // SAFETY: TABLES is a &'static reference, safe to take pointers from.
    unsafe {
        (*sb).s_op = &T::TABLES.super_ops as *const _ as *mut _;
        let handlers =
            &T::TABLES.xattr_handlers as *const _ as *mut [*const xattr_b::xattr_handler; 2];
        (*handlers)[0] = &T::TABLES.xattr_handler;
        (*handlers)[1] = core::ptr::null();
        (*sb).s_xattr = handlers as *mut *mut c_void;
    }

    let data = match T::fill_super(sb_new, T::TABLES) {
        Ok(d) => d,
        Err(e) => return e.to_errno(),
    };

    // Store per-sb data in s_fs_info — transitions to Ready state.
    let foreign = <T::Data as crate::types::ForeignOwnable>::into_foreign(data);
    unsafe { (*sb).s_fs_info = foreign as *mut c_void };

    // init_root gets a Ready-state superblock.
    let sb_ready = unsafe { SuperBlock::<T>::from_raw(sb) };
    let root = match T::init_root(sb_ready, T::TABLES) {
        Ok(r) => r,
        Err(e) => return e.to_errno(),
    };

    unsafe { (*sb).s_root = root.as_ptr() };
    // Prevent Root from dropping — kernel owns the dentry now.
    core::mem::forget(root);

    0
}

/// `kill_sb` callback — evicts inodes first, then reclaims per-sb Data.
///
/// Order matters: `kill_*_super` → `generic_shutdown_super` evicts all
/// inodes (calling `destroy_inode` for each). INode data may reference
/// superblock data, so `s_fs_info` must remain valid until all inodes
/// are destroyed.
///
/// # Safety
///
/// `sb` must be a valid pointer to a kernel `super_block`.
pub unsafe extern "C" fn kill_sb_callback<T: FileSystem>(sb: *mut bindings::super_block) {
    // 1. Kill super first — evicts inodes, calls destroy_inode for each.
    match T::SUPER_TYPE {
        super::sb::Type::Independent => unsafe { bindings::kill_anon_super(sb) },
        super::sb::Type::BlockDev => unsafe { bindings::kill_block_super(sb) },
    }
    // 2. Now safe to reclaim per-sb data (all inodes are gone).
    let fs_info = unsafe { (*sb).s_fs_info };
    if !fs_info.is_null() {
        unsafe { (*sb).s_fs_info = ptr::null_mut() };
        let _data = unsafe { <T::Data as crate::types::ForeignOwnable>::from_foreign(fs_info) };
    }
}
