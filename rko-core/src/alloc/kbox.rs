//! `KBox<T>` — kernel heap-allocated box using `Kmalloc`.

use core::alloc::Layout;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::ptr::NonNull;

use super::{AllocError, Allocator, Flags, Kmalloc};

/// A heap-allocated value using kernel allocator `A`.
pub struct Box<T: ?Sized, A: Allocator = Kmalloc> {
    ptr: NonNull<T>,
    _alloc: PhantomData<A>,
}

/// `KBox<T>` = `Box<T, Kmalloc>` — the common kernel heap box.
pub type KBox<T> = Box<T, Kmalloc>;

impl<T, A: Allocator> Box<T, A> {
    /// Allocate and initialize a value on the heap.
    pub fn new(val: T, flags: Flags) -> Result<Self, AllocError> {
        let layout = Layout::new::<T>();
        let raw = unsafe { A::realloc(None, layout, Layout::new::<()>(), flags)? };
        // Extract data pointer from NonNull<[u8]>
        let ptr = NonNull::new(raw.as_ptr() as *mut u8)
            .ok_or(AllocError)?
            .cast::<T>();
        unsafe { ptr.as_ptr().write(val) };
        Ok(Box {
            ptr,
            _alloc: PhantomData,
        })
    }

    /// Consume the box and return the inner value.
    pub fn into_inner(b: Self) -> T {
        let val = unsafe { b.ptr.as_ptr().read() };
        let layout = Layout::new::<T>();
        unsafe { A::free(b.ptr.cast(), layout) };
        core::mem::forget(b);
        val
    }
}

impl<T: ?Sized, A: Allocator> Box<T, A> {
    /// Create a `Box` from a raw non-null pointer.
    ///
    /// # Safety
    ///
    /// `ptr` must have been allocated by allocator `A` with the layout
    /// of `T`, and the caller transfers ownership.
    pub unsafe fn from_raw(ptr: NonNull<T>) -> Self {
        Box {
            ptr,
            _alloc: PhantomData,
        }
    }

    /// Consume the box and return the raw pointer without freeing.
    pub fn into_raw(b: Self) -> NonNull<T> {
        let ptr = b.ptr;
        core::mem::forget(b);
        ptr
    }

    /// Convert into a pinned box.
    pub fn into_pin(b: Self) -> Pin<Self> {
        // SAFETY: the value is heap-allocated and won't be moved.
        unsafe { Pin::new_unchecked(b) }
    }
}

impl<T, A: Allocator> Box<T, A> {
    /// Allocate and pin-initialize a value in place using a `PinInit`.
    pub fn pin_init<E>(init: impl pinned_init::PinInit<T, E>, flags: Flags) -> Result<Pin<Self>, E>
    where
        E: From<AllocError>,
    {
        let layout = Layout::new::<T>();
        let raw = unsafe { A::realloc(None, layout, Layout::new::<()>(), flags).map_err(E::from)? };
        let ptr = NonNull::new(raw.as_ptr() as *mut u8)
            .ok_or(AllocError)
            .map_err(E::from)?
            .cast::<T>();
        // SAFETY: ptr is valid, writable, and properly aligned.
        unsafe { pinned_init::PinInit::__pinned_init(init, ptr.as_ptr())? };
        let b = Box {
            ptr,
            _alloc: PhantomData,
        };
        Ok(Box::into_pin(b))
    }
}

impl<T: ?Sized, A: Allocator> Deref for Box<T, A> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { self.ptr.as_ref() }
    }
}

impl<T: ?Sized, A: Allocator> DerefMut for Box<T, A> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { self.ptr.as_mut() }
    }
}

impl<T: ?Sized, A: Allocator> Drop for Box<T, A> {
    fn drop(&mut self) {
        unsafe {
            core::ptr::drop_in_place(self.ptr.as_ptr());
            A::free(self.ptr.cast(), Layout::for_value(self.ptr.as_ref()));
        }
    }
}

// SAFETY: Box owns its value, so Send/Sync follow T.
unsafe impl<T: ?Sized + Send, A: Allocator> Send for Box<T, A> {}
unsafe impl<T: ?Sized + Sync, A: Allocator> Sync for Box<T, A> {}

// SAFETY: into_foreign/from_foreign correctly transfer ownership via raw pointer.
unsafe impl<T: Send, A: Allocator> crate::types::ForeignOwnable for Box<T, A> {
    fn into_foreign(self) -> *const core::ffi::c_void {
        Box::into_raw(self).as_ptr().cast()
    }

    unsafe fn from_foreign(ptr: *const core::ffi::c_void) -> Self {
        unsafe { Box::from_raw(NonNull::new_unchecked(ptr.cast_mut().cast())) }
    }

    unsafe fn borrow<'a>(ptr: *const core::ffi::c_void) -> &'a Self {
        // NOTE: This reinterprets the *const T as *const Box<T>.
        // Box is { ptr: NonNull<T>, _alloc: PhantomData } — not actually
        // at the address we have. Use SuperBlock::data() for typed access
        // instead of calling this directly.
        // TODO: Return a guard type instead of &Self.
        unsafe { &*((&raw const ptr).cast::<Self>()) }
    }
}
