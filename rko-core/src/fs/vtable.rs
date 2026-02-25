// SPDX-License-Identifier: GPL-2.0

//! C callback trampolines and vtable wiring for filesystem types.
//!
//! This module provides `Tables<T: Type>` which holds the static
//! operation tables (super_operations, inode_operations, file_operations,
//! address_space_operations) with C-ABI trampolines that dispatch to
//! the Rust `Type` trait methods.

use core::ffi::c_void;
use core::mem::size_of;
use core::ptr;

use rko_sys::rko::{
    dcache as dcache_b, fs as bindings, fs_context as fc_bindings, helpers as bindings_h,
    slab as slab_b,
};

use super::inode::INodeWithData;
use super::registration::Registration;
use super::sb::SuperBlock;
use super::{LockedFolio, Type};

/// Static operation tables for a filesystem type `T`.
///
/// The tables are constructed once per type and their pointers are stored
/// in the kernel's inode/superblock/file structures.
#[repr(C)]
pub struct Tables<T: Type> {
    pub(crate) super_ops: bindings::super_operations,
    pub(crate) dir_inode_ops: bindings::inode_operations,
    pub(crate) dir_file_ops: bindings::file_operations,
    pub(crate) reg_inode_ops: bindings::inode_operations,
    pub(crate) reg_file_ops: bindings::file_operations,
    pub(crate) reg_aops: bindings::address_space_operations,
    pub(crate) symlink_inode_ops: bindings::inode_operations,
    _marker: core::marker::PhantomData<T>,
}

// SAFETY: Tables contains only function pointers (as *mut isize) which are
// inherently thread-safe since they point to static code.
unsafe impl<T: Type> Send for Tables<T> {}
unsafe impl<T: Type> Sync for Tables<T> {}

