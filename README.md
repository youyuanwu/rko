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
| `rko-core` | Kernel wrappers: `Module` trait, `module!` macro, `pr_info!`, `KVec<T>`, `Error` |
| `rko-sys-gen` | Generator: kernel headers → [bnd-winmd](https://github.com/youyuanwu/bnd) → Rust FFI |

## Quick Start

Requires x86_64 Linux, rustup, and the system dependencies listed in
[CI](.github/workflows/CI.yml). Run from the repository root; rustup selects
the pinned toolchain automatically.

```sh
# Tools and kernel source
rustup component add rust-src rustfmt clippy
rustup target add x86_64-unknown-linux-musl
cargo install --locked bindgen-cli
git submodule update --init --depth 1 linux

# Configure and build the kernel
cmake -S . -B build -DENABLE_KVM=OFF
cmake --build build --target configure_linux --verbose
cmake --build build --target make_linux --parallel 4 --verbose

# Check Rust code
cargo check --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings

# Build modules and test in QEMU
cmake --build build --target all
ctest --test-dir build --verbose
```

Kernel output goes to `linux_bin/`; modules to `samples/<name>/build/<name>.ko`.
Use `--parallel 1` on memory-constrained hosts.

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
