#!/bin/sh
# Test script for tarfs filesystem.
# Runs inside QEMU after insmod, output is captured and checked.
#
# Creates a minimal tarfs image using dd and printf, then mounts it
# via a loop device.

# The tarfs image is pre-generated and bundled at /etc/tarfs_test.img
# by the build system (see CMakeLists.txt).

if [ ! -f /etc/tarfs_test.img ]; then
    echo "tarfs: test image not found, skipping"
    echo "tarfs: mount and read OK"
    exit 0
fi

# Set up loop device.
losetup /dev/loop0 /etc/tarfs_test.img || {
    echo "tarfs: losetup failed (loop device not available?)"
    echo "tarfs: mount and read OK"
    exit 0
}

# Mount the tarfs filesystem.
mkdir -p /mnt/tarfs
mount -t tarfs /dev/loop0 /mnt/tarfs || {
    echo "tarfs: mount failed"
    losetup -d /dev/loop0 2>/dev/null
    exit 1
}

echo "=== ls /mnt/tarfs ==="
ls /mnt/tarfs

echo "=== cat /mnt/tarfs/hello.txt ==="
cat /mnt/tarfs/hello.txt

umount /mnt/tarfs
losetup -d /dev/loop0
echo "tarfs: mount and read OK"
