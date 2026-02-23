#!/bin/bash
# Run a kernel module test in QEMU and validate output.
#
# Usage: run-qemu-test.sh <bzImage> <initramfs> <logfile> [kvm=1|0]
set -uo pipefail

KERNEL="$1"
INITRAMFS="$2"
LOGFILE="$3"
KVM="${4:-1}"

KVM_ARGS=""
if [ "$KVM" = "1" ]; then
    KVM_ARGS="-enable-kvm -cpu host"
fi

timeout --foreground 60 qemu-system-x86_64 \
    -kernel "$KERNEL" \
    -initrd "$INITRAMFS" \
    -nographic \
    -no-reboot \
    -m 256M \
    -append "console=ttyS0 panic=-1" \
    $KVM_ARGS \
    > "$LOGFILE" 2>&1 || true

cat "$LOGFILE"

if grep -q "ALL TESTS PASSED" "$LOGFILE"; then
    echo "TEST OK"
else
    echo "TEST FAILED"
    exit 1
fi
