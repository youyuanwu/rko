add_kernel_module(
  CHECKS
    "rofs_test: module loaded"
    "hello.txt"
    "Hello from rofs!"
    "rofs_test: mount and umount OK"
    "rofs_test: module unloaded"
)
