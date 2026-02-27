# bnd-winmd: Enum with forward declaration not extracted by sonar

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) v0.0.4
(sonar in `clang` crate)  
**Status:** ✅ Fixed — bnd commit `0ee989f` ("Handle bitfield correctly")

## Problem

`enum fs_value_type` is defined in `linux/fs_context.h` which is in
the `rko.fs_context` partition's traverse list. Other enums from the
same header (`fs_context_purpose`, `fs_context_phase`) ARE extracted,
but `fs_value_type` is not.

## Root Cause: Forward Declaration Poisons `seen` Set

`sonar::find_enums` iterates top-level AST entities. Its `next()`
function (in the `clang` crate) does:

```rust
if entity.get_kind() == EnumDecl {
    if let Some(name) = entity.get_name() {
        if !seen.contains(&name) {
            seen.insert(name);              // ← adds to seen unconditionally
            if entity.get_child(0).is_some() {  // ← but only returns if has children
                return Some(Declaration::new(...));
            }
        }
    }
}
```

When clang encounters `enum fs_value_type type:8;` inside
`struct fs_parameter` (line 65 of `fs_context.h`), it emits a
**forward declaration** `EnumDecl` at the top level — an AST node
with the enum's name but no children (no variant constants).

The clang AST for `fs_context.h` contains:

```
EnumDecl <line:66:1, col:6> fs_value_type          ← forward decl (no children)
...
EnumDecl prev 0x... <line:51:1, line:58:1> fs_value_type  ← definition (has children)
```

Sonar processes the forward declaration first:
1. Name `fs_value_type` not in `seen` → adds it to `seen`
2. `get_child(0)` returns `None` (no children) → does NOT return it
3. When the actual definition is encountered, `fs_value_type` is
   already in `seen` → **skipped entirely**

The other two enums (`fs_context_purpose`, `fs_context_phase`) do
not have forward declarations in the AST, so they are extracted
correctly. Note that `fs_context_purpose` is also used in a
bitfield (`purpose:8`), but its use in `struct fs_context` does
not generate a separate forward declaration — likely because it
appears later in the file after its definition.

## Disproof of Bitfield Hypothesis

Initial hypothesis was that bitfield usage causes sonar to miss
the enum. This is wrong:

- `fs_context_purpose` is also used as a bitfield (`:8`) and IS
  extracted
- The bnd simple test (`BitfieldKind` in `simple.h`) proves enum
  extraction works in bitfield context when there's no forward decl

The issue is specifically the **forward declaration** that clang
emits, not the bitfield usage itself.

## Suggested Fix

In `sonar::find_enums`'s `next()` function, do NOT add the name
to `seen` when the entity has no children (forward declarations):

```rust
if entity.get_kind() == EnumDecl {
    if let Some(name) = entity.get_name() {
        if !seen.contains(&name) {
            if entity.get_child(0).is_some() {
                seen.insert(name);  // only mark seen when definition found
                return Some(Declaration::new(...));
            }
            // forward decl: do NOT add to seen
        }
    }
}
```

This allows the actual definition (which appears later in the AST
with `prev` pointer) to be discovered even if a forward declaration
was encountered first.

The same pattern likely applies to `find_structs` — forward-declared
structs (`struct foo;`) could have the same problem.

## Impact

Any enum that has a forward declaration in the clang AST before its
definition will be silently missed by sonar. In kernel headers this
happens when an enum type is referenced in a struct field before its
definition is fully processed by clang's AST ordering.

Currently affects `fs_value_type` in rko (must use `[[inject_type]]`
as workaround).

## Workaround

Use `[[inject_type]]` with `kind = "enum"` and explicit variants.

## Related

- `docs/bugs/bnd-winmd-debugging.md` — feature request for
  "silent extraction warning" would help detect this class of bug
- bnd `BitfieldLayoutNotPreserved.md` — documents the separate
  bitfield layout issue (`:8` becomes full `u32`), which is fixed
  by `flatten_bitfields()` in a pending bnd commit
