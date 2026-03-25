//! A reference-counted pointer.
//!
//! Provides [`Arc<T>`], [`ArcBorrow<T>`], and [`UniqueArc<T>`] — the kernel
//! equivalent of `std::sync::Arc`.
//!
//! Key differences from the standard library's `Arc`:
//! 1. Backed by a kernel-compatible [`Refcount`] (will use `refcount_t` once
//!    C helpers are wired up).
//! 2. No weak references — the struct is half the size of `std::sync::Arc`.
//! 3. The data is always pinned (no `get_mut`).
//! 4. Supports unsized coercion (`Arc<Concrete>` → `Arc<dyn Trait>`).
// UPSTREAM_REF: linux/rust/kernel/sync/arc.rs

use crate::alloc::{AllocError, Flags, KBox};
use crate::sync::Refcount;
use core::alloc::Layout;
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::mem::MaybeUninit;
use core::ops::{Deref, DerefMut};
use core::pin::Pin;
use core::ptr::NonNull;

/// A reference-counted pointer to an instance of `T`.
///
/// The reference count is incremented when new instances of [`Arc`] are
/// created, and decremented when they are dropped. When the count reaches
/// zero, the underlying `T` is also dropped.
///
/// # Invariants
///
/// The reference count on an instance of [`Arc`] is always non-zero.
/// The object pointed to by [`Arc`] is always pinned.
#[repr(transparent)]
pub struct Arc<T: ?Sized> {
    ptr: NonNull<ArcInner<T>>,
    _p: PhantomData<ArcInner<T>>,
}

#[repr(C)]
struct ArcInner<T: ?Sized> {
    refcount: Refcount,
    data: T,
}

impl<T: ?Sized> ArcInner<T> {
    /// Converts a pointer to the data back to a pointer to the [`ArcInner`].
    ///
    /// Handles unsized types by computing the data offset via
    /// [`Layout::extend`], avoiding the need for `ptr_metadata`.
    ///
    /// # Safety
    ///
    /// `ptr` must have been returned by [`Arc::into_raw`] and the `Arc`
    /// must still be alive.
    unsafe fn container_of(ptr: *const T) -> NonNull<ArcInner<T>> {
        let refcount_layout = Layout::new::<Refcount>();
        // SAFETY: ptr is valid per caller contract.
        let val_layout = Layout::for_value(unsafe { &*ptr });
        // SAFETY: The layout of a real struct can't overflow.
        let val_offset = unsafe { refcount_layout.extend(val_layout).unwrap_unchecked().1 };

        // Pointer casts preserve metadata (the vtable pointer for dyn types).
        let ptr = ptr as *const ArcInner<T>;

        // SAFETY: The pointer was in-bounds when `into_raw` was called.
        let ptr = unsafe { ptr.byte_sub(val_offset) };

        // SAFETY: The pointer originated from a valid allocation.
        unsafe { NonNull::new_unchecked(ptr.cast_mut()) }
    }
}

// Allow coercion from Arc<T> to Arc<dyn Trait>.
impl<T: ?Sized + core::marker::Unsize<U>, U: ?Sized> core::ops::CoerceUnsized<Arc<U>> for Arc<T> {}

// Allow Arc<dyn Trait> dispatch.
impl<T: ?Sized + core::marker::Unsize<U>, U: ?Sized> core::ops::DispatchFromDyn<Arc<U>> for Arc<T> {}

// SAFETY: Arc is Send if T is Send+Sync (any thread may drop the last Arc).
unsafe impl<T: ?Sized + Sync + Send> Send for Arc<T> {}
// SAFETY: &Arc<T> effectively shares &T.
unsafe impl<T: ?Sized + Sync + Send> Sync for Arc<T> {}

impl<T> Arc<T> {
    /// Allocate a new reference-counted instance of `T`.
    pub fn new(contents: T, flags: Flags) -> Result<Self, AllocError> {
        // INVARIANT: The refcount is initialized to a non-zero value.
        let value = ArcInner {
            refcount: Refcount::new(1),
            data: contents,
        };

        let inner = KBox::new(value, flags)?;
        // Leak the box — Arc owns the allocation now.
        let inner = KBox::into_raw(inner);

        // SAFETY: We just created `inner` with refcount 1, owned by this Arc.
        Ok(unsafe { Self::from_inner(inner) })
    }

