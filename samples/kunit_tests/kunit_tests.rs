//! KUnit test suites for rko-core.
//!
//! This module registers test suites via the `.kunit_test_suites` ELF
//! section. When loaded with `insmod` into a CONFIG_KUNIT=y kernel,
//! KUnit discovers and runs the tests automatically, producing TAP output.

#![no_std]

use rko_core::error::Error;
use rko_core::prelude::*;

mod tests {
    pub mod arc;
    pub mod async_echo;
    pub mod completion;
    pub mod error;
    pub mod executor;
    pub mod ktime;
    pub mod kvec;
    pub mod memcache;
    pub mod refcount;
    pub mod revocable;
    pub mod workqueue;
    // unsafe_list: skipped — List::new() sentinel writes fault in module
    // rodata. Needs heap allocation or writable static to work in .ko context.
}

struct KunitTests;

impl Module for KunitTests {
    fn init() -> Result<Self, Error> {
        tests::kvec::kvec_tests::run()?;
        tests::completion::completion_tests::run()?;
        tests::error::error_tests::run()?;
        tests::ktime::ktime_tests::run()?;
        tests::arc::arc_tests::run()?;
        tests::revocable::revocable_tests::run()?;
        tests::refcount::refcount_tests::run()?;
        tests::memcache::memcache_tests::run()?;
        tests::async_echo::async_echo_tests::run()?;
        tests::workqueue::workqueue_tests::run()?;
        tests::executor::executor_tests::run()?;
        pr_info!("TEST OK\n");
        Ok(KunitTests)
    }

    fn exit(&self) {
        pr_info!("module unloaded\n");
    }
}

module! {
    type: KunitTests,
    name: "kunit_tests",
    license: "GPL",
    author: "rko",
    description: "KUnit test suites for rko-core",
}
