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
    ($val:literal) => {
        ::core::arch::global_asm!(
            ".section .modinfo,\"a\"",
            concat!(".ascii \"license=", $val, "\\0\""),
            ".previous",
        );
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
    ($val:literal) => {
        ::core::arch::global_asm!(
            ".section .modinfo,\"a\"",
            concat!(".ascii \"author=", $val, "\\0\""),
            ".previous",
        );
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
    ($val:literal) => {
        ::core::arch::global_asm!(
            ".section .modinfo,\"a\"",
            concat!(".ascii \"description=", $val, "\\0\""),
            ".previous",
        );
    };
}
