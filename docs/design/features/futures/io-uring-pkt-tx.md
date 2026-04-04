# Feature: io_uring Packet TX/RX — Raw Ethernet via io_uring_cmd

**Status**: 📋 Design — not yet implemented.  
**Depends on**: [io_uring_cmd](io-uring-cmd.md), [Raw Ethernet TX/RX](raw-ethernet.md)

## Goal

Enable userspace applications to **send and receive** raw Ethernet
frames through a kernel module at high throughput and low latency,
using io_uring custom commands as the interface. The kernel module:

- **TX**: Receives packet data via `IORING_OP_URING_CMD`, constructs an
  `sk_buff`, and transmits via `dev_queue_xmit` — bypassing TCP/UDP/IP.
- **RX**: Hooks into the kernel receive path via `dev_add_pack`,
  captures raw frames, and delivers them to userspace as multishot
  CQEs — one CQE per received packet, zero syscalls per packet.

This combines the two existing designs:
- **io-uring-cmd.md** — io_uring custom command framework (submission path)
- **raw-ethernet.md** — sk_buff + dev_queue_xmit / dev_add_pack (TX/RX path)

## Why io_uring?

Traditional approaches to raw packet TX from userspace:

| Approach | Throughput | Latency | Copies | Batching |
|----------|-----------|---------|--------|----------|
| `AF_PACKET` / raw socket | Moderate | High (syscall per packet) | 1-2 | No |
| `AF_PACKET` + `PACKET_MMAP` | Good | Medium | 1 (shared ring) | Yes |
| `AF_XDP` | Excellent | Low | 0 (zero-copy) | Yes |
| **io_uring_cmd → .ko** | Excellent | Low | 0-1 | Yes (SQ batching) |

io_uring_cmd provides:
- **Batched submission**: hundreds of packets per `io_uring_enter()` syscall
- **Async completion**: CQE tells userspace when each packet was consumed
- **Fixed buffers**: pre-registered memory for zero-copy into kernel
- **Custom protocol logic**: the .ko can do arbitrary processing
  (encryption, encapsulation, filtering) before TX — unlike AF_XDP which
  is limited to eBPF
- **No BPF verifier**: full Rust/C logic in the module, not constrained
  by BPF safety model

## Architecture

```
┌──────────────────────────────────────────────────────┐
│                    Userspace                          │
│                                                      │
│  io_uring ring (IORING_SETUP_SQE128)                 │
│                                                      │
│  ┌─ SQE ──────────────────────────────────────────┐  │
│  │ opcode = IORING_OP_URING_CMD                   │  │
│  │ fd     = /dev/rko_pkt                          │  │
│  │ cmd_op = PKT_OP_TX                             │  │
│  │ addr   = pointer to packet buffer ──────────┐  │  │
│  │ len    = packet length                      │  │  │
│  │ cmd[]  = TxCmd { ifindex, flags }           │  │  │
│  └─────────────────────────────────────────────│──┘  │
│                                                │      │
│  packet buffer: [eth_hdr | payload ...]  ◄─────┘      │
│                                                      │
└──────────────────────┬───────────────────────────────┘
                       │ io_uring_enter()
                       ▼
┌──────────────────────────────────────────────────────┐
│              io_uring core (kernel)                   │
│                                                      │
│  file->f_op->uring_cmd(io_uring_cmd, issue_flags)    │
└──────────────────────┬───────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────┐
│          rko kernel module (.ko)                      │
│                                                      │
│  1. Parse TxCmd from sqe->cmd[]                      │
│  2. Get packet data from sqe->addr/len               │
│     ├─ copy_from_user  (standard path)               │
│     └─ import_fixed    (zero-copy, registered bufs)  │
│  3. Allocate sk_buff                                 │
│  4. Copy/map packet data into skb                    │
│  5. Set skb->dev, skb->protocol                      │
│  6. dev_queue_xmit(skb)  or  dev_direct_xmit(skb)   │
│  7. io_uring_cmd_done(cmd, result, issue_flags)      │
│                                                      │
└──────────────────────┬───────────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────────┐
│              NIC driver → TX ring → wire              │
└──────────────────────────────────────────────────────┘
```

## Data Flow: SQE Fields

The `io_uring_sqe` carries both command metadata and the packet buffer
reference in different fields:

```
io_uring_sqe (128 bytes with IORING_SETUP_SQE128)
┌────────────────────────────────────────────────────┐
│ opcode (u8)  = IORING_OP_URING_CMD                 │
│ fd (s32)     = fd of /dev/rko_pkt                  │
│ cmd_op (u32) = PKT_OP_TX / PKT_OP_TX_BATCH / ...  │
│                                                    │
│ addr (u64)   = userspace pointer to packet data ───┼──► raw frame bytes
│ len (u32)    = length of packet data               │
│                                                    │
│ uring_cmd_flags (u32) = 0 or IOSQE_FIXED_FILE etc  │
│ buf_index (u16) = index if using fixed buffers     │
│ user_data (u64) = opaque tag returned in CQE       │
│                                                    │
│ cmd[0..80]   = TxCmd struct (command metadata) ────┼──► ifindex, flags, etc.
└────────────────────────────────────────────────────┘
```

**Key distinction**:
- `sqe->cmd[]` (80 bytes) → small, inline command descriptor parsed via
  `io_uring_sqe_cmd(sqe, struct TxCmd)`. Contains routing metadata.
- `sqe->addr` + `sqe->len` → the actual packet payload in userspace
  memory, read via `copy_from_user()` or `io_uring_cmd_import_fixed()`.

## Command Protocol

### Shared header for UAPI (kernel + userspace)

