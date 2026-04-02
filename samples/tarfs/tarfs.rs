//! TarFS — read-only filesystem for indexed tar archives.
//!
//! Mounts a block device containing a tar archive with an appended index.
//! The index (in the last sector) contains an inode table and directory
//! entry tables that allow efficient random-access lookup without scanning
//! the entire tar file.
//!
//! Based on the upstream VFS patch by Wedson Almeida Filho.

#![no_std]

use core::mem::size_of;

use rko_core::alloc::{Flags, KBox};
use rko_core::error::Error;
use rko_core::fs::dentry::{DEntry, Unhashed};
use rko_core::fs::mapper::Mapper;
use rko_core::fs::{
    self, DirEmitter, DirEntryType, INode, INodeParams, INodeType, LockedFolio, Root, Stat,
    SuperBlock, Time,
};
use rko_core::types::{ARef, FromBytes};

mod defs;
use defs::*;

const SECTOR_SIZE: u64 = 512;
const TARFS_BSIZE: u64 = 1 << TARFS_BSIZE_BITS;
const TARFS_BSIZE_BITS: u8 = 12;
const TARFS_MAGIC: u64 = 0x5441_5246; // "TARF"

/// S_IFMT mask and mode constants from <linux/stat.h>.
const S_IFMT: u16 = 0o170000;

/// Per-superblock data stored in s_fs_info via ForeignOwnable.
struct TarFsData {
    /// Total data size (sector_count * SECTOR_SIZE).
    data_size: u64,
    /// Byte offset to the inode table.
    inode_table_offset: u64,
    /// Number of inodes.
    inode_count: u64,
    /// Mapper for reading folios from the block device.
    mapper: Mapper,
}

/// Per-inode data.
pub struct InodeData {
    /// Byte offset to file/directory data on the block device.
    offset: u64,
    /// Tarfs flags (e.g., OPAQUE for overlay support).
    flags: u8,
}

struct TarFs;

static TABLES: fs::vtable::Tables<TarFs> = fs::vtable::Tables::new();

impl TarFs {
    /// Load an inode from the inode table.
    fn iget(sb: &SuperBlock<Self>, ino: u64) -> Result<ARef<INode<Self>>, Error> {
        let data: &TarFsData = unsafe { sb.data() };

        // Validate inode number.
        if ino == 0 || ino > data.inode_count {
            return Err(Error::new(-2)); // ENOENT
        }

        // Check cache first.
        match sb.iget(ino)? {
            Err(cached) => Ok(cached),
            Ok(new_inode) => {
                // Load inode details from storage.
                let offset = data.inode_table_offset + (ino - 1) * size_of::<Inode>() as u64;
                let mapped = data.mapper.mapped_folio(offset as i64)?;
                let idata = match Inode::from_bytes(mapped.data(), 0) {
                    Some(i) => i,
                    None => return Err(Error::new(-5)), // EIO
                };

                let mode = idata.mode.value();
                let size = idata.size.value();
                let doffset = idata.offset.value();
                let secs = u64::from(idata.lmtime.value()) | (u64::from(idata.hmtime & 0xf) << 32);
                let ts = Time { secs, nsecs: 0 };

                let typ = match mode & S_IFMT {
                    fs::S_IFREG => INodeType::Reg,
                    fs::S_IFDIR => INodeType::Dir,
                    fs::S_IFLNK => INodeType::Lnk(None),
                    fs::S_IFSOCK => INodeType::Sock,
                    fs::S_IFIFO => INodeType::Fifo,
                    fs::S_IFCHR => INodeType::Chr((doffset >> 32) as u32, doffset as u32),
                    fs::S_IFBLK => INodeType::Blk((doffset >> 32) as u32, doffset as u32),
                    _ => return Err(Error::new(-2)), // ENOENT
                };

                new_inode.init(
                    INodeParams {
                        mode: mode & 0o777,
                        typ,
                        size: size as i64,
                        blocks: size.div_ceil(TARFS_BSIZE),
                        nlink: 1,
                        uid: idata.owner.value(),
                        gid: idata.group.value(),
                        ctime: ts,
                        mtime: ts,
                        atime: ts,
                        value: InodeData {
                            offset: doffset,
                            flags: idata.flags,
                        },
                    },
                    &TABLES,
                )
            }
        }
    }

