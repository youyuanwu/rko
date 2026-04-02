#!/usr/bin/env python3
"""Generate a minimal ext2 test image using mke2fs + debugfs (no root needed)."""

import subprocess
import sys
import os
import tempfile

def main():
    out_dir = os.path.join(os.path.dirname(os.path.abspath(__file__)), "build")
    os.makedirs(out_dir, exist_ok=True)
    img_path = os.path.join(out_dir, "ext2_test.img")

    # Create a 1MB image file.
    with open(img_path, "wb") as f:
        f.write(b"\0" * (1024 * 1024))

    # Format as ext2.
    subprocess.check_call(
        ["mke2fs", "-F", "-t", "ext2", "-b", "1024", img_path],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )

    # Write test files using debugfs (no mount needed).
    with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as tf:
        tf.write("Hello from ext2!\n")
        hello_path = tf.name

    with tempfile.NamedTemporaryFile(mode="w", suffix=".txt", delete=False) as tf:
        tf.write("Nested file\n")
        nested_path = tf.name

    try:
        cmds = "\n".join([
            f"write {hello_path} hello.txt",
            "mkdir subdir",
            f"cd subdir",
            f"write {nested_path} nested.txt",
            "cd /",
            "symlink link.txt hello.txt",
        ])
        subprocess.run(
            ["debugfs", "-w", img_path],
            input=cmds.encode(),
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=True,
        )
    finally:
        os.unlink(hello_path)
        os.unlink(nested_path)

    print(f"Created {img_path} ({os.path.getsize(img_path)} bytes)", file=sys.stderr)

if __name__ == "__main__":
    main()
