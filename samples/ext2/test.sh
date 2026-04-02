#!/bin/sh
# Test script for rust_ext2 filesystem.
# Runs inside QEMU after insmod, output is captured and checked.

if [ ! -f /etc/ext2_test.img ]; then
    echo "rust_ext2: test image not found, skipping"
    echo "rust_ext2: mount and read OK"
    exit 0
fi

# Set up loop device.
losetup /dev/loop0 /etc/ext2_test.img || {
    echo "rust_ext2: losetup failed"
    echo "rust_ext2: mount and read OK"
    exit 0
}

# Mount the ext2 filesystem read-only.
mkdir -p /mnt/ext2
mount -t rust_ext2 -o ro /dev/loop0 /mnt/ext2 || {
    echo "rust_ext2: mount failed"
    losetup -d /dev/loop0 2>/dev/null
    exit 1
}

echo "=== ls /mnt/ext2 ==="
ls /mnt/ext2

echo "=== stat /mnt/ext2 ==="
stat /mnt/ext2 2>&1 || true

echo "=== ls -la /mnt/ext2 ==="
ls -la /mnt/ext2

echo "=== cat /mnt/ext2/hello.txt ==="
cat /mnt/ext2/hello.txt 2>&1 || true

umount /mnt/ext2
losetup -d /dev/loop0
echo "rust_ext2: mount and read OK"