    /// Compare a name on disk with a given byte slice.
    fn name_eq(data: &TarFsData, name: &[u8], offset: u64) -> Result<bool, Error> {
        let mut remaining = name;
        let ret =
            data.mapper
                .for_each_page(offset as i64, name.len() as i64, |page_data: &[u8]| {
                    let len = core::cmp::min(page_data.len(), remaining.len());
                    if page_data[..len] != remaining[..len] {
                        return Ok(Some(false));
                    }
                    remaining = &remaining[len..];
                    Ok(None)
                })?;
        Ok(ret.unwrap_or(true))
    }
}

impl fs::FileSystem for TarFs {
    type Data = KBox<TarFsData>;
    type INodeData = InodeData;

    const NAME: &'static core::ffi::CStr = c"tarfs";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;
    const SUPER_TYPE: fs::sb::Type = fs::sb::Type::BlockDev;

    fn fill_super(
        sb: &SuperBlock<Self>,
        _tables: &fs::vtable::Tables<Self>,
    ) -> Result<KBox<TarFsData>, Error> {
        let scount = sb.sector_count();
        if scount < (TARFS_BSIZE / SECTOR_SIZE) {
            return Err(Error::new(-6)); // ENXIO
        }

        if sb.min_blocksize(SECTOR_SIZE as i32) != SECTOR_SIZE as i32 {
            return Err(Error::new(-5)); // EIO
        }

        // Create the mapper for reading from the block device.
        let mapper = unsafe { Mapper::from_sb(sb.as_ptr()) };

        // Read the header from the last sector.
        let header_offset = (scount - 1) * SECTOR_SIZE;
        let mapped = mapper.mapped_folio(header_offset as i64)?;
        let hdr = match Header::from_bytes(mapped.data(), 0) {
            Some(h) => h,
            None => return Err(Error::new(-5)), // EIO
        };

        let inode_table_offset = hdr.inode_table_offset.value();
        let inode_count = hdr.inode_count.value();
        let data_size = scount * SECTOR_SIZE;

        // Drop the mapped folio before allocating (avoid nesting).
        drop(mapped);

        let tarfs_data = KBox::new(
            TarFsData {
                data_size,
                inode_table_offset,
                inode_count,
                mapper,
            },
            Flags::GFP_KERNEL,
        )
        .map_err(|_| Error::new(-12))?;

        // Validate inode table bounds.
        if inode_table_offset >= data_size {
            return Err(Error::new(-7)); // E2BIG
        }

        let table_end = inode_count
            .checked_mul(size_of::<Inode>() as u64)
            .and_then(|s| s.checked_add(inode_table_offset))
            .ok_or(Error::new(-34))?; // ERANGE
        if table_end > data_size {
            return Err(Error::new(-7)); // E2BIG
        }

        sb.set_magic(TARFS_MAGIC as usize);

        Ok(tarfs_data)
    }

    fn init_root(
        sb: &SuperBlock<Self>,
        _tables: &fs::vtable::Tables<Self>,
    ) -> Result<Root<Self>, Error> {
        // Inode #1 is the root directory.
        let inode = Self::iget(sb, 1)?;
        Root::try_new(inode)
    }

    fn lookup(
        parent: &INode<Self>,
        dentry: Unhashed<'_, Self>,
        _tables: &fs::vtable::Tables<Self>,
    ) -> Result<Option<ARef<DEntry<Self>>>, Error> {
        let sb = unsafe { SuperBlock::from_raw(parent.super_block()) };
        let data: &TarFsData = unsafe { sb.data() };
        let name = dentry.name();
        let parent_data = unsafe { parent.data() };

        // Iterate directory entries to find the name.
        let found = data.mapper.for_each_page(
            parent_data.offset as i64,
            parent.size(),
            |page_data: &[u8]| {
                let entries = match DirEntry::from_bytes_to_slice(page_data) {
                    Some(e) => e,
                    None => return Err(Error::new(-5)), // EIO
                };
                for e in entries {
                    if Self::name_eq(data, name, e.name_offset.value())? {
                        let inode = Self::iget(sb, e.ino.value())?;
                        return Ok(Some(inode));
                    }
                }
                Ok(None)
            },
        )?;

        dentry.splice_alias(found)
    }

