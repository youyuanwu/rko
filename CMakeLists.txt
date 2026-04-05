cmake_minimum_required(VERSION 3.20)
project(lnx LANGUAGES)

set(KDIR_ROOT ${CMAKE_SOURCE_DIR}/linux)
set(KBIN_ROOT ${CMAKE_SOURCE_DIR}/linux_bin)

# Check if SCCACHE_GHA_ENABLED envariable is set
if(DEFINED ENV{SCCACHE_GHA_ENABLED})
  message(STATUS "SCCACHE_GHA_ENABLED is set to: $ENV{SCCACHE_GHA_ENABLED}")
  set(K_CC 'sccache clang')
else()
  set(K_CC 'clang')
endif()



# Check that the kernel source directory exists
# Manually create a symlink: 
# ln -s ../linux linux
# ln -s ../linux_bin linux_bin
# Or manually unlink: 
# unlink ./linux
if(NOT EXISTS ${KDIR_ROOT}/Makefile)
  message(FATAL_ERROR "Kernel source directory not found: ${KDIR_ROOT}")
endif()


add_custom_target(configure_linux
  # Start from tinyconfig (minimal kernel) and enable only what QEMU tests need
  COMMAND $(MAKE) LLVM=1 tinyconfig O=${KBIN_ROOT}
  # 64-bit, ELF, modules, Rust
  COMMAND scripts/config --set-val CONFIG_64BIT y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_MODULES y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_MODULE_UNLOAD y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_RUST y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_LTO_NONE y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_BINFMT_ELF y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_BINFMT_SCRIPT y --file ${KBIN_ROOT}/.config
  # Serial console for QEMU -nographic
  COMMAND scripts/config --set-val CONFIG_TTY y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_SERIAL_8250 y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_SERIAL_8250_CONSOLE y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_PRINTK y --file ${KBIN_ROOT}/.config
  # Filesystems needed by init / test scripts
  COMMAND scripts/config --set-val CONFIG_PROC_FS y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_SYSFS y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_DEVTMPFS y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_DEVTMPFS_MOUNT y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_TMPFS y --file ${KBIN_ROOT}/.config
  # Block layer (needed for initramfs / general kernel infra)
  COMMAND scripts/config --set-val CONFIG_BLOCK y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_BLK_DEV_INITRD y --file ${KBIN_ROOT}/.config
  # Loop device (needed for mounting filesystem images in QEMU tests)
  COMMAND scripts/config --set-val CONFIG_BLK_DEV_LOOP y --file ${KBIN_ROOT}/.config
  # Minimal hardware for QEMU x86_64
  COMMAND scripts/config --set-val CONFIG_PCI y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_ATA y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_ATA_PIIX y --file ${KBIN_ROOT}/.config
  # ACPI needed for QEMU power off
  COMMAND scripts/config --set-val CONFIG_PM y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_ACPI y --file ${KBIN_ROOT}/.config
  # Busybox needs these
  COMMAND scripts/config --set-val CONFIG_FUTEX y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_EPOLL y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_SIGNALFD y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_TIMERFD y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_EVENTFD y --file ${KBIN_ROOT}/.config
  # SMP required for standard spinlock / mutex types used by bindings
  COMMAND scripts/config --set-val CONFIG_SMP y --file ${KBIN_ROOT}/.config
  # Full SLUB allocator (tinyconfig enables SLUB_TINY which stubs out SLAB_RECLAIM_ACCOUNT)
  COMMAND scripts/config --set-val CONFIG_SLUB_TINY n --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_SLUB y --file ${KBIN_ROOT}/.config
  # Rust emits retpoline thunks and stack-protector calls; kernel must export them
  COMMAND scripts/config --set-val CONFIG_CPU_MITIGATIONS y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_STACKPROTECTOR y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_STACKPROTECTOR_STRONG y --file ${KBIN_ROOT}/.config
  # Networking support (always enabled for rko networking modules)
  COMMAND scripts/config --set-val CONFIG_NET y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_INET y --file ${KBIN_ROOT}/.config
  # KUnit test framework (for rko kunit_tests module)
  COMMAND scripts/config --set-val CONFIG_KUNIT y --file ${KBIN_ROOT}/.config
  # io_uring (for IORING_OP_URING_CMD custom command support)
  COMMAND scripts/config --set-val CONFIG_IO_URING y --file ${KBIN_ROOT}/.config
  # Symbolized stack traces for debugging
  COMMAND scripts/config --set-val CONFIG_KALLSYMS y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_KALLSYMS_ALL y --file ${KBIN_ROOT}/.config
  # Resolve dependencies and verify Rust
  COMMAND $(MAKE) LLVM=1 olddefconfig O=${KBIN_ROOT}
  COMMAND $(MAKE) LLVM=1 rustavailable
  WORKING_DIRECTORY ${KDIR_ROOT}
)

# Build the minimal QEMU kernel
add_custom_target(make_linux
  COMMAND $(MAKE) LLVM=1 O=${KBIN_ROOT} CC=${K_CC}
  WORKING_DIRECTORY ${KDIR_ROOT}
)

# QEMU test options
option(ENABLE_KVM "Enable KVM acceleration for QEMU tests" ON)
if(ENABLE_KVM)
  set(KVM_FLAG "1")
else()
  set(KVM_FLAG "0")
endif()

enable_testing()

# Run rko-sys-gen to regenerate rko-sys bindings from kernel headers
add_custom_target(generate_bindings
  COMMAND cargo run -p rko-sys-gen
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Generating rko-sys bindings (bnd-winmd → windows-bindgen)"
  USES_TERMINAL
)

# Kernel module build function
include(cmake/kernel_module.cmake)

add_subdirectory(samples/hello)
add_subdirectory(samples/kvec_test)
add_subdirectory(samples/rofs_test)
add_subdirectory(samples/tarfs)
add_subdirectory(samples/ext2)
add_subdirectory(samples/workqueue_test)
add_subdirectory(samples/tcp_echo)
add_subdirectory(samples/async_echo)
add_subdirectory(samples/kunit_tests)
add_subdirectory(samples/http_uring)

# Clean all sample build directories
add_custom_target(clean_samples
  DEPENDS hello_ko_clean kvec_test_ko_clean rofs_test_ko_clean tarfs_ko_clean workqueue_test_ko_clean tcp_echo_ko_clean async_echo_ko_clean
  COMMENT "Cleaning all sample modules"
)

add_custom_target(rust_check
  COMMAND cargo fmt
  COMMAND cargo clippy --all-targets -- -D warnings
  COMMAND cd samples && cargo fmt
  COMMAND cd samples && cargo clippy --all-targets -- -D warnings
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Running cargo check on all Rust code"
  USES_TERMINAL
)