//! HTTP io_uring kernel module — creates `/dev/rko_http`.
//!
//! Userspace controls the kernel HTTP server via `IORING_OP_URING_CMD`
//! custom commands. The kernel handles TCP, HTTP parsing, keep-alive,
//! and connection management. Userspace receives parsed requests and
//! sends responses through io_uring.
//!
//! See `docs/design/features/futures/io-uring-http-api.md`.

#![no_std]

use rko_core::alloc::Flags;
use rko_core::error::Error;
use rko_core::io_uring::{IoUringCmd, IssueFlags};
use rko_core::miscdevice::{MiscDevice, MiscDeviceOptions, MiscDeviceRegistration};
use rko_core::prelude::*;
use rko_core::sync::Arc;
use rko_core::types::ForeignOwnable;
use rko_util::http::uring::HttpUringHandler;

/// Per-fd state: each open of `/dev/rko_http` gets its own handler.
struct HttpDeviceState {
    handler: Arc<HttpUringHandler>,
}

#[rko_core::vtable]
impl MiscDevice for HttpDeviceState {
    type Ptr = Arc<Self>;

    fn open(_misc: &MiscDeviceRegistration<Self>) -> Result<Arc<Self>, Error> {
        let handler = HttpUringHandler::new()?;
        let handler = Arc::new(handler, Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)?;
        Arc::new(HttpDeviceState { handler }, Flags::GFP_KERNEL).map_err(|_| Error::ENOMEM)
    }

    fn uring_cmd(
        device: <Arc<Self> as ForeignOwnable>::Borrowed<'_>,
        cmd: IoUringCmd,
        flags: IssueFlags,
    ) -> i32 {
        pr_info!(
            "http_uring: uring_cmd op={} flags={:#x}\n",
            cmd.cmd_op(),
            flags.raw()
        );
        let ret = HttpUringHandler::dispatch(&device.handler, cmd, flags);
        pr_info!("http_uring: uring_cmd returning ret={}\n", ret);
        ret
    }
}

struct HttpUringModule {
    _reg: MiscDeviceRegistration<HttpDeviceState>,
}

impl Module for HttpUringModule {
    fn init() -> Result<Self, Error> {
        pr_info!("http_uring: registering /dev/rko_http\n");
        let reg = MiscDeviceRegistration::<HttpDeviceState>::register(MiscDeviceOptions {
            name: c"rko_http",
        })?;
        pr_info!("http_uring: /dev/rko_http ready\n");
        Ok(Self { _reg: reg })
    }

    fn exit(&self) {
        pr_info!("http_uring: unregistering /dev/rko_http\n");
    }
}

module! {
    type: HttpUringModule,
    name: "http_uring",
    license: "GPL",
    author: "rko",
    description: "HTTP server controlled via io_uring custom commands",
}
