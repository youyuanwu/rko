// SPDX-License-Identifier: GPL-2.0

//! Read-only ext2 filesystem.
//!
//! Based on Wedson Almeida Filho's VFS patch (docs/patches/vfs.patch),
//! adapted to rko-core APIs.

#![no_std]

mod defs;

use core::mem::size_of;

use defs::*;
use rko_core::alloc::{Flags, KBox, KVec};
use rko_core::error::Error;
use rko_core::fs::iomap;
use rko_core::fs::{
    self, DEntry, DirEmitter, DirEntryType, File, FileOps, INode, INodeOps, LockedFolio, Mapper,
    Offset, Root, SuperBlock, Unhashed,
};
use rko_core::fs::{INodeParams, INodeType, Time};
use rko_core::types::{ARef, FromBytes, LE};

type Result<T = ()> = core::result::Result<T, Error>;

const SB_OFFSET: Offset = 1024;

static TABLES: fs::vtable::Tables<Ext2Fs> = fs::vtable::Tables::new();

/// Per-inode data: cached block pointers.
struct INodeData {
    data_blocks: [u32; EXT2_N_BLOCKS],
}

/// Per-superblock data.
struct Ext2Fs {
    mapper: Mapper,
    block_size: u32,
    has_file_type: bool,
    inodes_per_block: u32,
    inodes_per_group: u32,
    inode_count: u32,
    inode_size: u16,
    first_ino: u32,
    groups: KVec<Group>,
}

/// Static iomap-backed aops for regular files and page-based symlinks.
static IOMAP_AOPS: iomap::RoAops<Ext2Fs> = iomap::ro_aops::<Ext2Fs>();

impl Ext2Fs {
    fn iget(sb: &SuperBlock<Self>, ino: u32) -> Result<ARef<INode<Self>>> {
        let s = unsafe { sb.sb_data() };
        if (ino != EXT2_ROOT_INO && ino < s.first_ino) || ino > s.inode_count {
            return Err(Error::ENOENT);
        }
        let group = ((ino - 1) / s.inodes_per_group) as usize;
        let offset = (ino - 1) % s.inodes_per_group;

        if group >= s.groups.len() {
            return Err(Error::ENOENT);
        }

        // Look up or allocate an inode.
        let mut new_inode = match sb.iget(ino.into())? {
            Ok(new) => new,
            Err(cached) => return Ok(cached),
        };

        let inodes_block = Offset::from(s.groups[group].inode_table.value());
        let inode_block = inodes_block + Offset::from(offset / s.inodes_per_block);
        let off_in_block = (offset % s.inodes_per_block) as usize;
        let b = s
            .mapper
            .mapped_folio(inode_block * Offset::from(s.block_size))?;
        let idata = defs::INode::from_bytes(b.data(), off_in_block * s.inode_size as usize)
            .ok_or(Error::EIO)?; // EIO
        let mode = idata.mode.value();

        if idata.links_count.value() == 0 && (mode == 0 || idata.dtime.value() != 0) {
            return Err(Error::ESTALE);
        }

        let s_ifmt = mode & 0xF000;
        let mut size: Offset = idata.size.value().into();
        let typ = match s_ifmt {
            0o100000 => {
                // S_IFREG
                if let Some(hi) = Offset::from(idata.dir_acl.value()).checked_shl(32) {
                    size |= hi;
                }
                new_inode
                    .set_aops(IOMAP_AOPS.aops())
                    .set_fops(FileOps::regular(&TABLES));
                INodeType::Reg
            }
            0o040000 => {
                // S_IFDIR
                new_inode
                    .set_iops(INodeOps::dir(&TABLES))
                    .set_fops(FileOps::dir(&TABLES))
                    .set_aops(IOMAP_AOPS.aops());
                INodeType::Dir
            }
            0o120000 => {
                // S_IFLNK
                if idata.blocks.value() == 0 {
                    // Inline (fast) symlink: target in block[] array.
                    let blk_offset = core::mem::offset_of!(defs::INode, block);
                    let start = off_in_block * usize::from(s.inode_size) + blk_offset;
                    let name_len = size as usize;
                    let data = b.data();
                    if start + name_len > data.len() || name_len == 0 {
                        return Err(Error::EIO);
                    }
                    // SAFETY: The byte slice lives in a page-cache folio
                    // backed by the block device and remains valid for the
                    // life of the inode (the inode keeps the sb alive which
                    // keeps the bdev open). We extend the lifetime to 'static
                    // because the kernel guarantees page-cache pages for a
                    // mounted filesystem are not reclaimed while inodes using
                    // them exist.
                    let target: &'static [u8] =
                        unsafe { core::slice::from_raw_parts(data[start..].as_ptr(), name_len) };
                    INodeType::Lnk(Some(target))
                } else {
                    // Page-based symlink.
                    new_inode.set_aops(IOMAP_AOPS.aops());
                    INodeType::Lnk(None)
                }
            }
            0o140000 => INodeType::Sock, // S_IFSOCK
            0o010000 => INodeType::Fifo, // S_IFIFO
            0o020000 => {
                // S_IFCHR
                let (major, minor) = decode_dev(&idata.block);
                INodeType::Chr(major, minor)
            }
            0o060000 => {
                // S_IFBLK
                let (major, minor) = decode_dev(&idata.block);
                INodeType::Blk(major, minor)
            }
            _ => return Err(Error::ENOENT),
        };

        let t = Time { secs: 0, nsecs: 0 };
        new_inode.init(
            INodeParams {
                typ,
                mode: mode & 0o7777,
                size,
                blocks: idata.blocks.value().into(),
                nlink: idata.links_count.value().into(),
                uid: u32::from(idata.uid.value()) | (u32::from(idata.uid_high.value()) << 16),
                gid: u32::from(idata.gid.value()) | (u32::from(idata.gid_high.value()) << 16),
                ctime: Time {
                    secs: idata.ctime.value().into(),
                    ..t
                },
                mtime: Time {
                    secs: idata.mtime.value().into(),
                    ..t
                },
                atime: Time {
                    secs: idata.atime.value().into(),
                    ..t
                },
                value: INodeData {
                    data_blocks: core::array::from_fn(|i| idata.block[i].value()),
                },
            },
            &TABLES,
        )
    }

