//! Work queues.
//!
//! Provides safe(r) Rust abstractions over the kernel's `struct workqueue_struct`,
//! `struct work_struct`, and associated APIs.
//!
//! The `ID` const generic allows a single type to own multiple `work_struct`
//! fields — each field gets a different ID so the compiler selects the correct
//! [`HasWork`] / [`WorkItem`] implementation at compile time.
//!
//! # The raw API
//!
//! [`RawWorkItem`] is a low-level trait whose `__enqueue` method accepts an
//! arbitrary closure that performs the actual `queue_work_on` call.
//!
//! # The safe API
//!
//! The safe API consists of:
//!
//!  * [`Work`] — Rust wrapper for `work_struct`.
//!  * [`WorkItem`] — implemented by structs that can be executed on a workqueue.
//!  * [`WorkItemPointer`] — implemented by smart pointers (`Arc<T>`) that carry a
//!    [`WorkItem`] through the workqueue.
//!  * [`HasWork`] — links a struct to its embedded [`Work`] field.
//!  * [`impl_has_work!`] — generates [`HasWork`] impls via [`container_of!`].
//!  * [`new_work!`] — creates a [`Work`] pin-initializer with a fresh lockdep class.
//!
//! # Examples
//!
//! ```ignore
//! use rko_core::sync::Arc;
//! use rko_core::workqueue::{self, Work, WorkItem, impl_has_work, new_work};
//!
//! struct MyStruct {
//!     value: i32,
//!     work: Work<MyStruct>,
//! }
//!
//! impl_has_work! {
//!     impl HasWork<Self> for MyStruct { self.work }
//! }
//!
//! impl WorkItem for MyStruct {
//!     type Pointer = Arc<MyStruct>;
//!
//!     fn run(this: Arc<MyStruct>) {
//!         pr_info!("The value is: {}\n", this.value);
//!     }
//! }
//! ```
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs

use crate::sync::{Arc, LockClassKey};
use crate::types::Opaque;
use core::marker::PhantomData;

/// Type alias for the FFI `work_struct`.
type BindingsWorkStruct = rko_sys::rko::workqueue::work_struct;
/// Type alias for the FFI `delayed_work`.
type BindingsDelayedWork = rko_sys::rko::workqueue::delayed_work;

// ---------------------------------------------------------------------------
// init_work_with_key — stub until the C helper is wired up
// ---------------------------------------------------------------------------

