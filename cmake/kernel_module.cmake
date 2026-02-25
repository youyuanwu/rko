# cmake/kernel_module.cmake — reusable function for building kernel modules
#
# Usage (from a sample's CMakeLists.txt):
#   add_kernel_module(
#     CHECKS "hello: module loaded" "hello: module unloaded"
#   )
#
# NAME is inferred from the directory name (CMAKE_CURRENT_SOURCE_DIR).
# Expects KDIR_ROOT, KBIN_ROOT, KVM_FLAG to be set by the parent CMakeLists.txt.

# --- One-time helpers.o build (shared across all modules) ---

set(HELPERS_BUILD_DIR ${CMAKE_BINARY_DIR}/helpers)
set(HELPERS_OBJ ${HELPERS_BUILD_DIR}/helpers.o)

if(NOT TARGET helpers_obj)
  file(MAKE_DIRECTORY ${HELPERS_BUILD_DIR})
  # Kbuild: build as module component; modpost is skipped via || true
  file(WRITE ${HELPERS_BUILD_DIR}/Kbuild "obj-m := _helpers.o\n_helpers-y := helpers.o\n")
  if(NOT EXISTS ${HELPERS_BUILD_DIR}/helpers.c)
    file(CREATE_LINK ${CMAKE_SOURCE_DIR}/rko-sys/src/helpers.c
         ${HELPERS_BUILD_DIR}/helpers.c SYMBOLIC)
  endif()
  if(NOT EXISTS ${HELPERS_BUILD_DIR}/helpers.h)
    file(CREATE_LINK ${CMAKE_SOURCE_DIR}/rko-sys/src/helpers.h
         ${HELPERS_BUILD_DIR}/helpers.h SYMBOLIC)
  endif()

  # Build the object. modpost will fail (no MODULE_LICENSE) but helpers.o
  # is already compiled before that. We ignore the error and verify the file.
  add_custom_command(
    OUTPUT ${HELPERS_OBJ}
    COMMAND $(MAKE) -C ${KDIR_ROOT} O=${KBIN_ROOT}
            M=${HELPERS_BUILD_DIR} LLVM=1 modules 2>&1 || true
    COMMAND test -f ${HELPERS_OBJ}
    DEPENDS ${CMAKE_SOURCE_DIR}/rko-sys/src/helpers.c
            ${CMAKE_SOURCE_DIR}/rko-sys/src/helpers.h
    COMMENT "Kbuild helpers.o (once)"
    USES_TERMINAL
  )
  add_custom_target(helpers_obj DEPENDS ${HELPERS_OBJ})
endif()

# --- Per-module function ---

