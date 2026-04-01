// SPDX-License-Identifier: GPL-2.0

//! Directory entry (dentry) abstractions.
//!
//! Provides `DEntry<T>` (ref-counted dentry wrapper), `Unhashed<T>`
//! (type-state for unhashed dentries in lookup), and `Root<T>`
//! (root dentry wrapper for `init_root`).

use core::marker::PhantomData;
use core::ptr;

use crate::error::Error;
use crate::types::{ARef, AlwaysRefCounted, Opaque};
use rko_sys::rko::{dcache as bindings, helpers as bindings_h};

use super::inode::INode;
use super::sb::SuperBlock;

type Result<T = ()> = core::result::Result<T, Error>;

/// Wraps the kernel's `struct dentry`.
///
/// # Invariants
///
/// Instances are always ref-counted via `dget`/`dput`.
#[repr(transparent)]
pub struct DEntry<T: super::FileSystem>(Opaque<bindings::dentry>, PhantomData<T>);

// SAFETY: Ref-counted via dget/dput. These are atomic operations.
unsafe impl<T: super::FileSystem> AlwaysRefCounted for DEntry<T> {
    fn inc_ref(&self) {
        // SAFETY: Shared reference implies valid dentry with non-zero refcount.
        unsafe { bindings::dget(self.0.get()) };
    }

    unsafe fn dec_ref(obj: ptr::NonNull<Self>) {
        // SAFETY: Caller guarantees non-zero refcount.
        unsafe { bindings::dput(obj.cast().as_ptr()) }
    }
}

// SAFETY: Dentries are safe to share across threads — protected by d_lock.
unsafe impl<T: super::FileSystem> Send for DEntry<T> {}
unsafe impl<T: super::FileSystem> Sync for DEntry<T> {}

impl<T: super::FileSystem> DEntry<T> {
    /// Creates a reference from a raw pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must be a valid, non-null dentry pointer. The caller must
    /// ensure the lifetime of the reference does not outlive the dentry.
    pub(crate) unsafe fn from_raw<'a>(ptr: *mut bindings::dentry) -> &'a Self {
        unsafe { &*ptr.cast() }
    }

    /// Returns the raw dentry pointer.
    pub fn as_ptr(&self) -> *mut bindings::dentry {
        self.0.get()
    }

    /// Returns the super_block that this dentry belongs to.
    pub fn super_block(&self) -> &SuperBlock<T> {
        // SAFETY: d_sb is always valid for a live dentry.
        unsafe { SuperBlock::from_raw((*self.0.get()).d_sb.cast()) }
    }
}

/// An unhashed dentry — passed to `lookup` implementations.
///
/// The name is stable because unhashed dentries are not subject to
/// rename operations.
pub struct Unhashed<'a, T: super::FileSystem>(&'a DEntry<T>);

impl<'a, T: super::FileSystem> Unhashed<'a, T> {
    /// Creates an `Unhashed` from a raw dentry pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must point to a valid, unhashed dentry. The caller must
    /// ensure the lifetime is correct.
    #[allow(dead_code)] // Will be used when lookup trampoline passes Unhashed
    pub(crate) unsafe fn from_raw(ptr: *mut bindings::dentry) -> Self {
        unsafe { Self(DEntry::from_raw(ptr)) }
    }

    /// The name being looked up.
    pub fn name(&self) -> &[u8] {
        unsafe {
            let name_ptr = bindings_h::rust_helper_dentry_name(self.0.as_ptr());
            let name_len = bindings_h::rust_helper_dentry_name_len(self.0.as_ptr()) as usize;
            core::slice::from_raw_parts(name_ptr, name_len)
        }
    }

    /// Returns the raw dentry pointer.
    pub fn as_ptr(&self) -> *mut bindings::dentry {
        self.0.as_ptr()
    }

    /// Bind this dentry to an inode (or `None` for a negative dentry).
    ///
    /// Calls `d_splice_alias` internally. On success, returns the
    /// resulting dentry (which may differ from self if an alias exists).
    /// Returns `None` if the result is a negative dentry.
    pub fn splice_alias(self, inode: Option<ARef<INode<T>>>) -> Result<Option<ARef<DEntry<T>>>> {
        let inode_ptr = match inode {
            Some(aref) => ARef::into_raw(aref).as_ptr().cast(),
            None => ptr::null_mut(),
        };

        let result = unsafe { bindings::d_splice_alias(inode_ptr, self.as_ptr()) };

        if result.is_null() {
            // Negative dentry or successfully attached to the input dentry.
            Ok(None)
        } else if (result as isize) < 0 {
            // Error — encoded as ERR_PTR.
            Err(Error::new(result as i32))
        } else {
            // Got an existing alias dentry — d_splice_alias returned
            // it with an incremented refcount.
            let aref = unsafe { ARef::from_raw(ptr::NonNull::new_unchecked(result.cast())) };
            Ok(Some(aref))
        }
    }
}

impl<T: super::FileSystem> core::ops::Deref for Unhashed<'_, T> {
    type Target = DEntry<T>;
    fn deref(&self) -> &DEntry<T> {
        self.0
    }
}

/// A dentry to be used as a filesystem root.
///
/// Returned by `init_root` implementations. Created via `try_new`
/// which calls `d_make_root` internally.
pub struct Root<T: super::FileSystem>(ARef<DEntry<T>>);

impl<T: super::FileSystem> Root<T> {
    /// Create a root dentry from a root inode.
    ///
    /// Calls `d_make_root`, which consumes the inode reference
    /// (calls `iput` on failure).
    pub fn try_new(inode: ARef<INode<T>>) -> Result<Self> {
        let inode_ptr = ARef::into_raw(inode);
        let dentry = unsafe { bindings::d_make_root(inode_ptr.as_ptr().cast()) };
        if dentry.is_null() {
            return Err(Error::new(-12)); // ENOMEM
        }
        // SAFETY: d_make_root returns a dentry with refcount=1.
        let aref = unsafe { ARef::from_raw(ptr::NonNull::new_unchecked(dentry.cast())) };
        Ok(Self(aref))
    }

    /// Returns the raw dentry pointer.
    pub fn as_ptr(&self) -> *mut bindings::dentry {
        self.0.0.get()
    }

    /// Consume the Root and return the inner ARef.
    pub fn into_inner(self) -> ARef<DEntry<T>> {
        self.0
    }
}

impl<T: super::FileSystem> core::ops::Deref for Root<T> {
    type Target = DEntry<T>;
    fn deref(&self) -> &DEntry<T> {
        &self.0
    }
}
