//! Kernel type wrappers.

mod aref;
pub mod opaque;
mod scope_guard;

pub use aref::{ARef, AlwaysRefCounted};
pub use opaque::Opaque;
pub use scope_guard::ScopeGuard;