    fn offsets(block_size: u32, mut block: u64, out: &mut [u32]) -> Option<&[u32]> {
        let ptrs = u64::from(block_size / size_of::<u32>() as u32);
        let ptr_mask = ptrs - 1;
        let ptr_bits = ptrs.trailing_zeros();

        if block < EXT2_NDIR_BLOCKS as u64 {
            out[0] = block as u32;
            return Some(&out[..1]);
        }

        block -= EXT2_NDIR_BLOCKS as u64;
        if block < ptrs {
            out[0] = EXT2_IND_BLOCK as u32;
            out[1] = block as u32;
            return Some(&out[..2]);
        }

        block -= ptrs;
        if block < (1 << (2 * ptr_bits)) {
            out[0] = EXT2_DIND_BLOCK as u32;
            out[1] = (block >> ptr_bits) as u32;
            out[2] = (block & ptr_mask) as u32;
            return Some(&out[..3]);
        }

        block -= ptrs * ptrs;
        if block < ptrs * ptrs * ptrs {
            out[0] = EXT2_TIND_BLOCK as u32;
            out[1] = (block >> (2 * ptr_bits)) as u32;
            out[2] = ((block >> ptr_bits) & ptr_mask) as u32;
            out[3] = (block & ptr_mask) as u32;
            return Some(&out[..4]);
        }

        None
    }

