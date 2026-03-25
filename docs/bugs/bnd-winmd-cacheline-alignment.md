# bnd-winmd: `____cacheline_aligned` structs have wrong field offsets

**Component:** [bnd-winmd](https://github.com/youyuanwu/bnd) (local fork)  
**Status:** ✅ Fixed — local bnd fork generates `_pad_0: [u8; 24]` padding

## Problem

Structs annotated with `____cacheline_aligned_in_smp` (or
`____cacheline_aligned`) in kernel headers get incorrect field offsets
in the generated Rust bindings. The alignment attribute forces the
struct to a 64-byte boundary on SMP builds, but bnd-winmd does not
model this — it generates the struct with natural alignment.

This causes kernel panics when Rust code accesses fields at the wrong
offset through the generated struct projection.

## Reproduction

`struct socket_wq` in `linux/net.h`:

```c
struct socket_wq {
    wait_queue_head_t    wait;
    struct fasync_struct *fasync_list;
    unsigned long        flags;
    struct rcu_head      rcu;
} ____cacheline_aligned_in_smp;
```

`struct socket` embeds `socket_wq` directly:

```c
struct socket {
    socket_state  state;    // 4 bytes
    short         type;     // 2 bytes
    // padding: 2 bytes
    unsigned long flags;    // 8 bytes
    struct file  *file;     // 8 bytes
    struct sock  *sk;       // 8 bytes
    const struct proto_ops *ops;  // 8 bytes
    struct socket_wq wq;   // ← aligned to 64 bytes
};
```

**Kernel layout** (x86_64 SMP):
- Fields before `wq`: 40 bytes
- `wq` starts at offset **64** (padded to cacheline)
- Total `struct socket`: 128 bytes

**Generated bindings layout**:
- Fields before `wq`: 40 bytes
- `wq` starts at offset **40** (no cacheline padding)
- Total: ~96 bytes (wrong)

Accessing `(*sock).wq.wait` in Rust reads memory at offset 40 instead
of offset 64 — producing a garbage `wait_queue_head` pointer.

## Crash Symptoms

```
BUG: unable to handle page fault for address: ffffffffffffffe8
Workqueue: events 0xffffffffa0000c40
```

The address `0xffffffffffffffe8` = `NULL - 0x18` — the garbage
`wait_queue_head` pointer is NULL-ish, and `add_wait_queue` offsets
into the `list_head` field at `-0x18`, causing a page fault.

## Workaround

Use a C helper to access the field instead of Rust struct projection:

```c
// rko-sys/src/helpers.c
struct wait_queue_head *rust_helper_sock_wq_head(struct socket *sock)
{
    return &sock->wq.wait;
}
```

```rust
// Instead of this (WRONG — offset mismatch):
let wq_head = core::ptr::addr_of_mut!((*sock).wq.wait);

// Use this (CORRECT — C helper uses real struct layout):
let wq_head = rko_sys::rko::helpers::rust_helper_sock_wq_head(sock);
```

## Affected Types

Any kernel struct using `____cacheline_aligned_in_smp` or
`____cacheline_aligned` that is embedded by value in another struct.
Known instances in the networking code:

| Struct | Attribute | Embedded in |
|--------|-----------|-------------|
| `socket_wq` | `____cacheline_aligned_in_smp` | `socket.wq` |
| `inode` | `____cacheline_aligned_in_smp` (some fields) | Various |

## Potential Fix in bnd-winmd

bnd-winmd's struct layout pass should detect `____cacheline_aligned`
attributes (clang exposes them via `aligned(64)` in the AST) and add
appropriate `#[repr(align(64))]` or padding to the generated Rust
struct. Alternatively, mark such structs as opaque and require field
access through C helpers.

The `____cacheline_aligned_in_smp` macro expands to
`__attribute__((aligned(SMP_CACHE_BYTES)))` where `SMP_CACHE_BYTES`
is typically 64 on x86_64. Clang's `-fdump-record-layouts` shows the
correct padded layout.

## References

- Kernel header: `linux/include/linux/net.h` (socket_wq definition)
- Kernel header: `linux/include/linux/cache.h` (macro definition)
- Fix commit: `rko-core/src/kasync/net/mod.rs` — replaced
  `addr_of_mut!((*sock).wq.wait)` with `rust_helper_sock_wq_head`
- Related: `docs/bugs/rofs-alignment.md` (other alignment issues)
