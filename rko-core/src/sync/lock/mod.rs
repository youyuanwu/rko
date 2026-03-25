//! Generic kernel lock and guard.
//!
//! Provides `Lock<T, B>` and `Guard<T, B>` parameterized over a lock
//! [`Backend`]. Concrete backends (Mutex, SpinLock) live in sub-modules.
// UPSTREAM_REF: linux/rust/kernel/sync/lock.rs

pub mod mutex;
pub mod spinlock;

use super::LockClassKey;
use crate::types::Opaque;
use core::cell::UnsafeCell;
use core::marker::PhantomPinned;

/// The "backend" of a lock.
///
/// # Safety
///
/// Implementors must ensure that only one thread/CPU may access the protected
/// data once the lock is owned (between [`lock`] and [`unlock`]).
///
/// [`lock`]: Backend::lock
/// [`unlock`]: Backend::unlock
pub unsafe trait Backend {
    /// The kernel lock state (e.g. `mutex`, `spinlock_t`).
    type State;

    /// Per-guard state carried between [`lock`] and [`unlock`].
    ///
    /// [`lock`]: Backend::lock
    /// [`unlock`]: Backend::unlock
    type GuardState;

    /// Initialize the lock.
    ///
    /// # Safety
    ///
    /// `ptr` must be valid for write; `name` and `key` must remain valid
    /// for read indefinitely.
    unsafe fn init(
        ptr: *mut Self::State,
        name: *const core::ffi::c_char,
        key: *mut rko_sys::rko::fs::lock_class_key,
    );

    /// Acquire the lock.
    ///
    /// # Safety
    ///
    /// The lock must have been initialized via [`Backend::init`].
    #[must_use]
    unsafe fn lock(ptr: *mut Self::State) -> Self::GuardState;

    /// Try to acquire the lock without blocking.
    ///
    /// # Safety
    ///
    /// The lock must have been initialized via [`Backend::init`].
    unsafe fn try_lock(ptr: *mut Self::State) -> Option<Self::GuardState>;

    /// Release the lock.
    ///
    /// # Safety
    ///
    /// The caller must be the current owner.
    unsafe fn unlock(ptr: *mut Self::State, guard_state: &Self::GuardState);

    /// Assert that the lock is held (lockdep check).
    ///
    /// # Safety
    ///
    /// The lock must have been initialized via [`Backend::init`].
    unsafe fn assert_is_held(ptr: *mut Self::State);
}

/// A mutual exclusion primitive generic over a lock [`Backend`].
// UPSTREAM_REF: linux/rust/kernel/sync/lock.rs Lock<T, B>
#[repr(C)]
pub struct Lock<T: ?Sized, B: Backend> {
    /// The kernel lock object.
    pub(crate) state: Opaque<B::State>,
    /// Pin marker — some backends are self-referential.
    _pin: PhantomPinned,
    /// The data protected by the lock.
    pub(crate) data: UnsafeCell<T>,
}

// SAFETY: Lock can be transferred across threads if the data it protects can.
unsafe impl<T: ?Sized + Send, B: Backend> Send for Lock<T, B> {}
// SAFETY: Lock serializes interior mutability, so it is Sync if T is Send.
unsafe impl<T: ?Sized + Send, B: Backend> Sync for Lock<T, B> {}

impl<T, B: Backend> Lock<T, B> {
    /// Create a new lock initializer.
    ///
    /// Returns a [`PinInit`] that initializes a `Lock` in place.
    ///
    /// [`PinInit`]: pinned_init::PinInit
    pub fn new(
        t: T,
        name: &'static core::ffi::CStr,
        key: &'static LockClassKey,
    ) -> impl pinned_init::PinInit<Self> {
        // SAFETY: We initialize every field of Lock. The backend init is
        // called with valid pointers. The state is pinned via PhantomPinned.
        unsafe {
            pinned_init::pin_init_from_closure::<_, core::convert::Infallible>(
                move |slot: *mut Self| {
                    // Initialize data field (UnsafeCell<T> is repr(transparent)).
                    core::ptr::addr_of_mut!((*slot).data).cast::<T>().write(t);
                    // Initialize _pin field.
                    core::ptr::addr_of_mut!((*slot)._pin).write(PhantomPinned);
                    // Initialize state via backend.
                    B::init(
                        Opaque::raw_get(core::ptr::addr_of_mut!((*slot).state)),
                        name.as_ptr(),
                        key.as_ptr(),
                    );
                    Ok(())
                },
            )
        }
    }
}

impl<T: ?Sized, B: Backend> Lock<T, B> {
    /// Acquire the lock and return a guard.
    #[inline]
    pub fn lock(&self) -> Guard<'_, T, B> {
        // SAFETY: The lock was initialized during construction.
        let state = unsafe { B::lock(self.state.get()) };
        // SAFETY: We just acquired the lock.
        unsafe { Guard::new(self, state) }
    }

    /// Try to acquire the lock without blocking.
    #[must_use = "if unused, the lock will be immediately unlocked"]
    #[inline]
    pub fn try_lock(&self) -> Option<Guard<'_, T, B>> {
        // SAFETY: The lock was initialized during construction.
        unsafe { B::try_lock(self.state.get()).map(|state| Guard::new(self, state)) }
    }
}

/// A lock guard that releases the lock on drop.
// UPSTREAM_REF: linux/rust/kernel/sync/lock.rs Guard
#[must_use = "the lock unlocks immediately when the guard is unused"]
pub struct Guard<'a, T: ?Sized, B: Backend> {
    pub(crate) lock: &'a Lock<T, B>,
    pub(crate) state: B::GuardState,
    // Guards are not Send — they must be released on the same CPU/thread.
    _not_send: core::marker::PhantomData<*mut ()>,
}

// SAFETY: Guard is Sync if the protected data is Sync.
unsafe impl<T: Sync + ?Sized, B: Backend> Sync for Guard<'_, T, B> {}

impl<'a, T: ?Sized, B: Backend> Guard<'a, T, B> {
    /// Construct a new guard.
    ///
    /// # Safety
    ///
    /// The caller must have just acquired the lock.
    #[inline]
    pub(crate) unsafe fn new(lock: &'a Lock<T, B>, state: B::GuardState) -> Self {
        Self {
            lock,
            state,
            _not_send: core::marker::PhantomData,
        }
    }

    /// Returns the lock that this guard originates from.
    pub fn lock_ref(&self) -> &'a Lock<T, B> {
        self.lock
    }
}

impl<T: ?Sized, B: Backend> core::ops::Deref for Guard<'_, T, B> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &Self::Target {
        // SAFETY: The caller owns the lock.
        unsafe { &*self.lock.data.get() }
    }
}

impl<T: ?Sized + Unpin, B: Backend> core::ops::DerefMut for Guard<'_, T, B> {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        // SAFETY: The caller owns the lock.
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<T: ?Sized, B: Backend> Drop for Guard<'_, T, B> {
    #[inline]
    fn drop(&mut self) {
        // SAFETY: The caller owns the lock.
        unsafe { B::unlock(self.lock.state.get(), &self.state) };
    }
}
