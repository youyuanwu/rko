# bnd-winmd: `[[inject_type]]` for user-declared types

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.3  
**Status:** Implemented

## Problem

bnd-winmd cannot extract certain C type patterns:

1. **Bitfield enums** — `enum fs_value_type type:8;`
2. **Anonymous enums** — `enum { ... } type;` inside a struct
3. **Types from non-traversed headers** — embedded struct fields whose
   defining header isn't in the traverse list

## Solution

Top-level `[[inject_type]]` entries in the TOML config declare types
that are merged into partitions after clang extraction but before winmd
emission. See upstream docs:
[bnd/docs/design/InjectTypes.md](https://github.com/youyuanwu/bnd/blob/main/docs/design/InjectTypes.md)

### Usage in rko

`rko-sys-gen/rko.toml` uses ~40 inject entries to keep each partition's
traverse list minimal (one primary header). Types fall into three
categories:

- **Enums** — `fs_value_type`, `migrate_mode`, `pid_type`, `rw_hint`
  (bitfield or non-traversed enums with explicit variants)
- **Embedded structs** — `timespec64`, `xarray`, `rw_semaphore`,
  `mutex`, `lockref`, etc. (correct size/align from `clang sizeof`)
- **Pointer-only structs** — `super_block`, `task_struct`, `folio`,
  etc. (size included for completeness)

### Getting sizes

```sh
# Use clang to get sizeof/alignof (needs -include linux/kconfig.h)
clang --target=x86_64-linux-gnu -nostdinc -w \
  -Ilinux/arch/x86/include ... \
  -include linux/kconfig.h -include generated/autoconf.h \
  -D__KERNEL__ -DMODULE -D__BINDGEN__ \
  -S -emit-llvm -o - /tmp/check.c
# Parse @sz and @al from LLVM IR output
```