```c
/* uapi/rko_pkt.h — shared between userspace and kernel module */

#define RKO_PKT_MAGIC  0x524B4F50  /* "RKOP" */

/* Command opcodes (set in sqe->cmd_op) */
enum rko_pkt_op {
    PKT_OP_TX          = 0,  /* Transmit one raw frame */
    PKT_OP_TX_DIRECT   = 1,  /* Transmit bypassing qdisc */
    PKT_OP_SET_DEV     = 2,  /* Bind to a network device */
    PKT_OP_GET_INFO    = 3,  /* Query device info (MAC, MTU) */
};

/* Flags for PKT_OP_TX */
#define PKT_TX_F_NO_CSUM    (1 << 0)  /* Skip checksum offload */
#define PKT_TX_F_FLUSH      (1 << 1)  /* Hint: flush TX queue */

/* Command payload in sqe->cmd[] for PKT_OP_TX */
struct rko_tx_cmd {
    __u32 ifindex;         /* target interface index (0 = default) */
    __u32 flags;           /* PKT_TX_F_* flags */
    __u16 queue_hint;      /* TX queue for direct xmit (PKT_OP_TX_DIRECT) */
    __u16 reserved[3];
};
/* static_assert(sizeof(struct rko_tx_cmd) <= 80) */

/* Command payload for PKT_OP_SET_DEV */
struct rko_set_dev_cmd {
    char ifname[16];       /* interface name (e.g., "eth0") */
};

/* CQE result:
 * cqe->res >= 0  → bytes transmitted
 * cqe->res < 0   → -errno
 */
```

### Why `ifindex` in the command, not the fd?

A single device fd (`/dev/rko_pkt`) can route packets to different
network interfaces per-command. This avoids opening multiple fds
and lets the io_uring ring target any interface dynamically. The
module can also have a default interface set via `PKT_OP_SET_DEV`.

## Kernel Module Implementation

### Misc device registration

The module registers a `miscdevice` (auto-assigned minor) exposing
`/dev/rko_pkt` with `file_operations.uring_cmd` wired:

```rust
use rko_core::io_uring::{self, IoUringCmd, IssueFlags};
use rko_core::rawnet::{SkBuff, NetDevice};

struct PktDevice {
    default_dev: Option<NetDevice>,
}

#[rko_core::vtable]
impl io_uring::Operations for PktDevice {
    fn uring_cmd(cmd: IoUringCmd, flags: IssueFlags) -> Result<(), Error> {
        match cmd.cmd_op() {
            PKT_OP_TX => Self::handle_tx(cmd, flags, false),
            PKT_OP_TX_DIRECT => Self::handle_tx(cmd, flags, true),
            PKT_OP_SET_DEV => Self::handle_set_dev(cmd, flags),
            _ => {
                cmd.done(Error::EINVAL.to_errno(), flags);
                Ok(())
            }
        }
    }
}
```

### TX handler — the core path

```rust
impl PktDevice {
    fn handle_tx(
        cmd: IoUringCmd,
        flags: IssueFlags,
        direct: bool,
    ) -> Result<(), Error> {
        // 1. Parse command metadata from sqe->cmd[]
        let tx_cmd = unsafe { cmd.cmd_data::<RkoTxCmd>()? };

        // 2. Resolve target device
        let dev = if tx_cmd.ifindex != 0 {
            NetDevice::get_by_index(tx_cmd.ifindex)?
        } else {
            Self::default_device()?
        };

        // 3. Get packet data from sqe->addr/len
        let (user_addr, pkt_len) = cmd.user_buffer()?;

        // 4. Allocate sk_buff
        let mut skb = SkBuff::alloc_for_device(&dev, pkt_len)?;
        skb.reserve(NET_IP_ALIGN);

        // 5. Copy packet data from userspace into skb
        let dst = skb.put(pkt_len);
        cmd.copy_from_user(dst, user_addr, pkt_len)?;
        // — OR for zero-copy with fixed buffers: —
        // cmd.import_fixed_into_skb(&mut skb, flags)?;

        // 6. Set device and parse ethernet header
        skb.set_device(&dev);
        skb.eth_type_trans(&dev); // sets skb->protocol from ethhdr

        // 7. Transmit
        let result = if direct {
            skb.send_direct(tx_cmd.queue_hint)
        } else {
            skb.send()
        };

        // 8. Complete the io_uring command
        let ret = match result {
            Ok(()) => pkt_len as i32,
            Err(e) => e.to_errno(),
        };
        cmd.done(ret, flags);
        Ok(())
    }
}
```

### IoUringCmd extensions for buffer access

The `IoUringCmd` wrapper (from `io-uring-cmd.md`) needs methods to
access `sqe->addr`/`sqe->len`:

```rust
impl IoUringCmd {
    /// Get the user buffer address and length from sqe->addr/sqe->len.
    pub fn user_buffer(&self) -> Result<(u64, usize), Error> {
        let addr = unsafe { helpers::rust_helper_io_uring_sqe_addr(self.cmd) };
        let len = unsafe { helpers::rust_helper_io_uring_sqe_len(self.cmd) };
        if addr == 0 || len == 0 {
            return Err(Error::EINVAL);
        }
        Ok((addr, len as usize))
    }

    /// Copy `len` bytes from userspace address into `dst`.
    pub fn copy_from_user(
        &self,
        dst: &mut [u8],
        user_addr: u64,
        len: usize,
    ) -> Result<(), Error> {
        // SAFETY: dst is valid for len bytes, user_addr is validated
        // by copy_from_user which handles faults.
        let ret = unsafe {
            helpers::rust_helper_copy_from_user(
                dst.as_mut_ptr().cast(),
                user_addr as *const core::ffi::c_void,
                len,
            )
        };
        if ret != 0 { Err(Error::EFAULT) } else { Ok(()) }
    }

    /// Import a pre-registered fixed buffer into an iov_iter.
    /// Zero-copy path — the buffer is already pinned and mapped.
    pub fn import_fixed(
        &self,
        len: usize,
        direction: RwDirection,
        issue_flags: IssueFlags,
    ) -> Result<IoVecIter, Error> {
        let mut iter = core::mem::MaybeUninit::uninit();
        let ret = unsafe {
            helpers::io_uring_cmd_import_fixed(
                /* addr, len, rw, iter, cmd, issue_flags */
            )
        };
        if ret < 0 { Err(Error::from_errno(ret)) }
        else { Ok(IoVecIter::from_raw(iter.assume_init())) }
    }
}
```

