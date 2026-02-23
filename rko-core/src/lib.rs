//! Rust wrappers for Linux kernel APIs.
//!
//! This crate provides safe(r) abstractions on top of the raw FFI
//! bindings in `rko-sys`. Hand-written modules live here; generated
//! bindings stay in `rko-sys`.

#![no_std]

pub mod module;
pub mod printk;