    /// Byte offset from the start of `ArcInner<T>` to the `data` field.
    pub const DATA_OFFSET: usize = core::mem::offset_of!(ArcInner<T>, data);
}

impl<T: ?Sized> Arc<T> {
    /// Wrap an existing [`ArcInner`] pointer.
    ///
    /// # Safety
    ///
    /// `inner` must point to a valid `ArcInner` with non-zero refcount, and
    /// the caller transfers one reference to this `Arc`.
    unsafe fn from_inner(inner: NonNull<ArcInner<T>>) -> Self {
        Arc {
            ptr: inner,
            _p: PhantomData,
        }
    }

    /// Consume the `Arc`, returning a raw pointer to the data `T`.
    ///
    /// The caller takes ownership of the refcount.
    pub fn into_raw(self) -> *const T {
        let ptr = self.ptr.as_ptr();
        core::mem::forget(self);
        // SAFETY: ptr is valid.
        unsafe { core::ptr::addr_of!((*ptr).data) }
    }

    /// Return a raw pointer to the data without consuming the `Arc`.
    pub fn as_ptr(this: &Self) -> *const T {
        let ptr = this.ptr.as_ptr();
        // SAFETY: ptr is valid — the Arc holds a reference.
        unsafe { core::ptr::addr_of!((*ptr).data) }
    }

    /// Recreate an `Arc` from a raw pointer returned by [`Arc::into_raw`].
    ///
    /// # Safety
    ///
    /// `ptr` must come from a previous call to [`Arc::into_raw`] and must
    /// not have been used to recreate an `Arc` already.
    pub unsafe fn from_raw(ptr: *const T) -> Self {
        // SAFETY: Caller guarantees ptr came from into_raw.
        let inner = unsafe { ArcInner::container_of(ptr) };
        // SAFETY: The refcount from into_raw is transferred.
        unsafe { Self::from_inner(inner) }
    }

    /// Return an [`ArcBorrow`] referencing the same object.
    #[inline]
    pub fn as_arc_borrow(&self) -> ArcBorrow<'_, T> {
        // SAFETY: The Arc is alive for the lifetime of the borrow and no
        // mutable references exist (shared Arc references only).
        unsafe { ArcBorrow::new(self.ptr) }
    }

    /// Check whether two `Arc`s point to the same allocation.
    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        core::ptr::eq(this.ptr.as_ptr(), other.ptr.as_ptr())
    }
}

impl<T: ?Sized> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The refcount is non-zero so the object is alive.
        unsafe { &self.ptr.as_ref().data }
    }
}

impl<T: ?Sized> Clone for Arc<T> {
    fn clone(&self) -> Self {
        // INVARIANT: Refcount saturates so it cannot overflow to zero.
        // SAFETY: The refcount is non-zero.
        unsafe { self.ptr.as_ref() }.refcount.inc();

        // SAFETY: We just incremented the refcount — this is the new Arc's
        // owned reference.
        unsafe { Self::from_inner(self.ptr) }
    }
}

impl<T: ?Sized> Drop for Arc<T> {
    fn drop(&mut self) {
        // SAFETY: The refcount is non-zero.
        let is_zero = unsafe { self.ptr.as_ref() }.refcount.dec_and_test();
        if is_zero {
            // The count reached zero — free the memory.
            // SAFETY: ptr was created from KBox::into_raw.
            unsafe { drop(KBox::<ArcInner<T>>::from_raw(self.ptr)) };
        }
    }
}

impl<T: ?Sized> From<UniqueArc<T>> for Arc<T> {
    fn from(item: UniqueArc<T>) -> Self {
        item.inner
    }
}

impl<T: ?Sized> From<Pin<UniqueArc<T>>> for Arc<T> {
    fn from(item: Pin<UniqueArc<T>>) -> Self {
        // SAFETY: Arc's invariant guarantees the data is pinned.
        unsafe { Pin::into_inner_unchecked(item).inner }
    }
}

// ---------------------------------------------------------------------------
// ArcBorrow
// ---------------------------------------------------------------------------

/// A borrowed reference to an [`Arc`]-managed object.
///
/// Like `&Arc<T>` but avoids the double indirection. Can be converted into
/// an owned [`Arc<T>`] when needed (which increments the refcount).
///
/// # Invariants
///
/// There are no mutable references to the underlying [`Arc`], and it remains
/// valid for the lifetime of the [`ArcBorrow`] instance.
#[repr(transparent)]
pub struct ArcBorrow<'a, T: ?Sized + 'a> {
    inner: NonNull<ArcInner<T>>,
    _p: PhantomData<&'a ()>,
}