### C helpers for SQE field access

```c
// helpers.h additions
__u64 rust_helper_io_uring_sqe_addr(struct io_uring_cmd *cmd);
__u32 rust_helper_io_uring_sqe_len(struct io_uring_cmd *cmd);
__u16 rust_helper_io_uring_sqe_buf_index(struct io_uring_cmd *cmd);

// helpers.c
__u64 rust_helper_io_uring_sqe_addr(struct io_uring_cmd *cmd)
{
    return cmd->sqe->addr;
}

__u32 rust_helper_io_uring_sqe_len(struct io_uring_cmd *cmd)
{
    return cmd->sqe->len;
}

__u16 rust_helper_io_uring_sqe_buf_index(struct io_uring_cmd *cmd)
{
    return cmd->sqe->buf_index;
}
```

## Userspace API

### Simple: send one packet

```c
#include <liburing.h>
#include "rko_pkt.h"

int fd = open("/dev/rko_pkt", O_RDWR);

/* Build a raw Ethernet frame */
uint8_t frame[64];
struct ethhdr *eth = (struct ethhdr *)frame;
memcpy(eth->h_dest, dst_mac, 6);
memcpy(eth->h_source, src_mac, 6);
eth->h_proto = htons(0x88B5);
memcpy(frame + 14, "Hello", 5);

/* Prepare SQE */
struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
sqe->opcode = IORING_OP_URING_CMD;
sqe->fd = fd;
sqe->cmd_op = PKT_OP_TX;
sqe->addr = (uintptr_t)frame;   /* packet data */
sqe->len = 64;                   /* packet length */

struct rko_tx_cmd tx = { .ifindex = 2, .flags = 0 };
memcpy(sqe->cmd, &tx, sizeof(tx));

io_uring_submit(&ring);
```

### High-throughput: batched TX with fixed buffers

```c
/* Pre-register a large buffer pool */
#define POOL_SIZE (4096 * 256)   /* 1MB, 256 × 4KB frames */
void *pool = mmap(NULL, POOL_SIZE, PROT_READ | PROT_WRITE,
                  MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
struct iovec iov = { .iov_base = pool, .iov_len = POOL_SIZE };
io_uring_register_buffers(&ring, &iov, 1);

/* Submit a burst of 64 packets */
for (int i = 0; i < 64; i++) {
    uint8_t *frame = pool + i * 4096;
    build_packet(frame, i);  /* fill Ethernet frame */

    struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
    sqe->opcode = IORING_OP_URING_CMD;
    sqe->fd = fd;
    sqe->cmd_op = PKT_OP_TX;
    sqe->addr = (uintptr_t)frame;
    sqe->len = packet_size;
    sqe->buf_index = 0;  /* fixed buffer group 0 */
    sqe->flags |= IOSQE_FIXED_FILE;

    struct rko_tx_cmd tx = { .ifindex = 2 };
    memcpy(sqe->cmd, &tx, sizeof(tx));
}

/* Single syscall sends all 64 packets */
io_uring_submit(&ring);

/* Reap completions */
struct io_uring_cqe *cqe;
for (int i = 0; i < 64; i++) {
    io_uring_wait_cqe(&ring, &cqe);
    if (cqe->res < 0)
        fprintf(stderr, "TX failed: %s\n", strerror(-cqe->res));
    io_uring_cqe_seen(&ring, cqe);
}
```

## Performance Model

### Amortized costs vs `sendto()` (AF_PACKET)

| Operation | `sendto()` (AF_PACKET) | io_uring_cmd batch (this design) |
|-----------|------------------------|----------------------------------|
| Syscalls per 64 pkts | 64 | 1 (`io_uring_enter`) |
| Context switches | 64 | 1 |
| Buffer copies | 1 per pkt (user→kernel) | 0 with fixed bufs, 1 without |
| Completion notification | Synchronous (blocks) | Async (CQE ring) |
| Kernel lock contention | Per-syscall | Amortized across batch |

### Expected throughput range

Based on io_uring packet injection benchmarks and NVMe passthrough
performance data:
- **Copy path**: 1-5 Mpps (depending on frame size and CPU)
- **Fixed buffer path**: 5-15 Mpps (zero-copy, pre-pinned memory)
- **Bottleneck**: `dev_queue_xmit` + qdisc at high rates; direct
  xmit (`dev_direct_xmit`) removes this bottleneck

## Comparison with AF_PACKET

The table above compares against naive `sendto()` on AF_PACKET, which
is a strawman. The real competitor is **AF_PACKET with its own fast
paths** — `TPACKET_V3` mmap rings and `AF_PACKET + io_uring`. This
section gives an honest assessment.

### TX: Both paths converge on `dev_queue_xmit`

```
AF_PACKET + io_uring (IORING_OP_SEND):
  io_uring_enter → IORING_OP_SEND → packet_sendmsg() →
    sock alloc + sk_filter + skb build + dev_queue_xmit

Custom uring_cmd (this design):
  io_uring_enter → IORING_OP_URING_CMD → module.uring_cmd() →
    alloc_skb + copy_from_user + dev_queue_xmit
```

**Identical**: syscall batching (both use io_uring), final TX path
(`dev_queue_xmit`), one `copy_from_user`, one `alloc_skb`.

**Custom skips**: socket lookup, `sk_filter` (BPF), socket buffer
accounting (`sk_wmem_alloc`), `packet_sendmsg` protocol handling,
`sock_alloc_send_skb` overhead. This saves roughly **50-200 ns per
packet** — measurable above 1 Mpps but not transformative.

**Estimated TX advantage: ~5-10%** at high packet rates.

### RX: TPACKET_V3 is already very fast

The key insight is that `AF_PACKET + TPACKET_V3` already provides
its own zero-syscall mmap ring for RX — it doesn't need io_uring:

