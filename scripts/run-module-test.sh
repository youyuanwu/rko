#!/bin/bash
# All-in-one kernel module test: build initramfs, run QEMU, check results.
#
# Usage: run-module-test.sh <name> <ko> <bzImage> <build_dir> <kvm> <sample_dir> <checks...>
set -uo pipefail

MODULE="$1"; shift
KO_FILE="$1"; shift
KERNEL="$1"; shift
BUILD_DIR="$1"; shift
KVM="$1"; shift
SAMPLE_DIR="$1"; shift
CHECKS=("$@")

# 1. Build initramfs
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT

mkdir -p "$WORK"/{bin,lib/modules,proc,sys,dev,etc,mnt,tmp}
BUSYBOX=$(which busybox)
cp "$BUSYBOX" "$WORK/bin/busybox"
for cmd in sh mount umount insmod rmmod dmesg grep poweroff \
           ls cat mkdir echo wc head tail; do
    ln -s busybox "$WORK/bin/$cmd"
done

cp "$KO_FILE" "$WORK/lib/modules/"

# Copy optional test.sh from sample directory
HAS_TEST=0
if [ -f "$SAMPLE_DIR/test.sh" ]; then
    cp "$SAMPLE_DIR/test.sh" "$WORK/etc/test_custom.sh"
    chmod +x "$WORK/etc/test_custom.sh"
    HAS_TEST=1
fi

# Write test config sourced by init
{
    printf 'MODULE=%s\n' "$MODULE"
    printf 'HAS_TEST=%s\n' "$HAS_TEST"
} > "$WORK/etc/test.conf"

# Write one check string per line
for chk in "${CHECKS[@]}"; do
    printf '%s\n' "$chk"
done > "$WORK/etc/checks.txt"

# Write generic init script
cat > "$WORK/init" << 'INIT_EOF'
#!/bin/sh
set -e
export PATH=/bin
mount -t proc proc /proc
mount -t sysfs sysfs /sys
mount -t devtmpfs devtmpfs /dev
. /etc/test.conf
PASS=0; FAIL=0
check() {
    if dmesg | grep -qF "$1" || grep -qF "$1" /tmp/test.log 2>/dev/null; then
        echo "PASS: found '$1'"
        PASS=$((PASS + 1))
    else
        echo "FAIL: expected '$1'"
        FAIL=$((FAIL + 1))
    fi
}
echo "=== TEST: insmod ${MODULE}.ko ==="
insmod /lib/modules/${MODULE}.ko

# Run custom test script if present
if [ "$HAS_TEST" = "1" ]; then
    echo "=== TEST: running test.sh ==="
    /etc/test_custom.sh > /tmp/test.log 2>&1 || true
    cat /tmp/test.log
fi

echo "=== TEST: rmmod ${MODULE} ==="
rmmod ${MODULE}
while IFS= read -r chk; do check "$chk"; done < /etc/checks.txt
echo ""
echo "=== RESULTS: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -eq 0 ]; then echo "ALL TESTS PASSED"; else echo "SOME TESTS FAILED"; fi
poweroff -f
INIT_EOF
chmod +x "$WORK/init"

INITRAMFS="$BUILD_DIR/initramfs.cpio.gz"
(cd "$WORK" && find . | cpio -o -H newc --quiet | gzip) > "$INITRAMFS"

# 2. Run QEMU
KVM_ARGS=""
[ "$KVM" = "1" ] && KVM_ARGS="-enable-kvm -cpu host"

LOGFILE="$BUILD_DIR/test.log"
timeout --foreground 60 qemu-system-x86_64 \
    -kernel "$KERNEL" -initrd "$INITRAMFS" \
    -nographic -no-reboot -m 256M \
    -append "console=ttyS0 panic=-1" \
    $KVM_ARGS > "$LOGFILE" 2>&1 || true

cat "$LOGFILE"

if grep -q "ALL TESTS PASSED" "$LOGFILE"; then
    echo "TEST OK"
else
    echo "TEST FAILED"
    exit 1
fi
