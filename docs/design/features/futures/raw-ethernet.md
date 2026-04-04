# Feature: Raw Ethernet Packet TX/RX

**Status**: 📋 Design — not yet implemented.

## Goal

Enable Rust kernel modules built with rko to construct and transmit raw
Ethernet frames directly, bypassing the TCP/UDP/IP protocol stack. Also
enable receiving raw frames via protocol hooks. This gives modules
Layer 2 access for custom protocols, high-performance packet injection,
and network monitoring — all from safe Rust.

## Background

The Linux kernel provides multiple levels of packet transmission, from
the socket API (which rko already wraps for TCP) down to direct hardware
ring buffer access. For a loadable kernel module (.ko), the practical
API layers are:

### TX Path: Kernel → NIC

```
Protocol layer (TCP/IP)    ← rko has this (TcpListener/TcpStream)
        │
        ▼
dev_queue_xmit(skb)        ← standard kernel TX entry point
        │
        ├──► qdisc enqueue/dequeue (Traffic Control)
        │
        ▼
dev_hard_start_xmit(skb)   ← bypasses qdisc (direct path)
        │
        ▼
ndo_start_xmit(skb, dev)   ← NIC driver callback
        │
        ▼
NIC TX ring buffer (DMA)    ← hardware
```

### RX Path: NIC → Kernel

```
NIC RX ring buffer (DMA)    ← hardware
        │
        ▼
NAPI poll → napi_gro_receive(skb)
        │
        ▼
__netif_receive_skb()
        │
        ├──► rx_handler (per-device hook — bridge, macvlan)
        ├──► packet_type handlers (dev_add_pack — sniffer, custom proto)
        │
        ▼
Protocol handlers (ip_rcv, arp_rcv, ...)
```

### API levels accessible from a kernel module

| Level | TX API | RX API | Bypasses | Available to .ko |
|-------|--------|--------|----------|:---:|
| **Socket** | `kernel_sendmsg()` | `kernel_recvmsg()` | nothing | ✅ (rko has this) |
| **Network stack** | `dev_queue_xmit(skb)` | `dev_add_pack()` / `rx_handler` | TCP/UDP/IP | ✅ (exported) |
| **Direct xmit** | `__dev_direct_xmit(skb)` | — | TCP/UDP/IP + qdisc | ✅ (exported) |
| **XDP** | `XDP_TX` / `XDP_REDIRECT` | XDP program at driver | entire stack | ⚠️ (eBPF, not .ko) |
| **Driver ring** | `ndo_start_xmit()` | NAPI poll | everything | ❌ (driver-internal) |

**This design targets the "Network stack" and "Direct xmit" levels** —
the lowest layers accessible from a standard loadable kernel module
without writing a NIC driver.

## Kernel API Surface

### Core types

**`struct sk_buff`** (socket buffer) — the universal packet container
in the Linux kernel. Every packet moving through the network stack is
represented as an `sk_buff`. Key fields:

```c
struct sk_buff {
    struct net_device *dev;       // associated network device
    unsigned char     *head;      // start of allocated buffer
    unsigned char     *data;      // start of current data
    unsigned int      len;        // data length
    unsigned int      data_len;   // paged data length
    __u16             protocol;   // ethernet protocol (after eth_type_trans)
    // ... 200+ other fields
};
```

**`struct net_device`** — represents a network interface (eth0, lo, etc).
Obtained via `__dev_get_by_name(&init_net, "eth0")`.

**`struct ethhdr`** — 14-byte Ethernet header (dst MAC, src MAC, ethertype).

### TX functions (all exported in Module.symvers)

| Function | Symbol | Export | Purpose |
|----------|--------|--------|---------|
| `__alloc_skb()` | `EXPORT_SYMBOL` | Allocate sk_buff with data buffer |
| `__netdev_alloc_skb()` | `EXPORT_SYMBOL` | Allocate skb for a specific device |
| `skb_put()` | `EXPORT_SYMBOL` | Append data to tail, advance tail pointer |
| `skb_push()` | `EXPORT_SYMBOL` | Prepend data, move data pointer back |
| `__dev_get_by_name()` | `EXPORT_SYMBOL` | Look up net_device by name |
| `__dev_queue_xmit()` | `EXPORT_SYMBOL` | Standard TX entry (with qdisc) |
| `__dev_direct_xmit()` | `EXPORT_SYMBOL` | Direct TX (bypass qdisc) |
| `eth_type_trans()` | `EXPORT_SYMBOL` | Parse ethernet header, set skb->protocol |
| `skb_clone()` | `EXPORT_SYMBOL` | Clone skb (shared data) |
| `skb_trim()` | `EXPORT_SYMBOL` | Trim skb to given length |
| `consume_skb()` | — | Free transmitted skb (success path) |