```
AF_PACKET + TPACKET_V3 (best existing path):
  NAPI → tpacket_rcv() → copy skb into mmap'd ring slot →
    flip ring status word
  Userspace: poll() or busy-poll the ring directly

AF_PACKET + io_uring (IORING_OP_RECV, without TPACKET):
  NAPI → packet_rcv() → socket recv queue → io_uring poll →
    copy_to_user → CQE
  (Slower — socket queue adds overhead)

Custom uring_cmd multishot (this design):
  NAPI → dev_add_pack handler → buffer_select() →
    copy skb into provided buffer → mshot_post_cqe() → CQE
```

The custom multishot path eliminates the socket recv queue and
socket wakeup mechanism, but replaces them with `buffer_select()`
+ `mshot_post_cqe()` — similar cost. The copy from `skb->data` to
the user buffer is the same in both paths.

**TPACKET_V3** is extremely mature and optimized (`tcpdump`,
`Suricata`, `libpcap` all use it). The mmap ring avoids even the
io_uring CQE posting overhead.

**Estimated RX advantage over TPACKET_V3: negligible to ~15%.**
Against `AF_PACKET + io_uring RECV` (without TPACKET): ~20-30%.

### Honest throughput comparison

| Metric | AF_PACKET + TPACKET_V3 | AF_PACKET + io_uring | Custom uring_cmd |
|--------|:---:|:---:|:---:|
| **TX throughput** | Baseline | ~Same | ~5-10% better |
| **RX throughput** | Fastest (mmap ring) | Slower (socket queue) | ~Same as TPACKET |
| **TX copies** | 1 | 1 | 0 (fixed bufs) or 1 |
| **RX copies** | 1 (skb → mmap) | 1 (skb → user buf) | 1 (skb → provided buf) |
| **Kernel logic** | ❌ | ❌ | ✅ full Rust |
| **Complexity** | Low (no .ko) | Medium | High (custom .ko) |
| **Bidirectional on 1 fd** | ❌ (2 sockets) | ❌ (2 sockets) | ✅ |
| **Mixed with custom cmds** | ❌ | ❌ | ✅ |

### Where this design actually wins

The performance advantage over AF_PACKET is modest for pure packet
forwarding. The real value of the custom uring_cmd path is
**programmability** — the ability to run arbitrary Rust logic in
the kernel on every packet:

| Capability | AF_PACKET | cBPF/eBPF | Custom .ko |
|---|:---:|:---:|:---:|
| Encrypt/decrypt in kernel | ❌ | ❌ | ✅ |
| Custom protocol parse + respond | ❌ | Limited | ✅ |
| Stateful processing (connection tracking) | ❌ | Limited (maps) | ✅ |
| Modify/encapsulate before TX | ❌ | Limited | ✅ |
| Kernel-side state machine (handshake) | ❌ | ❌ | ✅ |
| Arbitrary memory allocation | ❌ | ❌ | ✅ |
| Mix packet I/O + device commands | ❌ | ❌ | ✅ |
| Full Rust type safety + error handling | ❌ | ❌ | ✅ |

### When to use which

| Use case | Recommended approach |
|----------|---------------------|
| Packet capture / monitoring | AF_PACKET + TPACKET_V3 |
| Simple raw frame injection | AF_PACKET + io_uring |
| Maximum throughput forwarding | AF_XDP (true zero-copy) |
| Custom protocol in kernel | **This design** (uring_cmd + .ko) |
| Kernel-side encrypt/transform | **This design** |
| Mixed control + data plane | **This design** |
| No kernel module allowed | AF_PACKET or AF_XDP |

### What about AF_XDP?

AF_XDP with XDP zero-copy is strictly faster than all of the above
for pure forwarding — it eliminates `skb` allocation entirely and
can achieve 14+ Mpps (line rate at 10 GbE). However:

- Requires NIC driver support for XDP zero-copy
- Processing logic must be eBPF (limited, no loops pre-6.x, no
  arbitrary allocation, BPF verifier constraints)
- Cannot run complex Rust logic
- Not available from a standard kernel module

AF_XDP is the right choice for simple high-speed packet processing.
This design fills the gap when you need **complex kernel-side logic**
with good (but not maximum) performance.

## Design Decisions

### Misc device, not char device or netdevice

**Decision**: Register as `miscdevice` at `/dev/rko_pkt`.

**Rationale**: Misc devices auto-assign minors, need minimal
registration code, and appear as regular files — perfect for
io_uring which operates on fds. A `netdevice` would require
implementing the full `net_device_ops` interface (overkill). A
raw `cdev` requires manual major/minor management.

### `copy_from_user` as default, fixed buffers as opt-in

**Decision**: The standard path copies packet data from userspace
(`sqe->addr/len` → `copy_from_user` → `skb`). Fixed buffers
(`io_uring_cmd_import_fixed`) are an optimization for high throughput.

**Rationale**:
- `copy_from_user` works with any userspace buffer, no pre-registration
- Fixed buffers require `io_uring_register_buffers()` upfront and
  careful lifetime management
- The copy path is simpler to implement and debug
- Performance-sensitive users opt into fixed buffers explicitly

### Packet includes the full Ethernet header

**Decision**: Userspace provides the complete Ethernet frame
(header + payload) in the buffer. The module does not construct
headers.

**Rationale**:
- Maximum flexibility — userspace controls dst/src MAC, ethertype,
  VLAN tags, and any custom L2 framing
- Matches `AF_PACKET` semantics (userspace builds the full frame)
- The module calls `eth_type_trans()` to parse the header and set
  `skb->protocol` — this is the same as what NIC drivers do on RX
- If header construction is desired, userspace libraries can provide
  builder functions (outside the kernel module)

### Per-command device selection via `ifindex`

**Decision**: Each TX command carries an `ifindex` to select the
target interface, rather than binding the fd to a fixed device.

**Rationale**:
- One fd + one io_uring ring can target multiple NICs
- Avoids opening/closing fds when switching interfaces
- `ifindex` is stable (unlike interface names which can change)
- A `PKT_OP_SET_DEV` command sets the default for commands with
  `ifindex == 0`

### All io_uring_cmd symbols are `EXPORT_SYMBOL_GPL`

