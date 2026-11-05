# rko-sys Bindings Design

`rko-sys/build.rs` uses [bindgen](https://rust-lang.github.io/rust-bindgen/)
and libclang to generate `no_std` FFI for the configured Linux kernel.

## Pipeline

```text
configured kernel headers + helpers.h -> bindgen -> OUT_DIR/bindings.rs
```

Cargo runs generation automatically. `src/lib.rs` includes the generated file
and exposes types, functions, and constants directly as `rko_sys::inode`,
`rko_sys::EINVAL`, etc. No generated Rust or metadata files are checked in.
The former `rko::*` partitions, Cargo features, and `rko-sys-gen` crate
have been removed.

## Inputs and requirements

- x86_64 Linux, clang, libclang, and the pinned Rust toolchain.
- Kernel sources in `linux/` and configured/built output in `linux_bin/`.
  Build the kernel before checking or building the Rust workspaces.
- Override those paths with `RKO_KERNEL_SOURCE` and `RKO_KERNEL_BUILD`
  (absolute paths). CMake forwards its kernel paths to Cargo.

`src/bindings.h` includes the kernel headers and C helper declarations.
The build script supplies the kernel's source/generated include paths,
configuration and compiler headers, and `__KERNEL__`, `MODULE`, and
`__BINDGEN__` defines. Header allowlists limit the public surface; referenced
types are generated recursively.

`bindgen::CargoCallbacks` tracks included headers, including the generated
configuration, so changes trigger regeneration. Kernel path environment
variables also trigger regeneration.

## ABI and callbacks

Bindgen generates real struct/union layouts, typed `Option<unsafe extern "C"
fn(...)>` callbacks, and compile-time size, alignment, and field-offset
assertions. These assertions remain enabled for host and kernel-target builds.
Use `Some(callback)` or `None`, not casts to untyped pointers.

A few x86 types are opaque using the same workarounds as Linux's bindgen
parameters. `sched_statistics` and `ns_common` also remain opaque because of
aligned empty structs and anonymous struct members that bindgen cannot
represent correctly. Their size and alignment are computed by clang,
not manually specified.

## C helpers

Static inline functions and function-like macros do not provide linkable
symbols. Declare `rust_helper_<name>` in `src/helpers.h` and implement it in
`src/helpers.c`. Kbuild compiles helpers once and CMake links them into each
module. See [Adding Kernel Bindings](../guides/adding-bindings.md).

## Commands

```sh
cargo check --locked -p rko-sys   # generate and check bindings
cargo check --locked             # check all host libraries
cmake --build build --target all # generate and build kernel-target modules
ctest --test-dir build --verbose # exercise modules in QEMU
```

The CMake `generate_bindings` target also runs the first command.
The standalone `bindgen-cli` tool is still required for building Linux's
own Rust support, independently of the Cargo build dependency.
