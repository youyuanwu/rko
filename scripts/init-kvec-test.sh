#!/bin/sh
# Kernel module test harness for kvec_test — runs as /init inside initramfs.
set -e
export PATH=/bin

mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev

PASS=0
FAIL=0

check() {
    if dmesg | grep -q "$1"; then
        echo "PASS: found '$1'"
        PASS=$((PASS + 1))
    else
        echo "FAIL: expected '$1' in dmesg"
        FAIL=$((FAIL + 1))
    fi
}

echo "=== TEST: insmod kvec_test.ko ==="
insmod /lib/modules/kvec_test.ko
check "kvec_test: all kvec tests passed"

echo "=== TEST: rmmod kvec_test ==="
rmmod kvec_test
check "kvec_test: module unloaded"

echo ""
echo "=== RESULTS: $PASS passed, $FAIL failed ==="

if [ "$FAIL" -eq 0 ]; then
    echo "ALL TESTS PASSED"
else
    echo "SOME TESTS FAILED"
fi

poweroff -f
