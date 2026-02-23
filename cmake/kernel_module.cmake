# cmake/kernel_module.cmake — reusable function for building kernel modules
#
# Usage (from a sample's CMakeLists.txt):
#   add_kernel_module(
#     CHECKS "hello: module loaded" "hello: module unloaded"
#   )
#
# NAME is inferred from the directory name (CMAKE_CURRENT_SOURCE_DIR).
# Expects KDIR_ROOT, KBIN_ROOT, KVM_FLAG to be set by the parent CMakeLists.txt.

function(add_kernel_module)
  cmake_parse_arguments(KM "" "" "CHECKS" ${ARGN})

  get_filename_component(KM_NAME ${CMAKE_CURRENT_SOURCE_DIR} NAME)
  set(SAMPLE_DIR ${CMAKE_CURRENT_SOURCE_DIR})
  set(BUILD_DIR ${SAMPLE_DIR}/build)
  set(SAMPLES_DIR ${CMAKE_SOURCE_DIR}/samples)

  # Generate Kbuild at configure time
  file(WRITE ${SAMPLE_DIR}/Kbuild
    "obj-m := ${KM_NAME}.o\n"
    "${KM_NAME}-y := ${KM_NAME}_rust.o\n"
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

  # Step 2: ld -r --whole-archive → <name>_rust.o
  add_custom_command(
    OUTPUT ${BUILD_DIR}/${KM_NAME}_rust.o
    COMMAND ld -r --whole-archive ${BUILD_DIR}/lib${KM_NAME}.a
            -o ${BUILD_DIR}/${KM_NAME}_rust.o
    DEPENDS ${BUILD_DIR}/lib${KM_NAME}.a
    COMMENT "ld --whole-archive ${KM_NAME}"
  )

  # Step 3: Kbuild → <name>.ko
  add_custom_target(${KM_NAME}_ko ALL
    COMMAND $(MAKE) -C ${KDIR_ROOT} O=${KBIN_ROOT}
            M=${SAMPLE_DIR} MO=${BUILD_DIR} LLVM=1 modules
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
