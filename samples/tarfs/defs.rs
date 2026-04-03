//! On-disk type definitions for the tarfs index format.
//!
//! A tarfs image is a tar archive with an index appended:
//! ```text
//! [tar data...] [inode table...] [header (last sector)]
//! ```
//!
//! The header is in the last 512-byte sector and contains the offset
//! to the inode table and the inode count. Each inode is a fixed-size
//! record describing a file, directory, symlink, or device.

use rko_core::types::LE;

/// Flags used in [`Inode::flags`].
pub mod inode_flags {
    /// Indicates that the inode is opaque (overlay whiteout).
    pub const OPAQUE: u8 = 0x1;
}

/// An inode in the tarfs inode table (32 bytes).
#[derive(rko_core::FromBytes)]
#[repr(C)]
pub struct Inode {
    /// File mode: bottom 9 bits are rwx permissions, upper bits are S_IFMT.
    pub mode: LE<u16>,
    /// Tarfs flags (e.g. OPAQUE for overlay support).
    pub flags: u8,
    /// Bottom 4 bits are the top 4 bits of mtime.
    pub hmtime: u8,
    /// Owner user ID.
    pub owner: LE<u32>,
    /// Owner group ID.
    pub group: LE<u32>,
    /// Bottom 32 bits of mtime.
    pub lmtime: LE<u32>,
    /// Size of the file contents in bytes.
    pub size: LE<u64>,
    /// Byte offset to file data, or major/minor for devices.
    pub offset: LE<u64>,
}

/// A directory entry in the tarfs directory table (32 bytes).
#[derive(rko_core::FromBytes)]
#[repr(C)]
pub struct DirEntry {
    /// Inode number this entry refers to.
    pub ino: LE<u64>,
    /// Byte offset to the name string.
    pub name_offset: LE<u64>,
    /// Length of the name in bytes.
    pub name_len: LE<u64>,
    /// Directory entry type (DT_REG, DT_DIR, etc.).
    pub etype: u8,
    /// Padding.
    pub _padding: [u8; 7],
}

/// The tarfs superblock header (16 bytes, in the last sector).
#[derive(rko_core::FromBytes)]
#[repr(C)]
pub struct Header {
    /// Byte offset to the start of the inode table.
    pub inode_table_offset: LE<u64>,
    /// Number of inodes in the filesystem.
    pub inode_count: LE<u64>,
}
