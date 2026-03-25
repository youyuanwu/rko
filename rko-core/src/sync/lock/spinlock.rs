//! A kernel spinlock.
//!
//! Backend for `Lock<T, SpinLockBackend>`, wrapping the kernel's `spinlock_t`.
// UPSTREAM_REF: linux/rust/kernel/sync/lock/spinlock.rs

/// Creates a [`SpinLock`] initializer with a freshly-created lock class.
///
/// It uses the name if one is given, otherwise it uses `"spinlock"`.
#[macro_export]
macro_rules! new_spinlock {
    ($inner:expr $(, $name:literal)? $(,)?) => {
        $crate::sync::SpinLock::new($inner, c"spinlock", $crate::static_lock_class!())
    };
}
pub use new_spinlock;

/// A spinlock — `Lock<T, SpinLockBackend>`.
///
/// Exposes the kernel's `spinlock_t`. Other CPUs spin-wait when the
/// lock is held.
pub type SpinLock<T> = super::Lock<T, SpinLockBackend>;

/// A guard for [`SpinLock`].
pub type SpinLockGuard<'a, T> = super::Guard<'a, T, SpinLockBackend>;

/// Backend implementing `spinlock_t` operations.
pub struct SpinLockBackend;

// SAFETY: The underlying kernel `spinlock_t` ensures mutual exclusion.
unsafe impl super::Backend for SpinLockBackend {
    type State = rko_sys::rko::sync::spinlock_t;
    type GuardState = ();

    unsafe fn init(
        ptr: *mut Self::State,
        name: *const core::ffi::c_char,
        key: *mut rko_sys::rko::fs::lock_class_key,
    ) {
        // SAFETY: ptr, name, key are valid per caller contract.
        unsafe { rko_sys::rko::helpers::rust_helper___spin_lock_init(ptr, name, key) };
    }

    unsafe fn lock(ptr: *mut Self::State) -> Self::GuardState {
        // SAFETY: ptr is valid and the lock is initialized.
        unsafe { rko_sys::rko::helpers::rust_helper_spin_lock(ptr) };
    }

    unsafe fn try_lock(ptr: *mut Self::State) -> Option<Self::GuardState> {
        // SAFETY: ptr is valid and the lock is initialized.
        // spin_trylock returns non-zero on success, 0 on failure.
        if unsafe { rko_sys::rko::helpers::rust_helper_spin_trylock(ptr) } != 0 {
            Some(())
        } else {
            None
        }
    }

    unsafe fn unlock(ptr: *mut Self::State, _guard_state: &Self::GuardState) {
        // SAFETY: ptr is valid and the lock is held.
        unsafe { rko_sys::rko::helpers::rust_helper_spin_unlock(ptr) };
    }

    unsafe fn assert_is_held(ptr: *mut Self::State) {
        // SAFETY: ptr is valid. Debug assertion that the spinlock is held.
        debug_assert!(unsafe { rko_sys::rko::helpers::rust_helper_spin_is_locked(ptr) } != 0);
    }
}
