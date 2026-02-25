//! Minimal read-only filesystem test module.
//!
//! Registers a filesystem type "rofs_test" that serves a single directory
//! containing one file ("hello.txt") with static content.

#![no_std]

use core::pin::Pin;

use rko_core::alloc::{Flags, KBox};
use rko_core::error::Error;
use rko_core::fs::{
    self, INode, INodeParams, INodeType, LockedFolio, Registration, SuperBlock, SuperParams, Time,
};
use rko_core::prelude::*;
use rko_core::types::ARef;

// --- Filesystem type ---

struct RofsTest;

/// Inode data: stores content for regular files.
pub struct InodeData {
    /// Content bytes for files, None for directories.
    content: Option<&'static [u8]>,
}

// DT_DIR / DT_REG constants from linux/fs.h
const DT_DIR: u8 = 4;
const DT_REG: u8 = 8;

const ROOT_INO: u64 = 1;
const HELLO_INO: u64 = 2;
const HELLO_CONTENT: &[u8] = b"Hello from rofs!\n";
const HELLO_NAME: &[u8] = b"hello.txt";

const EPOCH: Time = Time { secs: 0, nsecs: 0 };

static TABLES: fs::vtable::Tables<RofsTest> = fs::vtable::Tables::new();

impl fs::Type for RofsTest {
    type INodeData = InodeData;
    const NAME: &'static core::ffi::CStr = c"rofs_test";
    const TABLES: &'static fs::vtable::Tables<Self> = &TABLES;

    fn fill_super(
        sb: &SuperBlock<Self>,
        tables: &fs::vtable::Tables<Self>,
    ) -> Result<(), Error> {
        sb.init_simple(&SuperParams {
            maxbytes: i64::MAX,
            blocksize_bits: 12,
            magic: 0x524F_4653,
            time_gran: 1,
        });

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

        sb.set_root(root)?;
        Ok(())
    }

    fn lookup(
        parent: &INode<Self>,
        name: &[u8],
        tables: &fs::vtable::Tables<Self>,
    ) -> Result<Option<ARef<INode<Self>>>, Error> {
        if parent.ino() != ROOT_INO {
            return Ok(None);
        }

        if name == HELLO_NAME {
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
                    tables,
                )?,
                Err(cached) => cached,
            };
            Ok(Some(inode))
        } else {
            Ok(None)
        }
    }

    fn read_dir(
        inode: &INode<Self>,
        pos: &mut i64,
        emit: &mut dyn FnMut(&[u8], u64, u8) -> bool,
    ) -> Result<(), Error> {
        if inode.ino() != ROOT_INO {
            return Err(Error::new(-20)); // ENOTDIR
        }

        // Emit ".", "..", then "hello.txt".
        let entries: &[(&[u8], u64, u8)] = &[
            (b".", ROOT_INO, DT_DIR),
            (b"..", ROOT_INO, DT_DIR),
            (HELLO_NAME, HELLO_INO, DT_REG),
        ];

        let start = *pos as usize;
        for (i, &(name, ino, dt)) in entries.iter().enumerate() {
            if i < start {
                continue;
            }
            if !emit(name, ino, dt) {
                return Ok(());
            }
            *pos += 1;
        }
        Ok(())
    }

    fn read_folio(inode: &INode<Self>, folio: &mut LockedFolio<'_>) -> Result<(), Error> {
        // Get the file content.
        let data = unsafe { inode.data() };
        let content = data.content.unwrap_or(b"");

        // Get folio offset and size.
        let folio_offset = folio.pos() as usize;
        let folio_size = folio.size();

        // Zero out the folio first.
        folio.zero_out(0, folio_size)?;

        // Copy file content into the folio.
        if folio_offset < content.len() {
            let to_copy = core::cmp::min(content.len() - folio_offset, folio_size);
            let src = &content[folio_offset..folio_offset + to_copy];
            folio.write(0, src)?;
        }

        // folio_end_read is called by the trampoline — marks uptodate and unlocks.
        Ok(())
    }
}

struct RofsTestModule {
    _reg: Pin<KBox<Registration>>,
}

impl Module for RofsTestModule {
    fn init() -> Result<Self, Error> {
        pr_info!("module loaded\n");

        let mut reg = KBox::new(Registration::new_for::<RofsTest>()?, Flags::GFP_KERNEL)
            .map_err(|_| Error::new(-12))?;
        // SAFETY: KBox is heap-allocated and stable. We keep it alive in the module.
        unsafe { Pin::new_unchecked(&mut *reg).register()? };
        let pinned = KBox::into_pin(reg);

        Ok(RofsTestModule { _reg: pinned })
    }

    fn exit(&self) {
        pr_info!("module unloaded\n");
    }
}

module! {
    type: RofsTestModule,
    name: "rofs_test",
    license: "GPL",
    author: "rko",
    description: "Read-only filesystem test module",
}
