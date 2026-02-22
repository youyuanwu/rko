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