# Feature: Automated kernel module testing with QEMU

## Goal

Provide a `make test` target that automatically boots the kernel in QEMU,
loads `hello.ko`, validates the output, and exits with a pass/fail status.
No manual interaction required.

## Design

### Overview

Build a minimal initramfs containing only:
- A static `/init` shell script (the test harness)
- Busybox (provides `sh`, `insmod`, `rmmod`, `dmesg`, `grep`, `poweroff`)
- The `.ko` module under test

QEMU boots the kernel with this initramfs, runs the test, and powers off.
The exit code propagates back to the host via QEMU's `-device isa-debug-exit`.

### Architecture

```
make test
    │
    ├── 1. make (build hello.ko)
    │
    ├── 2. scripts/make-initramfs.sh
    │       ├── mkdir rootfs/{bin,lib/modules}
    │       ├── cp busybox → rootfs/bin/
    │       ├── cp hello.ko → rootfs/lib/modules/
    │       ├── write rootfs/init (test script)
    │       └── cpio + gzip → build/initramfs.cpio.gz
    │
    └── 3. qemu-system-x86_64
            -kernel linux_bin/arch/x86/boot/bzImage
            -initrd build/initramfs.cpio.gz
            -device isa-debug-exit,iobase=0xf4,iosize=0x04
            -nographic -append "console=ttyS0 panic=-1"
            │
            └── /init runs:
                  insmod /lib/modules/hello.ko
                  dmesg | grep "hello: module loaded"  → pass/fail
                  rmmod hello
                  dmesg | grep "hello: module unloaded" → pass/fail
                  write exit code to 0xf4 I/O port
                  poweroff -f
```

### `/init` test script

```sh
#!/bin/sh
set -e
export PATH=/bin

# Mount required pseudo-filesystems
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

echo "=== TEST: insmod hello.ko ==="
insmod /lib/modules/hello.ko

# Check that pr_info message appeared
if dmesg | grep -q "hello: module loaded"; then
    echo "PASS: module loaded"
else
    echo "FAIL: expected 'hello: module loaded' in dmesg"
    echo 1 > /proc/sysrq-trigger  # or use isa-debug-exit
    poweroff -f
fi

echo "=== TEST: rmmod hello ==="
rmmod hello

if dmesg | grep -q "hello: module unloaded"; then
    echo "PASS: module unloaded"
else
    echo "FAIL: expected 'hello: module unloaded' in dmesg"
fi

echo "=== ALL TESTS PASSED ==="
# Exit QEMU with success via isa-debug-exit (writes to I/O port 0xf4)
# Value written: (N << 1) | 1, so writing 0 gives exit code 1.
# We write 0x00 → QEMU exits with code 1 (success convention).
# Any other value → different exit code (failure).
poweroff -f
```

### QEMU exit code

The `isa-debug-exit` device maps an I/O port. Writing value `N` causes
QEMU to exit with code `(N << 1) | 1`. Convention:

| Outcome | Write to port | QEMU exit code |
|---------|---------------|----------------|
| Success | not used (poweroff) | 0 |
| Failure | 0x01 | 3 |

Simpler alternative: just scan QEMU's serial output (stdout) for
`ALL TESTS PASSED` on the host side. This avoids the isa-debug-exit
complexity:

```sh
timeout 30 qemu-system-x86_64 ... 2>&1 | tee build/test.log
grep -q "ALL TESTS PASSED" build/test.log
```

### Dependencies

| Package | What for |
|---------|----------|
| `busybox-static` | Static binary providing sh, insmod, rmmod, dmesg, grep, mount, poweroff |
| `cpio` | Building the initramfs archive |
| `qemu-system-x86` | Running the kernel |

### File layout

```
scripts/
  make-initramfs.sh     — builds initramfs.cpio.gz
  init.sh               — /init script baked into initramfs
samples/hello/
  Makefile              — gains `test` target
```

### Makefile integration

```makefile
INITRAMFS = $(MOUT)/initramfs.cpio.gz

test: all $(INITRAMFS)
	timeout 30 qemu-system-x86_64 \
	    -kernel $(KOUT)/arch/x86/boot/bzImage \
	    -initrd $(INITRAMFS) \
	    -nographic \
	    -append "console=ttyS0 panic=-1" \
	    -no-reboot \
	    -m 256M \
	    2>&1 | tee $(MOUT)/test.log
	@grep -q "ALL TESTS PASSED" $(MOUT)/test.log && echo "TEST OK" || (echo "TEST FAILED"; exit 1)

$(INITRAMFS): $(MOUT)/hello.ko
	$(CURDIR)/../../scripts/make-initramfs.sh $(MOUT)/hello.ko $@
```

### CMake integration

```cmake
enable_testing()

add_test(
  NAME hello_ko_test
  COMMAND ${CMAKE_COMMAND} --build ${CMAKE_BINARY_DIR} --target hello_ko_test_run
)

add_custom_target(hello_ko_test_run
  COMMAND $(MAKE) KSRC=${KDIR_ROOT} KOUT=${KBIN_ROOT} test
  WORKING_DIRECTORY ${CMAKE_SOURCE_DIR}/samples/hello
  COMMENT "Testing hello.ko in QEMU"
  USES_TERMINAL
  DEPENDS hello_ko
)
```

Run with `ctest --test-dir build` or `cmake --build build --target hello_ko_test_run`.

## Scope

| In scope | Out of scope |
|----------|-------------|
| insmod / rmmod + dmesg validation | Multi-module test suites |
| Serial console output checking | Network / block device testing |
| CI-friendly (no GUI, timeout, exit code) | Kernel debugging (gdb stub) |
| Single `make test` command | Custom rootfs with systemd |
