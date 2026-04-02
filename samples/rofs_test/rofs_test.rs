//! Minimal read-only filesystem test module.
//!
//! Registers a filesystem type "rofs_test" that serves a single directory
//! containing one file ("hello.txt") with static content.

#![no_std]

use rko_core::error::Error;
use rko_core::fs::{
    self, DEntry, DirEmitter, DirEntryType, INode, INodeParams, INodeType, LockedFolio, Root,
    SuperBlock, SuperParams, Time, Unhashed,
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
const HELLO_CONTENT: &[u8] = b"Hello from rofs!\n";
const HELLO_NAME: &[u8] = b"hello.txt";

const EPOCH: Time = Time { secs: 0, nsecs: 0 };

static TABLES: fs::vtable::Tables<RofsTest> = fs::vtable::Tables::new();

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

        folio.zero_out(0, folio_size)?;

        if folio_offset < content.len() {
            let to_copy = core::cmp::min(content.len() - folio_offset, folio_size);
            let src = &content[folio_offset..folio_offset + to_copy];
            folio.write(0, src)?;
        }

        Ok(())
    }
}

impl fs::inode::Operations for RofsTest {
    type FileSystem = Self;

    fn lookup(
        parent: &INode<Self>,
        dentry: Unhashed<'_, Self>,
    ) -> Result<Option<ARef<DEntry<Self>>>, Error> {
        if parent.ino() != ROOT_INO {
            return dentry.splice_alias(None);
        }

        if dentry.name() == HELLO_NAME {
            let sb = unsafe { SuperBlock::from_raw(parent.super_block()) };
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
        } else {
            dentry.splice_alias(None)
        }
    }
}

impl fs::file::Operations for RofsTest {
    type FileSystem = Self;

    fn read_dir(
        _file: &fs::File<Self>,
        inode: &INode<Self>,
        emitter: &mut DirEmitter,
    ) -> Result<(), Error> {
        if inode.ino() != ROOT_INO {
            return Err(Error::ENOTDIR);
        }

        let entries: &[(&[u8], u64, DirEntryType)] = &[
            (b".", ROOT_INO, DirEntryType::Dir),
            (b"..", ROOT_INO, DirEntryType::Dir),
            (HELLO_NAME, HELLO_INO, DirEntryType::Reg),
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
}

rko_core::module_fs! {
    type: RofsTest,
    name: "rofs_test",
    license: "GPL",
    author: "rko",
    description: "Read-only filesystem test module",
}
