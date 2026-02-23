# Feature: Module Macro

## Status: ✅ Phase 1 Complete

Phase 1 (simple modules) implemented. See `rko-core/src/module.rs`,
`rko-core/src/error.rs`, `rko-core/src/prelude.rs`.

## Usage

```rust
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
    description: "Hello world kernel module",
}
```

## What `module!` generates

- `.modinfo` entries (license, author, description) via `global_asm!`
- Log prefix via `set_log_prefix`
- `init_module`: calls `T::init()`, stores in `static MaybeUninit<T>`
- `cleanup_module`: calls `T::exit()`, then drops instance
- Addressability markers (`.init.data` / `.exit.data`)
- `#[panic_handler]`

## Design Decisions

- **`Module` trait** with `init()` + `exit(&self)` — explicit callbacks
  instead of `Drop` so both appear in one trait. Field `Drop` impls
  still run after `exit()`.
- **`Error`** wraps negative errno from `rko_sys::rko::err`
- **`prelude`** re-exports `Module`, `Error`, `module!`, `pr_info!`, etc.

## Phase 2: `InPlaceModule` (planned)

For modules holding pinned objects (mutexes, registrations):

```rust
pub trait InPlaceModule: Sized + Send + Sync {
    fn init() -> impl PinInit<Self, Error>;
    fn exit(&self) {}
}
```

Storage: `Pin<KBox<T>>` via [`pin-init`](https://github.com/Rust-for-Linux/pin-init)
crate (standalone, `no_std`). Needed for ROFS and device drivers.

## Future

- Module parameters (`parm=` modinfo entries)
- `module_fs!` / `module_pci!` specialized variants
- `BUG()` in panic handler
