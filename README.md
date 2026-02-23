# rko — Rust Kernel Objects

Build out-of-tree Linux kernel modules in Rust.

Unlike the in-tree kernel Rust support which uses Kbuild as the primary build
system, rko uses **Cargo** as the primary build tool — compiling Rust crates
into a `staticlib`, then feeding the result into Kbuild only for final `.ko`
linking. CMake orchestrates the two phases.

## Crates

| Crate | Description |
|-------|-------------|
| `rko-sys` | Generated FFI bindings for kernel-internal headers (`#![no_std]`) |
| `rko-core` | Kernel wrappers: `pr_info!`, module macros, `KVec<T>` allocator |
| `rko-sys-gen` | Generator: kernel headers → [bnd-winmd](https://github.com/youyuanwu/bnd) → Rust FFI |

## Quick Start

**Prerequisites:** Linux kernel built with `CONFIG_RUST=y` and `LLVM=1`, Rust 1.93.0+ with `rust-src`, CMake, QEMU, busybox.

```sh
# Symlink kernel source and build output
ln -s /path/to/linux linux
ln -s /path/to/linux-build linux_bin

# Build all kernel modules
cmake -B build
cmake --build build

# Test in QEMU
ctest --test-dir build
```

## Samples

| Module | Description |
|--------|-------------|
| `samples/hello` | Minimal init/exit module with `pr_info!` |
| `samples/kvec_test` | KVec allocation tests (push, pop, extend, iter) |

### Adding a new sample

1. Create `samples/<name>/` with `Cargo.toml` and `<name>.rs`
2. Add `samples/<name>/CMakeLists.txt`:
   ```cmake
   add_kernel_module(CHECKS "expected dmesg output")
   ```
3. Add `add_subdirectory(samples/<name>)` to root `CMakeLists.txt`
4. Add `"<name>"` to `samples/Cargo.toml` workspace members

## Design Docs

- [Bindings](docs/design/Bindings.md) — binding generation pipeline and crate layout
- [Kbuild](docs/design/Kbuild.md) — two-phase build (cargo → Kbuild) and rustc flags
- [Build Infrastructure](docs/design/features/build-infra.md) — CMake orchestration
- [KVec](docs/design/features/kvec.md) — kernel allocator and vector implementation

## License

See [LICENSE](LICENSE).
