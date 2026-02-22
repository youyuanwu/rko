//! Minimal "hello world" out-of-tree kernel module using rko-sys.
//!
//! Build with the kernel build system (Kbuild), not `cargo build` directly.

#![no_std]

use rko_sys::{module_author, module_description, module_license, printk};

module_license!("GPL");
module_author!("rko");
module_description!("Hello world kernel module using rko-sys");

/// # Safety
///
/// Called by the kernel module loader. Must not be called manually.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe extern "C" fn init_module() -> core::ffi::c_int {
    printk::_printk(c"\x016hello: module loaded\n".as_ptr());
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
    printk::_printk(c"\x016hello: module unloaded\n".as_ptr());
}

#[used]
#[unsafe(link_section = ".exit.data")]
#[allow(non_upper_case_globals)]
static __UNIQUE_ID___ADDRESSABLE_CLEANUP_MODULE: extern "C" fn() = cleanup_module;

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