**Decision**: The kernel module must be GPL-licensed.

**Rationale**: `__io_uring_cmd_done`, `io_uring_cmd_mark_cancelable`,
`io_uring_cmd_import_fixed`, and `io_uring_cmd_import_fixed_vec` are
all `EXPORT_SYMBOL_GPL` in `io_uring/uring_cmd.c`. There is no
non-GPL path to complete io_uring commands. This is consistent with
rko's existing GPL-2.0 license.

## Implementation Plan

**Prerequisites**: io-uring-cmd.md Phase 1-2 and raw-ethernet.md Phase 1-2
must be completed first (bindings for `io_uring_cmd` and `sk_buff`).

### Phase 1: Misc device skeleton

1. Implement `rko-core/src/miscdevice.rs` — minimal `miscdevice`
   registration wrapper (register/deregister, file_operations)
2. Create `samples/rko_pkt/` module that registers `/dev/rko_pkt`
3. Wire `uring_cmd` into the miscdevice's `file_operations`
4. Test: `open("/dev/rko_pkt")` succeeds from userspace

### Phase 2: Copy-path TX

1. Implement `IoUringCmd::user_buffer()` and `copy_from_user()`
2. Implement `PKT_OP_TX` handler: parse TxCmd → copy packet →
   alloc skb → dev_queue_xmit → done
3. C helpers for `sqe->addr`, `sqe->len` access
4. Userspace test program: send a single frame, capture with tcpdump
5. QEMU test: verify frame arrives on virtual NIC

### Phase 3: Fixed buffer zero-copy TX

1. Implement `IoUringCmd::import_fixed()` wrapper
2. Add `import_fixed` path in TX handler (check `buf_index`)
3. Userspace test: register buffers, send burst of 64 packets
4. Benchmark: compare copy vs fixed buffer throughput

### Phase 4: Direct xmit and batching

1. Implement `PKT_OP_TX_DIRECT` via `__dev_direct_xmit`
2. Add `PKT_OP_SET_DEV` for default interface binding
3. Benchmark: measure latency improvement with direct xmit
4. SQ polling (`IORING_SETUP_SQPOLL`) testing for lowest latency

### Phase 5: Multishot RX — packet receive via io_uring

1. Implement `PKT_OP_RX_START` handler with multishot + provided buffers
2. Implement `dev_add_pack` hook → `io_uring_mshot_cmd_post_cqe` path
3. Implement `PKT_OP_RX_STOP` to cancel/disarm
4. Userspace test: receive and count frames with multishot CQE loop
5. QEMU test: ping host → guest, verify packets appear in CQ

### Phase 6: Filtering and advanced RX

1. Add BPF-style filter support in `RxCmd` (ethertype, MAC prefix)
2. Per-device RX (filter by ifindex)
3. Benchmark: measure RX throughput at various packet rates
4. Bidirectional test: TX + RX simultaneously on one ring

---

## RX Path: Receiving Packets via io_uring Multishot

### Overview

The RX path uses **multishot uring_cmd** — a single SQE arms a
persistent packet receiver that delivers every captured frame as a
separate CQE. This eliminates per-packet syscall overhead entirely:

```
NIC RX ring → NAPI → __netif_receive_skb()
                          │
                          ▼
                   dev_add_pack handler (module)
                          │
                          ├─ select buffer from provided group
                          ├─ copy skb data into user buffer
                          ├─ post CQE via io_uring_mshot_cmd_post_cqe()
                          │
                          ▼
                   io_uring CQ ring → userspace reads CQE
                                      (IORING_CQE_F_MORE = more coming)
```

**One SQE → unlimited CQEs** until canceled or buffer exhaustion.

### Kernel API: Multishot uring_cmd

The kernel provides two functions specifically for multishot
uring_cmd drivers (both `EXPORT_SYMBOL_GPL` in `uring_cmd.c`):

```c
// Select a buffer from the userspace-provided buffer group.
// Returns io_br_sel with .addr (user buffer pointer) and .val (size).
// The driver writes packet data to this address.
struct io_br_sel io_uring_cmd_buffer_select(
    struct io_uring_cmd *ioucmd,
    unsigned buf_group,         // provided buffer group ID
    size_t *len,                // out: max buffer size
    unsigned int issue_flags);

// Post a CQE for one received packet.
// Sets IORING_CQE_F_MORE if the multishot is still active.
// Returns false if successfully posted, true if multishot must end.
bool io_uring_mshot_cmd_post_cqe(
    struct io_uring_cmd *ioucmd,
    struct io_br_sel *sel,      // from buffer_select
    unsigned int issue_flags);
```

**`struct io_br_sel`** (from `io_uring_types.h`):
```c
struct io_br_sel {
    struct io_buffer_list *buf_list;
    union {
        void __user *addr;   // userspace buffer address
        ssize_t val;         // or error code (if negative)
    };
};
```

**CQE flags for multishot**:
- `IORING_CQE_F_MORE` (bit 1) — more CQEs will follow from this SQE
- `IORING_CQE_F_BUFFER` (bit 0) — upper 16 bits of `cqe->flags`
  contain the buffer ID from the provided group
- `IORING_CQE_BUFFER_SHIFT` = 16 — shift to extract buffer ID

### Command Protocol: RX additions