**Inline functions (need C helpers)**:

| Function | Header | Purpose |
|----------|--------|---------|
| `skb_reserve()` | `skbuff.h:2926` | Reserve headroom in fresh skb |
| `skb_headroom()` | `skbuff.h:2887` | Query available headroom |
| `skb_tailroom()` | `skbuff.h:2898` | Query available tailroom |
| `skb_reset_mac_header()` | `skbuff.h:3155` | Set MAC header pointer to current data |
| `eth_hdr()` | `if_ether.h:25` | Cast skb MAC header to ethhdr |

### RX hook functions (all exported)

| Function | Export | Purpose |
|----------|--------|---------|
| `dev_add_pack()` | `EXPORT_SYMBOL` | Register protocol handler (ETH_P_ALL for all frames) |
| `dev_remove_pack()` | `EXPORT_SYMBOL` | Unregister protocol handler |
| `netdev_rx_handler_register()` | `EXPORT_SYMBOL_GPL` | Per-device RX handler (like bridge) |
| `netdev_rx_handler_unregister()` | `EXPORT_SYMBOL_GPL` | Remove per-device RX handler |
| `netif_receive_skb()` | `EXPORT_SYMBOL` | Re-inject packet into RX path |

## Architecture

```
                    ┌─────────────────────────────┐
                    │       User Rust module       │
                    │                              │
                    │  RawSocket::send(dev, frame) │
                    │  RawSocket::on_recv(handler)  │
                    └──────────┬──────────────────┘
                               │
                    ┌──────────▼──────────────────┐
                    │   rko-core/src/rawnet/       │
                    │                              │
                    │  SkBuff  — safe sk_buff      │
                    │  NetDevice — device lookup   │
                    │  EthFrame — frame builder    │
                    │  PacketHook — RX handler     │
                    └──────────┬──────────────────┘
                               │
                    ┌──────────▼──────────────────┐
                    │   rko-sys (new partitions)   │
                    │                              │
                    │  rko.skb   — sk_buff types   │
                    │  rko.netdev — net_device     │
                    │  + C helpers for inlines     │
                    └──────────┬──────────────────┘
                               │
                    ┌──────────▼──────────────────┐
                    │   Kernel (exported symbols)  │
                    │                              │
                    │  __dev_queue_xmit            │
                    │  __dev_direct_xmit           │
                    │  dev_add_pack / dev_remove   │
                    └─────────────────────────────┘
```

### Layer breakdown

| Layer | Crate | Responsibility |
|-------|-------|----------------|
| **rko-sys `rko.skb`** | rko-sys | FFI: `sk_buff`, `skb_shared_info`, alloc/put/push, constants |
| **rko-sys `rko.netdev`** | rko-sys | FFI: `net_device`, `ethhdr`, `packet_type`, device lookup, xmit |
| **rko-sys helpers** | helpers.{c,h} | C wrappers for inline functions: `skb_reserve`, `skb_headroom`, `eth_hdr`, etc. |
| **rko-core `rawnet`** | rko-core | Safe wrappers: `SkBuff`, `NetDevice`, `EthFrame`, `PacketHook` |
| **Driver module** | user crate | Constructs frames, sends/receives via safe API |

## Bindings (rko-sys)

### New partition: `rko.skb`

The `sk_buff` struct is the most complex type in the kernel networking
stack (~200 fields, unions, anonymous structs). A full traverse of
`linux/skbuff.h` will cascade into hundreds of types. Strategy:
**inject `sk_buff` as opaque + bind only the functions we need**.

