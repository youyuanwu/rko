//! A kernel mutex.
//!
//! Backend for `Lock<T, MutexBackend>`, wrapping the kernel's `struct mutex`.
// UPSTREAM_REF: linux/rust/kernel/sync/lock/mutex.rs

/// Creates a [`Mutex`] initializer with a freshly-created lock class.
///
/// It uses the name if one is given, otherwise it uses `"mutex"`.
#[macro_export]
macro_rules! new_mutex {
    ($inner:expr $(, $name:literal)? $(,)?) => {
        $crate::sync::Mutex::new($inner, c"mutex", $crate::static_lock_class!())
    };
}
pub use new_mutex;

/// A mutex — `Lock<T, MutexBackend>`.
///
/// Exposes the kernel's `struct mutex`. Since it may block, it must not be
/// used in atomic/interrupt context.
pub type Mutex<T> = super::Lock<T, MutexBackend>;

/// A guard for [`Mutex`].
pub type MutexGuard<'a, T> = super::Guard<'a, T, MutexBackend>;

/// Backend implementing `struct mutex` operations.
pub struct MutexBackend;

// SAFETY: The underlying kernel `struct mutex` ensures mutual exclusion.
unsafe impl super::Backend for MutexBackend {
    type State = rko_sys::rko::sync::mutex;
    type GuardState = ();

    unsafe fn init(
        ptr: *mut Self::State,
        name: *const core::ffi::c_char,
        key: *mut rko_sys::rko::fs::lock_class_key,
    ) {
        // SAFETY: ptr, name, key are valid per caller contract.
        unsafe { rko_sys::rko::helpers::rust_helper___mutex_init(ptr, name, key) };
    }

    unsafe fn lock(ptr: *mut Self::State) -> Self::GuardState {
        // SAFETY: ptr is valid and the lock is initialized.
        unsafe { rko_sys::rko::helpers::rust_helper_mutex_lock(ptr) };
    }

    unsafe fn try_lock(ptr: *mut Self::State) -> Option<Self::GuardState> {
        // SAFETY: ptr is valid and the lock is initialized.
        // mutex_trylock returns 1 on success, 0 on failure.
        if unsafe { rko_sys::rko::helpers::rust_helper_mutex_trylock(ptr) } != 0 {
            Some(())
        } else {
            None
        }
    }

    unsafe fn unlock(ptr: *mut Self::State, _guard_state: &Self::GuardState) {
        // SAFETY: ptr is valid and the lock is held.
        unsafe { rko_sys::rko::helpers::rust_helper_mutex_unlock(ptr) };
    }

    unsafe fn assert_is_held(ptr: *mut Self::State) {
        // SAFETY: ptr is valid. Debug assertion that the mutex is held.
        debug_assert!(unsafe { rko_sys::rko::helpers::rust_helper_mutex_is_locked(ptr) });
    }
}