```c
/* Additional opcodes in sqe->cmd_op */
enum rko_pkt_op {
    PKT_OP_TX          = 0,  /* Transmit one raw frame */
    PKT_OP_TX_DIRECT   = 1,  /* Transmit bypassing qdisc */
    PKT_OP_SET_DEV     = 2,  /* Bind to a network device */
    PKT_OP_GET_INFO    = 3,  /* Query device info (MAC, MTU) */
    PKT_OP_RX_START    = 4,  /* Start multishot packet receive */
    PKT_OP_RX_STOP     = 5,  /* Stop receiving (cancel multishot) */
};

/* RX filter flags */
#define PKT_RX_F_PROMISC    (1 << 0)  /* Promiscuous: all frames */
#define PKT_RX_F_BROADCAST  (1 << 1)  /* Include broadcast */
#define PKT_RX_F_MULTICAST  (1 << 2)  /* Include multicast */

/* Command payload in sqe->cmd[] for PKT_OP_RX_START */
struct rko_rx_cmd {
    __u32 ifindex;         /* interface to capture on (0 = all) */
    __u16 ethertype;       /* filter by ethertype (0 = all, ETH_P_ALL) */
    __u16 snap_len;        /* max bytes to capture per frame (0 = full) */
    __u32 flags;           /* PKT_RX_F_* flags */
    __u32 reserved;
};
/* static_assert(sizeof(struct rko_rx_cmd) <= 80) */

/* CQE result for received packets:
 * cqe->res >= 0    → bytes written to provided buffer
 * cqe->res < 0     → -errno (e.g., buffer exhaustion)
 * cqe->flags & IORING_CQE_F_MORE   → multishot still active
 * cqe->flags & IORING_CQE_F_BUFFER → buffer ID in upper 16 bits
 *
 * Buffer layout:
 *   [0..1]   __u16 frame_len    — original frame length (before snap)
 *   [2..3]   __u16 ifindex      — interface the frame arrived on
 *   [4..7]   __u32 reserved
 *   [8..]    raw Ethernet frame data (up to snap_len bytes)
 */
#define RKO_RX_HDR_SIZE  8
```

### RX Architecture in the kernel module

```
PKT_OP_RX_START SQE                     per-packet CQEs
     │                                        ▲  ▲  ▲
     ▼                                        │  │  │
┌─────────────────────────────────────────────────────────┐
│                   rko_pkt module                         │
│                                                          │
│  handle_rx_start(cmd, flags):                            │
│    1. Parse RxCmd from sqe->cmd[]                        │
│    2. Store deferred cmd (cmd.defer() → IoUringCmdAsync) │
│    3. Register dev_add_pack(&pt) with our handler        │
│    4. Return -EIOCBQUEUED (multishot stays armed)        │
│                                                          │
│  rx_handler(skb, dev, pt):  ← called per received frame │
│    1. Apply filters (ifindex, ethertype, snap_len)       │
│    2. io_uring_cmd_buffer_select(cmd, group, &len, fl)   │
│    3. Write RX header + skb data into selected buffer    │
│    4. io_uring_mshot_cmd_post_cqe(cmd, &sel, fl)         │
│    5. If post returns true → multishot ended (cleanup)   │
│    6. kfree_skb(skb) / return                            │
│                                                          │
│  handle_rx_stop / cancel:                                │
│    1. dev_remove_pack(&pt)                               │
│    2. io_uring_cmd_done(cmd, 0, flags) — final CQE       │
│                                                          │
└──────────────────────────────────────────────────────────┘
```

### Key design: deferred multishot command

The `PKT_OP_RX_START` handler does **not** complete the command
immediately. Instead:

1. Calls `cmd.defer()` → `IoUringCmdAsync` (owns the command)
2. Stores the async handle in module state
3. Returns `-EIOCBQUEUED` to io_uring core
4. The `dev_add_pack` callback posts CQEs on each packet via
   `io_uring_mshot_cmd_post_cqe()`
5. The multishot persists until:
   - Userspace cancels (`IORING_ASYNC_CANCEL`)
   - Provided buffer pool is exhausted
   - `PKT_OP_RX_STOP` is submitted
   - Module unload

### Kernel-side implementation (Rust)

```rust
impl PktDevice {
    fn handle_rx_start(
        cmd: IoUringCmd,
        flags: IssueFlags,
    ) -> Result<(), Error> {
        let rx_cmd = unsafe { cmd.cmd_data::<RkoRxCmd>()? };

        // Validate
        if rx_cmd.snap_len != 0 && (rx_cmd.snap_len as usize) < ETH_HLEN {
            cmd.done(Error::EINVAL.to_errno(), flags);
            return Ok(());
        }

        // Defer the command — it will produce CQEs over time
        let async_cmd = cmd.defer();

        // Create RX state
        let rx_state = RxState {
            cmd: async_cmd,
            ifindex: rx_cmd.ifindex,
            ethertype: rx_cmd.ethertype,
            snap_len: rx_cmd.snap_len,
            flags: rx_cmd.flags,
        };

        // Register packet handler
        // The packet_type callback calls rx_packet_handler()
        let hook = PacketHook::register_with_state(
            rx_cmd.ethertype,
            rx_state,
            rx_packet_handler,
        )?;

        // Store hook in module state for cleanup
        Self::store_rx_hook(hook);

        // Return EIOCBQUEUED — command stays open
        Err(Error::EIOCBQUEUED)
    }
}

/// Called from softirq context for every received frame.
fn rx_packet_handler(
    skb: &SkBuff,
    dev: &NetDevice,
    state: &RxState,
) -> Verdict {
    // 1. Filter by interface
    if state.ifindex != 0 && dev.ifindex() != state.ifindex as i32 {
        return Verdict::Pass;
    }

    // 2. Select a buffer from the provided group
    let mut len: usize = 0;
    let sel = state.cmd.buffer_select(
        state.buf_group,
        &mut len,
        state.issue_flags,
    );
    if sel.is_err() {
        // Buffer pool exhausted — multishot ends
        return Verdict::Pass;
    }

    // 3. Determine capture length
    let frame_len = skb.len();
    let snap = if state.snap_len > 0 {
        core::cmp::min(frame_len, state.snap_len as usize)
    } else {
        frame_len
    };
    let total = RKO_RX_HDR_SIZE + snap;

    // 4. Write RX header + frame data into user buffer
    let hdr = RkoRxHeader {
        frame_len: frame_len as u16,
        ifindex: dev.ifindex() as u16,
        reserved: 0,
    };
    // copy_to_user: header then frame data
    state.cmd.copy_to_user(sel.addr(), &hdr, RKO_RX_HDR_SIZE);
    state.cmd.copy_to_user(
        sel.addr() + RKO_RX_HDR_SIZE,
        skb.data(),
        snap,
    );

    // 5. Post CQE
    let ended = state.cmd.mshot_post_cqe(&sel, total, state.issue_flags);
    if ended {
        // Multishot terminated (CQ overflow or cancel)
        Self::cleanup_rx();
    }

    Verdict::Pass // let normal stack processing continue
}
```