```toml
[[partition]]
namespace = "rko.skb"
library = "kernel"
headers = ["linux/skbuff.h"]
traverse = [
  "linux/skbuff.h",
]

# sk_buff is too complex for full extraction — inject as opaque.
# Access fields through C helpers only.
[[inject_type]]
name = "rko.skb.sk_buff"
size = 232    # sizeof(struct sk_buff) — verify with clang on target kernel
align = 8
```

**Rationale**: `sk_buff` contains dozens of anonymous unions, bitfields,
and conditional fields (`CONFIG_*` dependent). Injecting as opaque and
accessing all fields through C helpers is far more maintainable than
trying to match the exact layout, which changes across kernel versions.

### New partition: `rko.netdev`

```toml
[[partition]]
namespace = "rko.netdev"
library = "kernel"
headers = ["linux/netdevice.h"]
traverse = [
  "linux/netdevice.h",
  "linux/if_ether.h",
  "uapi/linux/if_ether.h",
  "linux/etherdevice.h",
]
```

This extracts `net_device` (also very large — may need inject_type),
`ethhdr`, `packet_type`, and Ethernet constants (`ETH_ALEN`, `ETH_HLEN`,
`ETH_P_ALL`, `ETH_P_IP`, etc.).

If `net_device` cascades too aggressively:

```toml
[[inject_type]]
name = "rko.netdev.net_device"
size = 2688   # sizeof(struct net_device) — verify with clang
align = 64    # typically cacheline aligned
```

### C helpers

```c
// helpers.h additions
#include <linux/skbuff.h>
#include <linux/netdevice.h>
#include <linux/if_ether.h>

// --- sk_buff field accessors ---
struct net_device *rust_helper_skb_dev(const struct sk_buff *skb);
void rust_helper_skb_set_dev(struct sk_buff *skb, struct net_device *dev);
unsigned char *rust_helper_skb_data(const struct sk_buff *skb);
unsigned int rust_helper_skb_len(const struct sk_buff *skb);
__be16 rust_helper_skb_protocol(const struct sk_buff *skb);
void rust_helper_skb_set_protocol(struct sk_buff *skb, __be16 proto);

// --- sk_buff inline wrappers ---
void rust_helper_skb_reserve(struct sk_buff *skb, int len);
unsigned int rust_helper_skb_headroom(const struct sk_buff *skb);
unsigned int rust_helper_skb_tailroom(const struct sk_buff *skb);
void rust_helper_skb_reset_mac_header(struct sk_buff *skb);
void rust_helper_skb_set_network_header(struct sk_buff *skb, int offset);

// --- Ethernet helpers ---
struct ethhdr *rust_helper_eth_hdr(const struct sk_buff *skb);

// --- Device lookup ---
struct net_device *rust_helper_dev_get_by_name(
    struct net *net, const char *name);

// helpers.c implementations
void rust_helper_skb_reserve(struct sk_buff *skb, int len)
{
    skb_reserve(skb, len);
}

unsigned int rust_helper_skb_headroom(const struct sk_buff *skb)
{
    return skb_headroom(skb);
}

unsigned int rust_helper_skb_tailroom(const struct sk_buff *skb)
{
    return skb_tailroom(skb);
}

void rust_helper_skb_reset_mac_header(struct sk_buff *skb)
{
    skb_reset_mac_header(skb);
}

struct ethhdr *rust_helper_eth_hdr(const struct sk_buff *skb)
{
    return eth_hdr(skb);
}

struct net_device *rust_helper_dev_get_by_name(
    struct net *net, const char *name)
{
    return __dev_get_by_name(net, name);
}
```

## Safe Rust API (rko-core)

### `rko-core/src/rawnet/mod.rs`

```rust
pub mod skb;
pub mod device;
pub mod frame;
pub mod hook;
```

### `SkBuff` — safe socket buffer wrapper

