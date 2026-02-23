//! Minimal "hello world" out-of-tree kernel module using rko-core.

#![no_std]

use rko_core::prelude::*;

struct Hello;

impl Module for Hello {
    fn init() -> Result<Self, Error> {
        pr_info!("module loaded\n");
        Ok(Hello)
    }

    fn exit(&self) {
        pr_info!("module unloaded\n");
    }
}

module! {
    type: Hello,
    name: "hello",
    license: "GPL",
    author: "rko",
    description: "Hello world kernel module using rko",
}
