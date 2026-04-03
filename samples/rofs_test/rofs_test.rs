//! Minimal read-only filesystem test module.
//!
//! Registers a filesystem type "rofs_test" that serves a single directory
//! containing one file ("hello.txt") with static content.

#![no_std]

use rko_core::error::Error;
use rko_core::fs::{
    self, DEntry, DirEmitter, DirEntryType, INode, INodeParams, INodeType, LockedFolio, Offset,
    Root, SuperBlock, SuperParams, Time, Unhashed, Whence,
};
use rko_core::types::ARef;

// --- Filesystem type ---

struct RofsTest;

/// Inode data: stores content for regular files.
pub struct InodeData {
    /// Content bytes for files, None for directories.
    content: Option<&'static [u8]>,
}

const ROOT_INO: u64 = 1;
const HELLO_INO: u64 = 2;
const INFO_INO: u64 = 3;
const LINK_INO: u64 = 4;
const HELLO_CONTENT: &[u8] = b"Hello from rofs!\n";
const INFO_CONTENT: &[u8] = b"custom read works\n";
const HELLO_NAME: &[u8] = b"hello.txt";
const INFO_NAME: &[u8] = b"info.txt";
const LINK_NAME: &[u8] = b"link.txt";
const LINK_TARGET: &[u8] = b"hello.txt";

const EPOCH: Time = Time { secs: 0, nsecs: 0 };

static TABLES: fs::vtable::Tables<RofsTest> = fs::vtable::Tables::new();

#[rko_core::vtable]
impl fs::FileSystem for RofsTest {
    type Data = ();
    type INodeData = InodeData;
    const NAME: &'static core::ffi::CStr = c"rofs_test";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;

    fn fill_super(
        sb: &SuperBlock<Self, fs::sb::New>,
        _tables: &fs::vtable::Tables<Self>,
    ) -> Result<(), Error> {
        sb.init_simple(&SuperParams {
            maxbytes: i64::MAX,
            blocksize_bits: 12,
            magic: 0x524F_4653,
            time_gran: 1,
        });
        Ok(())
    }

    fn init_root(
        sb: &SuperBlock<Self>,
        tables: &fs::vtable::Tables<Self>,
    ) -> Result<Root<Self>, Error> {
        let root = match sb.iget(ROOT_INO)? {
            Ok(new_inode) => new_inode.init(
                INodeParams {
                    mode: 0o555,
                    typ: INodeType::Dir,
                    size: 0,
                    blocks: 0,
                    nlink: 2,
                    uid: 0,
                    gid: 0,
                    ctime: EPOCH,
                    mtime: EPOCH,
                    atime: EPOCH,
                    value: InodeData { content: None },
                },
                tables,
            )?,
            Err(cached) => cached,
        };

        Root::try_new(root)
    }

    fn read_folio(
        inode: &INode<Self>,
        folio: &mut LockedFolio<'_, fs::PageCache<Self>>,
    ) -> Result<(), Error> {
        let data = unsafe { inode.data() };
        let content = data.content.unwrap_or(b"");

        let folio_offset = folio.pos() as usize;
        let folio_size = folio.size();

        // Use Folio::map() to get direct byte access (exercises FolioMap).
        let mut map = folio.map(0)?;
        let buf = map.data_mut();

        // Zero-fill, then copy content.
        buf[..folio_size].fill(0);
        if folio_offset < content.len() {
            let to_copy = core::cmp::min(content.len() - folio_offset, folio_size);
            buf[..to_copy].copy_from_slice(&content[folio_offset..folio_offset + to_copy]);
        }
        drop(map); // explicit unmap

        folio.flush_dcache();
        Ok(())
    }
}

#[rko_core::vtable]
impl fs::inode::Operations for RofsTest {
    type FileSystem = Self;