```rust
/// Owned wrapper around `struct sk_buff`.
///
/// When dropped, frees the skb via `kfree_skb`. For transmitted
/// skbs, ownership transfers to the network stack (via `send()`),
/// so drop is a no-op.
pub struct SkBuff {
    skb: *mut bindings::sk_buff,
    owned: bool,
}

impl SkBuff {
    /// Allocate a new skb with `size` bytes of data space.
    pub fn alloc(size: usize, flags: Flags) -> Result<Self, Error> { ... }

    /// Allocate an skb for a specific device (uses device's
    /// NUMA node for optimal memory placement).
    pub fn alloc_for_device(dev: &NetDevice, size: usize) -> Result<Self, Error> { ... }

    /// Reserve headroom (must be called before put/push).
    pub fn reserve(&mut self, len: usize) { ... }

    /// Append `data` to the skb tail. Returns a mutable slice
    /// to the newly added region for in-place writes.
    pub fn put(&mut self, len: usize) -> &mut [u8] { ... }

    /// Append data by copying from a slice.
    pub fn put_data(&mut self, data: &[u8]) { ... }

    /// Prepend `len` bytes, moving data pointer backward.
    /// Returns a mutable slice to the new region (for headers).
    pub fn push(&mut self, len: usize) -> &mut [u8] { ... }

    /// Current data as a byte slice.
    pub fn data(&self) -> &[u8] { ... }

    /// Current data length.
    pub fn len(&self) -> usize { ... }

    /// Available headroom.
    pub fn headroom(&self) -> usize { ... }

    /// Available tailroom.
    pub fn tailroom(&self) -> usize { ... }

    /// Trim skb to `len` bytes.
    pub fn trim(&mut self, len: usize) { ... }

    /// Set the associated network device.
    pub fn set_device(&mut self, dev: &NetDevice) { ... }

    /// Set the Ethernet protocol field (after eth_type_trans
    /// or manual assignment).
    pub fn set_protocol(&mut self, proto: u16) { ... }

    /// Consume the skb and transmit it via dev_queue_xmit.
    /// The network stack takes ownership; the skb must not
    /// be used after this call.
    pub fn send(self) -> Result<(), Error> { ... }

    /// Consume the skb and transmit it via dev_direct_xmit,
    /// bypassing the qdisc layer for lower latency.
    /// `queue` selects the hardware TX queue.
    pub fn send_direct(self, queue: u16) -> Result<(), Error> { ... }
}

impl Drop for SkBuff {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: we still own this skb.
            unsafe { kfree_skb(self.skb) };
        }
    }
}
```

### `NetDevice` — network device handle

```rust
/// Borrowed reference to a kernel `struct net_device`.
///
/// Obtained via `NetDevice::get_by_name()`. The device reference
/// is valid as long as RTNL lock or RCU read-side is held, or
/// the device is held with `dev_hold`.
pub struct NetDevice {
    dev: *mut bindings::net_device,
}

impl NetDevice {
    /// Look up a network device by name (e.g., "eth0").
    ///
    /// Must be called under RTNL lock or in a context where the
    /// device cannot disappear. For long-lived references, the
    /// caller should call `dev_hold`.
    pub fn get_by_name(name: &CStr) -> Result<Self, Error> { ... }

    /// Get the device's MAC address.
    pub fn mac_addr(&self) -> [u8; 6] { ... }

    /// Get the device's MTU.
    pub fn mtu(&self) -> u32 { ... }

    /// Get the device's interface index.
    pub fn ifindex(&self) -> i32 { ... }
}
```

### `EthFrame` — Ethernet frame builder

```rust
/// Builder for constructing raw Ethernet frames.
///
/// Allocates an skb, fills the payload, then prepends the
/// 14-byte Ethernet header. Provides a fluent API.
pub struct EthFrame {
    skb: SkBuff,
}

impl EthFrame {
    /// Create a new frame builder for the given device.
    ///
    /// Allocates an skb with enough space for `payload_len`
    /// plus Ethernet header (14 bytes) plus alignment.
    pub fn new(dev: &NetDevice, payload_len: usize) -> Result<Self, Error> {
        let total = payload_len + ETH_HLEN + NET_IP_ALIGN;
        let mut skb = SkBuff::alloc_for_device(dev, total)?;
        skb.reserve(ETH_HLEN + NET_IP_ALIGN);
        skb.set_device(dev);
        Ok(Self { skb })
    }

    /// Write the payload data.
    pub fn payload(mut self, data: &[u8]) -> Self {
        self.skb.put_data(data);
        self
    }

    /// Finalize with Ethernet header and return a transmittable skb.
    ///
    /// `dst` and `src` are 6-byte MAC addresses.
    /// `ethertype` is the protocol (e.g., `ETH_P_IP`).
    pub fn build(
        mut self,
        dst: [u8; 6],
        src: [u8; 6],
        ethertype: u16,
    ) -> SkBuff {
        let hdr = self.skb.push(ETH_HLEN);
        // Write ethhdr: dst[6] + src[6] + proto[2]
        hdr[0..6].copy_from_slice(&dst);
        hdr[6..12].copy_from_slice(&src);
        hdr[12..14].copy_from_slice(&ethertype.to_be_bytes());
        self.skb.set_protocol(ethertype);
        self.skb
    }
}
```

