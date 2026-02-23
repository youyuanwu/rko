//! Kernel module that exercises `KVec` from rko-core.

#![no_std]

use rko_core::alloc::{Flags, KVec};
use rko_core::{module_author, module_description, module_license, pr_info};

module_license!("GPL");
module_author!("rko");
module_description!("KVec test kernel module");

/// # Safety
///
/// Called by the kernel module loader. Must not be called manually.
#[unsafe(no_mangle)]
#[unsafe(link_section = ".init.text")]
pub unsafe extern "C" fn init_module() -> core::ffi::c_int {
    unsafe { rko_core::printk::set_log_prefix(b"kvec_test\0") };
    match test_kvec() {
        Ok(()) => {
            pr_info!("all kvec tests passed\n");
            0
        }
        Err(()) => {
            pr_info!("kvec test FAILED\n");
            -1
        }
    }
}

fn test_kvec() -> Result<(), ()> {
    // Test 1: push and len
    let mut v = KVec::new();
    v.push(10i32, Flags::GFP_KERNEL).map_err(|_| ())?;
    v.push(20, Flags::GFP_KERNEL).map_err(|_| ())?;
    v.push(30, Flags::GFP_KERNEL).map_err(|_| ())?;
    if v.len() != 3 {
        pr_info!("FAIL: expected len 3, got {}\n", v.len());
        return Err(());
    }
    pr_info!("PASS: push and len\n");

    // Test 2: indexing via Deref<[T]>
    if v[0] != 10 || v[1] != 20 || v[2] != 30 {
        pr_info!("FAIL: unexpected element values\n");
        return Err(());
    }
    pr_info!("PASS: indexing\n");

    // Test 3: pop
    if v.pop() != Some(30) {
        pr_info!("FAIL: pop didn't return 30\n");
        return Err(());
    }
    if v.len() != 2 {
        pr_info!("FAIL: len after pop\n");
        return Err(());
    }
    pr_info!("PASS: pop\n");

    // Test 4: with_capacity
    let v2 = KVec::<u64>::with_capacity(16, Flags::GFP_KERNEL).map_err(|_| ())?;
    if v2.capacity() < 16 {
        pr_info!("FAIL: with_capacity\n");
        return Err(());
    }
    pr_info!("PASS: with_capacity\n");

    // Test 5: extend_from_slice
    let mut v3 = KVec::new();
    v3.extend_from_slice(&[1u8, 2, 3, 4, 5], Flags::GFP_KERNEL)
        .map_err(|_| ())?;
    if v3.len() != 5 || v3[4] != 5 {
        pr_info!("FAIL: extend_from_slice\n");
        return Err(());
    }
    pr_info!("PASS: extend_from_slice\n");

    // Test 6: clear
    v3.clear();
    if !v3.is_empty() {
        pr_info!("FAIL: clear\n");
        return Err(());
    }
    pr_info!("PASS: clear\n");

    // Test 7: into_iter
    let mut v4 = KVec::new();
    v4.push(100u32, Flags::GFP_KERNEL).map_err(|_| ())?;
    v4.push(200, Flags::GFP_KERNEL).map_err(|_| ())?;
    let mut sum = 0u32;
    for x in v4 {
        sum += x;
    }
    if sum != 300 {
        pr_info!("FAIL: into_iter sum\n");
        return Err(());
    }
    pr_info!("PASS: into_iter\n");

    Ok(())
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
