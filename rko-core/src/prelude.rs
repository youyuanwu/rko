//! Convenience re-exports for kernel module authors.
//!
//! ```ignore
//! use rko_core::prelude::*;
//! ```

pub use crate::error::Error;
pub use crate::module::Module;
pub use crate::{module, module_author, module_description, module_license};
pub use crate::{pr_info, pr_warn};
