// SPDX-License-Identifier: GPL-2.0

//! On-disk definitions for the ext2 filesystem.

use rko_core::types::LE;

pub const EXT2_SUPER_MAGIC: u16 = 0xEF53;
pub const EXT2_MAX_BLOCK_LOG_SIZE: u32 = 16;
pub const EXT2_GOOD_OLD_REV: u32 = 0;
pub const EXT2_DYNAMIC_REV: u32 = 1;
pub const EXT2_GOOD_OLD_INODE_SIZE: u16 = 128;
pub const EXT2_ROOT_INO: u32 = 2;
pub const EXT2_GOOD_OLD_FIRST_INO: u32 = 11;
pub const EXT2_FEATURE_INCOMPAT_FILETYPE: u32 = 0x0002;

pub const EXT2_NDIR_BLOCKS: usize = 12;
pub const EXT2_IND_BLOCK: usize = EXT2_NDIR_BLOCKS;
pub const EXT2_DIND_BLOCK: usize = EXT2_IND_BLOCK + 1;
pub const EXT2_TIND_BLOCK: usize = EXT2_DIND_BLOCK + 1;
pub const EXT2_N_BLOCKS: usize = EXT2_TIND_BLOCK + 1;

pub const FT_REG_FILE: u8 = 1;
pub const FT_DIR: u8 = 2;
pub const FT_CHRDEV: u8 = 3;
pub const FT_BLKDEV: u8 = 4;
pub const FT_FIFO: u8 = 5;
pub const FT_SOCK: u8 = 6;
pub const FT_SYMLINK: u8 = 7;

rko_core::derive_readable_from_bytes! {
    /// Ext2 superblock (on-disk layout).
    pub struct Super {
        pub inodes_count: LE<u32>,
        pub blocks_count: LE<u32>,
        pub r_blocks_count: LE<u32>,
        pub free_blocks_count: LE<u32>,
        pub free_inodes_count: LE<u32>,
        pub first_data_block: LE<u32>,
        pub log_block_size: LE<u32>,
        pub log_frag_size: LE<u32>,
        pub blocks_per_group: LE<u32>,
        pub frags_per_group: LE<u32>,
        pub inodes_per_group: LE<u32>,
        pub mtime: LE<u32>,
        pub wtime: LE<u32>,
        pub mnt_count: LE<u16>,
        pub max_mnt_count: LE<u16>,
        pub magic: LE<u16>,
        pub state: LE<u16>,
        pub errors: LE<u16>,
        pub minor_rev_level: LE<u16>,
        pub lastcheck: LE<u32>,
        pub checkinterval: LE<u32>,
        pub creator_os: LE<u32>,
        pub rev_level: LE<u32>,
        pub def_resuid: LE<u16>,
        pub def_resgid: LE<u16>,
        // EXT2_DYNAMIC_REV fields
        pub first_ino: LE<u32>,
        pub inode_size: LE<u16>,
        pub block_group_nr: LE<u16>,
        pub feature_compat: LE<u32>,
        pub feature_incompat: LE<u32>,
        pub feature_ro_compat: LE<u32>,
        pub uuid: [u8; 16],
        pub volume_name: [u8; 16],
        pub last_mounted: [u8; 64],
        pub algorithm_usage_bitmap: LE<u32>,
        pub prealloc_blocks: u8,
        pub prealloc_dir_blocks: u8,
        _padding1: u16,
        pub journal_uuid: [u8; 16],
        pub journal_inum: u32,
        pub journal_dev: u32,
        pub last_orphan: u32,
        pub hash_seed: [u32; 4],
        pub def_hash_version: u8,
        _reserved_char_pad: u8,
        _reserved_word_pad: u16,
        pub default_mount_opts: LE<u32>,
        pub first_meta_bg: LE<u32>,
        _reserved: [u32; 190],
    }

    /// Ext2 block group descriptor.
    #[derive(Clone, Copy)]
    pub struct Group {
        pub block_bitmap: LE<u32>,
        pub inode_bitmap: LE<u32>,
        pub inode_table: LE<u32>,
        pub free_blocks_count: LE<u16>,
        pub free_inodes_count: LE<u16>,
        pub used_dirs_count: LE<u16>,
        _pad: LE<u16>,
        _reserved: [u32; 3],
    }

    /// Ext2 inode (on-disk layout).
    pub struct INode {
        pub mode: LE<u16>,
        pub uid: LE<u16>,
        pub size: LE<u32>,
        pub atime: LE<u32>,
        pub ctime: LE<u32>,
        pub mtime: LE<u32>,
        pub dtime: LE<u32>,
        pub gid: LE<u16>,
        pub links_count: LE<u16>,
        pub blocks: LE<u32>,
        pub flags: LE<u32>,
        pub reserved1: LE<u32>,
        pub block: [LE<u32>; EXT2_N_BLOCKS],
        pub generation: LE<u32>,
        pub file_acl: LE<u32>,
        pub dir_acl: LE<u32>,
        pub faddr: LE<u32>,
        pub frag: u8,
        pub fsize: u8,
        pub pad1: LE<u16>,
        pub uid_high: LE<u16>,
        pub gid_high: LE<u16>,
        pub reserved2: LE<u32>,
    }

    /// Ext2 directory entry.
    pub struct DirEntry {
        pub inode: LE<u32>,
        pub rec_len: LE<u16>,
        pub name_len: u8,
        pub file_type: u8,
    }
}