    fn offset_to_block(inode: &INode<Self>, block: Offset) -> Result<u64> {
        let sb = unsafe { SuperBlock::<Self>::from_raw(inode.super_block()) };
        let s = unsafe { sb.sb_data() };
        let idata = unsafe { inode.data() };

        let mut indices = [0u32; 4];
        let boffsets = Self::offsets(s.block_size, block as u64, &mut indices).ok_or(Error::EIO)?;
        let mut boffset = idata.data_blocks[boffsets[0] as usize];
        for i in &boffsets[1..] {
            let b = s
                .mapper
                .mapped_folio(boffset as Offset * Offset::from(s.block_size))?;
            let table = <LE<u32> as FromBytes>::from_bytes_to_slice(b.data()).ok_or(Error::EIO)?;
            boffset = table[*i as usize].value();
        }
        Ok(boffset.into())
    }

    fn check_descriptors(s: &defs::Super, groups: &[Group]) -> Result {
        for (i, g) in groups.iter().enumerate() {
            let first = i as u32 * s.blocks_per_group.value() + s.first_data_block.value();
            let last = if i == groups.len() - 1 {
                s.blocks_count.value()
            } else {
                first + s.blocks_per_group.value() - 1
            };

            if g.block_bitmap.value() < first || g.block_bitmap.value() > last {
                return Err(Error::EINVAL);
            }
            if g.inode_bitmap.value() < first || g.inode_bitmap.value() > last {
                return Err(Error::EINVAL);
            }
            if g.inode_table.value() < first || g.inode_table.value() > last {
                return Err(Error::EINVAL);
            }
        }
        Ok(())
    }
}

#[rko_core::vtable]
impl fs::FileSystem for Ext2Fs {
    type Data = rko_core::alloc::KBox<Ext2Fs>;
    type INodeData = INodeData;
    const NAME: &'static core::ffi::CStr = c"rust_ext2";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;
    const SUPER_TYPE: fs::sb::Type = fs::sb::Type::BlockDev;

    fn fill_super(
        sb: &SuperBlock<Self, fs::sb::New>,
        _tables: &fs::vtable::Tables<Self>,
    ) -> Result<rko_core::alloc::KBox<Ext2Fs>> {
        if sb.min_blocksize(4096) == 0 {
            return Err(Error::EINVAL);
        }

        // Create mapper for reading from the block device.
        let mapper = Mapper::new(sb);

        // Read and validate the superblock.
        let mapped = mapper.mapped_folio(SB_OFFSET)?;
        let s = defs::Super::from_bytes(mapped.data(), 0).ok_or(Error::EIO)?;

        if s.magic.value() != EXT2_SUPER_MAGIC {
            return Err(Error::EINVAL);
        }

        let mut has_file_type = false;
        if s.rev_level.value() >= EXT2_DYNAMIC_REV {
            let features = s.feature_incompat.value();
            if features & !EXT2_FEATURE_INCOMPAT_FILETYPE != 0 {
                return Err(Error::EINVAL);
            }
            has_file_type = features & EXT2_FEATURE_INCOMPAT_FILETYPE != 0;

            if !sb.rdonly() && s.feature_ro_compat.value() != 0 {
                return Err(Error::EINVAL);
            }
        }

        let block_size_bits = s.log_block_size.value();
        if block_size_bits > EXT2_MAX_BLOCK_LOG_SIZE - 10 {
            return Err(Error::EINVAL);
        }
        let block_size = 1024u32 << block_size_bits;
        if sb.min_blocksize(block_size as i32) != block_size as i32 {
            return Err(Error::ENXIO);
        }

        let (inode_size, first_ino) = if s.rev_level.value() == EXT2_GOOD_OLD_REV {
            (EXT2_GOOD_OLD_INODE_SIZE, EXT2_GOOD_OLD_FIRST_INO)
        } else {
            let sz = s.inode_size.value();
            if sz < EXT2_GOOD_OLD_INODE_SIZE || !sz.is_power_of_two() || u32::from(sz) > block_size
            {
                return Err(Error::EINVAL);
            }
            (sz, s.first_ino.value())
        };

        let inode_count = s.inodes_count.value();
        let inodes_per_group = s.inodes_per_group.value();
        let inodes_per_block = block_size / u32::from(inode_size);
        if inodes_per_group == 0 || inodes_per_block == 0 {
            return Err(Error::EINVAL);
        }

        let blocks_per_group = s.blocks_per_group.value();
        let blocks_count = s.blocks_count.value();

        let group_count = (blocks_count - s.first_data_block.value() - 1) / blocks_per_group + 1;

        // Read group descriptors under NoFsGuard — all allocations in this
        // scope avoid filesystem recursion (equivalent to GFP_NOFS).
        let _nofs = rko_core::alloc::NoFsGuard::new();
        let mut groups = KVec::new();
        groups.reserve(group_count as usize, Flags::GFP_KERNEL)?;

        let mut remain = group_count;
        let mut gd_offset = (SB_OFFSET / Offset::from(block_size) + 1) * Offset::from(block_size);
        while remain > 0 {
            let b = mapper.mapped_folio(gd_offset)?;
            let slice = Group::from_bytes_to_slice(b.data()).ok_or(Error::EIO)?;
            for g in slice {
                groups.push(*g, Flags::GFP_KERNEL)?;
                remain -= 1;
                if remain == 0 {
                    break;
                }
            }
            gd_offset += b.data().len() as Offset;
        }

        Self::check_descriptors(s, &groups)?;

        sb.set_magic(s.magic.value().into());

        Ok(KBox::new(
            Ext2Fs {
                mapper,
                block_size,
                has_file_type,
                inodes_per_block,
                inodes_per_group,
                inode_count,
                inode_size,
                first_ino,
                groups,
            },
            Flags::GFP_KERNEL,
        )?)
    }