### `PacketHook` — RX frame interception

```rust
/// Registers a protocol handler to intercept received Ethernet frames.
///
/// Uses `dev_add_pack()` internally. Automatically deregisters
/// on drop via `dev_remove_pack()`.
pub struct PacketHook<T: PacketHandler> {
    pt: Pin<KBox<PacketTypeWrapper<T>>>,
}

/// Trait for handling received packets.
pub trait PacketHandler: Send + Sync + 'static {
    /// Called for each matching received frame.
    ///
    /// `skb` is a borrowed reference — clone it if you need to
    /// keep it beyond this callback. Return `Verdict` to indicate
    /// whether the stack should continue processing.
    fn receive(&self, skb: &SkBuff, dev: &NetDevice) -> Verdict;
}

/// What to do with the packet after the handler.
pub enum Verdict {
    /// Allow normal stack processing to continue.
    Pass,
    /// Consume the packet (prevent further processing).
    Consume,
}

impl<T: PacketHandler> PacketHook<T> {
    /// Register a handler for all Ethernet frames (ETH_P_ALL).
    pub fn register_all(handler: T) -> Result<Self, Error> { ... }

    /// Register a handler for a specific ethertype.
    pub fn register(handler: T, ethertype: u16) -> Result<Self, Error> { ... }
}

impl<T: PacketHandler> Drop for PacketHook<T> {
    fn drop(&mut self) {
        // SAFETY: deregisters the packet_type we registered.
        unsafe { dev_remove_pack(&mut self.pt.pt) };
    }
}
```

## Design Decisions

### `sk_buff` as opaque type with accessor helpers

**Decision**: Inject `sk_buff` as an opaque blob, access all fields
through C helper functions.

**Rationale**: `sk_buff` is the most complex structure in the kernel
networking stack — ~200 fields, nested anonymous unions, bitfields,
and `CONFIG_*`-dependent layout. The struct changes between kernel
versions. Attempting to match the exact layout in Rust bindings would
be extremely fragile. The C helper approach:
- Is immune to layout changes between kernel versions
- Avoids cascading thousands of dependent types through rko-sys-gen
- Matches the pattern already used successfully in rko for `file`,
  `inode`, and other complex kernel types

### `send()` consumes ownership

**Decision**: `SkBuff::send()` and `send_direct()` consume `self`,
preventing use-after-free.

**Rationale**: `dev_queue_xmit` takes ownership of the skb — the
caller must not touch it after the call (even on error, the skb is
freed). By consuming `self`, the Rust type system enforces this
contract at compile time.

### `EthFrame` builder pattern

**Decision**: Provide a high-level `EthFrame` builder alongside
the low-level `SkBuff` API.

**Rationale**: Raw skb manipulation requires careful ordering
(reserve → put → push → set device → set protocol). The builder
encapsulates the correct sequence, preventing common mistakes like
forgetting to reserve headroom or pushing the header before the
payload. Advanced users can still use `SkBuff` directly.

### `dev_queue_xmit` vs `dev_direct_xmit`

**Decision**: Expose both as `send()` and `send_direct()`.

**Rationale**:
- `dev_queue_xmit` (`__dev_queue_xmit`) — the standard path. Goes
  through the qdisc layer, respects Traffic Control rules, and handles
  queue selection automatically. Correct for most use cases.
- `dev_direct_xmit` (`__dev_direct_xmit`) — bypasses qdisc for
  minimum latency. Requires the caller to specify the TX queue.
  Useful for latency-critical protocols or when the module manages
  its own queue discipline.

Both are `EXPORT_SYMBOL` and safe to call from a loadable module.

