# Generate the ext2 test image at build time.
set(EXT2_BUILD_DIR ${CMAKE_CURRENT_SOURCE_DIR}/build)
add_custom_command(
  OUTPUT ${EXT2_BUILD_DIR}/ext2_test.img
  COMMAND ${CMAKE_COMMAND} -E make_directory ${EXT2_BUILD_DIR}
  COMMAND python3 ${CMAKE_CURRENT_SOURCE_DIR}/mkimage.py
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/mkimage.py
  COMMENT "Generating ext2 test image"
)

add_kernel_module(
  CHECKS
    "rust_ext2: module loaded"
    "rust_ext2: mount and read OK"
    "rust_ext2: module unloaded"
)

# Make the ko target depend on the test image.
add_custom_target(ext2_test_img DEPENDS ${EXT2_BUILD_DIR}/ext2_test.img)
add_dependencies(ext2_ko ext2_test_img)
