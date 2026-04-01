# Generate the tarfs test image at build time.
set(TARFS_BUILD_DIR ${CMAKE_CURRENT_SOURCE_DIR}/build)
add_custom_command(
  OUTPUT ${TARFS_BUILD_DIR}/tarfs_test.img
  COMMAND ${CMAKE_COMMAND} -E make_directory ${TARFS_BUILD_DIR}
  COMMAND python3 ${CMAKE_CURRENT_SOURCE_DIR}/mkimage.py > ${TARFS_BUILD_DIR}/tarfs_test.img
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/mkimage.py
  COMMENT "Generating tarfs test image"
)

add_kernel_module(
  CHECKS
    "tarfs: module loaded"
    "tarfs: mount and read OK"
    "tarfs: module unloaded"
)

# Make the ko target depend on the test image so it's built before testing.
add_custom_target(tarfs_test_img DEPENDS ${TARFS_BUILD_DIR}/tarfs_test.img)
add_dependencies(tarfs_ko tarfs_test_img)
