//! Simple RAII scope guard.

/// Runs a closure when dropped. Used for cleanup actions like `kunmap_local`.
pub struct ScopeGuard<F: FnOnce()>(Option<F>);

impl<F: FnOnce()> ScopeGuard<F> {
    /// Creates a new scope guard that will run `cleanup` on drop.
    pub fn new(cleanup: F) -> Self {
        Self(Some(cleanup))
    }
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.0.take() {
            f();
        }
    }
}