/// Initialize a `work_struct` with a callback and lockdep metadata.
///
/// # Safety
///
/// `work` must point at valid (possibly uninitialized) memory for a
/// `work_struct`. `name` and `key` must be valid for read.
unsafe fn init_work_with_key(
    work: *mut BindingsWorkStruct,
    func: unsafe extern "C" fn(*mut BindingsWorkStruct),
    name: *const core::ffi::c_char,
    key: *mut rko_sys::rko::fs::lock_class_key,
) {
    // SAFETY: Transmute the callback to work_func_t. On Linux,
    // `extern "C"` and `extern "system"` share the same ABI, and
    // `*mut` / `*const` are representation-identical.
    #[allow(clippy::missing_transmute_annotations)]
    let work_func: rko_sys::rko::fs::work_func_t = unsafe { Some(core::mem::transmute(func)) };

    // SAFETY: `work` points at valid memory per caller contract.
    // The helper initializes the work_struct with proper lockdep
    // metadata, list_head, and callback.
    unsafe {
        rko_sys::rko::helpers::rust_helper_init_work_with_key(work, work_func, false, name, key);
    }
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

/// A kernel work queue.
///
/// Wraps the kernel's C `struct workqueue_struct`.
///
/// Several system-wide queues are always available, e.g. [`system()`],
/// [`system_highpri()`], [`system_long()`], [`system_unbound()`].
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs Queue
#[repr(C)]
pub struct Queue {
    // workqueue_struct is fully opaque — all FFI functions accept
    // `*mut c_void` — so we use a zero-length array as a stand-in.
    _opaque: [u8; 0],
    // Prevent construction outside of `from_raw`.
    _pin: PhantomData<core::marker::PhantomPinned>,
}

// SAFETY: Kernel workqueues are designed for concurrent use.
unsafe impl Send for Queue {}
// SAFETY: Kernel workqueues are designed for concurrent use.
unsafe impl Sync for Queue {}

impl Queue {
    /// Use the provided `struct workqueue_struct` with Rust.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `ptr` points at a valid workqueue that
    /// remains valid for the lifetime `'a`.
    pub unsafe fn from_raw<'a>(ptr: *mut core::ffi::c_void) -> &'a Queue {
        // SAFETY: The caller guarantees the pointer is valid.
        unsafe { &*(ptr as *const Queue) }
    }

    /// Enqueue a work item.
    ///
    /// Returns an error (giving the item back) if the work item is already
    /// enqueued.  The item is submitted using `WORK_CPU_UNBOUND`.
    // UPSTREAM_REF: linux/rust/kernel/workqueue.rs Queue::enqueue
    pub fn enqueue<W, const ID: u64>(&self, w: W) -> W::EnqueueOutput
    where
        W: RawWorkItem<ID> + Send + 'static,
    {
        let queue_ptr = self as *const Queue as *mut core::ffi::c_void;

        // SAFETY: We only return `false` (via the closure) when the
        // `work_struct` is already queued. The `W: Send + 'static` bound
        // ensures the work item is safe to run on any thread.
        unsafe {
            w.__enqueue(move |work_ptr| {
                rko_sys::rko::workqueue::queue_work_on(
                    rko_sys::rko::workqueue::WORK_CPU_UNBOUND as i32,
                    queue_ptr,
                    work_ptr,
                )
            })
        }
    }
}

// ---------------------------------------------------------------------------
// RawWorkItem
// ---------------------------------------------------------------------------

/// A raw work item.
///
/// Low-level trait whose `__enqueue` method accepts a closure that performs the
/// actual `queue_work_on` call.
///
/// The `ID` const generic lets one type provide multiple implementations (one
/// per `work_struct` field).
///
/// # Safety
///
/// Implementers must ensure that any pointers passed to the closure by
/// [`__enqueue`](RawWorkItem::__enqueue) remain valid as documented there.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs RawWorkItem
pub unsafe trait RawWorkItem<const ID: u64> {
    /// The return type of [`Queue::enqueue`].
    type EnqueueOutput;

    /// Enqueue this work item using the provided `queue_work_on` closure.
    ///
    /// # Guarantees
    ///
    /// If this method calls the closure, the raw pointer is valid for the
    /// duration of that call.  If the closure returns `true`, the pointer
    /// remains valid until the function pointer stored in the `work_struct`
    /// is invoked by the kernel.
    ///
    /// # Safety
    ///
    /// The closure may only return `false` if the `work_struct` is already
    /// in a workqueue.
    unsafe fn __enqueue<F>(self, queue_work_on: F) -> Self::EnqueueOutput
    where
        F: FnOnce(*mut BindingsWorkStruct) -> bool;
}

// ---------------------------------------------------------------------------
// WorkItemPointer
// ---------------------------------------------------------------------------

/// Defines the `extern "C"` callback for a smart-pointer work item.
///
/// Implemented by `Arc<T>` — not usually implemented directly by user code.
///
/// # Safety
///
/// Implementers must ensure that [`__enqueue`](RawWorkItem::__enqueue) stores
/// a `work_struct` whose function pointer is [`run`](WorkItemPointer::run).
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs WorkItemPointer
pub unsafe trait WorkItemPointer<const ID: u64>: RawWorkItem<ID> {
    /// Run the work item.
    ///
    /// # Safety
    ///
    /// `ptr` must originate from a previous successful enqueue and still be
    /// valid.
    unsafe extern "C" fn run(ptr: *mut BindingsWorkStruct);
}

// ---------------------------------------------------------------------------
// WorkItem
// ---------------------------------------------------------------------------

/// Defines the method called when a work item is executed.
///
/// Implemented by user structs; the `Pointer` associated type is usually
/// `Arc<Self>`.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs WorkItem
pub trait WorkItem<const ID: u64 = 0> {
    /// The pointer type wrapping `Self` (e.g. `Arc<Self>`).
    type Pointer: WorkItemPointer<ID>;

    /// Called when the work item is dequeued and executed.
    fn run(this: Self::Pointer);
}

// ---------------------------------------------------------------------------
// Work<T, ID>
// ---------------------------------------------------------------------------

/// Links for a work item.
///
/// Wraps the kernel's `struct work_struct`.  The type parameter `T` ties
/// this work link to the struct that contains it, and `ID` disambiguates
/// when a struct has multiple `Work` fields.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs Work
#[repr(transparent)]
pub struct Work<T: ?Sized, const ID: u64 = 0> {
    work: Opaque<BindingsWorkStruct>,
    _inner: PhantomData<T>,
}

// SAFETY: Kernel work items are usable from any thread.
// The `Work` does not actually contain a `T`.
unsafe impl<T: ?Sized, const ID: u64> Send for Work<T, ID> {}
// SAFETY: Kernel work items are usable from any thread.
unsafe impl<T: ?Sized, const ID: u64> Sync for Work<T, ID> {}

impl<T: ?Sized, const ID: u64> Work<T, ID> {
    /// Create a pin-initializer for a new [`Work`].
    ///
    /// The returned initializer sets up the inner `work_struct` with the
    /// correct callback pointer for `T::Pointer::run`.
    #[inline]
    pub fn new(
        name: &'static core::ffi::CStr,
        key: &'static LockClassKey,
    ) -> impl pinned_init::PinInit<Self>
    where
        T: WorkItem<ID>,
    {
        // SAFETY: We initialize the work_struct in place via init_work_with_key.
        // The Work is #[repr(transparent)] so the slot cast is valid.
        unsafe {
            pinned_init::pin_init_from_closure::<_, core::convert::Infallible>(
                move |slot: *mut Self| {
                    init_work_with_key(
                        slot.cast::<BindingsWorkStruct>(),
                        <T as WorkItem<ID>>::Pointer::run,
                        name.as_ptr(),
                        key.as_ptr(),
                    );
                    Ok(())
                },
            )
        }
    }

    /// Initialize a [`Work`] in place from a raw pointer.
    ///
    /// This is useful when constructing a containing struct via
    /// [`UniqueArc::new_uninit`](crate::sync::UniqueArc::new_uninit).
    ///
    /// # Safety
    ///
    /// `slot` must point at valid memory for a `Work<T, ID>`.  The memory
    /// must be pinned after this call (i.e. not moved).
    pub unsafe fn init(slot: *mut Self, name: &'static core::ffi::CStr, key: &'static LockClassKey)
    where
        T: WorkItem<ID>,
    {
        // SAFETY: Caller guarantees the pointer is valid.
        unsafe {
            init_work_with_key(
                slot.cast::<BindingsWorkStruct>(),
                <T as WorkItem<ID>>::Pointer::run,
                name.as_ptr(),
                key.as_ptr(),
            );
        }
    }

    /// Get a raw pointer to the inner `work_struct`.
    ///
    /// # Safety
    ///
    /// `ptr` must not be dangling and must be properly aligned.
    #[inline]
    pub unsafe fn raw_get(ptr: *const Self) -> *mut BindingsWorkStruct {
        // SAFETY: Work is #[repr(transparent)] over Opaque<work_struct>,
        // which is #[repr(transparent)] over UnsafeCell<MaybeUninit<work_struct>>.
        ptr.cast::<BindingsWorkStruct>().cast_mut()
    }
}

// ---------------------------------------------------------------------------
// HasWork<T, ID>
// ---------------------------------------------------------------------------

/// Declares that a type contains a [`Work<T, ID>`] field.
///
/// Use [`impl_has_work!`] to implement this trait safely.
///
/// # Safety
///
/// [`raw_get_work`](HasWork::raw_get_work) and
/// [`work_container_of`](HasWork::work_container_of) must be true inverses.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs HasWork
pub unsafe trait HasWork<T, const ID: u64 = 0> {
    /// Returns a pointer to the [`Work<T, ID>`] field.
    ///
    /// # Safety
    ///
    /// `ptr` must point at a valid instance of `Self`.
    unsafe fn raw_get_work(ptr: *mut Self) -> *mut Work<T, ID>;

    /// Returns a pointer to the struct containing the [`Work<T, ID>`] field.
    ///
    /// # Safety
    ///
    /// `ptr` must point at a [`Work<T, ID>`] field inside a valid `Self`.
    unsafe fn work_container_of(ptr: *mut Work<T, ID>) -> *mut Self;
}

// ---------------------------------------------------------------------------
// impl_has_work! macro
// ---------------------------------------------------------------------------

/// Generates a [`HasWork`] implementation using [`container_of!`].
///
/// # Examples
///
/// ```ignore
/// use rko_core::workqueue::{impl_has_work, Work};
///
/// struct MyWorkItem {
///     work_field: Work<MyWorkItem, 1>,
/// }
///
/// impl_has_work! {
///     impl HasWork<MyWorkItem, 1> for MyWorkItem { self.work_field }
/// }
/// ```
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs impl_has_work!
#[macro_export]
macro_rules! impl_has_work {
    ($(impl$({$($generics:tt)*})?
       HasWork<$work_type:ty $(, $id:tt)?>
       for $self:ty
       { self.$field:ident }
    )*) => {$(
        // SAFETY: The implementation compiles only when the field has the
        // correct type, and container_of! provides the inverse mapping.
        unsafe impl$(<$($generics)+>)? $crate::workqueue::HasWork<$work_type $(, $id)?> for $self {
            #[inline]
            unsafe fn raw_get_work(
                ptr: *mut Self,
            ) -> *mut $crate::workqueue::Work<$work_type $(, $id)?> {
                // SAFETY: The caller promises that the pointer is not dangling.
                unsafe { ::core::ptr::addr_of_mut!((*ptr).$field) }
            }

            #[inline]
            unsafe fn work_container_of(
                ptr: *mut $crate::workqueue::Work<$work_type $(, $id)?>,
            ) -> *mut Self {
                // SAFETY: The caller promises the pointer points at the correct
                // field in the correct struct.
                unsafe { $crate::container_of!(ptr, Self, $field) }
            }
        }
    )*};
}
pub use impl_has_work;

// ---------------------------------------------------------------------------
// new_work! macro
// ---------------------------------------------------------------------------

/// Creates a [`Work`] pin-initializer with a fresh lockdep class.
///
/// # Examples
///
/// ```ignore
/// use rko_core::workqueue::{new_work, Work};
///
/// // In a pin-init context:
/// // work <- new_work!("MyStruct::work"),
/// ```
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs new_work!
#[macro_export]
macro_rules! new_work {
    ($name:literal) => {
        $crate::workqueue::Work::new(
            // SAFETY: concat! ensures NUL termination with no interior NULs
            // (assuming the user literal has none).
            unsafe {
                ::core::ffi::CStr::from_bytes_with_nul_unchecked(
                    ::core::concat!($name, "\0").as_bytes(),
                )
            },
            $crate::static_lock_class!(),
        )
    };
}
pub use new_work;

// ---------------------------------------------------------------------------
// DelayedWork<T, ID>
// ---------------------------------------------------------------------------

/// Links for a delayed work item.
///
/// Wraps the kernel's `struct delayed_work`.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs DelayedWork
#[repr(transparent)]
pub struct DelayedWork<T: ?Sized, const ID: u64 = 0> {
    dwork: Opaque<BindingsDelayedWork>,
    _inner: PhantomData<T>,
}

// SAFETY: Kernel work items are usable from any thread.
unsafe impl<T: ?Sized, const ID: u64> Send for DelayedWork<T, ID> {}
// SAFETY: Kernel work items are usable from any thread.
unsafe impl<T: ?Sized, const ID: u64> Sync for DelayedWork<T, ID> {}

impl<T: ?Sized, const ID: u64> DelayedWork<T, ID> {
    /// Get a pointer to the inner `work_struct` (inside the `delayed_work`).
    ///
    /// # Safety
    ///
    /// `ptr` must be aligned and not dangling.
    #[inline]
    pub unsafe fn raw_as_work(ptr: *const Self) -> *mut Work<T, ID> {
        // SAFETY: DelayedWork is #[repr(transparent)] over Opaque<delayed_work>.
        let dw: *mut BindingsDelayedWork = ptr.cast::<BindingsDelayedWork>().cast_mut();
        // SAFETY: The caller promises the pointer is valid.
        let wrk: *mut BindingsWorkStruct = unsafe { core::ptr::addr_of_mut!((*dw).work) };
        wrk.cast()
    }
}

// ---------------------------------------------------------------------------
// HasDelayedWork<T, ID>
// ---------------------------------------------------------------------------

/// Declares that a type contains a [`DelayedWork<T, ID>`].
///
/// # Safety
///
/// The [`HasWork<T, ID>`] implementation must return a `work_struct` that
/// lives inside the `work` field of a `delayed_work`.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs HasDelayedWork
pub unsafe trait HasDelayedWork<T, const ID: u64 = 0>: HasWork<T, ID> {}

// ---------------------------------------------------------------------------
// Arc<T> as WorkItemPointer / RawWorkItem
// ---------------------------------------------------------------------------

// SAFETY: The `run` callback is set during `Work::new` / `Work::init` via
// `init_work_with_key`, which stores `<Arc<T> as WorkItemPointer<ID>>::run`
// as the function pointer in the `work_struct`.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs (impl WorkItemPointer for Arc)
unsafe impl<T, const ID: u64> WorkItemPointer<ID> for Arc<T>
where
    T: WorkItem<ID, Pointer = Self>,
    T: HasWork<T, ID>,
{
    unsafe extern "C" fn run(ptr: *mut BindingsWorkStruct) {
        // The `__enqueue` method always uses a `work_struct` stored in a
        // `Work<T, ID>`.
        let ptr = ptr.cast::<Work<T, ID>>();
        // SAFETY: Recovers the container pointer that `__enqueue` computed
        // from `Arc::into_raw`.
        let ptr = unsafe { T::work_container_of(ptr) };
        // SAFETY: Ownership was transferred to the workqueue in `__enqueue`
        // via `Arc::into_raw`; we reclaim it here.
        let arc = unsafe { Arc::from_raw(ptr) };

        T::run(arc)
    }
}

// SAFETY: The `work_struct` pointer is valid for the closure's duration
// because it comes from an `Arc` (refcount ≥ 1).  If `queue_work_on`
// returns true, the pointer stays valid until the callback runs (the Arc
// was leaked via `into_raw` and is reclaimed in `WorkItemPointer::run`).
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs (impl RawWorkItem for Arc)
unsafe impl<T, const ID: u64> RawWorkItem<ID> for Arc<T>
where
    T: WorkItem<ID, Pointer = Self>,
    T: HasWork<T, ID>,
{
    type EnqueueOutput = Result<(), Self>;

    unsafe fn __enqueue<F>(self, queue_work_on: F) -> Self::EnqueueOutput
    where
        F: FnOnce(*mut BindingsWorkStruct) -> bool,
    {
        let ptr = Arc::into_raw(self).cast_mut();

        // SAFETY: Pointers into an `Arc` point at a valid value.
        let work_ptr = unsafe { T::raw_get_work(ptr) };
        // SAFETY: `raw_get_work` returns a pointer to a valid value.
        let work_ptr = unsafe { Work::raw_get(work_ptr) };

        if queue_work_on(work_ptr) {
            Ok(())
        } else {
            // SAFETY: The workqueue did not take ownership.
            Err(unsafe { Arc::from_raw(ptr) })
        }
    }
}

// ---------------------------------------------------------------------------
// System queues
// ---------------------------------------------------------------------------

// The kernel exports these as global `struct workqueue_struct *` variables.
// They are not (yet) in the rko-sys generated bindings, so we declare them
// directly.  The linker resolves them at module load time.
unsafe extern "C" {
    static system_wq: *mut core::ffi::c_void;
    static system_highpri_wq: *mut core::ffi::c_void;
    static system_long_wq: *mut core::ffi::c_void;
    static system_unbound_wq: *mut core::ffi::c_void;
}

/// Returns the system work queue (`system_wq`).
///
/// Multi-CPU, multi-threaded.  Callers should not queue long-running items.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs system()
pub fn system() -> &'static Queue {
    // SAFETY: `system_wq` is always valid while a kernel module is loaded.
    unsafe { Queue::from_raw(system_wq) }
}

/// Returns the system high-priority work queue (`system_highpri_wq`).
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs system_highpri()
pub fn system_highpri() -> &'static Queue {
    // SAFETY: `system_highpri_wq` is always valid while a module is loaded.
    unsafe { Queue::from_raw(system_highpri_wq) }
}

/// Returns the system long-running work queue (`system_long_wq`).
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs system_long()
pub fn system_long() -> &'static Queue {
    // SAFETY: `system_long_wq` is always valid while a module is loaded.
    unsafe { Queue::from_raw(system_long_wq) }
}

/// Returns the system unbound work queue (`system_unbound_wq`).
///
/// Workers are not bound to any specific CPU.
// UPSTREAM_REF: linux/rust/kernel/workqueue.rs system_unbound()
pub fn system_unbound() -> &'static Queue {
    // SAFETY: `system_unbound_wq` is always valid while a module is loaded.
    unsafe { Queue::from_raw(system_unbound_wq) }
}
