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
  COMMAND $(MAKE) LLVM=1 defconfig O=${KBIN_ROOT}
  COMMAND scripts/config --set-val CONFIG_RUST y --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_NETDEVICES n --file ${KBIN_ROOT}/.config
  COMMAND scripts/config --set-val CONFIG_LTO_NONE y --file ${KBIN_ROOT}/.config
  COMMAND $(MAKE) LLVM=1 olddefconfig O=${KBIN_ROOT}
  COMMAND $(MAKE) LLVM=1 rustavailable
  WORKING_DIRECTORY ${KDIR_ROOT}
)

# disable net device to avoid driver conflicts.
add_custom_target(make_linux
  COMMAND $(MAKE) LLVM=1 O=${KBIN_ROOT} CC=${K_CC}
  WORKING_DIRECTORY ${KDIR_ROOT}
)

# Build hello.ko sample kernel module
add_custom_target(hello_ko ALL
  COMMAND RUSTC_BOOTSTRAP=1 $(MAKE) KSRC=${KDIR_ROOT} KOUT=${KBIN_ROOT}
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}/samples/hello
  COMMENT "Building hello.ko kernel module"
  USES_TERMINAL
)

add_custom_target(hello_ko_clean
  COMMAND $(MAKE) KSRC=${KDIR_ROOT} KOUT=${KBIN_ROOT} clean
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}/samples/hello
  COMMENT "Cleaning hello.ko build artifacts"
  USES_TERMINAL
)