    fn lookup(
        parent: &rko_core::types::Locked<'_, INode<Self>, fs::inode::ReadSem>,
        dentry: Unhashed<'_, Self>,
    ) -> Result<Option<ARef<DEntry<Self>>>, Error> {
        if parent.ino() != ROOT_INO {
            return dentry.splice_alias(None);
        }

        let sb = unsafe { SuperBlock::from_raw(parent.super_block()) };

        if dentry.name() == HELLO_NAME {
            let inode = match sb.iget(HELLO_INO)? {
                Ok(new_inode) => new_inode.init(
                    INodeParams {
                        mode: 0o444,
                        typ: INodeType::Reg,
                        size: HELLO_CONTENT.len() as i64,
                        blocks: 1,
                        nlink: 1,
                        uid: 0,
                        gid: 0,
                        ctime: EPOCH,
                        mtime: EPOCH,
                        atime: EPOCH,
                        value: InodeData {
                            content: Some(HELLO_CONTENT),
                        },
                    },
                    &TABLES,
                )?,
                Err(cached) => cached,
            };
            dentry.splice_alias(Some(inode))
        } else if dentry.name() == INFO_NAME {
            let inode = match sb.iget(INFO_INO)? {
                Ok(new_inode) => new_inode.init(
                    INodeParams {
                        mode: 0o444,
                        typ: INodeType::Reg,
                        size: INFO_CONTENT.len() as i64,
                        blocks: 1,
                        nlink: 1,
                        uid: 0,
                        gid: 0,
                        ctime: EPOCH,
                        mtime: EPOCH,
                        atime: EPOCH,
                        value: InodeData {
                            content: Some(INFO_CONTENT),
                        },
                    },
                    &TABLES,
                )?,
                Err(cached) => cached,
            };
            dentry.splice_alias(Some(inode))
        } else if dentry.name() == LINK_NAME {
            let inode = match sb.iget(LINK_INO)? {
                Ok(mut new_inode) => {
                    // Use custom symlink ops so get_link is called.
                    new_inode.set_iops(fs::INodeOps::custom_symlink(&TABLES));
                    new_inode.init(
                        INodeParams {
                            mode: 0o777,
                            typ: INodeType::Lnk(None),
                            size: LINK_TARGET.len() as i64,
                            blocks: 0,
                            nlink: 1,
                            uid: 0,
                            gid: 0,
                            ctime: EPOCH,
                            mtime: EPOCH,
                            atime: EPOCH,
                            value: InodeData { content: None },
                        },
                        &TABLES,
                    )?
                }
                Err(cached) => cached,
            };
            dentry.splice_alias(Some(inode))
        } else {
            dentry.splice_alias(None)
        }
    }

    fn get_link(
        _dentry: Option<&DEntry<Self>>,
        _inode: &INode<Self>,
    ) -> Result<fs::GetLinkResult, Error> {
        // Build the target dynamically using CString (exercises CString +
        // KBox::new_slice + ForeignOwnable + delayed_call cleanup).
        let target = rko_core::types::CString::try_from_slice(
            LINK_TARGET,
            rko_core::alloc::Flags::GFP_KERNEL,
        )?;
        Ok(fs::GetLinkResult::Owned(target))
    }
}

#[rko_core::vtable]
impl fs::file::Operations for RofsTest {
    type FileSystem = Self;

    fn read_dir(
        _file: &fs::File<Self>,
        inode: &rko_core::types::Locked<'_, INode<Self>, fs::inode::ReadSem>,
        emitter: &mut DirEmitter,
    ) -> Result<(), Error> {
        if inode.ino() != ROOT_INO {
            return Err(Error::ENOTDIR);
        }

        let entries: &[(&[u8], u64, DirEntryType)] = &[
            (b".", ROOT_INO, DirEntryType::Dir),
            (b"..", ROOT_INO, DirEntryType::Dir),
            (HELLO_NAME, HELLO_INO, DirEntryType::Reg),
            (INFO_NAME, INFO_INO, DirEntryType::Reg),
            (LINK_NAME, LINK_INO, DirEntryType::Lnk),
        ];

        let start = emitter.pos() as usize;
        for (i, &(name, ino, dt)) in entries.iter().enumerate() {
            if i < start {
                continue;
            }
            if !emitter.emit(1, name, ino, dt) {
                return Ok(());
            }
        }
        Ok(())
    }

    fn seek(file: &fs::File<Self>, offset: Offset, whence: Whence) -> Result<Offset, Error> {
        let inode = file.inode();
        let size = inode.size();

        match whence {
            // SEEK_DATA: return offset of next data at or after `offset`.
            // File is fully dense — data starts at `offset` itself.
            Whence::Data => {
                if offset < 0 || offset >= size {
                    return Err(Error::ENXIO);
                }
                Ok(offset)
            }
            // SEEK_HOLE: return offset of next hole at or after `offset`.
            // File has no holes — first hole is at EOF.
            Whence::Hole => {
                if offset < 0 || offset >= size {
                    return Err(Error::ENXIO);
                }
                Ok(size)
            }
            // Standard seeks — delegate to EINVAL so the kernel's
            // generic_file_llseek handles them. (We should only be
            // called for Data/Hole when we override.)
            _ => Err(Error::EINVAL),
        }
    }

    fn read_iter(
        file: &fs::File<Self>,
        iter: &mut rko_core::iov::IoVecIter,
        offset: Offset,
    ) -> Result<usize, Error> {
        let inode = file.inode();
        let data = unsafe { inode.data() };
        let content = data.content.unwrap_or(b"");

        let pos = offset as usize;
        if pos >= content.len() {
            return Ok(0); // EOF
        }

        let available = content.len() - pos;
        let to_copy = available.min(iter.count());
        iter.write_all(&content[pos..pos + to_copy])?;
        Ok(to_copy)
    }
}

rko_core::module_fs! {
    type: RofsTest,
    name: "rofs_test",
    license: "GPL",
    author: "rko",
    description: "Read-only filesystem test module",
}