### Userspace: receiving packets

```c
/* Provide a pool of buffers for the kernel to fill */
#define BUF_COUNT  256
#define BUF_SIZE   2048  /* enough for any Ethernet frame + RX header */

/* Register provided buffer group */
struct io_uring_buf_ring *br = io_uring_setup_buf_ring(&ring,
    BUF_COUNT, 0 /* group_id */, 0, &ret);
for (int i = 0; i < BUF_COUNT; i++) {
    void *buf = buffers + i * BUF_SIZE;
    io_uring_buf_ring_add(br, buf, BUF_SIZE, i, BUF_COUNT - 1, i);
}
io_uring_buf_ring_advance(br, BUF_COUNT);

/* Submit multishot RX command */
struct io_uring_sqe *sqe = io_uring_get_sqe(&ring);
sqe->opcode = IORING_OP_URING_CMD;
sqe->fd = pkt_fd;
sqe->cmd_op = PKT_OP_RX_START;
sqe->flags |= IOSQE_BUFFER_SELECT;
sqe->buf_group = 0;
sqe->uring_cmd_flags = IORING_URING_CMD_MULTISHOT;

struct rko_rx_cmd rx = {
    .ifindex = 2,       /* capture on eth0 */
    .ethertype = 0,     /* all protocols */
    .snap_len = 0,      /* full frames */
    .flags = PKT_RX_F_PROMISC,
};
memcpy(sqe->cmd, &rx, sizeof(rx));
io_uring_submit(&ring);

/* Receive loop — zero syscalls per packet */
while (running) {
    struct io_uring_cqe *cqe;
    io_uring_wait_cqe(&ring, &cqe);

    if (cqe->res < 0) {
        fprintf(stderr, "RX error: %s\n", strerror(-cqe->res));
        break;
    }

    /* Extract buffer ID from CQE flags */
    int buf_id = cqe->flags >> IORING_CQE_BUFFER_SHIFT;
    uint8_t *data = buffers + buf_id * BUF_SIZE;

    /* Parse RX header */
    uint16_t frame_len = *(uint16_t *)(data + 0);
    uint16_t ifindex   = *(uint16_t *)(data + 2);
    uint8_t *frame     = data + RKO_RX_HDR_SIZE;

    struct ethhdr *eth = (struct ethhdr *)frame;
    printf("RX %u bytes on if%u: %02x:%02x:%02x:%02x:%02x:%02x → "
           "%02x:%02x:%02x:%02x:%02x:%02x proto=0x%04x\n",
        frame_len, ifindex,
        eth->h_source[0], eth->h_source[1], eth->h_source[2],
        eth->h_source[3], eth->h_source[4], eth->h_source[5],
        eth->h_dest[0], eth->h_dest[1], eth->h_dest[2],
        eth->h_dest[3], eth->h_dest[4], eth->h_dest[5],
        ntohs(eth->h_proto));

    /* Return buffer to the pool for reuse */
    io_uring_buf_ring_add(br, data, BUF_SIZE, buf_id,
                          BUF_COUNT - 1, 0);
    io_uring_buf_ring_advance(br, 1);

    bool more = cqe->flags & IORING_CQE_F_MORE;
    io_uring_cqe_seen(&ring, cqe);

    if (!more) {
        printf("Multishot ended, re-arming...\n");
        /* Re-submit PKT_OP_RX_START to continue */
        break;
    }
}
```

### Full-duplex: TX + RX on one ring

```c
/* One io_uring ring handles both directions */
struct io_uring ring;
io_uring_queue_init_params(256, &ring, &params);

/* Submit RX multishot (armed once) */
submit_rx_start(&ring, fd, ifindex);

/* TX loop: submit packets as needed */
while (running) {
    /* Check CQ for both TX completions and RX deliveries */
    struct io_uring_cqe *cqe;
    while (io_uring_peek_cqe(&ring, &cqe) == 0) {
        uint64_t tag = cqe->user_data;
        if (tag == RX_TAG) {
            handle_rx_packet(cqe, buffers);
        } else {
            handle_tx_completion(cqe);
        }
        io_uring_cqe_seen(&ring, cqe);
    }

    /* Submit TX if we have packets to send */
    if (have_outgoing_packets()) {
        submit_tx_batch(&ring, fd, packets, count);
    }

    io_uring_submit_and_wait(&ring, 1);
}
```

## RX Design Decisions

### Multishot uring_cmd, not poll + read

**Decision**: Use `IORING_URING_CMD_MULTISHOT` with provided buffers,
not a `poll()` + `read()` pattern.

**Rationale**:
- One SQE produces unlimited CQEs — zero per-packet submission overhead
- Provided buffer groups let the kernel select buffers without
  userspace involvement — true zero-syscall-per-packet receive
- `IORING_CQE_F_MORE` tells userspace when to re-arm
- This is the same pattern used by multishot `recv()` in io_uring
  and by the ublk block device driver

### `dev_add_pack` in softirq, `copy_to_user` into provided buffers

**Decision**: The packet handler runs in softirq context (NAPI) and
copies skb data into the selected provided buffer.

**Rationale**:
- `dev_add_pack` callbacks run in softirq (not process context)
- Cannot sleep, allocate with `GFP_KERNEL`, or take sleeping locks
- `io_uring_cmd_buffer_select()` and `io_uring_mshot_cmd_post_cqe()`
  are designed for this context — they use the `issue_flags` to adapt
- The copy from `skb->data` to the user buffer is a single memcpy
  (the provided buffer pages are already pinned and mapped by io_uring)
- True zero-copy is not possible here because the skb is owned by
  the network stack and may be shared (cloned) — we must copy

### 8-byte RX header prefix

**Decision**: Each received buffer starts with an 8-byte header
containing the original frame length and source interface index.