// Allow ArcBorrow<dyn Trait> dispatch.
impl<T: ?Sized + core::marker::Unsize<U>, U: ?Sized> core::ops::DispatchFromDyn<ArcBorrow<'_, U>>
    for ArcBorrow<'_, T>
{
}

impl<T: ?Sized> Clone for ArcBorrow<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for ArcBorrow<'_, T> {}

impl<'a, T: ?Sized> ArcBorrow<'a, T> {
    /// Create a new [`ArcBorrow`].
    ///
    /// # Safety
    ///
    /// The `ArcInner` must remain valid and no mutable references must exist
    /// for the lifetime `'a`.
    unsafe fn new(inner: NonNull<ArcInner<T>>) -> Self {
        Self {
            inner,
            _p: PhantomData,
        }
    }
}

impl<T: ?Sized> From<ArcBorrow<'_, T>> for Arc<T> {
    fn from(b: ArcBorrow<'_, T>) -> Self {
        // SAFETY: The ArcBorrow guarantees the refcount is non-zero.
        // ManuallyDrop prevents the temporary Arc from decrementing.
        ManuallyDrop::new(unsafe { Arc::from_inner(b.inner) })
            .deref()
            .clone()
    }
}

impl<T: ?Sized> Deref for ArcBorrow<'_, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        // SAFETY: The underlying object is alive with no mutable references.
        unsafe { &self.inner.as_ref().data }
    }
}

// ---------------------------------------------------------------------------
// UniqueArc
// ---------------------------------------------------------------------------

/// An [`Arc`] known to have a refcount of exactly 1.
///
/// This allows mutable access to the data before sharing it. Convert to
/// an [`Arc`] via `Into<Arc<T>>` when done mutating.
///
/// # Invariants
///
/// `inner` always has a reference count of 1.
pub struct UniqueArc<T: ?Sized> {
    inner: Arc<T>,
}

impl<T> UniqueArc<T> {
    /// Allocate a new `UniqueArc`.
    pub fn new(contents: T, flags: Flags) -> Result<Self, AllocError> {
        Ok(Self {
            inner: Arc::new(contents, flags)?,
        })
    }

    /// Allocate a `UniqueArc` with uninitialized contents.
    pub fn new_uninit(flags: Flags) -> Result<UniqueArc<MaybeUninit<T>>, AllocError> {
        let value = ArcInner {
            refcount: Refcount::new(1),
            data: MaybeUninit::<T>::uninit(),
        };
        let inner = KBox::new(value, flags)?;
        let inner = KBox::into_raw(inner);
        Ok(UniqueArc {
            // SAFETY: refcount is 1.
            inner: unsafe { Arc::from_inner(inner) },
        })
    }
}

impl<T> UniqueArc<MaybeUninit<T>> {
    /// Assume the contents have been initialized.
    ///
    /// # Safety
    ///
    /// The caller must have fully initialized the `MaybeUninit<T>`.
    pub unsafe fn assume_init(self) -> UniqueArc<T> {
        let me = ManuallyDrop::new(self);
        let ptr: NonNull<ArcInner<MaybeUninit<T>>> = me.inner.ptr;
        // SAFETY: MaybeUninit<T> and T have the same layout, and repr(C)
        // ArcInner preserves that. The caller guarantees init is done.
        UniqueArc {
            inner: unsafe { Arc::from_inner(ptr.cast()) },
        }
    }

    /// Get a mutable pointer to the uninitialized data.
    pub fn as_mut_ptr(&mut self) -> *mut T {
        let ptr = self.inner.ptr.as_ptr();
        // SAFETY: ptr is valid, and we have unique access.
        unsafe { core::ptr::addr_of_mut!((*ptr).data).cast() }
    }
}

impl<T: ?Sized> Deref for UniqueArc<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T: ?Sized + Unpin> DerefMut for UniqueArc<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: refcount is 1, so no other references exist.
        unsafe { &mut self.inner.ptr.as_mut().data }
    }
}

impl<T: ?Sized> From<UniqueArc<T>> for Pin<UniqueArc<T>> {
    fn from(obj: UniqueArc<T>) -> Self {
        // SAFETY: The inner data was pinned since creation (Arc pins its data).
        unsafe { Pin::new_unchecked(obj) }
    }
}
