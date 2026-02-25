//! Convenience re-exports for kernel module authors.
//!
//! ```ignore
//! use rko_core::prelude::*;
//! ```

pub use crate::error::Error;
pub use crate::module::{InPlaceModule, Module};
pub use crate::types::{ARef, AlwaysRefCounted, Opaque};
pub use crate::{module, module_author, module_description, module_license};
pub use crate::{pr_info, pr_warn};
