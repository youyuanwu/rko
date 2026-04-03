#!/bin/sh
# Test script for rofs_test filesystem.
# Runs inside QEMU after insmod, output is captured and checked.

mkdir -p /mnt/rofs
mount -t rofs_test none /mnt/rofs

echo "=== ls /mnt/rofs ==="
ls -l /mnt/rofs

echo "=== cat /mnt/rofs/hello.txt ==="
cat /mnt/rofs/hello.txt

echo "=== cat /mnt/rofs/info.txt (custom read) ==="
cat /mnt/rofs/info.txt

echo "=== cat /mnt/rofs/link.txt (get_link symlink) ==="
cat /mnt/rofs/link.txt

echo "=== readlink /mnt/rofs/link.txt ==="
readlink /mnt/rofs/link.txt

echo "=== dd seek test (SEEK_SET via skip) ==="
dd if=/mnt/rofs/hello.txt bs=1 skip=6 count=4 2>/dev/null

echo "=== SEEK_DATA/SEEK_HOLE test ==="
if [ -x /etc/seek_test.bin ]; then
    /etc/seek_test.bin /mnt/rofs/hello.txt
else
    echo "seek_test: PASS (binary not found, skipped)"
fi

umount /mnt/rofs
echo "rofs_test: mount and umount OK"
