# Feature: In-Kernel Test Framework

## Status: ✅ Implemented (Phase 1 + Phase 2)

## Problem

Tests today are hand-rolled: each sample writes ad-hoc `pr_info!("PASS/FAIL")`
checks inside `Module::init()` (see `samples/kvec_test/kvec_test.rs`). This
leads to boilerplate, inconsistent output, and no structured test reporting.

The upstream kernel solves this with KUnit + a Rust proc-macro layer
(`#[kunit_tests]`). We want the same ergonomics for rko.

## Approach: Two-Phase

### Phase 1 — Lightweight `#[rko_tests]` (no C KUnit dependency)

Self-contained test framework using existing `printk` infrastructure.
Works immediately with the current QEMU test runner (`ctest`).

### Phase 2 — Full KUnit integration (optional, future)

Add `kunit_*` C bindings to rko-sys and wire assertions into KUnit's
reporting infrastructure. Phase 1 is designed so this is an additive
change (swap the assertion backend), not a rewrite.

---

## Phase 1 Design

### Three components

| Component | Location | Purpose |
|-----------|----------|---------|
| Proc macro | `rko-macros/src/rko_tests.rs` | Transform `#[test]` fns into runnable suite |
| Runtime | `rko-core/src/kunit.rs` | Assertion macros, test runner, result reporting |
| Test modules | `samples/*/` | Actual tests using the framework |

### User-facing API

```rust
use rko_core::prelude::*;
use rko_core::alloc::{Flags, KVec};

#[rko_tests]
mod tests {
    use super::*;

    #[test]
    fn push_and_len() {
        let mut v = KVec::new();
        v.push(10i32, Flags::GFP_KERNEL).unwrap();
        v.push(20, Flags::GFP_KERNEL).unwrap();
        v.push(30, Flags::GFP_KERNEL).unwrap();
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn pop() {
        let mut v = KVec::new();
        v.push(30i32, Flags::GFP_KERNEL).unwrap();
        assert_eq!(v.pop(), Some(30));
        assert!(v.is_empty());
    }
}

struct KvecTest;

impl Module for KvecTest {
    fn init() -> Result<Self, Error> {
        // Run all tests; returns Err on any failure.
        tests::run()?;
        Ok(KvecTest)
    }
    fn exit(&self) {}
}
```

### Proc macro: `#[rko_tests]`

Applied to an inline module. For each `#[test]` fn inside:

1. **Strip `#[test]`** attribute (won't compile with `no_std`).
2. **Shadow `assert!` / `assert_eq!`** with macros that call rko-core's
   test-aware versions (include file + line in output).
3. **Generate a `run()` function** that calls each test, catches panics
   (or checks `Result`), and emits structured `pr_info!` output.

#### Generated code sketch

```rust
// Input:
//   #[test] fn push_and_len() { assert_eq!(v.len(), 3); }

// Output:
mod tests {
    // per-test assert! override (scoped via macro_rules)
    macro_rules! assert {
        ($cond:expr $(,)?) => {
            if !$cond {
                $crate::pr_err!(
                    "  FAIL: {}, {}:{}\n",
                    stringify!($cond), file!(), line!()
                );
                return Err($crate::error::Error::EINVAL);
            }
        };
    }
    macro_rules! assert_eq {
        ($left:expr, $right:expr $(,)?) => {
            if $left != $right {
                $crate::pr_err!(
                    "  FAIL: {} != {}, {}:{}\n",
                    stringify!($left), stringify!($right), file!(), line!()
                );
                return Err($crate::error::Error::EINVAL);
            }
        };
    }

    fn push_and_len() -> Result<(), $crate::error::Error> {
        /* original body, assert! now returns Err on failure */
        Ok(())
    }

    fn pop() -> Result<(), $crate::error::Error> { /* ... */ Ok(()) }

    // --- generated runner ---
    pub fn run() -> Result<(), $crate::error::Error> {
        $crate::pr_info!("---- {} ----\n", "tests");
        let mut pass = 0u32;
        let mut fail = 0u32;

        match push_and_len() {
            Ok(()) => { pass += 1; $crate::pr_info!("  PASS: push_and_len\n"); }
            Err(_) => { fail += 1; }
        }
        match pop() {
            Ok(()) => { pass += 1; $crate::pr_info!("  PASS: pop\n"); }
            Err(_) => { fail += 1; }
        }

        $crate::pr_info!("---- {}: {} passed, {} failed ----\n", "tests", pass, fail);
        if fail > 0 { Err($crate::error::Error::EINVAL) } else { Ok(()) }
    }
}
```

### Runtime: `rko-core/src/kunit.rs`

Minimal runtime support:

```rust
/// Trait to normalize test return types.
/// `()` → Ok, `Result<T, E>` → check is_ok().
pub trait TestResult {
    fn is_test_ok(self) -> bool;
}

impl TestResult for () {
    fn is_test_ok(self) -> bool { true }
}

impl<T, E> TestResult for Result<T, E> {
    fn is_test_ok(self) -> bool { self.is_ok() }
}
```

The proc macro generates assertion macros inline (scoped per test), so
the runtime module stays small. Phase 2 would add `kunit_assert!` etc.
here.

### Output format

Designed to work with the existing QEMU test runner which greps for
`TEST OK`:

```
[  1.234] kvec_test: ---- tests ----
[  1.234] kvec_test:   PASS: push_and_len
[  1.234] kvec_test:   PASS: pop
[  1.234] kvec_test: ---- tests: 2 passed, 0 failed ----
[  1.234] kvec_test: TEST OK
```

On failure:

```
[  1.234] kvec_test: ---- tests ----
[  1.234] kvec_test:   FAIL: v.len() == 3, kvec_test.rs:12
[  1.234] kvec_test:   PASS: pop
[  1.234] kvec_test: ---- tests: 1 passed, 1 failed ----
```

The `TEST OK` sentinel is emitted by the module (not the framework) so
modules control pass criteria. The CMake `CHECKS` string in
`add_kernel_module()` continues to work unchanged.

### Test return types

Tests may return `()` or `Result<(), Error>`:

```rust
#[test]
fn unit_return() {
    assert_eq!(1, 1);  // failure → early return Err
}

#[test]
fn result_return() -> Result<(), Error> {
    let v = KVec::<u8>::with_capacity(16, Flags::GFP_KERNEL)?;
    assert!(v.capacity() >= 16);
    Ok(())
}
```

The proc macro wraps `()` returns to always produce `Result`.

---

## Phase 2 Design (KUnit Integration)

### How it works

`#[rko_tests]` generates **both** test paths in a single module:

1. **Phase 1** — `tests::run()` called from `Module::init()`, printk assertions
2. **Phase 2** — KUnit suite registered via `.kunit_test_suites` ELF section

When loaded into a `CONFIG_KUNIT=y` kernel, KUnit discovers the suites
at `insmod` time and runs them alongside the Phase 1 `run()` path.
Both produce output; the QEMU test script checks Phase 1 output
(`TEST OK`), while KUnit produces TAP format in dmesg.

### Components

| Component | Location | Purpose |
|-----------|----------|---------|
| C helpers | `rko-sys/src/helpers.{h,c}` | `kunit_get_current_test`, `kunit_mark_failed` |
| bnd partition | `rko-sys-gen/rko.toml` → `rko.kunit` | Generated types: `kunit_case`, `kunit_suite`, etc. |
| Runtime | `rko-core/src/kunit.rs` | `new_kunit_case()`, `kunit_case_null()`, `kunit_unsafe_test_suite!` |
| Proc macro | `rko-macros/src/rko_tests.rs` | Generates `kunit_rust_wrapper_*` + `KUNIT_TEST_CASES` + suite |
| Test module | `samples/kunit_tests/` | Dedicated `.ko` with `#[rko_tests]` suites |

### Generated code (per test function)

```rust
// For each #[test] fn foo():
unsafe extern "C" fn kunit_rust_wrapper_foo(_test: *mut c_void) {
    if foo().is_err() {
        unsafe { kunit_mark_failed(_test); }
    }
}

// Array + suite registration:
static mut KUNIT_TEST_CASES: [kunit_case; N+1] = [
    new_kunit_case(c"foo", kunit_rust_wrapper_foo),
    // ...
    kunit_case_null(),  // NULL terminator
];
kunit_unsafe_test_suite!(suite_name, KUNIT_TEST_CASES);
```

### Usage

```sh
cmake --build build --target kunit_tests_ko       # build module
cmake --build build --target kunit_tests_ko_test   # QEMU test
ctest --test-dir build -R kunit_tests              # or via ctest
```

KUnit TAP output in dmesg:

```
    # Subtest: kvec_tests
    1..7
    ok 1 push_and_len
    ok 2 indexing
    ...
# kvec_tests: pass:7 fail:0 skip:0 total:7
ok 7 kvec_tests
```

### Kernel config

`CONFIG_KUNIT=y` is enabled in the `configure_linux` CMake target.
KUnit adds only the framework (~small); individual kernel test suites
default to `n` and are not compiled.

---

## Implementation Checklist

### Phase 1 — ✅

- [x] `rko-macros/src/rko_tests.rs` — proc macro implementation
- [x] `rko-macros/src/lib.rs` — register `#[rko_tests]` attribute
- [x] `rko-core/src/kunit.rs` — `TestResult` trait, re-export
- [x] `rko-core/src/lib.rs` — declare `pub mod kunit`
- [x] Convert `samples/kvec_test/` to use `#[rko_tests]`

### Phase 2 — ✅

- [x] `rko-sys/src/helpers.{h,c}` — kunit C helpers
- [x] `rko-sys-gen/rko.toml` — add `rko.kunit` partition
- [x] Regenerate bindings (`cargo run -p rko-sys-gen`)
- [x] `rko-core/src/kunit.rs` — `new_kunit_case()`, `kunit_unsafe_test_suite!`
- [x] `rko-macros/src/rko_tests.rs` — KUnit wrapper + suite registration
- [x] `samples/kunit_tests/` — dedicated test module (`.ko`)
- [x] `CONFIG_KUNIT=y` in `configure_linux`

## References

- Upstream proc macro: `linux/rust/macros/kunit.rs`
- Upstream runtime: `linux/rust/kernel/kunit.rs`
- Existing test runner: `docs/design/features/qemu-test.md`
- Build infra: `docs/design/features/build-infra.md`
