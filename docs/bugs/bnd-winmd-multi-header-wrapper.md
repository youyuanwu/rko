# bnd-winmd: Multi-header partitions silently extract nothing

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.4  
**Status:** ✅ Fixed (unstaged in local bnd)

## Problem

When a partition specified multiple `headers`, bnd-winmd generated a
wrapper `.c` file in `/tmp/` with `#include "..."` using relative paths.
Since clang compiled from `/tmp/`, the relative paths didn't resolve,
silently producing an empty translation unit (0 structs, 0 functions).

## Fix

The wrapper now uses angle-bracket includes (`#include <...>`) with the
original header paths, and adds `-I base_dir` to clang arguments so
headers resolve via search paths — same mechanism as single-header
partitions.

## Previous Workaround (no longer needed)

Create a single aggregate header (e.g. `sync.h`) that includes all
desired headers, and use that as the single `headers` entry.