    fn read_dir(
        _file: &fs::File<Self>,
        inode: &INode<Self>,
        emitter: &mut DirEmitter,
    ) -> Result<(), Error> {
        let sb: &SuperBlock<Self> = unsafe { SuperBlock::from_raw(inode.super_block()) };
        let data: &TarFsData = unsafe { sb.data() };
        let inode_data = unsafe { inode.data() };
        let pos = emitter.pos();

        if pos < 0 || pos % size_of::<DirEntry>() as i64 != 0 {
            return Err(Error::new(-2)); // ENOENT
        }

        if pos >= inode.size() {
            return Ok(());
        }

        // Validate data bounds.
        let size_u = inode.size() as u64;
        if inode_data
            .offset
            .checked_add(size_u)
            .is_none_or(|end| end > data.data_size)
        {
            return Err(Error::new(-5)); // EIO
        }

        data.mapper.for_each_page(
            inode_data.offset as i64 + pos,
            inode.size() - pos,
            |page_data: &[u8]| {
                let entries = match DirEntry::from_bytes_to_slice(page_data) {
                    Some(e) => e,
                    None => return Err(Error::new(-5)),
                };
                for e in entries {
                    let name_len = e.name_len.value() as usize;

                    // Read the name. For simplicity, use a fixed buffer.
                    let mut name_buf = [0u8; 256];
                    if name_len > name_buf.len() {
                        return Err(Error::new(-36)); // ENAMETOOLONG
                    }
                    // Read name from disk.
                    let mut name_off = 0usize;
                    data.mapper.for_each_page(
                        e.name_offset.value() as i64,
                        name_len as i64,
                        |name_data: &[u8]| {
                            let copy_len = core::cmp::min(name_data.len(), name_len - name_off);
                            name_buf[name_off..name_off + copy_len]
                                .copy_from_slice(&name_data[..copy_len]);
                            name_off += copy_len;
                            Ok(None::<()>)
                        },
                    )?;

                    let dt = match e.etype {
                        4 => DirEntryType::Dir,
                        8 => DirEntryType::Reg,
                        10 => DirEntryType::Lnk,
                        2 => DirEntryType::Chr,
                        6 => DirEntryType::Blk,
                        1 => DirEntryType::Fifo,
                        12 => DirEntryType::Sock,
                        _ => DirEntryType::Unknown,
                    };

                    if !emitter.emit(
                        size_of::<DirEntry>() as i64,
                        &name_buf[..name_len],
                        e.ino.value(),
                        dt,
                    ) {
                        return Ok(Some(()));
                    }
                }
                Ok(None)
            },
        )?;

        Ok(())
    }

    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_>) -> Result<(), Error> {
        // Read file content from the block device.
        let sb: &SuperBlock<Self> = unsafe { SuperBlock::from_raw(inode.super_block()) };
        let data: &TarFsData = unsafe { sb.data() };
        let inode_data = unsafe { inode.data() };

        let folio_offset = folio.pos() as u64;
        let folio_size = folio.size();
        let file_size = inode.size() as u64;

        // Zero the folio first.
        folio.zero_out(0, folio_size)?;

        // If this folio is beyond the file, leave it zeroed.
        if folio_offset >= file_size {
            return Ok(());
        }

        // Read file data from the block device.
        let to_read = core::cmp::min(folio_size as u64, file_size - folio_offset) as usize;
        let disk_offset = inode_data.offset + folio_offset;

        let mut written = 0usize;
        data.mapper
            .for_each_page(disk_offset as i64, to_read as i64, |page_data: &[u8]| {
                let copy_len = core::cmp::min(page_data.len(), to_read - written);
                folio.write(written, &page_data[..copy_len])?;
                written += copy_len;
                Ok(None::<()>)
            })?;

        Ok(())
    }

    fn read_xattr(
        _dentry: &DEntry<Self>,
        inode: &INode<Self>,
        name: &core::ffi::CStr,
        outbuf: &mut [u8],
    ) -> Result<usize, Error> {
        let inode_data = unsafe { inode.data() };

        // Only support the overlay opaque xattr.
        if inode_data.flags & inode_flags::OPAQUE == 0 {
            return Err(Error::new(-61)); // ENODATA
        }
        if name.to_bytes() != b"trusted.overlay.opaque" {
            return Err(Error::new(-61)); // ENODATA
        }

        if !outbuf.is_empty() {
            outbuf[0] = b'y';
        }
        Ok(1)
    }

    fn statfs(dentry: &DEntry<Self>) -> Result<Stat, Error> {
        let sb = dentry.super_block();
        let data: &TarFsData = unsafe { sb.data() };
        Ok(Stat {
            magic: TARFS_MAGIC as usize,
            namelen: 255,
            bsize: TARFS_BSIZE as isize,
            blocks: data.inode_table_offset / TARFS_BSIZE,
            files: data.inode_count,
        })
    }
}

rko_core::module_fs! {
    type: TarFs,
    name: "tarfs",
    license: "GPL",
    author: "rko",
    description: "Read-only filesystem for indexed tar archives",
}
