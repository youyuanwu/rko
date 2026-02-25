//! Atomic reference-counted pointer for kernel objects.

use core::marker::PhantomData;
use core::ops::Deref;
use core::ptr::NonNull;

/// Trait for types that are always reference-counted by the kernel.
///
/// Kernel objects like inodes and folios have their own refcount managed
/// by kernel functions (e.g. `ihold`/`iput`). This trait provides the
/// increment/decrement hooks so `ARef<T>` can manage them automatically.
///
/// # Safety
///
/// Implementors must ensure `inc_ref` and `dec_ref` correctly call the
/// kernel's refcount functions. `dec_ref` must handle the final release
/// (freeing the object when the count reaches zero).
pub unsafe trait AlwaysRefCounted {
    /// Increment the reference count.
    fn inc_ref(&self);

    /// Decrement the reference count.
    ///
    /// # Safety
    ///
    /// The caller must hold a reference. If this is the last reference,
    /// the kernel will free the object — the pointer must not be used
    /// after this call.
    unsafe fn dec_ref(obj: NonNull<Self>);
}

/// An owned reference to a kernel ref-counted object.
///
/// `ARef<T>` represents one counted reference. On clone it increments
/// the count; on drop it decrements. The underlying object is freed by
/// the kernel when the last reference is released.
pub struct ARef<T: AlwaysRefCounted> {
    ptr: NonNull<T>,
    _p: PhantomData<T>,
}

impl<T: AlwaysRefCounted> ARef<T> {
    /// Create an `ARef` from a raw pointer, taking ownership of one
    /// existing reference.
    ///
    /// # Safety
    ///
    /// The caller must own a reference to `ptr` (i.e. the refcount has
    /// already been incremented for this reference). `ptr` must be
    /// valid and properly aligned.
    pub unsafe fn from_raw(ptr: NonNull<T>) -> Self {
        ARef {
            ptr,
            _p: PhantomData,
        }
    }

    /// Return the raw pointer without consuming the `ARef` or
    /// changing the refcount.
    pub fn as_ptr(&self) -> *const T {
        self.ptr.as_ptr()
    }

    /// Consume the `ARef` and return the raw pointer without
    /// decrementing the refcount.
    pub fn into_raw(self) -> NonNull<T> {
        let ptr = self.ptr;
        core::mem::forget(self);
        ptr
    }
}

impl<T: AlwaysRefCounted> Clone for ARef<T> {
    fn clone(&self) -> Self {
        // SAFETY: we hold a reference, so the object is alive.
        unsafe { self.ptr.as_ref() }.inc_ref();
        ARef {
            ptr: self.ptr,
            _p: PhantomData,
        }
    }
}

impl<T: AlwaysRefCounted> Deref for ARef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: we hold a reference, so the object is alive.
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: AlwaysRefCounted> Drop for ARef<T> {
    fn drop(&mut self) {
        // SAFETY: we own this reference.
        unsafe { T::dec_ref(self.ptr) };
    }
}

// SAFETY: ARef is Send/Sync if T is, since refcounting is atomic.
unsafe impl<T: AlwaysRefCounted + Send + Sync> Send for ARef<T> {}
unsafe impl<T: AlwaysRefCounted + Send + Sync> Sync for ARef<T> {}
