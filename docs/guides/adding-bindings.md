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
| Use a constant from a `#define` | Wrap in `enum {}` in helpers.h, or add header to traverse |
| Use an expression-based `#define` | Wrap in `enum { RKO_NAME = MACRO };` in helpers.h |
| Use a constant from an anonymous enum | Define in rko-core (bnd-winmd can't extract) |

## Adding a new partition

A partition tells bnd-winmd to parse a kernel header and extract its
types, functions, and constants into a Rust module.

### 1. Edit rko-sys-gen/rko.toml

```toml
[[partition]]
namespace = "rko.net"       # becomes rko-sys/src/rko/net/mod.rs
library = "kernel"
headers = ["linux/net.h"]   # headers clang parses
traverse = [                # headers whose types to extract
  "linux/net.h",
  "linux/socket.h",
  "linux/uio.h",
]
```

**headers** — the header(s) clang actually parses. Only types and
functions **visible to clang** after parsing these headers can be
extracted. If a type is defined in a header that isn't `#include`d
(directly or transitively) by any header in `headers`, clang never
sees it and bnd-winmd cannot extract it.

**traverse** — which of the parsed headers' types/constants to
include in the output. Types from headers NOT in traverse but
referenced by traversed types will be resolved from other partitions
or must be injected.

**Critical**: a header in `traverse` but NOT reachable from `headers`
has no effect. Always ensure traverse headers are either:
1. Listed in `headers`, OR
2. Transitively `#include`d by a header in `headers`

Example of the common mistake:

```toml
# WRONG — completion.h is not #included by wait.h
headers = ["linux/wait.h"]
traverse = ["linux/wait.h", "linux/completion.h"]  # completion types won't be extracted!

# CORRECT — add completion.h to headers too
headers = ["linux/wait.h", "linux/completion.h"]
traverse = ["linux/wait.h", "linux/completion.h"]
```

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

**Type ownership rule**: when multiple partitions traverse the same
header, the type goes to the **first partition** (in `rko.toml` order)
that traverses it. Use this to control where types live:

```toml
# mm_types partition is listed BEFORE fs partition, so folio, page,
# vm_area_struct etc. are owned by rko.mm_types, not rko.fs.
[[partition]]
namespace = "rko.mm_types"     # owns folio, page, vm_area_struct
headers = ["linux/mm_types.h"]
traverse = ["linux/mm_types.h", ...]

[[partition]]
namespace = "rko.fs"           # references super::mm_types::folio
headers = ["linux/fs.h"]
traverse = ["linux/fs.h", ...]
```

**Splitting a large partition**: if a partition like `rko.fs` is too
large (9000+ lines), create smaller partitions BEFORE it for logically
distinct header groups. Types from the earlier partitions are resolved
cross-partition by the later ones. The rko.fs partition was split into
`rko.wait`, `rko.mm_types`, `rko.cred`, `rko.ds` this way.

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

**Before adding a helper**, check if bnd-winmd already generated the
binding — some functions that look inline are actually exported:

```sh
# Check if the function already exists in generated bindings
grep 'fn inode_lock_shared' rko-sys/src/rko/fs/mod.rs

# Check if the kernel actually exports it (needed for kbuild linking)
grep 'inode_lock_shared' linux_bin/Module.symvers
```

If the function exists in the generated bindings AND in Module.symvers,
you don't need a helper — use it directly from the partition.

### When a helper IS needed

- The function is `static inline` in the kernel header (not exported)
- The function is a C macro (e.g., `dir_emit`, `container_of`)
- The function exists in bindings but is NOT in Module.symvers

### Parameter type issues

If a helper parameter uses a struct type that isn't in any partition
(e.g., `struct block_device`), use `void *` in the helper declaration
and cast inside the implementation:

```c
// helpers.h — use void* for opaque types
unsigned long long rust_helper_bdev_nr_sectors(void *bdev);

// helpers.c — cast to the real type
unsigned long long rust_helper_bdev_nr_sectors(void *bdev)
{
    return bdev_nr_sectors((struct block_device *)bdev);
}
```

This avoids needing to add the type's header to the helpers partition's
traverse list, which would cascade into more unresolved types.

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

**Preferred approach**: wrap macro constants in a named `enum` in
`helpers.h`. bnd-winmd extracts enum values as `pub const`:

```c
// rko-sys/src/helpers.h
#include <linux/kdev_t.h>
enum {
    RKO_MINORMASK = MINORMASK,   // expression-based #define
    RKO_MINORBITS = MINORBITS,   // simple #define
};
```

This generates:

```rust
// rko-sys/src/rko/helpers/mod.rs  (auto-generated)
pub const RKO_MINORMASK: u32 = 1048575u32;
pub const RKO_MINORBITS: u32 = 20u32;
```

Use the `RKO_` prefix to avoid name collisions with kernel symbols.

**Fallback**: if the enum trick doesn't work (e.g., the constant is a
non-integer type), define it directly in rko-core Rust code:

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

