// SPDX-License-Identifier: GPL-2.0

//! Kernel test framework runtime support.
//!
//! Phase 1: lightweight test infrastructure using printk.
//! Phase 2: KUnit C integration for automatic test discovery.
//!
//! See `docs/design/features/test-framework.md`.

use rko_sys::rko::kunit as bindings;

/// Trait to normalize test return types.
///
/// Allows test functions to return either `()` or `Result<T, E>`.
pub trait TestResult {
    /// Returns `true` if the test passed.
    fn is_test_ok(&self) -> bool;
}

impl TestResult for () {
    fn is_test_ok(&self) -> bool {
        true
    }
}

impl<T, E> TestResult for Result<T, E> {
    fn is_test_ok(&self) -> bool {
        self.is_ok()
    }
}

/// Check if a test result indicates success.
///
/// Used by generated test wrappers. Not intended for direct use.
#[doc(hidden)]
pub fn is_test_result_ok(t: &impl TestResult) -> bool {
    t.is_test_ok()
}

// ---------------------------------------------------------------------------
// Phase 2: KUnit FFI wrappers
// ---------------------------------------------------------------------------

/// Returns `true` if the current task is running inside a KUnit test.
pub fn in_kunit_test() -> bool {
    !unsafe { rko_sys::rko::helpers::rust_helper_kunit_get_current_test() }.is_null()
}

/// Get a pointer to the currently-running KUnit test, or null.
#[doc(hidden)]
pub fn kunit_get_current_test() -> *mut core::ffi::c_void {
    unsafe { rko_sys::rko::helpers::rust_helper_kunit_get_current_test() }
}

/// Mark a KUnit test as failed. No-op when `CONFIG_KUNIT` is disabled.
///
/// # Safety
///
/// `test` must be a valid pointer obtained from `kunit_get_current_test()`.
#[doc(hidden)]
pub unsafe fn kunit_mark_failed(test: *mut core::ffi::c_void) {
    unsafe { rko_sys::rko::helpers::rust_helper_kunit_mark_failed(test) }
}

// Re-export generated types used by the proc macro's codegen.
#[doc(hidden)]
pub use bindings::{
    KUNIT_SKIPPED, KUNIT_SPEED_NORMAL, KUNIT_SUCCESS, kunit_attributes, kunit_case, kunit_status,
    kunit_suite,
};

/// Create a `kunit_case` entry for a test function.
///
/// Used by `#[rko_tests]` generated code.
#[doc(hidden)]
pub const fn new_kunit_case(
    name: *const core::ffi::c_char,
    run_case: unsafe extern "C" fn(*mut core::ffi::c_void),
) -> bindings::kunit_case {
    bindings::kunit_case {
        // bnd-winmd represents function pointers as `*mut isize`.
        run_case: run_case as *mut isize,
        name: name as *mut _,
        generate_params: core::ptr::null_mut(),
        attr: bindings::kunit_attributes {
            speed: bindings::KUNIT_SPEED_NORMAL,
        },
        param_init: core::ptr::null_mut(),
        param_exit: core::ptr::null_mut(),
        status: bindings::KUNIT_SUCCESS,
        module_name: core::ptr::null_mut(),
        log: core::ptr::null_mut(),
    }
}

/// Create a zeroed (NULL-terminator) `kunit_case`.
#[doc(hidden)]
pub const fn kunit_case_null() -> bindings::kunit_case {
    // SAFETY: zeroed kunit_case is valid as an array terminator.
    unsafe { core::mem::zeroed() }
}

/// Register a KUnit test suite in the `.kunit_test_suites` ELF section.
///
/// # Safety
///
/// `$test_cases` must be a zeroed-element-terminated array of valid
/// `kunit_case` entries with `'static` lifetime.
#[doc(hidden)]
#[macro_export]
macro_rules! kunit_unsafe_test_suite {
    ($name:ident, $test_cases:ident) => {
        const _: () = {
            const KUNIT_SUITE_NAME: [core::ffi::c_char; 256] = {
                let src = core::stringify!($name).as_bytes();
                let mut buf = [0i8; 256];
                if src.len() > 255 {
                    panic!(concat!(
                        "Test suite name `",
                        core::stringify!($name),
                        "` exceeds 255 bytes"
                    ));
                }
                let mut i = 0;
                while i < src.len() {
                    buf[i] = src[i] as core::ffi::c_char;
                    i += 1;
                }
                buf
            };

            static mut KUNIT_TEST_SUITE: $crate::kunit::kunit_suite = $crate::kunit::kunit_suite {
                name: KUNIT_SUITE_NAME,
                suite_init: core::ptr::null_mut(),
                suite_exit: core::ptr::null_mut(),
                init: core::ptr::null_mut(),
                exit: core::ptr::null_mut(),
                #[allow(unused_unsafe)]
                test_cases: unsafe {
                    core::ptr::addr_of_mut!($test_cases).cast::<$crate::kunit::kunit_case>()
                },
                attr: $crate::kunit::kunit_attributes {
                    speed: $crate::kunit::KUNIT_SPEED_NORMAL,
                },
                status_comment: [0; 256],
                debugfs: core::ptr::null_mut(),
                log: core::ptr::null_mut(),
                suite_init_err: 0,
                is_init: false,
            };

            #[used]
            #[allow(unused_unsafe)]
            #[unsafe(link_section = ".kunit_test_suites")]
            static mut KUNIT_TEST_SUITE_ENTRY: *const $crate::kunit::kunit_suite =
                unsafe { core::ptr::addr_of_mut!(KUNIT_TEST_SUITE) };
        };
    };
}
