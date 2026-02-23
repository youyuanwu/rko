#!/bin/bash
# Build a minimal initramfs containing busybox, the test init script, and a .ko module.
#
# Usage: make-initramfs.sh <module.ko> <output.cpio.gz> [init-script]
set -euo pipefail

KO_FILE="$1"
OUTPUT="$2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INIT_SCRIPT="${3:-$SCRIPT_DIR/init.sh}"

TMPDIR=$(mktemp -d)
trap "rm -rf $TMPDIR" EXIT

# Create rootfs layout
mkdir -p "$TMPDIR"/{bin,lib/modules,proc,sys,dev}

# Install busybox and create symlinks
BUSYBOX=$(which busybox)
cp "$BUSYBOX" "$TMPDIR/bin/busybox"
for cmd in sh mount insmod rmmod dmesg grep poweroff; do
    ln -s busybox "$TMPDIR/bin/$cmd"
done

# Install init script
cp "$INIT_SCRIPT" "$TMPDIR/init"
chmod +x "$TMPDIR/init"

# Install kernel module
cp "$KO_FILE" "$TMPDIR/lib/modules/"

# Build cpio archive
(cd "$TMPDIR" && find . | cpio -o -H newc --quiet | gzip) > "$OUTPUT"

echo "Created initramfs: $OUTPUT ($(du -h "$OUTPUT" | cut -f1))"
