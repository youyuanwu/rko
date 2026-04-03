add_custom_command(
  OUTPUT ${CMAKE_CURRENT_SOURCE_DIR}/build/seek_test.bin
  COMMAND ${CMAKE_COMMAND} -E make_directory ${CMAKE_CURRENT_SOURCE_DIR}/build
  COMMAND gcc -static -o ${CMAKE_CURRENT_SOURCE_DIR}/build/seek_test.bin
          ${CMAKE_CURRENT_SOURCE_DIR}/seek_test.c
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/seek_test.c
  COMMENT "Compiling seek_test"
)

add_custom_target(rofs_test_seek_test
  DEPENDS ${CMAKE_CURRENT_SOURCE_DIR}/build/seek_test.bin
)

add_kernel_module(
  CHECKS
    "rofs_test: module loaded"
    "hello.txt"
    "Hello from rofs!"
    "custom read works"
    "get_link symlink"
    "seek_test: PASS"
    "rofs_test: mount and umount OK"
    "rofs_test: module unloaded"
)

add_dependencies(rofs_test_ko rofs_test_seek_test)
