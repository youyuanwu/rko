//! Minimal "hello world" out-of-tree kernel module using rko-core.
//!
//! Build with the kernel build system (Kbuild), not `cargo build` directly.

#![no_std]

use rko_core::{module_author, module_description, module_license, pr_info};

module_license!("GPL");
module_author!("rko");
module_description!("Hello world kernel module using rko-sys");

/// # Safety
///
/// Called by the kernel module loader. Must not be called manually.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe extern "C" fn init_module() -> core::ffi::c_int {
    unsafe { rko_core::printk::set_log_prefix(b"hello\0"); }
    pr_info!("module loaded\n");
    0
}

#[used]
#[unsafe(link_section = ".init.data")]
#[allow(non_upper_case_globals)]
static __UNIQUE_ID___ADDRESSABLE_INIT_MODULE: unsafe extern "C" fn() -> core::ffi::c_int =
    init_module;

#[unsafe(no_mangle)]
#[unsafe(link_section = ".exit.text")]
pub extern "C" fn cleanup_module() {
    pr_info!("module unloaded\n");
}

#[used]
#[unsafe(link_section = ".exit.data")]
#[allow(non_upper_case_globals)]
static __UNIQUE_ID___ADDRESSABLE_CLEANUP_MODULE: extern "C" fn() = cleanup_module;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
