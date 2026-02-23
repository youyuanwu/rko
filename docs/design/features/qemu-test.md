# Feature: Automated QEMU Testing

## Status: ✅ Implemented (superseded by build-infra)

Testing is now handled by `scripts/run-module-test.sh` and CMake's
`add_kernel_module()` function. See `docs/design/features/build-infra.md`.

## How It Works

1. Builds minimal initramfs: busybox + `.ko` + generic init script
2. Boots QEMU with kernel bzImage and initramfs
3. Init script: `insmod` → `rmmod` → check dmesg for expected strings
4. Host scans serial output for `TEST OK`

```sh
cmake -B build -DENABLE_KVM=OFF
ctest --test-dir build              # test all modules
```

Requires: `busybox-static`, `cpio`, `qemu-system-x86`.