**Traverse header not in headers** — The most common cause of
"unresolved type" errors when splitting partitions. If a header is in
`traverse` but not reachable from `headers` via `#include`, clang
never parses it and its types are invisible. Fix: add the header to
`headers` as well.

**Type moved after partition split** — When you split a large
partition, types move to the new partition. All `rko-core` code
referencing `rko_sys::rko::OLD::type` must be updated to
`rko_sys::rko::NEW::type`. Use:

```sh
cargo check -p rko-core 2>&1 | grep 'cannot find'
```

**Injecting a type that exists in another partition** — If bnd reports
a type as unresolved but you know it's in another partition, the issue
is likely that the other partition's `headers` field doesn't include
the header defining the type. Fix the `headers` list rather than
adding an inject_type.

## Checking kernel symbol availability

Not all functions in the generated bindings are actually exported by
the kernel. The linker resolves symbols at `kbuild` time (not `cargo
check` time), so a binding can compile fine but fail during module
linking.

### Check Module.symvers

```sh
# Is the function exported?
grep 'my_function' linux_bin/Module.symvers

# List all exported symbols matching a pattern
grep 'read.*folio' linux_bin/Module.symvers
```

If a function is NOT in Module.symvers, you cannot call it from a
module. Options:

1. **Use a C helper** that wraps a different (exported) function with
   equivalent behavior
2. **Use an alternative API** — e.g., `read_cache_folio` (exported)
   instead of `read_mapping_folio` (not exported)
3. **Enable the kernel config** that exports the symbol — e.g.,
   `CONFIG_IOMAP` for iomap functions

### Diagnosing kbuild link errors

If `cmake --build . --target <name>_ko` fails with:

```
ERROR: modpost: "some_function" [my_module.ko] undefined!
```

This means `some_function` is referenced in the Rust code but not
exported by the kernel. Common causes:

| Symptom | Cause | Fix |
|---------|-------|-----|
| Function in rko-sys bindings but not in Module.symvers | bnd-winmd extracted the declaration but the kernel doesn't export it | Use an alternative API or add a C helper |
| Function in rko-core but not used by the sample | Dead code in a `pub` module still gets linked into the staticlib | Make the module private or `#[cfg]`-gate it |
| Function in a C helper but helper.c include is wrong | helpers.c compiles but the inline expands to a call to an unexported function | Check the inline's implementation for further calls |

**Tip**: After adding a new rko-sys partition, always check which of
its symbols are actually exported before using them:

```sh
# How many of the generated functions are exported?
grep 'fn ' rko-sys/src/rko/NEW_PARTITION/mod.rs \
  | sed 's/.*fn \([a-zA-Z_]*\).*/\1/' \
  | while read f; do grep -q "$f" linux_bin/Module.symvers && echo "✅ $f" || echo "❌ $f"; done \
  | head -20
```

## Wiring bindings into rko-core

After adding bindings to rko-sys, wire them into rko-core:

### 1. Enable the rko-sys feature in rko-core

If your new partition needs to be used by rko-core, add the feature
to `rko-core/Cargo.toml`:

```toml
[dependencies]
rko-sys = { path = "../rko-sys", features = ["my_partition"] }
```

In practice, rko-sys uses `default` features which include all
partitions, so this step is often unnecessary.

### 2. Import and wrap in rko-core

Create a safe wrapper module in `rko-core/src/` that imports the
raw bindings and provides a safe Rust API:

```rust
use rko_sys::rko::my_partition as bindings;
```

### 3. Verify the full stack

```sh
cargo check -p rko-sys                      # bindings compile
cargo check -p rko-core                     # safe wrappers compile
cd samples && cargo check -p my_sample      # sample compiles
cd build && cmake --build . --target my_sample_ko  # kbuild links
```

The last step is critical — it's the only one that checks symbol
availability against the actual kernel.