function(add_kernel_module)
  cmake_parse_arguments(KM "" "" "CHECKS;CSOURCES" ${ARGN})

  get_filename_component(KM_NAME ${CMAKE_CURRENT_SOURCE_DIR} NAME)
  set(SAMPLE_DIR ${CMAKE_CURRENT_SOURCE_DIR})
  set(BUILD_DIR ${SAMPLE_DIR}/build)
  set(SAMPLES_DIR ${CMAKE_SOURCE_DIR}/samples)

  # Build Kbuild content: single combined object
  set(KBUILD_OBJS "${KM_NAME}_rust.o")
  set(CSRC_OUTPUTS "")

  if(KM_CSOURCES)
    foreach(CSRC ${KM_CSOURCES})
      get_filename_component(CSRC_FNAME ${CSRC} NAME)
      get_filename_component(CSRC_EXT ${CSRC} EXT)
      if(CSRC_EXT STREQUAL ".c")
        get_filename_component(CSRC_NAME ${CSRC} NAME_WE)
        set(KBUILD_OBJS "${KBUILD_OBJS} ${CSRC_NAME}.o")
        list(APPEND CSRC_OUTPUTS "${CSRC_NAME}.o")
      endif()
    endforeach()
  endif()

  # Generate Kbuild in build dir at configure time
  file(MAKE_DIRECTORY ${BUILD_DIR})
  file(WRITE ${BUILD_DIR}/Kbuild
    "obj-m := ${KM_NAME}.o\n"
    "${KM_NAME}-y := ${KBUILD_OBJS}\n"
  )

  # Step 1: cargo build → lib<name>.a
  set(CARGO_TARGET_CFG "build.target=\"${KBIN_ROOT}/scripts/target.json\"")
  add_custom_command(
    OUTPUT ${BUILD_DIR}/lib${KM_NAME}.a
    COMMAND ${CMAKE_COMMAND} -E make_directory ${BUILD_DIR}
    COMMAND ${CMAKE_COMMAND} -E env RUSTC_BOOTSTRAP=1
      cargo
        --config ${SAMPLES_DIR}/cargo-kernel.toml
        --config ${CARGO_TARGET_CFG}
        -Z unstable-options build --release
        -p ${KM_NAME}
        --manifest-path ${SAMPLE_DIR}/Cargo.toml
        --artifact-dir ${BUILD_DIR}
    WORKING_DIRECTORY ${SAMPLES_DIR}
    DEPENDS ${SAMPLE_DIR}/${KM_NAME}.rs ${SAMPLE_DIR}/Cargo.toml
    COMMENT "cargo build ${KM_NAME}"
    USES_TERMINAL
    VERBATIM
  )

  # Step 2: ld -r --whole-archive Rust .a + helpers.o → <name>_rust.o
  add_custom_command(
    OUTPUT ${BUILD_DIR}/${KM_NAME}_rust.o
    COMMAND ld -r --whole-archive ${BUILD_DIR}/lib${KM_NAME}.a
            ${HELPERS_OBJ}
            -o ${BUILD_DIR}/${KM_NAME}_rust.o
    DEPENDS ${BUILD_DIR}/lib${KM_NAME}.a helpers_obj
    COMMENT "ld -r ${KM_NAME} + helpers"
  )

  # Step 2b: Symlink C sources into build dir for Kbuild
  if(KM_CSOURCES)
    foreach(CSRC ${KM_CSOURCES})
      get_filename_component(CSRC_FNAME ${CSRC} NAME)
      if(NOT IS_ABSOLUTE ${CSRC})
        set(CSRC "${CMAKE_SOURCE_DIR}/${CSRC}")
      endif()
      if(NOT EXISTS ${BUILD_DIR}/${CSRC_FNAME})
        file(CREATE_LINK ${CSRC} ${BUILD_DIR}/${CSRC_FNAME} SYMBOLIC)
      endif()
    endforeach()
  endif()

  # Step 3: Kbuild → <name>.ko (M= points to build dir)
  add_custom_target(${KM_NAME}_ko ALL
    COMMAND $(MAKE) -C ${KDIR_ROOT} O=${KBIN_ROOT}
            M=${BUILD_DIR} LLVM=1 modules
    DEPENDS ${BUILD_DIR}/${KM_NAME}_rust.o
    COMMENT "Kbuild ${KM_NAME}.ko"
    USES_TERMINAL
  )

  # Clean target
  add_custom_target(${KM_NAME}_ko_clean
    COMMAND ${CMAKE_COMMAND} -E rm -rf ${BUILD_DIR}
    COMMENT "Cleaning ${KM_NAME}.ko"
  )

  # Test target (QEMU)
  add_custom_target(${KM_NAME}_ko_test
    COMMAND ${CMAKE_SOURCE_DIR}/scripts/run-module-test.sh
            ${KM_NAME}
            ${BUILD_DIR}/${KM_NAME}.ko
            ${KBIN_ROOT}/arch/x86/boot/bzImage
            ${BUILD_DIR}
            ${KVM_FLAG}
            ${SAMPLE_DIR}
            ${KM_CHECKS}
    DEPENDS ${KM_NAME}_ko
    COMMENT "Testing ${KM_NAME}.ko in QEMU"
    USES_TERMINAL
  )

  add_test(
    NAME ${KM_NAME}_ko
    COMMAND ${CMAKE_COMMAND} --build ${CMAKE_BINARY_DIR}
            --target ${KM_NAME}_ko_test
  )
endfunction()