impl<T: Type> Default for Tables<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T: Type> Tables<T> {
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
                statfs: simple_statfs_trampoline as *mut isize,
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
                llseek: generic_file_llseek_trampoline as *mut isize,
                read_iter: read_iter_trampoline as *mut isize,
                ..const_default_file_operations()
            },
            reg_aops: bindings::address_space_operations {
                read_folio: read_folio_trampoline::<T> as *mut isize,
                ..const_default_address_space_operations()
            },
            symlink_inode_ops: bindings::inode_operations {
                get_link: page_get_link_trampoline as *mut isize,
                ..const_default_inode_operations()
            },
            _marker: core::marker::PhantomData,
        }
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
unsafe extern "C" fn alloc_inode_callback<T: Type>(
    sb: *mut bindings::super_block,
) -> *mut bindings::inode {
    let super_type = unsafe { (*sb).s_type };
    // Registration.fs_type is the first field, so s_type == &Registration.
    let reg = super_type as *const Registration;
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
unsafe extern "C" fn destroy_inode_callback<T: Type>(inode: *mut bindings::inode) {
    let is_bad = unsafe { bindings_h::rust_helper_is_bad_inode(inode) };

    let super_type = unsafe { (*(*inode).i_sb).s_type };
    let reg = super_type as *const Registration;
    let cache = unsafe { (*reg).inode_cache() };

    let outer = unsafe { super::inode::container_of_mut::<T::INodeData>(inode.cast()) };

    if !is_bad {
        // Drop the user data.
        unsafe { ptr::drop_in_place((*outer).data.as_mut_ptr()) };
    }

    unsafe { slab_b::kmem_cache_free(cache, outer.cast()) };
}

/// `super_operations::statfs` → `simple_statfs` (kernel-provided).
unsafe extern "C" fn simple_statfs_trampoline(
    dentry: *mut dcache_b::dentry,
    buf: *mut c_void,
) -> i32 {
    unsafe { bindings::simple_statfs(dentry, buf) }
}

/// `inode_operations::lookup` → `T::lookup`.
unsafe extern "C" fn lookup_trampoline<T: Type>(
    dir: *mut bindings::inode,
    dentry: *mut dcache_b::dentry,
    _flags: u32,
) -> *mut dcache_b::dentry {
    let parent = unsafe { &*(dir as *const super::INode<T>) };

    // Get the name from the dentry via C helper.
    let name = unsafe {
        let name_ptr = bindings_h::rust_helper_dentry_name(dentry);
        let name_len = bindings_h::rust_helper_dentry_name_len(dentry) as usize;
        core::slice::from_raw_parts(name_ptr, name_len)
    };

    match T::lookup(parent, name, T::TABLES) {
        Ok(Some(aref)) => {
            // Convert ARef<INode<T>> to *mut inode for d_splice_alias.
            let inode_ptr = crate::types::ARef::into_raw(aref);
            unsafe { dcache_b::d_splice_alias(inode_ptr.as_ptr().cast(), dentry) }
        }
        Ok(None) => {
            // Negative dentry — splice a NULL inode.
            unsafe { dcache_b::d_splice_alias(core::ptr::null_mut(), dentry) }
        }
        Err(e) => unsafe { bindings_h::rust_helper_ERR_PTR(e.to_errno() as i64) }.cast(),
    }
}

/// `file_operations::iterate_shared` → `T::read_dir`.
unsafe extern "C" fn iterate_shared_trampoline<T: Type>(
    file: *mut bindings::file,
    ctx: *mut bindings::dir_context,
) -> i32 {
    let inode = unsafe { bindings_h::rust_helper_file_inode(file) };
    let inode_ref = unsafe { &*(inode as *const super::INode<T>) };

    let pos = unsafe { &mut (*ctx).pos };

    let mut emit = |name: &[u8], ino: u64, typ: u8| -> bool {
        unsafe {
            bindings_h::rust_helper_dir_emit(ctx, name.as_ptr().cast(), name.len() as i32, ino, typ)
        }
    };

    match T::read_dir(inode_ref, pos, &mut emit) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
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

/// `file_operations::read_iter` → `generic_file_read_iter` via helper.
unsafe extern "C" fn read_iter_trampoline(iocb: *mut bindings::kiocb, iter: *mut c_void) -> isize {
    unsafe { bindings_h::rust_helper_generic_file_read_iter(iocb, iter.cast()) as isize }
}

/// `address_space_operations::read_folio` → `T::read_folio`.
unsafe extern "C" fn read_folio_trampoline<T: Type>(
    file: *mut bindings::file,
    folio: *mut c_void,
) -> i32 {
    // For readahead (file is null), we can't easily get inode from folio
    // since folio is opaque. Use file_inode when file is available.
    let inode = if !file.is_null() {
        unsafe { bindings_h::rust_helper_file_inode(file) }
    } else {
        // Fallback: shouldn't happen for a simple ROFS, return error.
        unsafe { bindings_h::rust_helper_folio_end_read(folio.cast(), false) };
        return -5; // EIO
    };

    let inode_ref = unsafe { &*(inode as *const super::INode<T>) };
    let mut locked_folio = unsafe { LockedFolio::from_raw(folio.cast()) };

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

/// `inode_operations::get_link` → `page_get_link` (kernel-provided).
unsafe extern "C" fn page_get_link_trampoline(
    dentry: *mut dcache_b::dentry,
    inode: *mut bindings::inode,
    done: *mut bindings::delayed_call,
) -> *mut i8 {
    unsafe { bindings::page_get_link(dentry, inode, done) }
}

// --- fs_context callbacks ---

/// Returns the `fs_context_operations` for type `T`.
///
/// Used by the filesystem module to build a `Registration`.
pub const fn fs_context_ops<T: Type>() -> fc_bindings::fs_context_operations {
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
pub unsafe extern "C" fn init_fs_context_callback<T: Type>(
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
/// Calls `get_tree_nodev` with `fill_super_callback::<T>`.
unsafe extern "C" fn get_tree_callback<T: Type>(fc: *mut fc_bindings::fs_context) -> i32 {
    unsafe { fc_bindings::get_tree_nodev(fc, fill_super_callback::<T> as *mut isize) }
}

/// `fill_super` callback passed to `get_tree_nodev`.
unsafe extern "C" fn fill_super_callback<T: Type>(
    sb: *mut bindings::super_block,
    _fc: *mut fc_bindings::fs_context,
) -> i32 {
    let sb_ref = unsafe { SuperBlock::<T>::from_raw(sb) };

    // Set s_op from the tables.
    unsafe {
        (*sb).s_op = &T::TABLES.super_ops as *const _ as *mut _;
    }

    match T::fill_super(sb_ref, T::TABLES) {
        Ok(()) => 0,
        Err(e) => e.to_errno(),
    }
}

/// `kill_sb` callback — delegates to `kill_anon_super`, then drops fs data.
///
/// # Safety
///
/// `sb` must be a valid pointer to a kernel `super_block`.
pub unsafe extern "C" fn kill_sb_callback<T: Type>(sb: *mut bindings::super_block) {
    unsafe { bindings::kill_anon_super(sb) };
    T::kill_sb(sb);
}