**Rationale**:
- `cqe->res` gives the number of bytes written, but not the original
  frame length (if snap_len truncated it)
- The interface index lets userspace demux frames from multiple NICs
  when receiving with `ifindex == 0` (all interfaces)
- 8 bytes keeps natural alignment for the frame data that follows
- Minimal overhead — less than 0.5% of a typical 1500-byte frame

### Provided buffer recycling in userspace

**Decision**: Userspace is responsible for returning consumed buffers
to the provided buffer ring via `io_uring_buf_ring_add()`.

**Rationale**:
- This is the standard io_uring provided buffer contract
- The kernel doesn't know when userspace is done processing a buffer
- If buffers aren't recycled fast enough, `buffer_select()` will
  fail and the multishot terminates with a final CQE (no crash)
- Userspace can tune the pool size for its processing speed

### `Verdict::Pass` — non-consuming capture

**Decision**: The packet handler returns `Verdict::Pass` (equivalent
to `NET_RX_SUCCESS`) so the packet continues through the normal
network stack.

**Rationale**:
- Matches `tcpdump` / `AF_PACKET` behavior — capture is non-intrusive
- Other protocol handlers (IP, ARP) still process the packet
- If a future use case needs consuming capture (e.g., custom protocol
  stack), add a `PKT_RX_F_CONSUME` flag

## Updated Performance Model (TX + RX combined)

See the detailed [Comparison with AF_PACKET](#comparison-with-af_packet)
section above for an honest assessment against TPACKET_V3 and AF_XDP.

Summary for this design's combined TX + RX:

- **TX**: 1-5 Mpps (copy), 5-15 Mpps (fixed buffers). ~5-10% faster
  than AF_PACKET + io_uring due to socket layer bypass.
- **RX**: 2-8 Mpps (softirq copy to provided buffer). Comparable to
  TPACKET_V3 mmap ring; ~20-30% faster than AF_PACKET + `IORING_OP_RECV`.
- **Bidirectional**: Single ring handles TX submissions + RX multishot
  CQEs concurrently. AF_PACKET requires two sockets.
- **10 GbE line rate** (14.88 Mpps at 64B): Not achievable — use AF_XDP
  for pure forwarding at this rate.
- **Sweet spot**: 1-10 Gbps with kernel-side per-packet processing.

## Open Questions

1. **Skb allocation pressure** (TX): At high TX rates, `alloc_skb`
   per packet may bottleneck. Consider a pre-allocated skb pool
   or `kmem_cache` for fixed-size frames.

2. **Deferred TX completion**: Complete immediately (simpler) vs
   defer until NIC DMA finishes (true "on wire" semantics)?

3. **MTU enforcement**: Reject oversized TX frames early, or let
   `dev_queue_xmit` handle it?

4. **Security**: Require `CAP_NET_RAW` on `open("/dev/rko_pkt")`?

5. **RX softirq budget**: The `dev_add_pack` handler runs per-packet
   in softirq. If `buffer_select` + `copy` + `mshot_post_cqe` is
   too slow, it can starve other softirq work. May need to batch
   or defer to a workqueue for very high packet rates.

6. **RX buffer sizing**: Should the module advertise minimum buffer
   size requirements via `PKT_OP_GET_INFO`? Buffers smaller than
   `ETH_HLEN + RKO_RX_HDR_SIZE` would silently truncate.

7. **Multiple concurrent RX sessions**: Can multiple fds or SQEs
   independently arm multishot RX on the same interface? Need
   reference counting on the `dev_add_pack` registration.

8. **Provided buffer group ID**: Should the module mandate a
   specific group ID, or let userspace choose? The SQE's
   `buf_group` field passes it through.

## Future Work

- **TX completion notification**: Deferred completion with NIC TX
  interrupt for true "packet on wire" semantics
- **Scatter-gather TX**: Accept `iovec` arrays for multi-buffer
  frames without copying into contiguous skb
- **Hardware timestamping**: Return TX/RX timestamps in 32-byte
  CQEs (`io_uring_cmd_done32`) for PTP/precision timing
- **Traffic shaping**: Module-managed rate limiting per flow
- **BPF filter integration**: Attach cBPF/eBPF filter programs
  to `PKT_OP_RX_START` for kernel-side packet filtering
- **Per-device RX handler** (`rx_handler_register`): For scenarios
  requiring consuming capture without affecting other stack users
- **Kernel-side protocol processing**: The module could parse
  received frames (ARP, custom protocols) and respond directly
  from kernel space, combining RX + TX in a single fast path

## References

- io_uring cmd design: `docs/design/features/futures/io-uring-cmd.md`
- Raw ethernet TX/RX design: `docs/design/features/futures/raw-ethernet.md`
- Kernel: `io_uring/uring_cmd.c` — `EXPORT_SYMBOL_GPL` for
  `io_uring_cmd_buffer_select`, `io_uring_mshot_cmd_post_cqe`,
  `__io_uring_cmd_done`, `io_uring_cmd_import_fixed`
- Kernel: `include/uapi/linux/io_uring.h` — SQE struct, CQE flags,
  `IORING_URING_CMD_MULTISHOT`, `IORING_CQE_F_MORE`
- Kernel: `include/linux/io_uring/cmd.h` — `io_br_sel`, buffer select API
- Kernel: `include/linux/io_uring_types.h:94` — `struct io_br_sel`
- NVMe passthrough: `drivers/nvme/host/ioctl.c`
- ublk: first user of multishot uring_cmd with provided buffers
- Multishot io_uring: [Arch manpage](https://man.archlinux.org/man/io_uring_multishot.7.en)
- FUSE over io_uring: [kernel docs](https://docs.kernel.org/next/filesystems/fuse-io-uring.html)
- liburing: [NVMe passthrough](https://deepwiki.com/axboe/liburing/3.5-nvme-and-passthrough-commands),
  [multishot ops](https://deepwiki.com/axboe/liburing/4.2-multishot-operations)
- Networking design: `docs/design/features/networking.md`