### `PacketHook` via `dev_add_pack` (not `rx_handler`)

**Decision**: Use `dev_add_pack()` for RX interception, not
`netdev_rx_handler_register()`.

**Rationale**:
- `dev_add_pack` is `EXPORT_SYMBOL` (any module can use it)
- `rx_handler_register` is `EXPORT_SYMBOL_GPL` and only one handler
  per device is allowed (conflicts with bridge/macvlan)
- `dev_add_pack` with `ETH_P_ALL` sees all frames on all interfaces,
  which is the common case for monitoring/interception
- Per-device filtering can be done in the handler callback

### Not exposing XDP or driver ring access

**Decision**: This feature targets the `sk_buff` + `dev_queue_xmit`
layer, not XDP or direct ring buffer manipulation.

**Rationale**:
- XDP programs are eBPF, not loadable kernel modules — they require
  the BPF verifier and cannot be written in arbitrary Rust
- Direct TX ring access requires writing a NIC driver (`ndo_start_xmit`),
  which is device-specific and out of scope for a generic framework
- The `sk_buff` layer is the lowest **portable** layer accessible to
  all loadable modules, and already bypasses TCP/UDP/IP entirely
- For true zero-copy userspace paths, AF_XDP is the right tool
  (but it's a userspace API, not kernel module)

## User API

### Sending a raw Ethernet frame

```rust
use rko_core::rawnet::{EthFrame, NetDevice};

fn send_custom_frame() -> Result<(), Error> {
    let dev = NetDevice::get_by_name(c"eth0")?;

    let payload = b"Hello from Rust kernel module!";

    let skb = EthFrame::new(&dev, payload.len())?
        .payload(payload)
        .build(
            [0xff, 0xff, 0xff, 0xff, 0xff, 0xff], // dst: broadcast
            dev.mac_addr(),                         // src: our MAC
            0x88B5,                                 // ethertype: local experimental
        );

    skb.send()?;
    Ok(())
}
```

### Receiving raw Ethernet frames

```rust
use rko_core::rawnet::{PacketHook, PacketHandler, SkBuff, NetDevice, Verdict};

struct MyMonitor;

impl PacketHandler for MyMonitor {
    fn receive(&self, skb: &SkBuff, dev: &NetDevice) -> Verdict {
        pr_info!("Received {} bytes on {}\n", skb.len(), dev.ifindex());
        Verdict::Pass // let normal stack processing continue
    }
}

// In Module::init():
let hook = PacketHook::register_all(MyMonitor)?;
// hook deregisters automatically on drop
```

### Low-level skb manipulation

```rust
use rko_core::rawnet::{SkBuff, NetDevice};

fn send_raw_skb() -> Result<(), Error> {
    let dev = NetDevice::get_by_name(c"eth0")?;

    let mut skb = SkBuff::alloc_for_device(&dev, 128)?;
    skb.reserve(14 + 2); // ETH_HLEN + NET_IP_ALIGN
    skb.set_device(&dev);

    // Write payload
    skb.put_data(b"raw payload data");

    // Prepend Ethernet header manually
    let hdr = skb.push(14);
    hdr[0..6].copy_from_slice(&[0xff; 6]);           // dst
    hdr[6..12].copy_from_slice(&dev.mac_addr());      // src
    hdr[12..14].copy_from_slice(&0x88B5u16.to_be_bytes()); // ethertype

    skb.set_protocol(0x88B5);
    skb.send()?;
    Ok(())
}
```

## Implementation Plan

### Phase 1: Bindings — skb and netdev

1. Test `linux/skbuff.h` and `linux/netdevice.h` traverse cascades
   via `rko-sys-gen` — determine inject_type sizes with clang:
   ```c
   printf("sk_buff: size=%zu align=%zu\n", sizeof(struct sk_buff), _Alignof(struct sk_buff));
   printf("net_device: size=%zu align=%zu\n", sizeof(struct net_device), _Alignof(struct net_device));
   ```
2. Add `rko.skb` and `rko.netdev` partitions to `rko-sys-gen/rko.toml`
   (with inject_types if cascades are too large)
3. Add C helpers for inline functions to `helpers.{c,h}`
4. Regenerate bindings: `cargo run -p rko-sys-gen -- rko-sys-gen/rko.toml`
5. Add `skb` and `netdev` features to `rko-sys/Cargo.toml`
6. Verify: `cargo check -p rko-sys --features skb,netdev`

### Phase 2: Safe wrappers — TX path

1. Create `rko-core/src/rawnet/mod.rs` with submodules
2. Implement `SkBuff` (alloc, reserve, put, push, send, drop)
3. Implement `NetDevice` (get_by_name, mac_addr, mtu)
4. Implement `EthFrame` builder
5. Add `pub mod rawnet;` to `rko-core/src/lib.rs`
6. Verify: `cargo check -p rko-core`

### Phase 3: Sample — raw TX

1. Create `samples/raw_tx/` — module that sends a broadcast Ethernet
   frame on init and logs it
2. QEMU test with tcpdump/packet capture to verify the frame reaches
   the virtual NIC
3. CMake target: `cmake --build build --target raw_tx_ko_test`

### Phase 4: RX path

1. Implement `PacketHook` and `PacketHandler` trait
2. Implement the `packet_type` C trampoline
3. Test: module that registers a handler, counts received frames,
   logs via `pr_info!`
4. QEMU test: ping from host → guest, verify module sees the frames

### Phase 5: Direct xmit and advanced features

1. Implement `send_direct()` via `__dev_direct_xmit`
2. Add `skb_clone()` support for packet duplication
3. Add per-device RX handler (`rx_handler_register`) as opt-in
   GPL-only feature
4. Performance test: measure TX latency with direct vs queued path

## Open Questions

1. **`sk_buff` inject size**: The exact `sizeof(struct sk_buff)` varies
   with kernel config (`CONFIG_NET_SCHED`, `CONFIG_XFRM`, etc.). Need
   to measure on the rko build's kernel config specifically.

2. **`net_device` lifetime**: `__dev_get_by_name` returns a pointer
   without incrementing the refcount. Should the wrapper call
   `dev_hold()`/`dev_put()` for safety? This adds overhead but
   prevents use-after-free if the device is removed while the module
   holds a reference.

3. **`packet_type` callback context**: The `dev_add_pack` callback
   runs in softirq context (NAPI). Must ensure the Rust handler
   doesn't sleep or allocate with `GFP_KERNEL`. Need to document
   and enforce this constraint.

4. **Multi-queue awareness**: `dev_direct_xmit` requires a queue
   index. Should the wrapper expose queue selection, or auto-select
   based on CPU/hash?

5. **Scatter-gather TX**: Advanced NICs support scatter-gather DMA
   via skb frags. The initial API only supports linear skbs. Should
   frag support be Phase 5+?

6. **`init_net` vs current namespace**: Device lookup currently uses
   `init_net` (the root network namespace). Should the API support
   looking up devices in other namespaces?

## Future Work

- **Scatter-gather TX**: `skb_add_frag()` for zero-copy from
  kernel pages (e.g., page cache → NIC without memcpy)
- **Checksum offload**: `skb->ip_summed` for hardware checksum
- **GSO/GRO**: Generic segmentation/receive offload for high throughput
- **VLAN support**: `vlan_insert_tag()` / VLAN-aware frame builder
- **Netfilter hooks**: `nf_register_net_hook()` for packet filtering
  at L3/L4 (complementary to raw L2 access)
- **XDP from Rust**: If eBPF toolchains support Rust compilation to
  BPF bytecode, XDP programs could be written in Rust (external to
  rko, via aya or similar)

## References

- Kernel source: `linux/include/linux/skbuff.h`, `linux/include/linux/netdevice.h`
- Kernel docs: [struct sk_buff](https://docs.kernel.org/networking/skbuff.html)
- Kernel docs: [Network Devices](https://docs.kernel.org/networking/netdevices.html)
- Kernel docs: [Softnet Driver Issues](https://docs.kernel.org/networking/driver.html)
- Kernel docs: [AF_XDP](https://docs.kernel.org/networking/af_xdp.html)
- Linux Foundation: [Kernel Flow Diagram](https://wiki.linuxfoundation.org/networking/kernel_flow)
- rko networking design: `docs/design/features/networking.md`
- rko bindings guide: `docs/guides/adding-bindings.md`
