# Feature: In-Kernel Test Framework

## Status: ✅ Implemented

See `rko-macros/src/rko_tests.rs`, `rko-core/src/kunit.rs`,
`samples/kunit_tests/`.

## Usage

```rust
#[rko_core::rko_tests]
mod my_tests {
    use super::*;

    #[test]
    fn it_works() { assert_eq!(1 + 1, 2); }

    #[test]
    fn fallible() -> Result<(), Error> {
        let v = KVec::<u8>::with_capacity(16, Flags::GFP_KERNEL)?;
        assert!(v.capacity() >= 16);
        Ok(())
    }
}

// In Module::init():
my_tests::run()?;
```

The macro generates two test paths: `run()` for printk output (Phase 1)
and `.kunit_test_suites` ELF section for KUnit TAP discovery (Phase 2).
Tests return `()` or `Result<(), Error>`.

## `block_on` for async tests

`WorkqueueExecutor::block_on()` bridges async→sync via kernel
`struct completion`. Server uses async accept (yields to workqueue);
client uses blocking connect (instant on loopback).

```rust
#[test]
fn async_echo() -> Result<(), Error> {
    let handle = WorkqueueExecutor::new(workqueue::system())?;
    let exec = handle.executor_arc();
    let exec2 = exec.clone();
    let listener = TcpListener::try_new(ns, &addr)?;

    let result = exec.block_on(async move {
        exec2.as_arc_borrow().spawn(async move {
            yield_now().await;
            let stream = TcpStream::connect(ns, &addr).unwrap();
            stream.write_all(b"ping").unwrap();
        }).unwrap();
        let stream = listener.accept().await?;
        let mut buf = [0u8; 16];
        let n = stream.read(&mut buf).await?;
        Ok::<bool, Error>(&buf[..n] == b"ping")
    })?;
    assert!(result?);
    Ok(())
}
```

Requires `test.sh` with `pre` hook to bring up loopback before insmod.

## Test suites (56 tests)

| File | Suite | Tests |
|------|-------|-------|
| `tests/kvec.rs` | kvec_tests | 11 |
| `tests/error.rs` | error_tests | 7 |
| `tests/ktime.rs` | ktime_tests | 6 |
| `tests/arc.rs` | arc_tests | 5 |
| `tests/revocable.rs` | revocable_tests | 4 |
| `tests/refcount.rs` | refcount_tests | 6 |
| `tests/memcache.rs` | memcache_tests | 3 |
| `tests/completion.rs` | completion_tests | 1 |
| `tests/workqueue.rs` | workqueue_tests | 3 |
| `tests/executor.rs` | executor_tests | 6 |
| `tests/async_echo.rs` | async_echo_tests | 4 |

## Build & run

```sh
cmake --build build --target kunit_tests_ko       # build
cmake --build build --target kunit_tests_ko_test   # QEMU test
ctest --test-dir build                             # all tests
```

## Adding a new suite

1. Create `samples/kunit_tests/tests/foo.rs` with
   `#[rko_core::rko_tests] pub mod foo_tests { ... }`
2. Add `pub mod foo;` to the `mod tests` block in `kunit_tests.rs`
3. Add `tests::foo::foo_tests::run()?;` in `Module::init()`

## Design notes

- **`Completion` must be in-place initialized** — `init_completion`
  sets self-referential pointers; moving after init → page fault.
  Use `MaybeUninit` + `Completion::init(ptr)`.
- **Async accept required on single-CPU** — blocking `accept(true)`
  deadlocks the workqueue worker. Async yields back.
- **`CONFIG_KUNIT=y`** enabled in `configure_linux`. Adds only the
  framework; individual kernel test suites default to `n`.

## References

- Upstream: `linux/rust/macros/kunit.rs`, `linux/rust/kernel/kunit.rs`
- Build infra: `docs/design/features/build-infra.md`
- Networking: `docs/design/features/networking.md`
