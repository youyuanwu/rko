use std::env;
use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let root = manifest.parent().unwrap();
    println!("cargo:rerun-if-env-changed=RKO_KERNEL_SOURCE");
    println!("cargo:rerun-if-env-changed=RKO_KERNEL_BUILD");
    let source = env::var_os("RKO_KERNEL_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("linux"));
    let build = env::var_os("RKO_KERNEL_BUILD")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("linux_bin"));

    assert_eq!(
        env::var("CARGO_CFG_TARGET_ARCH").unwrap(),
        "x86_64",
        "rko-sys currently supports only x86_64 kernels"
    );
    for path in [
        source.join("include/linux/kconfig.h"),
        build.join("include/generated/autoconf.h"),
        build.join("include/generated/timeconst.h"),
    ] {
        assert!(
            path.is_file(),
            "missing kernel header {}; configure and build the kernel first (see README)",
            path.display()
        );
    }

    let mut bindings = bindgen::Builder::default()
        .header(manifest.join("src/bindings.h").to_str().unwrap())
        .use_core()
        .ctypes_prefix("core::ffi")
        .derive_default(true)
        .generate_comments(false)
        .prepend_enum_name(false)
        .default_macro_constant_type(bindgen::MacroTypeVariation::Signed)
        .wrap_unsafe_ops(true)
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .clang_args([
            "--target=x86_64-linux-gnu",
            "-nostdinc",
            "-D__KERNEL__",
            "-DMODULE",
            "-D__BINDGEN__",
            "-fno-builtin",
            "-fno-PIE",
            "-fno-strict-aliasing",
            "-fno-common",
            "-fms-extensions",
            "-std=gnu11",
        ])
        .clang_args(["-include", "linux/compiler-version.h"])
        .clang_args(["-include", "linux/kconfig.h"])
        .clang_args(["-include", "linux/compiler_types.h"])
        .allowlist_file(".*/rko-sys/src/helpers.h")
        // As in the kernel's bindgen parameters: packed x86 types cannot
        // transitively contain Rust types with explicit alignment.
        .opaque_type("alt_instr|x86_msi_data|x86_msi_addr_lo")
        .opaque_type("xregs_state|desc_struct|arch_lbr_state|local_apic")
        // bindgen cannot represent aligned empty structs or the anonymous
        // struct ns_tree member. Keep their clang-computed size/alignment.
        .opaque_type("sched_statistics|ns_common.*")
        .blocklist_function("__list_.*_report");

    for include in [
        source.join("arch/x86/include"),
        build.join("arch/x86/include/generated"),
        source.join("include"),
        build.join("include"),
        source.join("arch/x86/include/uapi"),
        build.join("arch/x86/include/generated/uapi"),
        source.join("include/uapi"),
        build.join("include/generated/uapi"),
    ] {
        bindings = bindings.clang_arg(format!("-I{}", include.display()));
    }
    for header in [
        "linux/types.h",
        "linux/errno.h",
        "uapi/asm-generic/errno.*.h",
        "linux/slab.h",
        "linux/gfp_types.h",
        "linux/spinlock_types.h",
        "linux/mutex_types.h",
        "linux/rwsem.h",
        "linux/lockref.h",
        "linux/refcount_types.h",
        "linux/osq_lock.h",
        "linux/wait.h",
        "linux/wait_bit.h",
        "linux/completion.h",
        "linux/swait.h",
        "linux/mm_types.h",
        "linux/cred.h",
        "linux/fs.h",
        "linux/fs/super_types.h",
        "linux/fs_context.h",
        "linux/dcache.h",
        "linux/pagemap.h",
        "linux/highmem.h",
        "linux/statfs.h",
        "linux/xattr.h",
        "linux/iomap.h",
        "linux/net.h",
        "linux/socket.h",
        "linux/uio.h",
        "linux/pid.h",
        "linux/workqueue.h",
        "uapi/asm-generic/poll.h",
        "kunit/test.h",
        "linux/io_uring/cmd.h",
        "linux/miscdevice.h",
    ] {
        bindings = bindings.allowlist_file(format!(".*/include/{header}"));
    }

    bindings
        .generate()
        .expect("failed to generate rko-sys bindings")
        .write_to_file(PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"))
        .expect("failed to write rko-sys bindings");
}