    fn init_root(sb: &SuperBlock<Self>, _t: &fs::vtable::Tables<Self>) -> Result<Root<Self>> {
        let inode = Self::iget(sb, EXT2_ROOT_INO)?;
        Root::try_new(inode)
    }

    fn read_folio(
        _inode: &INode<Self>,
        _folio: &mut LockedFolio<'_, fs::PageCache<Self>>,
    ) -> Result<()> {
        // For ext2, read_folio is handled by iomap via the aops.
        // This should not be called directly — it's a fallback.
        Err(Error::EIO) // EIO
    }
}

#[rko_core::vtable]
impl fs::inode::Operations for Ext2Fs {
    type FileSystem = Self;

    fn lookup(
        parent: &rko_core::types::Locked<'_, INode<Self>, fs::inode::ReadSem>,
        dentry: Unhashed<'_, Self>,
    ) -> Result<Option<ARef<DEntry<Self>>>> {
        let sb = unsafe { SuperBlock::<Self>::from_raw(parent.super_block()) };
        let s = unsafe { sb.sb_data() };

        // Walk the directory blocks looking for a matching entry.
        let dir_size = parent.size();
        let mut pos: Offset = 0;

        while pos < dir_size {
            let block_idx = pos / Offset::from(s.block_size);
            let boffset = Self::offset_to_block(parent, block_idx)?;
            if boffset == 0 {
                pos += Offset::from(s.block_size);
                continue;
            }
            let b = s
                .mapper
                .mapped_folio(boffset as Offset * Offset::from(s.block_size))?;
            let data = b.data();
            let mut off = 0usize;
            let limit = data.len().saturating_sub(size_of::<DirEntry>());
            while off < limit {
                let de = DirEntry::from_bytes(data, off).ok_or(Error::EIO)?;
                let rec_len = de.rec_len.value() as usize;
                if rec_len == 0 || off + rec_len > data.len() {
                    break;
                }
                let name_start = off + size_of::<DirEntry>();
                let name_len = de.name_len as usize;
                if name_start + name_len > data.len() {
                    break;
                }
                let name = &data[name_start..name_start + name_len];
                let ino = de.inode.value();
                if ino != 0 && name == dentry.name() {
                    let found = Self::iget(sb, ino)?;
                    return dentry.splice_alias(Some(found));
                }
                off += rec_len;
            }
            pos += Offset::from(s.block_size);
        }

        dentry.splice_alias(None)
    }
}

#[rko_core::vtable]
impl fs::file::Operations for Ext2Fs {
    type FileSystem = Self;

