# Adding Kernel Bindings

How to expose new Linux kernel types and functions to Rust through
the rko-sys binding pipeline.

## Overview

```
rko-sys-gen/rko.toml   ─→  cargo run -p rko-sys-gen  ─→  rko-sys/src/rko/*/mod.rs
    (config)                    (generator)                   (Rust FFI)
```

The pipeline has three parts:

1. **Partitions** — tell bnd-winmd which kernel headers to parse
2. **Inject types** — declare opaque types that bnd-winmd can't extract
3. **C helpers** — wrap inline functions/macros into linkable symbols

## Quick reference

| I need to… | Do this |
|---|---|
| Use a kernel function | Add its header to an existing or new partition |
| Use a kernel struct's fields | Add its header to traverse list |
| Use a kernel struct as a pointer only | It's already `*mut c_void` or inject it |
| Call an inline function or macro | Add a C helper |
| Use a constant from a `#define` | Add header to traverse, or define in rko-core |
| Use a constant from an anonymous enum | Define in rko-core (bnd-winmd can't extract) |

## Adding a new partition

A partition tells bnd-winmd to parse a kernel header and extract its
types, functions, and constants into a Rust module.

### 1. Edit rko-sys-gen/rko.toml

```toml
[[partition]]
namespace = "rko.net"       # becomes rko-sys/src/rko/net/mod.rs
library = "kernel"
headers = ["linux/net.h"]   # entry point header
traverse = [                # headers whose types to extract
  "linux/net.h",
  "linux/socket.h",
  "linux/uio.h",
]
```

**headers** — the header(s) clang parses. Functions declared here are
extracted.

**traverse** — which headers' types/constants to include in output.
Start minimal, expand as needed. Types from headers NOT in traverse
but referenced by traversed types will be resolved from other
partitions or must be injected.

### 2. Run the generator

```sh
cargo run -p rko-sys-gen
```

If it fails with `unresolved type reference(s)`, you have three
options for each missing type:

| Option | When to use |
|---|---|
| Add header to `traverse` | The type is in a small, self-contained header |
| Add `[[inject_type]]` | The type cascades too deeply or is pointer-only |
| Do nothing | The type is already defined in another partition (bnd-winmd resolves it cross-partition) |

### 3. Add the Cargo feature

In `rko-sys/Cargo.toml`:

```toml
[features]
net = ["Foundation", "types", "sync"]   # list partitions this one references
```

Add to `default` if it should be compiled by default. Feature
dependencies should match the `super::` cross-partition references
in the generated code. Check with:

```sh
grep 'super::' rko-sys/src/rko/<name>/mod.rs | grep -oE 'super::[a-z_]+::' | sort -u
```

### 4. Verify

```sh
cargo run -p rko-sys-gen    # generate
cargo check -p rko-sys      # compile check
```

## Cross-partition type resolution

Types defined in one partition are automatically available to others.
bnd-winmd writes all partitions into a single `.winmd` file, so
windows-bindgen resolves references across partition boundaries.

In generated code this appears as `super::fs::file`,
`super::types::size_t`, etc.

**This means**: if a type like `task_struct`, `folio`, or `file` is
already defined (or injected) in another partition like `rko.fs`, you
do NOT need to inject it again in your new partition. bnd-winmd
will resolve it automatically.

Check existing partitions before adding inject_types:

```sh
# What types does rko.fs already provide?
grep 'pub struct' rko-sys/src/rko/fs/mod.rs | head -20

# What inject_types exist?
grep 'name = ' rko-sys-gen/rko.toml | grep inject_type -A1
```

## Adding inject_types

When bnd-winmd encounters a type it can't resolve from any partition's
traverse list, you must declare it manually.

### When to inject

- The type's defining header cascades into too many dependencies
  (e.g., `linux/sched.h` → `task_struct` → hundreds of types)
- The type uses macros or anonymous enums that bnd-winmd can't parse
  (e.g., `kuid_t`, `seqcount_spinlock`)
- The type is a forward declaration with no header-accessible definition
  (e.g., `mnt_idmap`)

### Getting the size and alignment

```sh
cd linux
cat > /tmp/sizes.c << 'EOF'
#include <linux/kconfig.h>
#include <linux/THE_HEADER.h>

#define EMIT(t) \
  _Static_assert(sizeof(struct t) > 0, "sz_" #t); \
  enum { sz_##t = sizeof(struct t), al_##t = _Alignof(struct t) };

EMIT(your_type)
EOF

clang -Xclang -fdump-record-layouts -fsyntax-only \
  -include linux/kconfig.h \
  -I arch/x86/include -I ../linux_bin/arch/x86/include/generated \
  -I include -I ../linux_bin/include \
  -I arch/x86/include/uapi -I ../linux_bin/arch/x86/include/generated/uapi \
  -I include/uapi -I ../linux_bin/include/generated/uapi \
  --target=x86_64-linux-gnu -nostdinc \
  -include ../linux_bin/include/generated/autoconf.h \
  -D__KERNEL__ -DMODULE -D__BINDGEN__ \
  -fno-builtin -fno-PIE -fno-strict-aliasing -fno-common \
  -fms-extensions -std=gnu11 -Wno-everything \
  /tmp/sizes.c 2>&1 | awk '
/^\*\*\* Dumping AST Record Layout/ { getline; name=$NF }
/sizeof=/ {
  match($0, /sizeof=([0-9]+)/, sz)
  match($0, /align=([0-9]+)/, al)
  if (name == "your_type") printf "size=%s align=%s\n", sz[1], al[1]
}'
```

### Adding the entry

```toml
[[inject_type]]
namespace = "rko.net"      # which partition owns this type
name = "sock"              # struct name in C
kind = "struct"
size = 728                 # from sizeof
align = 8                  # from alignof
```

**Size rules**:
- If the type is embedded **by value** in another struct → use real size
- If the type is only used as **`*mut T`** → size=8 is fine (pointer)
- When unsure, use real size (safer)

### Verifying inject_types are necessary

After generation, check if an inject_type is actually referenced:

```sh
# Count real references (excluding the struct def and Default impl)
grep '\byour_type\b' rko-sys/src/rko/net/mod.rs \
  | grep -v 'pub struct\|impl Default\|fn default\|unsafe' \
  | wc -l
```

If the count is 0, the inject_type is dead — remove it.

## Adding C helpers

Kernel inline functions and macros can't be called directly from Rust.
Wrap them in C helper functions.

### 1. Declare in helpers.h

```c
// rko-sys/src/helpers.h
#include <linux/net.h>

void rust_helper_get_net(struct net *net);
void rust_helper_put_net(struct net *net);
```

Naming convention: `rust_helper_<kernel_function_name>`.

### 2. Implement in helpers.c

```c
// rko-sys/src/helpers.c
#include <net/net_namespace.h>
#include "helpers.h"

void rust_helper_get_net(struct net *net)
{
    get_net(net);
}

void rust_helper_put_net(struct net *net)
{
    put_net(net);
}
```

### 3. Regenerate

```sh
cargo run -p rko-sys-gen
```

The helpers partition (`rko.helpers`) parses `helpers.h`, so your new
declarations appear as `rust_helper_get_net` in the generated
`rko-sys/src/rko/helpers/mod.rs`.

### 4. Verify the helper compiles

The helper C code is compiled by kbuild during `cmake --build build`.
Check for errors there, not during `cargo check`.

## Adding constants that bnd-winmd can't extract

Some kernel constants are defined as macros or anonymous enums:

```c
#define IPPROTO_TCP  6          // macro — invisible to bnd-winmd
enum { AF_INET = 2, ... };     // may be extracted if not anonymous
#define PF_INET  AF_INET        // alias macro — invisible
```

Define these directly in rko-core Rust code:

```rust
// rko-core/src/net/mod.rs
pub const IPPROTO_TCP: i32 = 6;
pub const PF_INET: i32 = super::bindings::AF_INET;  // if AF_INET was extracted
```

Check what WAS extracted:

```sh
grep 'pub const' rko-sys/src/rko/net/mod.rs | grep -i 'AF_INET\|SOCK_'
```

## Worked example: networking partition

The networking partition (`rko.net`) was added with these results:

| Metric | Value |
|---|---|
| Headers traversed | 8 (`linux/net.h`, `linux/socket.h`, `linux/uio.h`, + UAPI) |
| Functions extracted | 124 (including `sock_create_kern`, `kernel_bind`, `kernel_accept`, …) |
| Structs extracted | 36 (including `socket`, `socket_wq`, `msghdr`, `kvec`, `iov_iter`, …) |
| Constants extracted | 184 (including `AF_INET`, `SOCK_STREAM`, `SOMAXCONN`, `MSG_DONTWAIT`, …) |
| Inject types needed | 2 (`old_timespec32`, `__kernel_timespec` — only used by syscall wrappers) |
| Cross-partition refs | `super::fs::file`, `super::fs::fasync_struct`, `super::types::size_t`, … |

Types like `file`, `folio`, `task_struct` resolved automatically from
the `rko.fs` partition — no inject_types needed.

## Common pitfalls

**Adding too many traverse headers** — Each header can cascade into
dozens of types. Start with the minimum and add one at a time.

**Injecting types that already exist** — Always check if the type is
already defined or injected in another partition. Cross-partition
resolution handles it automatically.

**Wrong inject_type size** — If a type is embedded by value and the
size is wrong, you get memory corruption at runtime with no compile
error. Always verify sizes with clang.

**Forgetting the Cargo feature** — The generated module won't compile
without a matching feature in `rko-sys/Cargo.toml`. Check which
`super::` partitions it references and add them as feature deps.

**Missing C helper include** — `helpers.h` must `#include` the header
that declares the types used in helper signatures. `helpers.c` must
`#include` the header that defines the inline function being wrapped.
