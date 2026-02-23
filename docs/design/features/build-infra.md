# Feature: Unified Build & Test Infrastructure

## Status: ✅ Implemented

See `cmake/kernel_module.cmake`, `scripts/run-module-test.sh`,
`CMakeLists.txt`, and per-sample `CMakeLists.txt` files.

## Design

CMake is the sole build orchestrator. No per-sample Makefiles.

### Two workspaces

| Workspace | Members | Panic | Purpose |
|-----------|---------|-------|---------|
| Root (`Cargo.toml`) | rko-sys, rko-sys-gen, rko-core | unwind | Host libs + generator |
| Samples (`samples/Cargo.toml`) | hello, kvec_test | abort | Kernel staticlib modules |

Separate workspaces because `staticlib` + `no_std` needs `panic = "abort"`,
which can't be set per-crate. `build-std` workspace-wide also fails
(duplicate `core` lang items with `std`-using crates).

Both use `[workspace.dependencies]` to centralize versions.

### `add_kernel_module()` CMake function

Handles per sample: Kbuild generation (configure time), cargo build,
`ld --whole-archive`, Kbuild modules, QEMU test, clean.

### Adding a new sample

```
samples/new_module/
├── Cargo.toml            # name + rko-core.workspace = true
├── CMakeLists.txt        # add_kernel_module(CHECKS "expected output")
└── new_module.rs
```

Plus: add to `samples/Cargo.toml` members, add `add_subdirectory` in
root `CMakeLists.txt`.

### Build commands

```sh
cmake -B build [-DENABLE_KVM=OFF]
cmake --build build                          # all modules
cmake --build build --target hello_ko        # one module
ctest --test-dir build                       # test all
cmake --build build --target hello_ko_clean  # clean one
```