    fn read_dir(
        _file: &File<Self>,
        inode: &rko_core::types::Locked<'_, INode<Self>, fs::inode::ReadSem>,
        emitter: &mut DirEmitter,
    ) -> Result<()> {
        let sb = unsafe { SuperBlock::<Self>::from_raw(inode.super_block()) };
        let s = unsafe { sb.sb_data() };

        let dir_size = inode.size();
        let mut pos = emitter.pos();

        while pos < dir_size {
            let block_idx = pos / Offset::from(s.block_size);
            let boffset = Self::offset_to_block(inode, block_idx)?;
            if boffset == 0 {
                pos += Offset::from(s.block_size);
                continue;
            }
            let b = s
                .mapper
                .mapped_folio(boffset as Offset * Offset::from(s.block_size))?;
            let data = b.data();

            // Start within this block at the offset within the block.
            let block_start = block_idx * Offset::from(s.block_size);
            let mut off = if pos > block_start {
                (pos - block_start) as usize
            } else {
                0
            };

            let limit = data.len().saturating_sub(size_of::<DirEntry>());
            while off < limit {
                let de = DirEntry::from_bytes(data, off).ok_or(Error::EIO)?;
                let rec_len = de.rec_len.value() as usize;
                if rec_len == 0 || off + rec_len > data.len() {
                    break;
                }
                let name_start = off + size_of::<DirEntry>();
                let name_len = de.name_len as usize;
                let ino = de.inode.value();
                off += rec_len;

                if ino == 0 || name_start + name_len > data.len() {
                    continue;
                }

                let name = &data[name_start..name_start + name_len];
                let t = if !s.has_file_type {
                    DirEntryType::Unknown
                } else {
                    match de.file_type {
                        FT_REG_FILE => DirEntryType::Reg,
                        FT_DIR => DirEntryType::Dir,
                        FT_SYMLINK => DirEntryType::Lnk,
                        FT_CHRDEV => DirEntryType::Chr,
                        FT_BLKDEV => DirEntryType::Blk,
                        FT_FIFO => DirEntryType::Fifo,
                        FT_SOCK => DirEntryType::Sock,
                        _ => DirEntryType::Unknown,
                    }
                };

                if !emitter.emit(rec_len as Offset, name, ino.into(), t) {
                    return Ok(());
                }
            }
            pos = block_start + Offset::from(s.block_size);
        }
        Ok(())
    }
}

#[rko_core::vtable]
impl iomap::Operations for Ext2Fs {
    type FileSystem = Self;

    fn begin<'a>(
        inode: &'a INode<Self>,
        pos: Offset,
        length: Offset,
        _flags: u32,
        map: &mut iomap::Map<'a>,
        _srcmap: &mut iomap::Map<'a>,
    ) -> Result {
        let size = inode.size();
        if pos >= size {
            map.set_offset(pos)
                .set_length(length as u64)
                .set_flags(iomap::map_flags::MERGED)
                .set_type(iomap::Type::Hole);
            return Ok(());
        }

        let sb = unsafe { SuperBlock::<Self>::from_raw(inode.super_block()) };
        let s = unsafe { sb.sb_data() };
        let block_size = s.block_size as Offset;
        let block = pos / block_size;

        let boffset = Self::offset_to_block(inode, block)?;
        map.set_offset(block * block_size)
            .set_length(block_size as u64)
            .set_flags(iomap::map_flags::MERGED)
            .set_type(iomap::Type::Mapped)
            .set_bdev(sb)
            .set_addr(boffset * block_size as u64);

        Ok(())
    }
}

fn decode_dev(block: &[LE<u32>]) -> (u32, u32) {
    let v = block[0].value();
    if v != 0 {
        ((v >> 8) & 255, v & 255)
    } else {
        let v = block[1].value();
        ((v & 0xfff00) >> 8, (v & 0xff) | ((v >> 12) & 0xfff00))
    }
}

rko_core::module_fs! {
    type: Ext2Fs,
    name: "rust_ext2",
    license: "GPL",
    author: "rko",
    description: "Read-only ext2 filesystem",
}
