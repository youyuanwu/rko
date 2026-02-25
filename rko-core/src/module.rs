//! Helpers for kernel module registration.
//!
//! Provides the `Module` trait and `module!` macro for declaring kernel
//! modules. The macro generates modinfo entries, `init_module` /
//! `cleanup_module` entry points, addressability markers, and a panic handler.

/// Trait implemented by kernel modules.
///
/// The `module!` macro generates the glue code that calls `init()` on
/// module load and `exit()` + drop on module unload.
pub trait Module: Sized + Send + Sync {
    /// Called on module load. Returns the module instance or an error.
    fn init() -> Result<Self, crate::error::Error>;

    /// Called on module unload, before the instance is dropped.
    ///
    /// Override this for cleanup logging or explicit teardown. Field
    /// `Drop` impls still run automatically after `exit()` returns.
    fn exit(&self) {}
}

/// Trait for modules that require in-place (pinned) initialization.
///
/// Use this when the module state contains pinned fields (e.g. kernel
/// registrations, mutexes) that cannot be moved after initialization.
/// The `module!` macro with `init: pin` stores the instance in a
/// `Pin<KBox<T>>`.
pub trait InPlaceModule: Send + Sync {
    /// Returns a pin-initializer for the module.
    fn init() -> impl pinned_init::PinInit<Self, crate::error::Error>;

    /// Called on module unload, before the instance is dropped.
    fn exit(&self) {}
}

/// Declare the module license (`.modinfo` section). Required.
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

/// Declare the module author (`.modinfo` section).
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

/// Declare the module description (`.modinfo` section).
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

/// Declare a kernel module.
///
/// Generates all boilerplate: modinfo, init/exit entry points,
/// addressability markers, and a panic handler. The given type must
/// implement [`Module`].
///
/// # Example
///
/// ```ignore
/// use rko_core::prelude::*;
///
/// struct Hello;
///
/// impl Module for Hello {
///     fn init() -> Result<Self, Error> {
///         pr_info!("loaded\n");
///         Ok(Hello)
///     }
///     fn exit(&self) {
///         pr_info!("unloaded\n");
///     }
/// }
///
/// module! {
///     type: Hello,
///     name: "hello",
///     license: "GPL",
///     author: "rko",
///     description: "Hello world",
/// }
/// ```
#[macro_export]
macro_rules! module {
    (
        type: $type:ty,
        name: $name:literal,
        license: $license:literal,
        author: $author:literal,
        description: $desc:literal $(,)?
    ) => {
        $crate::module_license!($license);
        $crate::module_author!($author);
        $crate::module_description!($desc);

        /// Module instance storage.
        static mut __MOD: ::core::mem::MaybeUninit<$type> = ::core::mem::MaybeUninit::uninit();

        /// # Safety
        ///
        /// Called by the kernel module loader. Must not be called manually.
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".init.text")]
        pub unsafe extern "C" fn init_module() -> ::core::ffi::c_int {
            unsafe {
                $crate::printk::set_log_prefix(concat!($name, "\0").as_bytes());
            }
            match <$type as $crate::module::Module>::init() {
                Ok(m) => {
                    unsafe { __MOD.write(m) };
                    0
                }
                Err(e) => e.to_errno(),
            }
        }

        #[used]
        #[unsafe(link_section = ".init.data")]
        #[allow(non_upper_case_globals)]
        static __UNIQUE_ID___ADDRESSABLE_INIT_MODULE: unsafe extern "C" fn() -> ::core::ffi::c_int =
            init_module;

        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".exit.text")]
        pub extern "C" fn cleanup_module() {
            unsafe {
                __MOD.assume_init_ref().exit();
                __MOD.assume_init_drop();
            }
        }

        #[used]
        #[unsafe(link_section = ".exit.data")]
        #[allow(non_upper_case_globals)]
        static __UNIQUE_ID___ADDRESSABLE_CLEANUP_MODULE: extern "C" fn() = cleanup_module;

        #[panic_handler]
        fn panic(_info: &::core::panic::PanicInfo) -> ! {
            loop {}
        }
    };

    // In-place (pinned) module variant — for types implementing InPlaceModule.
    (
        type: $type:ty,
        name: $name:literal,
        init: pin,
        license: $license:literal,
        author: $author:literal,
        description: $desc:literal $(,)?
    ) => {
        $crate::module_license!($license);
        $crate::module_author!($author);
        $crate::module_description!($desc);

        /// Pinned module instance storage.
        static mut __MOD: ::core::option::Option<::core::pin::Pin<$crate::alloc::KBox<$type>>> =
            None;

        /// # Safety
        ///
        /// Called by the kernel module loader. Must not be called manually.
        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".init.text")]
        pub unsafe extern "C" fn init_module() -> ::core::ffi::c_int {
            unsafe {
                $crate::printk::set_log_prefix(concat!($name, "\0").as_bytes());
            }
            match $crate::alloc::KBox::pin_init(
                <$type as $crate::module::InPlaceModule>::init(),
                $crate::alloc::Flags::GFP_KERNEL,
            ) {
                Ok(b) => {
                    unsafe { __MOD = Some(b) };
                    0
                }
                Err(e) => e.to_errno(),
            }
        }

        #[used]
        #[unsafe(link_section = ".init.data")]
        #[allow(non_upper_case_globals)]
        static __UNIQUE_ID___ADDRESSABLE_INIT_MODULE: unsafe extern "C" fn() -> ::core::ffi::c_int =
            init_module;

        #[unsafe(no_mangle)]
        #[unsafe(link_section = ".exit.text")]
        pub extern "C" fn cleanup_module() {
            unsafe {
                if let Some(ref m) = __MOD {
                    m.as_ref().get_ref().exit();
                }
                __MOD = None; // Drop the KBox
            }
        }

        #[used]
        #[unsafe(link_section = ".exit.data")]
        #[allow(non_upper_case_globals)]
        static __UNIQUE_ID___ADDRESSABLE_CLEANUP_MODULE: extern "C" fn() = cleanup_module;

        #[panic_handler]
        fn panic(_info: &::core::panic::PanicInfo) -> ! {
            loop {}
        }
    };
}
