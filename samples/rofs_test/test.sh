#!/bin/sh
# Test script for rofs_test filesystem.
# Runs inside QEMU after insmod, output is captured and checked.

mkdir -p /mnt/rofs
mount -t rofs_test none /mnt/rofs

echo "=== ls /mnt/rofs ==="
ls /mnt/rofs

echo "=== cat /mnt/rofs/hello.txt ==="
cat /mnt/rofs/hello.txt

umount /mnt/rofs
echo "rofs_test: mount and umount OK"
