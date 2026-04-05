set(TEST_BIN_SRC ${CMAKE_SOURCE_DIR}/target/x86_64-unknown-linux-musl/release/http_uring_test)
set(SAMPLE_BUILD_DIR ${CMAKE_CURRENT_SOURCE_DIR}/build)

# Build the userspace test binary (static, musl)
add_custom_target(http_uring_test_bin
  COMMAND cargo build -p rko-test-uring --release --target x86_64-unknown-linux-musl
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}
  COMMENT "Building http_uring_test (musl static)"
)

# Copy test binary to sample build dir for initramfs inclusion
file(MAKE_DIRECTORY ${SAMPLE_BUILD_DIR}/test_bin)
add_custom_command(
  OUTPUT ${SAMPLE_BUILD_DIR}/test_bin/http_uring_test
  COMMAND ${CMAKE_COMMAND} -E copy ${TEST_BIN_SRC} ${SAMPLE_BUILD_DIR}/test_bin/http_uring_test
  DEPENDS http_uring_test_bin
)
add_custom_target(http_uring_test_bin_copy
  DEPENDS ${SAMPLE_BUILD_DIR}/test_bin/http_uring_test
)

add_kernel_module(
  CHECKS "http_uring: /dev/rko_http ready" "http_uring: unregistering" "http_uring_test: ALL TESTS PASSED"
)

# Make the test target depend on the test binary
add_dependencies(http_uring_ko_test http_uring_test_bin_copy)
