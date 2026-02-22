//! Helpers for kernel module registration.
//!
//! Provides macros for `MODULE_LICENSE`, `MODULE_AUTHOR`, `MODULE_DESCRIPTION`,
//! and the module init/exit entry points. These are all macro/section-attribute
//! constructs that cannot be auto-generated from C headers.

/// Declare the module license. Required for all kernel modules.
///
/// # Example
///
/// ```ignore
/// module_license!("GPL");
/// ```
#[macro_export]
macro_rules! module_license {
    ($license:literal) => {
        #[cfg_attr(not(target_os = "macos"), unsafe(link_section = ".modinfo"))]
        #[used]
        static _MODULE_LICENSE: &[u8] = concat!("license=", $license, "\0").as_bytes();
    };
}

/// Declare the module author.
///
/// # Example
///
/// ```ignore
/// module_author!("Your Name <email@example.com>");
/// ```
#[macro_export]
macro_rules! module_author {
    ($author:literal) => {
        #[cfg_attr(not(target_os = "macos"), unsafe(link_section = ".modinfo"))]
        #[used]
        static _MODULE_AUTHOR: &[u8] = concat!("author=", $author, "\0").as_bytes();
    };
}

/// Declare the module description.
///
/// # Example
///
/// ```ignore
/// module_description!("A simple kernel module");
/// ```
#[macro_export]
macro_rules! module_description {
    ($desc:literal) => {
        #[cfg_attr(not(target_os = "macos"), unsafe(link_section = ".modinfo"))]
        #[used]
        static _MODULE_DESCRIPTION: &[u8] = concat!("description=", $desc, "\0").as_bytes();
    };
}
