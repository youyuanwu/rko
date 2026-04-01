#!/usr/bin/env python3
"""Generate a minimal tarfs test image.

Layout (block size = 4096, sector size = 512):
  Block 0 (sectors 0-7):   file data ("Hello from tarfs!\\n")
  Block 1 (sectors 8-15):  dir entry table for root dir
  Block 2 (sectors 16-23): name strings ("hello.txt\\0")
  Block 3 (sectors 24-31): inode table (2 inodes)
  Block 4 (sectors 32-39): padding + Header in last sector (sector 39)

Total: 40 sectors = 20480 bytes (5 pages)
"""

import struct
import sys

SECTOR = 512
BLOCK = 4096

# --- File content ---
file_content = b"Hello from tarfs!\n"
file_content_padded = file_content.ljust(BLOCK, b'\0')

# --- Name strings ---
name_hello = b"hello.txt"
names_block = name_hello.ljust(BLOCK, b'\0')

# --- Inode table (2 inodes, each 32 bytes) ---
# struct Inode { mode: LE<u16>, flags: u8, hmtime: u8, owner: LE<u32>,
#                group: LE<u32>, lmtime: LE<u32>, size: LE<u64>, offset: LE<u64> }
INODE_FMT = '<HBBIIIQq'  # Note: offset is signed in some contexts but stored as u64
assert struct.calcsize(INODE_FMT) == 32

S_IFDIR = 0o040000
S_IFREG = 0o100000

# Block layout:
#   Block 0 (sector 0): file data
#   Block 1 (sector 8): dir entries for root
#   Block 2 (sector 16): name strings
#   Block 3 (sector 24): inode table
#   Block 4 (sector 32-39): padding + header in last sector (39)

FILE_DATA_OFFSET = 0 * BLOCK       # block 0
DIR_ENTRY_OFFSET = 1 * BLOCK       # block 1
NAMES_OFFSET = 2 * BLOCK           # block 2
INODE_TABLE_OFFSET = 3 * BLOCK     # block 3

# Inode 1: root directory
#   mode = S_IFDIR | 0o755, size = 1 dir entry (32 bytes), offset = DIR_ENTRY_OFFSET
inode_root = struct.pack(INODE_FMT,
    S_IFDIR | 0o755,  # mode
    0,                 # flags
    0,                 # hmtime
    0,                 # owner
    0,                 # group
    0,                 # lmtime
    32,                # size (1 DirEntry = 32 bytes)
    DIR_ENTRY_OFFSET,  # offset to dir entries
)

# Inode 2: hello.txt
#   mode = S_IFREG | 0o644, size = len(file_content), offset = FILE_DATA_OFFSET
inode_hello = struct.pack(INODE_FMT,
    S_IFREG | 0o644,        # mode
    0,                       # flags
    0,                       # hmtime
    0,                       # owner
    0,                       # group
    0,                       # lmtime
    len(file_content),       # size
    FILE_DATA_OFFSET,        # offset to file data
)

inode_table = (inode_root + inode_hello).ljust(BLOCK, b'\0')

# --- Directory entry table for root (1 entry: hello.txt) ---
# struct DirEntry { ino: LE<u64>, name_offset: LE<u64>, name_len: LE<u64>,
#                   etype: u8, _padding: [u8; 7] }
DIRENTRY_FMT = '<QQQb7s'
assert struct.calcsize(DIRENTRY_FMT) == 32

DT_REG = 8
dir_entry_hello = struct.pack(DIRENTRY_FMT,
    2,                          # ino (inode 2)
    NAMES_OFFSET,               # name_offset
    len(name_hello),            # name_len
    DT_REG,                     # etype
    b'\0' * 7,                  # padding
)

dir_entries = dir_entry_hello.ljust(BLOCK, b'\0')

# --- Header (last sector) ---
# struct Header { inode_table_offset: LE<u64>, inode_count: LE<u64> }
HEADER_FMT = '<QQ'
header = struct.pack(HEADER_FMT,
    INODE_TABLE_OFFSET,  # inode_table_offset
    2,                   # inode_count
)
header_sector = header.ljust(SECTOR, b'\0')

# --- Assemble image ---
# Blocks: [file_data] [dir_entries] [names] [inode_table] [padding + header]
# The header goes in the LAST sector. Block 4 = 8 sectors of padding,
# with the header written into the last sector (sector 39).
padding_block = b'\0' * (BLOCK - SECTOR) + header_sector  # 3584 + 512 = 4096

image = file_content_padded + dir_entries + names_block + inode_table + padding_block

# Total sectors
total_sectors = len(image) // SECTOR
print(f"Image size: {len(image)} bytes ({total_sectors} sectors)", file=sys.stderr)
print(f"Inode table at offset {INODE_TABLE_OFFSET}, count 2", file=sys.stderr)
print(f"Header at sector {total_sectors - 1}", file=sys.stderr)

sys.stdout.buffer.write(image)
