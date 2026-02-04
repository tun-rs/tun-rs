/*!
# Linux Offload Support Module

This module provides Generic Receive Offload (GRO) and Generic Segmentation Offload (GSO)
support for Linux TUN devices, significantly improving throughput for TCP and UDP traffic.

## Overview

Modern network cards and drivers use offload techniques to reduce CPU overhead:

- **GSO (Generic Segmentation Offload)**: Allows sending large packets that are segmented by
  the kernel/driver, reducing per-packet processing overhead.

- **GRO (Generic Receive Offload)**: Coalesces multiple received packets into larger segments,
  reducing the number of packets passed to the application.

This module implements GRO/GSO for TUN devices using the `virtio_net` header format, compatible
with the Linux kernel's TUN/TAP driver offload capabilities.

## Performance Benefits

Enabling offload can provide:
- 2-10x improvement in throughput for TCP traffic
- Reduced CPU usage per gigabit of traffic
- Better handling of high-bandwidth applications

The actual improvement depends on:
- Packet sizes
- TCP window sizes
- Network round-trip time
- CPU capabilities

## Usage

Enable offload when building a device:

```no_run
# #[cfg(target_os = "linux")]
# {
use tun_rs::{DeviceBuilder, GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

let dev = DeviceBuilder::new()
    .offload(true)  // Enable offload
    .ipv4("10.0.0.1", 24, None)
    .build_sync()?;

// Allocate buffers for batch operations
let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
let mut sizes = vec![0; IDEAL_BATCH_SIZE];

// Create GRO table for coalescing
let mut gro_table = GROTable::default();

loop {
    // Receive multiple packets at once
    let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;

    for i in 0..num {
        // Process each packet
        println!("Packet {}: {} bytes", i, sizes[i]);
    }
}
# }
# Ok::<(), std::io::Error>(())
```

## Key Types

- [`VirtioNetHdr`]: Header structure for virtio network offload
- [`GROTable`]: Manages TCP and UDP flow coalescing for GRO
- [`TcpGROTable`]: TCP-specific GRO state
- [`UdpGROTable`]: UDP-specific GRO state

## Key Functions

- [`handle_gro`]: Process received packets and perform GRO coalescing
- [`gso_split`]: Split a GSO packet into multiple segments
- [`apply_tcp_coalesce_accounting`]: Update TCP headers after coalescing

## Constants

- [`VIRTIO_NET_HDR_LEN`]: Size of the virtio network header (12 bytes)
- [`IDEAL_BATCH_SIZE`]: Recommended batch size for packet operations (128)
- [`VIRTIO_NET_HDR_GSO_NONE`], [`VIRTIO_NET_HDR_GSO_TCPV4`], etc.: GSO type constants

## References

- [Linux virtio_net.h](https://github.com/torvalds/linux/blob/master/include/uapi/linux/virtio_net.h)
- [WireGuard-go offload implementation](https://github.com/WireGuard/wireguard-go/blob/master/tun/offload_linux.go)

## Platform Requirements

- Linux kernel with TUN/TAP driver
- Kernel support for IFF_VNET_HDR (available since Linux 2.6.32)
- Root privileges to create TUN devices with offload enabled
*/

/// https://github.com/WireGuard/wireguard-go/blob/master/tun/offload_linux.go
use crate::platform::linux::checksum::{checksum, pseudo_header_checksum_no_fold};
use byteorder::{BigEndian, ByteOrder};
use bytes::BytesMut;
use libc::{IPPROTO_TCP, IPPROTO_UDP};
use std::collections::HashMap;
use std::io;

/// GSO type: Not a GSO frame (normal packet).
///
/// This indicates a regular packet without Generic Segmentation Offload applied.
/// See: <https://github.com/torvalds/linux/blob/master/include/uapi/linux/virtio_net.h>
pub const VIRTIO_NET_HDR_GSO_NONE: u8 = 0;

/// Flag: Use csum_start and csum_offset fields for checksum calculation.
///
/// When this flag is set, the packet requires checksum calculation.
/// The `csum_start` field indicates where checksumming should begin,
/// and `csum_offset` indicates where to write the checksum.
pub const VIRTIO_NET_HDR_F_NEEDS_CSUM: u8 = 1;

/// GSO type: IPv4 TCP segmentation (TSO - TCP Segmentation Offload).
///
/// Large TCP packets can be sent and will be segmented by the kernel/driver.
pub const VIRTIO_NET_HDR_GSO_TCPV4: u8 = 1;

/// GSO type: IPv6 TCP segmentation (TSO).
///
/// Similar to TCPV4 but for IPv6 packets.
pub const VIRTIO_NET_HDR_GSO_TCPV6: u8 = 4;

/// GSO type: UDP segmentation for IPv4 and IPv6 (USO - UDP Segmentation Offload).
///
/// Available in newer Linux kernels for UDP packet segmentation.
pub const VIRTIO_NET_HDR_GSO_UDP_L4: u8 = 5;

/// Recommended batch size for packet operations with offload.
///
/// This constant defines the optimal number of packets to handle per `recv_multiple`
/// or `send_multiple` call. It balances between:
/// - Amortizing system call overhead
/// - Keeping latency reasonable  
/// - Memory usage for packet buffers
///
/// Based on WireGuard-go's implementation.
///
/// # Example
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::IDEAL_BATCH_SIZE;
///
/// // Allocate buffers for batch operations
/// let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
/// let mut sizes = vec![0; IDEAL_BATCH_SIZE];
/// # }
/// ```
///
/// See: <https://github.com/WireGuard/wireguard-go/blob/master/conn/conn.go#L19>
pub const IDEAL_BATCH_SIZE: usize = 128;

const TCP_FLAGS_OFFSET: usize = 13;

const TCP_FLAG_FIN: u8 = 0x01;
const TCP_FLAG_PSH: u8 = 0x08;
const TCP_FLAG_ACK: u8 = 0x10;

/// Virtio network header for offload support.
///
/// This structure precedes each packet when offload is enabled on a Linux TUN device.
/// It provides metadata about Generic Segmentation Offload (GSO) and checksum requirements,
/// allowing the kernel to perform hardware-accelerated operations.
///
/// The header matches the Linux kernel's `virtio_net_hdr` structure defined in
/// `include/uapi/linux/virtio_net.h`.
///
/// # Memory Layout
///
/// The structure is `#[repr(C)]` and has a fixed size of 12 bytes ([`VIRTIO_NET_HDR_LEN`]).
/// All multi-byte fields are in native endianness.
///
/// # Usage
///
/// When reading from a TUN device with offload enabled:
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::{VirtioNetHdr, VIRTIO_NET_HDR_LEN};
///
/// let mut buf = vec![0u8; VIRTIO_NET_HDR_LEN + 1500];
/// // let n = dev.recv(&mut buf)?;
///
/// // Decode the header
/// // let hdr = VirtioNetHdr::decode(&buf[..VIRTIO_NET_HDR_LEN])?;
/// // let packet = &buf[VIRTIO_NET_HDR_LEN..n];
/// # }
/// ```
///
/// # Fields
///
/// - `flags`: Bit flags for header processing (e.g., [`VIRTIO_NET_HDR_F_NEEDS_CSUM`])
/// - `gso_type`: Type of GSO applied (e.g., [`VIRTIO_NET_HDR_GSO_TCPV4`])
/// - `hdr_len`: Length of packet headers (Ethernet + IP + TCP/UDP)
/// - `gso_size`: Maximum segment size for GSO
/// - `csum_start`: Offset to start checksum calculation
/// - `csum_offset`: Offset within checksum area to store the checksum
///
/// # References
///
/// - [Linux virtio_net.h](https://github.com/torvalds/linux/blob/master/include/uapi/linux/virtio_net.h)
///
/// See: <https://github.com/torvalds/linux/blob/master/include/uapi/linux/virtio_net.h>
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct VirtioNetHdr {
    // #define VIRTIO_NET_HDR_F_NEEDS_CSUM	1	/* Use csum_start, csum_offset */
    // #define VIRTIO_NET_HDR_F_DATA_VALID	2	/* Csum is valid */
    // #define VIRTIO_NET_HDR_F_RSC_INFO	4	/* rsc info in csum_ fields */
    pub flags: u8,
    // #define VIRTIO_NET_HDR_GSO_NONE		0	/* Not a GSO frame */
    // #define VIRTIO_NET_HDR_GSO_TCPV4	1	/* GSO frame, IPv4 TCP (TSO) */
    // #define VIRTIO_NET_HDR_GSO_UDP		3	/* GSO frame, IPv4 UDP (UFO) */
    // #define VIRTIO_NET_HDR_GSO_TCPV6	4	/* GSO frame, IPv6 TCP */
    // #define VIRTIO_NET_HDR_GSO_UDP_L4	5	/* GSO frame, IPv4& IPv6 UDP (USO) */
    // #define VIRTIO_NET_HDR_GSO_ECN		0x80	/* TCP has ECN set */
    pub gso_type: u8,
    // Ethernet + IP + tcp/udp hdrs
    pub hdr_len: u16,
    // Bytes to append to hdr_len per frame
    pub gso_size: u16,
    // Checksum calculation
    pub csum_start: u16,
    pub csum_offset: u16,
}

impl VirtioNetHdr {
    /// Decode a virtio network header from a byte buffer.
    ///
    /// Reads the first [`VIRTIO_NET_HDR_LEN`] bytes from the buffer and interprets
    /// them as a `VirtioNetHdr` structure.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer is too short (less than [`VIRTIO_NET_HDR_LEN`] bytes).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "linux")]
    /// # {
    /// use tun_rs::{VirtioNetHdr, VIRTIO_NET_HDR_LEN};
    ///
    /// let buffer = vec![0u8; VIRTIO_NET_HDR_LEN + 1500];
    /// let header = VirtioNetHdr::decode(&buffer)?;
    /// println!("GSO type: {:?}", header.gso_type);
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn decode(buf: &[u8]) -> io::Result<VirtioNetHdr> {
        if buf.len() < VIRTIO_NET_HDR_LEN {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "too short"));
        }
        let mut hdr = std::mem::MaybeUninit::<VirtioNetHdr>::uninit();
        unsafe {
            // Safety:
            // hdr is written by `buf`, both pointers satisfy the alignment requirement of `u8`
            std::ptr::copy_nonoverlapping(
                buf.as_ptr(),
                hdr.as_mut_ptr() as *mut _,
                std::mem::size_of::<VirtioNetHdr>(),
            );
            Ok(hdr.assume_init())
        }
    }

    /// Encode a virtio network header into a byte buffer.
    ///
    /// Writes this header into the first [`VIRTIO_NET_HDR_LEN`] bytes of the buffer.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer is too short (less than [`VIRTIO_NET_HDR_LEN`] bytes).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "linux")]
    /// # {
    /// use tun_rs::{VirtioNetHdr, VIRTIO_NET_HDR_GSO_NONE, VIRTIO_NET_HDR_LEN};
    ///
    /// let header = VirtioNetHdr {
    ///     gso_type: VIRTIO_NET_HDR_GSO_NONE,
    ///     ..Default::default()
    /// };
    ///
    /// let mut buffer = vec![0u8; VIRTIO_NET_HDR_LEN + 1500];
    /// header.encode(&mut buffer)?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn encode(&self, buf: &mut [u8]) -> io::Result<()> {
        if buf.len() < VIRTIO_NET_HDR_LEN {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "too short"));
        }
        unsafe {
            let hdr_ptr = self as *const VirtioNetHdr as *const u8;
            std::ptr::copy_nonoverlapping(hdr_ptr, buf.as_mut_ptr(), VIRTIO_NET_HDR_LEN);
            Ok(())
        }
    }
}

/// Size of the virtio network header in bytes (12 bytes).
///
/// This constant represents the fixed size of the [`VirtioNetHdr`] structure.
/// When offload is enabled on a TUN device, this header precedes every packet.
///
/// # Example
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::VIRTIO_NET_HDR_LEN;
///
/// // Allocate buffer with space for header + packet
/// let mut buffer = vec![0u8; VIRTIO_NET_HDR_LEN + 1500];
///
/// // Header is at the start
/// // let header_bytes = &buffer[..VIRTIO_NET_HDR_LEN];
/// // Packet data follows the header
/// // let packet_data = &buffer[VIRTIO_NET_HDR_LEN..];
/// # }
/// ```
pub const VIRTIO_NET_HDR_LEN: usize = std::mem::size_of::<VirtioNetHdr>();

/// Identifier for a TCP flow used in Generic Receive Offload (GRO).
///
/// This structure uniquely identifies a TCP connection for packet coalescing.
/// Packets belonging to the same flow can be coalesced into larger segments,
/// reducing per-packet processing overhead.
///
/// # Fields
///
/// The flow is identified by:
/// - Source and destination IP addresses (IPv4 or IPv6)
/// - Source and destination ports
/// - TCP acknowledgment number (to avoid coalescing segments with different ACKs)
/// - IP version flag
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct TcpFlowKey {
    src_addr: [u8; 16],
    dst_addr: [u8; 16],
    src_port: u16,
    dst_port: u16,
    rx_ack: u32, // varying ack values should not be coalesced. Treat them as separate flows.
    is_v6: bool,
}

/// TCP Generic Receive Offload (GRO) table.
///
/// Manages the coalescing of TCP packets belonging to the same flow into larger segments.
/// This reduces the number of packets that need to be processed by the application,
/// improving throughput and reducing CPU usage.
///
/// # How TCP GRO Works
///
/// 1. Packets are received from the TUN device
/// 2. The GRO table identifies packets belonging to the same TCP flow
/// 3. Consecutive packets in the same flow are coalesced into a single large segment
/// 4. The coalesced segment is passed to the application
///
/// # Usage
///
/// The GRO table is typically used in conjunction with [`handle_gro`]:
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::GROTable;
///
/// let mut gro_table = GROTable::default();
///
/// // Process received packets
/// // handle_gro(..., &mut gro_table, ...)?;
/// # }
/// ```
///
/// # Performance Considerations
///
/// - Maintains a hash map of active flows
/// - Preallocates buffers for [`IDEAL_BATCH_SIZE`] flows
/// - Memory pooling reduces allocations
/// - State is maintained across multiple recv_multiple calls
pub struct TcpGROTable {
    items_by_flow: HashMap<TcpFlowKey, Vec<TcpGROItem>>,
    items_pool: Vec<Vec<TcpGROItem>>,
}

impl Default for TcpGROTable {
    fn default() -> Self {
        Self::new()
    }
}

impl TcpGROTable {
    fn new() -> Self {
        let mut items_pool = Vec::with_capacity(IDEAL_BATCH_SIZE);
        for _ in 0..IDEAL_BATCH_SIZE {
            items_pool.push(Vec::with_capacity(IDEAL_BATCH_SIZE));
        }
        TcpGROTable {
            items_by_flow: HashMap::with_capacity(IDEAL_BATCH_SIZE),
            items_pool,
        }
    }
}

impl TcpFlowKey {
    fn new(pkt: &[u8], src_addr_offset: usize, dst_addr_offset: usize, tcph_offset: usize) -> Self {
        let mut key = TcpFlowKey {
            src_addr: [0; 16],
            dst_addr: [0; 16],
            src_port: 0,
            dst_port: 0,
            rx_ack: 0,
            is_v6: false,
        };

        let addr_size = dst_addr_offset - src_addr_offset;
        key.src_addr[..addr_size].copy_from_slice(&pkt[src_addr_offset..dst_addr_offset]);
        key.dst_addr[..addr_size]
            .copy_from_slice(&pkt[dst_addr_offset..dst_addr_offset + addr_size]);
        key.src_port = BigEndian::read_u16(&pkt[tcph_offset..]);
        key.dst_port = BigEndian::read_u16(&pkt[tcph_offset + 2..]);
        key.rx_ack = BigEndian::read_u32(&pkt[tcph_offset + 8..]);
        key.is_v6 = addr_size == 16;
        key
    }
}

impl TcpGROTable {
    /// lookupOrInsert looks up a flow for the provided packet and metadata,
    /// returning the packets found for the flow, or inserting a new one if none
    /// is found.
    fn lookup_or_insert(
        &mut self,
        pkt: &[u8],
        src_addr_offset: usize,
        dst_addr_offset: usize,
        tcph_offset: usize,
        tcph_len: usize,
        bufs_index: usize,
    ) -> Option<&mut Vec<TcpGROItem>> {
        let key = TcpFlowKey::new(pkt, src_addr_offset, dst_addr_offset, tcph_offset);
        if self.items_by_flow.contains_key(&key) {
            return self.items_by_flow.get_mut(&key);
        }
        // Insert the new item into the table
        self.insert(
            pkt,
            src_addr_offset,
            dst_addr_offset,
            tcph_offset,
            tcph_len,
            bufs_index,
        );
        None
    }
    /// insert an item in the table for the provided packet and packet metadata.
    fn insert(
        &mut self,
        pkt: &[u8],
        src_addr_offset: usize,
        dst_addr_offset: usize,
        tcph_offset: usize,
        tcph_len: usize,
        bufs_index: usize,
    ) {
        let key = TcpFlowKey::new(pkt, src_addr_offset, dst_addr_offset, tcph_offset);
        let item = TcpGROItem {
            key,
            bufs_index: bufs_index as u16,
            num_merged: 0,
            gso_size: pkt[tcph_offset + tcph_len..].len() as u16,
            iph_len: tcph_offset as u8,
            tcph_len: tcph_len as u8,
            sent_seq: BigEndian::read_u32(&pkt[tcph_offset + 4..tcph_offset + 8]),
            psh_set: pkt[tcph_offset + TCP_FLAGS_OFFSET] & TCP_FLAG_PSH != 0,
        };

        let items = self
            .items_by_flow
            .entry(key)
            .or_insert_with(|| self.items_pool.pop().unwrap_or_default());
        items.push(item);
    }
}
// func (t *tcpGROTable) updateAt(item tcpGROItem, i int) {
// 	items, _ := t.itemsByFlow[item.key]
// 	items[i] = item
// }
//
// func (t *tcpGROTable) deleteAt(key tcpFlowKey, i int) {
// 	items, _ := t.itemsByFlow[key]
// 	items = append(items[:i], items[i+1:]...)
// 	t.itemsByFlow[key] = items
// }

/// tcpGROItem represents bookkeeping data for a TCP packet during the lifetime
/// of a GRO evaluation across a vector of packets.
#[derive(Debug, Clone, Copy)]
pub struct TcpGROItem {
    key: TcpFlowKey,
    sent_seq: u32,   // the sequence number
    bufs_index: u16, // the index into the original bufs slice
    num_merged: u16, // the number of packets merged into this item
    gso_size: u16,   // payload size
    iph_len: u8,     // ip header len
    tcph_len: u8,    // tcp header len
    psh_set: bool,   // psh flag is set
}

// func (t *tcpGROTable) newItems() []tcpGROItem {
// 	var items []tcpGROItem
// 	items, t.itemsPool = t.itemsPool[len(t.itemsPool)-1], t.itemsPool[:len(t.itemsPool)-1]
// 	return items
// }
impl TcpGROTable {
    fn reset(&mut self) {
        for (_key, mut items) in self.items_by_flow.drain() {
            items.clear();
            self.items_pool.push(items);
        }
    }
}

/// udpFlowKey represents the key for a UDP flow.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct UdpFlowKey {
    src_addr: [u8; 16], // srcAddr
    dst_addr: [u8; 16], // dstAddr
    src_port: u16,      // srcPort
    dst_port: u16,      // dstPort
    is_v6: bool,        // isV6
}

///  udpGROTable holds flow and coalescing information for the purposes of UDP GRO.
pub struct UdpGROTable {
    items_by_flow: HashMap<UdpFlowKey, Vec<UdpGROItem>>,
    items_pool: Vec<Vec<UdpGROItem>>,
}

impl Default for UdpGROTable {
    fn default() -> Self {
        UdpGROTable::new()
    }
}

impl UdpGROTable {
    pub fn new() -> Self {
        let mut items_pool = Vec::with_capacity(IDEAL_BATCH_SIZE);
        for _ in 0..IDEAL_BATCH_SIZE {
            items_pool.push(Vec::with_capacity(IDEAL_BATCH_SIZE));
        }
        UdpGROTable {
            items_by_flow: HashMap::with_capacity(IDEAL_BATCH_SIZE),
            items_pool,
        }
    }
}

impl UdpFlowKey {
    pub fn new(
        pkt: &[u8],
        src_addr_offset: usize,
        dst_addr_offset: usize,
        udph_offset: usize,
    ) -> UdpFlowKey {
        let mut key = UdpFlowKey {
            src_addr: [0; 16],
            dst_addr: [0; 16],
            src_port: 0,
            dst_port: 0,
            is_v6: false,
        };
        let addr_size = dst_addr_offset - src_addr_offset;
        key.src_addr[..addr_size].copy_from_slice(&pkt[src_addr_offset..dst_addr_offset]);
        key.dst_addr[..addr_size]
            .copy_from_slice(&pkt[dst_addr_offset..dst_addr_offset + addr_size]);
        key.src_port = BigEndian::read_u16(&pkt[udph_offset..]);
        key.dst_port = BigEndian::read_u16(&pkt[udph_offset + 2..]);
        key.is_v6 = addr_size == 16;
        key
    }
}

impl UdpGROTable {
    /// Looks up a flow for the provided packet and metadata.
    /// Returns a reference to the packets found for the flow and a boolean indicating if the flow already existed.
    /// If the flow is not found, inserts a new flow and returns `None` for the items.
    fn lookup_or_insert(
        &mut self,
        pkt: &[u8],
        src_addr_offset: usize,
        dst_addr_offset: usize,
        udph_offset: usize,
        bufs_index: usize,
    ) -> Option<&mut Vec<UdpGROItem>> {
        let key = UdpFlowKey::new(pkt, src_addr_offset, dst_addr_offset, udph_offset);
        if self.items_by_flow.contains_key(&key) {
            self.items_by_flow.get_mut(&key)
        } else {
            // If the flow does not exist, insert a new entry.
            self.insert(
                pkt,
                src_addr_offset,
                dst_addr_offset,
                udph_offset,
                bufs_index,
                false,
            );
            None
        }
    }
    /// Inserts an item in the table for the provided packet and its metadata.
    fn insert(
        &mut self,
        pkt: &[u8],
        src_addr_offset: usize,
        dst_addr_offset: usize,
        udph_offset: usize,
        bufs_index: usize,
        c_sum_known_invalid: bool,
    ) {
        let key = UdpFlowKey::new(pkt, src_addr_offset, dst_addr_offset, udph_offset);
        let item = UdpGROItem {
            key,
            bufs_index: bufs_index as u16,
            num_merged: 0,
            gso_size: (pkt.len() - (udph_offset + UDP_H_LEN)) as u16,
            iph_len: udph_offset as u8,
            c_sum_known_invalid,
        };
        let items = self
            .items_by_flow
            .entry(key)
            .or_insert_with(|| self.items_pool.pop().unwrap_or_default());
        items.push(item);
    }
}
// func (u *udpGROTable) updateAt(item udpGROItem, i int) {
// 	items, _ := u.itemsByFlow[item.key]
// 	items[i] = item
// }

/// udpGROItem represents bookkeeping data for a UDP packet during the lifetime
/// of a GRO evaluation across a vector of packets.
#[derive(Debug, Clone, Copy)]
pub struct UdpGROItem {
    key: UdpFlowKey,           // udpFlowKey
    bufs_index: u16,           // the index into the original bufs slice
    num_merged: u16,           // the number of packets merged into this item
    gso_size: u16,             // payload size
    iph_len: u8,               // ip header len
    c_sum_known_invalid: bool, // UDP header checksum validity; a false value DOES NOT imply valid, just unknown.
}
// func (u *udpGROTable) newItems() []udpGROItem {
// 	var items []udpGROItem
// 	items, u.itemsPool = u.itemsPool[len(u.itemsPool)-1], u.itemsPool[:len(u.itemsPool)-1]
// 	return items
// }

impl UdpGROTable {
    fn reset(&mut self) {
        for (_key, mut items) in self.items_by_flow.drain() {
            items.clear();
            self.items_pool.push(items);
        }
    }
}

/// canCoalesce represents the outcome of checking if two TCP packets are
/// candidates for coalescing.
#[derive(Copy, Clone, Eq, PartialEq)]
enum CanCoalesce {
    Prepend,
    Unavailable,
    Append,
}

/// ipHeadersCanCoalesce returns true if the IP headers found in pktA and pktB
/// meet all requirements to be merged as part of a GRO operation, otherwise it
/// returns false.
fn ip_headers_can_coalesce(pkt_a: &[u8], pkt_b: &[u8]) -> bool {
    if pkt_a.len() < 9 || pkt_b.len() < 9 {
        return false;
    }

    if pkt_a[0] >> 4 == 6 {
        if pkt_a[0] != pkt_b[0] || pkt_a[1] >> 4 != pkt_b[1] >> 4 {
            // cannot coalesce with unequal Traffic class values
            return false;
        }
        if pkt_a[7] != pkt_b[7] {
            // cannot coalesce with unequal Hop limit values
            return false;
        }
    } else {
        if pkt_a[1] != pkt_b[1] {
            // cannot coalesce with unequal ToS values
            return false;
        }
        if pkt_a[6] >> 5 != pkt_b[6] >> 5 {
            // cannot coalesce with unequal DF or reserved bits. MF is checked
            // further up the stack.
            return false;
        }
        if pkt_a[8] != pkt_b[8] {
            // cannot coalesce with unequal TTL values
            return false;
        }
    }

    true
}

/// udpPacketsCanCoalesce evaluates if pkt can be coalesced with the packet
/// described by item. iphLen and gsoSize describe pkt. bufs is the vector of
/// packets involved in the current GRO evaluation. bufsOffset is the offset at
/// which packet data begins within bufs.
fn udp_packets_can_coalesce<B: ExpandBuffer>(
    pkt: &[u8],
    iph_len: u8,
    gso_size: u16,
    item: &UdpGROItem,
    bufs: &[B],
    bufs_offset: usize,
) -> CanCoalesce {
    let pkt_target = &bufs[item.bufs_index as usize].as_ref()[bufs_offset..];
    if !ip_headers_can_coalesce(pkt, pkt_target) {
        return CanCoalesce::Unavailable;
    }
    if (pkt_target[(iph_len as usize + UDP_H_LEN)..].len()) % (item.gso_size as usize) != 0 {
        // A smaller than gsoSize packet has been appended previously.
        // Nothing can come after a smaller packet on the end.
        return CanCoalesce::Unavailable;
    }
    if gso_size > item.gso_size {
        // We cannot have a larger packet following a smaller one.
        return CanCoalesce::Unavailable;
    }
    CanCoalesce::Append
}

/// tcpPacketsCanCoalesce evaluates if pkt can be coalesced with the packet
/// described by item. This function makes considerations that match the kernel's
/// GRO self tests, which can be found in tools/testing/selftests/net/gro.c.
#[allow(clippy::too_many_arguments)]
fn tcp_packets_can_coalesce<B: ExpandBuffer>(
    pkt: &[u8],
    iph_len: u8,
    tcph_len: u8,
    seq: u32,
    psh_set: bool,
    gso_size: u16,
    item: &TcpGROItem,
    bufs: &[B],
    bufs_offset: usize,
) -> CanCoalesce {
    let pkt_target = &bufs[item.bufs_index as usize].as_ref()[bufs_offset..];

    if tcph_len != item.tcph_len {
        // cannot coalesce with unequal tcp options len
        return CanCoalesce::Unavailable;
    }

    if tcph_len > 20
        && pkt[iph_len as usize + 20..iph_len as usize + tcph_len as usize]
            != pkt_target[item.iph_len as usize + 20..item.iph_len as usize + tcph_len as usize]
    {
        // cannot coalesce with unequal tcp options
        return CanCoalesce::Unavailable;
    }

    if !ip_headers_can_coalesce(pkt, pkt_target) {
        return CanCoalesce::Unavailable;
    }

    // seq adjacency
    let mut lhs_len = item.gso_size as usize;
    lhs_len += (item.num_merged as usize) * (item.gso_size as usize);

    if seq == item.sent_seq.wrapping_add(lhs_len as u32) {
        // pkt aligns following item from a seq num perspective
        if item.psh_set {
            // We cannot append to a segment that has the PSH flag set, PSH
            // can only be set on the final segment in a reassembled group.
            return CanCoalesce::Unavailable;
        }

        if pkt_target[iph_len as usize + tcph_len as usize..].len() % item.gso_size as usize != 0 {
            // A smaller than gsoSize packet has been appended previously.
            // Nothing can come after a smaller packet on the end.
            return CanCoalesce::Unavailable;
        }

        if gso_size > item.gso_size {
            // We cannot have a larger packet following a smaller one.
            return CanCoalesce::Unavailable;
        }

        return CanCoalesce::Append;
    } else if seq.wrapping_add(gso_size as u32) == item.sent_seq {
        // pkt aligns in front of item from a seq num perspective
        if psh_set {
            // We cannot prepend with a segment that has the PSH flag set, PSH
            // can only be set on the final segment in a reassembled group.
            return CanCoalesce::Unavailable;
        }

        if gso_size < item.gso_size {
            // We cannot have a larger packet following a smaller one.
            return CanCoalesce::Unavailable;
        }

        if gso_size > item.gso_size && item.num_merged > 0 {
            // There's at least one previous merge, and we're larger than all
            // previous. This would put multiple smaller packets on the end.
            return CanCoalesce::Unavailable;
        }

        return CanCoalesce::Prepend;
    }

    CanCoalesce::Unavailable
}

fn checksum_valid(pkt: &[u8], iph_len: u8, proto: u8, is_v6: bool) -> bool {
    let (src_addr_at, addr_size) = if is_v6 {
        (IPV6_SRC_ADDR_OFFSET, 16)
    } else {
        (IPV4_SRC_ADDR_OFFSET, 4)
    };

    let len_for_pseudo = (pkt.len() as u16).saturating_sub(iph_len as u16);

    let c_sum = pseudo_header_checksum_no_fold(
        proto,
        &pkt[src_addr_at..src_addr_at + addr_size],
        &pkt[src_addr_at + addr_size..src_addr_at + addr_size * 2],
        len_for_pseudo,
    );

    !checksum(&pkt[iph_len as usize..], c_sum) == 0
}

/// coalesceResult represents the result of attempting to coalesce two TCP
/// packets.
enum CoalesceResult {
    InsufficientCap,
    PSHEnding,
    ItemInvalidCSum,
    PktInvalidCSum,
    Success,
}

/// coalesceUDPPackets attempts to coalesce pkt with the packet described by
/// item, and returns the outcome.
fn coalesce_udp_packets<B: ExpandBuffer>(
    pkt: &[u8],
    item: &mut UdpGROItem,
    bufs: &mut [B],
    bufs_offset: usize,
    is_v6: bool,
) -> CoalesceResult {
    let buf = bufs[item.bufs_index as usize].as_ref();
    // let pkt_head = &buf[bufs_offset..]; // the packet that will end up at the front
    let headers_len = item.iph_len as usize + UDP_H_LEN;
    let coalesced_len = buf[bufs_offset..].len() + pkt.len() - headers_len;
    if bufs[item.bufs_index as usize].buf_capacity() < bufs_offset * 2 + coalesced_len {
        // We don't want to allocate a new underlying array if capacity is
        // too small.
        return CoalesceResult::InsufficientCap;
    }

    if item.num_merged == 0
        && (item.c_sum_known_invalid
            || !checksum_valid(&buf[bufs_offset..], item.iph_len, IPPROTO_UDP as _, is_v6))
    {
        return CoalesceResult::ItemInvalidCSum;
    }

    if !checksum_valid(pkt, item.iph_len, IPPROTO_UDP as _, is_v6) {
        return CoalesceResult::PktInvalidCSum;
    }
    bufs[item.bufs_index as usize].buf_extend_from_slice(&pkt[headers_len..]);
    item.num_merged += 1;
    CoalesceResult::Success
}

/// coalesceTCPPackets attempts to coalesce pkt with the packet described by
/// item, and returns the outcome. This function may swap bufs elements in the
/// event of a prepend as item's bufs index is already being tracked for writing
/// to a Device.
#[allow(clippy::too_many_arguments)]
fn coalesce_tcp_packets<B: ExpandBuffer>(
    mode: CanCoalesce,
    pkt: &[u8],
    pkt_bufs_index: usize,
    gso_size: u16,
    seq: u32,
    psh_set: bool,
    item: &mut TcpGROItem,
    bufs: &mut [B],
    bufs_offset: usize,
    is_v6: bool,
) -> CoalesceResult {
    let pkt_head: &[u8]; // the packet that will end up at the front
    let headers_len = (item.iph_len + item.tcph_len) as usize;
    let coalesced_len =
        bufs[item.bufs_index as usize].as_ref()[bufs_offset..].len() + pkt.len() - headers_len;
    // Copy data
    if mode == CanCoalesce::Prepend {
        pkt_head = pkt;
        if bufs[pkt_bufs_index].buf_capacity() < 2 * bufs_offset + coalesced_len {
            // We don't want to allocate a new underlying array if capacity is
            // too small.
            return CoalesceResult::InsufficientCap;
        }
        if psh_set {
            return CoalesceResult::PSHEnding;
        }
        if item.num_merged == 0
            && !checksum_valid(
                &bufs[item.bufs_index as usize].as_ref()[bufs_offset..],
                item.iph_len,
                IPPROTO_TCP as _,
                is_v6,
            )
        {
            return CoalesceResult::ItemInvalidCSum;
        }
        if !checksum_valid(pkt, item.iph_len, IPPROTO_TCP as _, is_v6) {
            return CoalesceResult::PktInvalidCSum;
        }
        item.sent_seq = seq;
        let extend_by = coalesced_len - pkt_head.len();
        let len = bufs[pkt_bufs_index].as_ref().len();
        bufs[pkt_bufs_index].buf_resize(len + extend_by, 0);
        let src = bufs[item.bufs_index as usize].as_ref()[bufs_offset + headers_len..].as_ptr();
        let dst = bufs[pkt_bufs_index].as_mut()[bufs_offset + pkt.len()..].as_mut_ptr();
        unsafe {
            std::ptr::copy_nonoverlapping(src, dst, extend_by);
        }
        // Flip the slice headers in bufs as part of prepend. The index of item
        // is already being tracked for writing.
        bufs.swap(item.bufs_index as usize, pkt_bufs_index);
    } else {
        // pkt_head = &bufs[item.bufs_index as usize][bufs_offset..];
        if bufs[item.bufs_index as usize].buf_capacity() < 2 * bufs_offset + coalesced_len {
            // We don't want to allocate a new underlying array if capacity is
            // too small.
            return CoalesceResult::InsufficientCap;
        }
        if item.num_merged == 0
            && !checksum_valid(
                &bufs[item.bufs_index as usize].as_ref()[bufs_offset..],
                item.iph_len,
                IPPROTO_TCP as _,
                is_v6,
            )
        {
            return CoalesceResult::ItemInvalidCSum;
        }
        if !checksum_valid(pkt, item.iph_len, IPPROTO_TCP as _, is_v6) {
            return CoalesceResult::PktInvalidCSum;
        }
        if psh_set {
            // We are appending a segment with PSH set.
            item.psh_set = psh_set;
            bufs[item.bufs_index as usize].as_mut()
                [bufs_offset + item.iph_len as usize + TCP_FLAGS_OFFSET] |= TCP_FLAG_PSH;
        }
        // https://github.com/WireGuard/wireguard-go/blob/12269c2761734b15625017d8565745096325392f/tun/offload_linux.go#L495
        // extendBy := len(pkt) - int(headersLen)
        // 		bufs[item.bufsIndex] = append(bufs[item.bufsIndex], make([]byte, extendBy)...)
        // 		copy(bufs[item.bufsIndex][bufsOffset+len(pktHead):], pkt[headersLen:])
        bufs[item.bufs_index as usize].buf_extend_from_slice(&pkt[headers_len..]);
    }

    if gso_size > item.gso_size {
        item.gso_size = gso_size;
    }

    item.num_merged += 1;
    CoalesceResult::Success
}

const IPV4_FLAG_MORE_FRAGMENTS: u8 = 0x20;

const IPV4_SRC_ADDR_OFFSET: usize = 12;
const IPV6_SRC_ADDR_OFFSET: usize = 8;
// maxUint16         = 1<<16 - 1

#[derive(PartialEq, Eq)]
enum GroResult {
    Noop,
    TableInsert,
    Coalesced,
}

/// tcpGRO evaluates the TCP packet at pktI in bufs for coalescing with
/// existing packets tracked in table. It returns a groResultNoop when no
/// action was taken, groResultTableInsert when the evaluated packet was
/// inserted into table, and groResultCoalesced when the evaluated packet was
/// coalesced with another packet in table.
fn tcp_gro<B: ExpandBuffer>(
    bufs: &mut [B],
    offset: usize,
    pkt_i: usize,
    table: &mut TcpGROTable,
    is_v6: bool,
) -> GroResult {
    let pkt = unsafe { &*(&bufs[pkt_i].as_ref()[offset..] as *const [u8]) };
    if pkt.len() > u16::MAX as usize {
        // A valid IPv4 or IPv6 packet will never exceed this.
        return GroResult::Noop;
    }

    let mut iph_len = ((pkt[0] & 0x0F) * 4) as usize;
    if is_v6 {
        iph_len = 40;
        let ipv6_h_payload_len = u16::from_be_bytes([pkt[4], pkt[5]]) as usize;
        if ipv6_h_payload_len != pkt.len() - iph_len {
            return GroResult::Noop;
        }
    } else {
        let total_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
        if total_len != pkt.len() {
            return GroResult::Noop;
        }
    }

    if pkt.len() < iph_len {
        return GroResult::Noop;
    }

    let tcph_len = ((pkt[iph_len + 12] >> 4) * 4) as usize;
    if !(20..=60).contains(&tcph_len) {
        return GroResult::Noop;
    }

    if pkt.len() < iph_len + tcph_len {
        return GroResult::Noop;
    }

    if !is_v6 && (pkt[6] & IPV4_FLAG_MORE_FRAGMENTS != 0 || pkt[6] << 3 != 0 || pkt[7] != 0) {
        // no GRO support for fragmented segments for now
        return GroResult::Noop;
    }

    let tcp_flags = pkt[iph_len + TCP_FLAGS_OFFSET];
    let mut psh_set = false;

    // not a candidate if any non-ACK flags (except PSH+ACK) are set
    if tcp_flags != TCP_FLAG_ACK {
        if pkt[iph_len + TCP_FLAGS_OFFSET] != TCP_FLAG_ACK | TCP_FLAG_PSH {
            return GroResult::Noop;
        }
        psh_set = true;
    }

    let gso_size = (pkt.len() - tcph_len - iph_len) as u16;
    // not a candidate if payload len is 0
    if gso_size < 1 {
        return GroResult::Noop;
    }

    let seq = u32::from_be_bytes([
        pkt[iph_len + 4],
        pkt[iph_len + 5],
        pkt[iph_len + 6],
        pkt[iph_len + 7],
    ]);

    let mut src_addr_offset = IPV4_SRC_ADDR_OFFSET;
    let mut addr_len = 4;
    if is_v6 {
        src_addr_offset = IPV6_SRC_ADDR_OFFSET;
        addr_len = 16;
    }

    let items = if let Some(items) = table.lookup_or_insert(
        pkt,
        src_addr_offset,
        src_addr_offset + addr_len,
        iph_len,
        tcph_len,
        pkt_i,
    ) {
        items
    } else {
        return GroResult::TableInsert;
    };

    for i in (0..items.len()).rev() {
        // In the best case of packets arriving in order iterating in reverse is
        // more efficient if there are multiple items for a given flow. This
        // also enables a natural table.delete_at() in the
        // coalesce_item_invalid_csum case without the need for index tracking.
        // This algorithm makes a best effort to coalesce in the event of
        // unordered packets, where pkt may land anywhere in items from a
        // sequence number perspective, however once an item is inserted into
        // the table it is never compared across other items later.
        let item = &mut items[i];
        let can = tcp_packets_can_coalesce(
            pkt,
            iph_len as u8,
            tcph_len as u8,
            seq,
            psh_set,
            gso_size,
            item,
            bufs,
            offset,
        );

        match can {
            CanCoalesce::Unavailable => {}
            _ => {
                let result = coalesce_tcp_packets(
                    can, pkt, pkt_i, gso_size, seq, psh_set, item, bufs, offset, is_v6,
                );

                match result {
                    CoalesceResult::Success => {
                        // table.update_at(item, i);
                        return GroResult::Coalesced;
                    }
                    CoalesceResult::ItemInvalidCSum => {
                        // delete the item with an invalid csum
                        // table.delete_at(item.key, i);
                        items.remove(i);
                    }
                    CoalesceResult::PktInvalidCSum => {
                        // no point in inserting an item that we can't coalesce
                        return GroResult::Noop;
                    }
                    _ => {}
                }
            }
        }
    }

    // failed to coalesce with any other packets; store the item in the flow
    table.insert(
        pkt,
        src_addr_offset,
        src_addr_offset + addr_len,
        iph_len,
        tcph_len,
        pkt_i,
    );
    GroResult::TableInsert
}

/// Update packet headers after TCP packet coalescing.
///
/// After [`handle_gro`] coalesces multiple TCP packets into larger segments,
/// this function updates the packet headers to reflect the coalesced state.
/// It writes virtio headers with GSO information and updates IP/TCP headers.
///
/// # Arguments
///
/// * `bufs` - Mutable slice of packet buffers that were processed by GRO
/// * `offset` - Offset where packet data begins (typically [`VIRTIO_NET_HDR_LEN`])
/// * `table` - The TCP GRO table containing coalescing metadata
///
/// # What It Does
///
/// For each coalesced packet:
/// 1. Creates a virtio header with GSO type set to TCP (v4 or v6)
/// 2. Sets the segment size (`gso_size`) for future segmentation
/// 3. Calculates and stores the pseudo-header checksum for TCP
/// 4. Updates IP total length field
/// 5. Recalculates IPv4 header checksum if needed
///
/// The resulting packets can be efficiently segmented by the kernel when transmitted.
///
/// # Usage
///
/// This function is typically called automatically by [`handle_gro`] after packet
/// coalescing is complete. You usually don't need to call it directly.
///
/// # Errors
///
/// Returns an error if:
/// - Buffer sizes are incorrect
/// - Header encoding fails
/// - Packet structure is invalid
///
/// # See Also
///
/// - [`handle_gro`] - Main GRO processing function that calls this
/// - [`TcpGROTable`] - Maintains TCP flow state for coalescing
pub fn apply_tcp_coalesce_accounting<B: ExpandBuffer>(
    bufs: &mut [B],
    offset: usize,
    table: &TcpGROTable,
) -> io::Result<()> {
    for items in table.items_by_flow.values() {
        for item in items {
            if item.num_merged > 0 {
                let mut hdr = VirtioNetHdr {
                    flags: VIRTIO_NET_HDR_F_NEEDS_CSUM,
                    hdr_len: (item.iph_len + item.tcph_len) as u16,
                    gso_size: item.gso_size,
                    csum_start: item.iph_len as u16,
                    csum_offset: 16,
                    gso_type: 0, // Will be set later
                };
                let buf = bufs[item.bufs_index as usize].as_mut();
                let pkt = &mut buf[offset..];
                let pkt_len = pkt.len();

                // Calculate the pseudo header checksum and place it at the TCP
                // checksum offset. Downstream checksum offloading will combine
                // this with computation of the tcp header and payload checksum.
                let addr_len = if item.key.is_v6 { 16 } else { 4 };
                let src_addr_at = if item.key.is_v6 {
                    IPV6_SRC_ADDR_OFFSET
                } else {
                    IPV4_SRC_ADDR_OFFSET
                };

                let src_addr =
                    unsafe { &*(&pkt[src_addr_at..src_addr_at + addr_len] as *const [u8]) };
                let dst_addr = unsafe {
                    &*(&pkt[src_addr_at + addr_len..src_addr_at + addr_len * 2] as *const [u8])
                };
                // Recalculate the total len (IPv4) or payload len (IPv6).
                // Recalculate the (IPv4) header checksum.
                if item.key.is_v6 {
                    hdr.gso_type = VIRTIO_NET_HDR_GSO_TCPV6;
                    BigEndian::write_u16(&mut pkt[4..6], pkt_len as u16 - item.iph_len as u16);
                } else {
                    hdr.gso_type = VIRTIO_NET_HDR_GSO_TCPV4;
                    pkt[10] = 0;
                    pkt[11] = 0;
                    BigEndian::write_u16(&mut pkt[2..4], pkt_len as u16);
                    let iph_csum = !checksum(&pkt[..item.iph_len as usize], 0);
                    BigEndian::write_u16(&mut pkt[10..12], iph_csum);
                }

                hdr.encode(&mut buf[offset - VIRTIO_NET_HDR_LEN..])?;

                let pkt = &mut buf[offset..];

                let psum = pseudo_header_checksum_no_fold(
                    IPPROTO_TCP as _,
                    src_addr,
                    dst_addr,
                    pkt_len as u16 - item.iph_len as u16,
                );
                let tcp_csum = checksum(&[], psum);
                BigEndian::write_u16(
                    &mut pkt[(hdr.csum_start + hdr.csum_offset) as usize..],
                    tcp_csum,
                );
            } else {
                let hdr = VirtioNetHdr::default();
                hdr.encode(
                    &mut bufs[item.bufs_index as usize].as_mut()[offset - VIRTIO_NET_HDR_LEN..],
                )?;
            }
        }
    }
    Ok(())
}

// applyUDPCoalesceAccounting updates bufs to account for coalescing based on the
// metadata found in table.
pub fn apply_udp_coalesce_accounting<B: ExpandBuffer>(
    bufs: &mut [B],
    offset: usize,
    table: &UdpGROTable,
) -> io::Result<()> {
    for items in table.items_by_flow.values() {
        for item in items {
            if item.num_merged > 0 {
                let hdr = VirtioNetHdr {
                    flags: VIRTIO_NET_HDR_F_NEEDS_CSUM, // this turns into CHECKSUM_PARTIAL in the skb
                    hdr_len: item.iph_len as u16 + UDP_H_LEN as u16,
                    gso_size: item.gso_size,
                    csum_start: item.iph_len as u16,
                    csum_offset: 6,
                    gso_type: VIRTIO_NET_HDR_GSO_UDP_L4,
                };

                let buf = bufs[item.bufs_index as usize].as_mut();
                let pkt = &mut buf[offset..];
                let pkt_len = pkt.len();

                // Calculate the pseudo header checksum and place it at the UDP
                // checksum offset. Downstream checksum offloading will combine
                // this with computation of the udp header and payload checksum.
                let (addr_len, src_addr_at) = if item.key.is_v6 {
                    (16, IPV6_SRC_ADDR_OFFSET)
                } else {
                    (4, IPV4_SRC_ADDR_OFFSET)
                };

                let src_addr =
                    unsafe { &*(&pkt[src_addr_at..(src_addr_at + addr_len)] as *const [u8]) };
                let dst_addr = unsafe {
                    &*(&pkt[(src_addr_at + addr_len)..(src_addr_at + addr_len * 2)]
                        as *const [u8])
                };

                // Recalculate the total len (IPv4) or payload len (IPv6).
                // Recalculate the (IPv4) header checksum.
                if item.key.is_v6 {
                    BigEndian::write_u16(&mut pkt[4..6], pkt_len as u16 - item.iph_len as u16);
                    // set new IPv6 header payload len
                } else {
                    pkt[10] = 0;
                    pkt[11] = 0;
                    BigEndian::write_u16(&mut pkt[2..4], pkt_len as u16); // set new total length
                    let iph_csum = !checksum(&pkt[..item.iph_len as usize], 0);
                    BigEndian::write_u16(&mut pkt[10..12], iph_csum); // set IPv4 header checksum field
                }

                hdr.encode(&mut buf[offset - VIRTIO_NET_HDR_LEN..])?;
                let pkt = &mut buf[offset..];
                // Recalculate the UDP len field value
                BigEndian::write_u16(
                    &mut pkt[(item.iph_len as usize + 4)..(item.iph_len as usize + 6)],
                    pkt_len as u16 - item.iph_len as u16,
                );

                let psum = pseudo_header_checksum_no_fold(
                    IPPROTO_UDP as _,
                    src_addr,
                    dst_addr,
                    pkt_len as u16 - item.iph_len as u16,
                );

                let udp_csum = checksum(&[], psum);
                BigEndian::write_u16(
                    &mut pkt[(hdr.csum_start + hdr.csum_offset) as usize..],
                    udp_csum,
                );
            } else {
                let hdr = VirtioNetHdr::default();
                hdr.encode(
                    &mut bufs[item.bufs_index as usize].as_mut()[offset - VIRTIO_NET_HDR_LEN..],
                )?;
            }
        }
    }
    Ok(())
}

#[derive(PartialEq, Eq)]
pub enum GroCandidateType {
    NotGRO,
    Tcp4GRO,
    Tcp6GRO,
    Udp4GRO,
    Udp6GRO,
}

pub fn packet_is_gro_candidate(b: &[u8], can_udp_gro: bool) -> GroCandidateType {
    if b.len() < 28 {
        return GroCandidateType::NotGRO;
    }
    if b[0] >> 4 == 4 {
        if b[0] & 0x0F != 5 {
            // IPv4 packets w/IP options do not coalesce
            return GroCandidateType::NotGRO;
        }
        match b[9] {
            6 if b.len() >= 40 => return GroCandidateType::Tcp4GRO,
            17 if can_udp_gro => return GroCandidateType::Udp4GRO,
            _ => {}
        }
    } else if b[0] >> 4 == 6 {
        match b[6] {
            6 if b.len() >= 60 => return GroCandidateType::Tcp6GRO,
            17 if b.len() >= 48 && can_udp_gro => return GroCandidateType::Udp6GRO,
            _ => {}
        }
    }
    GroCandidateType::NotGRO
}

const UDP_H_LEN: usize = 8;

/// udpGRO evaluates the UDP packet at pktI in bufs for coalescing with
/// existing packets tracked in table. It returns a groResultNoop when no
/// action was taken, groResultTableInsert when the evaluated packet was
/// inserted into table, and groResultCoalesced when the evaluated packet was
/// coalesced with another packet in table.
fn udp_gro<B: ExpandBuffer>(
    bufs: &mut [B],
    offset: usize,
    pkt_i: usize,
    table: &mut UdpGROTable,
    is_v6: bool,
) -> GroResult {
    let pkt = unsafe { &*(&bufs[pkt_i].as_ref()[offset..] as *const [u8]) };
    if pkt.len() > u16::MAX as usize {
        // A valid IPv4 or IPv6 packet will never exceed this.
        return GroResult::Noop;
    }

    let mut iph_len = ((pkt[0] & 0x0F) * 4) as usize;
    if is_v6 {
        iph_len = 40;
        let ipv6_payload_len = u16::from_be_bytes([pkt[4], pkt[5]]) as usize;
        if ipv6_payload_len != pkt.len() - iph_len {
            return GroResult::Noop;
        }
    } else {
        let total_len = u16::from_be_bytes([pkt[2], pkt[3]]) as usize;
        if total_len != pkt.len() {
            return GroResult::Noop;
        }
    }

    if pkt.len() < iph_len || pkt.len() < iph_len + UDP_H_LEN {
        return GroResult::Noop;
    }

    if !is_v6 && (pkt[6] & IPV4_FLAG_MORE_FRAGMENTS != 0 || pkt[6] << 3 != 0 || pkt[7] != 0) {
        // No GRO support for fragmented segments for now.
        return GroResult::Noop;
    }

    let gso_size = (pkt.len() - UDP_H_LEN - iph_len) as u16;
    if gso_size < 1 {
        return GroResult::Noop;
    }

    let (src_addr_offset, addr_len) = if is_v6 {
        (IPV6_SRC_ADDR_OFFSET, 16)
    } else {
        (IPV4_SRC_ADDR_OFFSET, 4)
    };

    let items = table.lookup_or_insert(
        pkt,
        src_addr_offset,
        src_addr_offset + addr_len,
        iph_len,
        pkt_i,
    );

    let items = if let Some(items) = items {
        items
    } else {
        return GroResult::TableInsert;
    };

    // Only check the last item to prevent reordering packets for a flow.
    let items_len = items.len();
    let item = &mut items[items_len - 1];
    let can = udp_packets_can_coalesce(pkt, iph_len as u8, gso_size, item, bufs, offset);
    let mut pkt_csum_known_invalid = false;

    if can == CanCoalesce::Append {
        match coalesce_udp_packets(pkt, item, bufs, offset, is_v6) {
            CoalesceResult::Success => {
                // 前面是引用，这里不需要再更新
                // table.update_at(*item, items_len - 1);
                return GroResult::Coalesced;
            }
            CoalesceResult::ItemInvalidCSum => {
                // If the existing item has an invalid checksum, take no action.
                // A new item will be stored, and the existing item won't be revisited.
            }
            CoalesceResult::PktInvalidCSum => {
                // Insert a new item but mark it with invalid checksum to avoid repeat checks.
                pkt_csum_known_invalid = true;
            }
            _ => {}
        }
    }
    let pkt = &bufs[pkt_i].as_ref()[offset..];
    // Failed to coalesce; store the packet in the flow.
    table.insert(
        pkt,
        src_addr_offset,
        src_addr_offset + addr_len,
        iph_len,
        pkt_i,
        pkt_csum_known_invalid,
    );
    GroResult::TableInsert
}

/// handleGRO evaluates bufs for GRO, and writes the indices of the resulting
/// Process received packets and apply Generic Receive Offload (GRO) coalescing.
///
/// This function examines a batch of received packets and coalesces packets belonging
/// to the same TCP or UDP flow into larger segments, reducing per-packet overhead.
///
/// # Arguments
///
/// * `bufs` - Mutable slice of packet buffers. Each buffer should contain a full packet
///   starting at `offset` (with space before offset for the virtio header).
/// * `offset` - Offset where packet data begins (typically [`VIRTIO_NET_HDR_LEN`]).
///   The virtio header will be written before this offset.
/// * `tcp_table` - TCP GRO table for tracking TCP flows.
/// * `udp_table` - UDP GRO table for tracking UDP flows.
/// * `can_udp_gro` - Whether UDP GRO is supported (kernel feature).
/// * `to_write` - Output vector that will be filled with indices of packets to write.
///   Initially should be empty.
///
/// # Returns
///
/// Returns `Ok(())` on success, or an error if packet processing fails.
///
/// # Behavior
///
/// 1. Examines each packet to determine if it's a GRO candidate (TCP or UDP)
/// 2. Attempts to coalesce the packet with previous packets in the same flow
/// 3. Writes indices of final packets (coalesced or standalone) to `to_write`
/// 4. Updates packet headers with appropriate virtio headers
///
/// # Example
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::{handle_gro, GROTable, VIRTIO_NET_HDR_LEN};
///
/// let mut gro_table = GROTable::default();
/// let mut bufs = vec![vec![0u8; 1500]; 128];
/// let mut to_write = Vec::new();
///
/// // After receiving packets into bufs with recv_multiple:
/// // handle_gro(
/// //     &mut bufs,
/// //     VIRTIO_NET_HDR_LEN,
/// //     &mut gro_table.tcp_table,
/// //     &mut gro_table.udp_table,
/// //     true,  // UDP GRO supported
/// //     &mut to_write
/// // )?;
///
/// // to_write now contains indices of packets to process
/// // for idx in &to_write {
/// //     let packet = &bufs[*idx];
/// //     // process packet...
/// // }
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Performance
///
/// - Coalescing reduces the number of packets passed to the application
/// - Typical coalescing ratios: 5-20 packets into 1 for bulk TCP transfers
/// - Most effective for sequential TCP traffic with large receive windows
///
/// # See Also
///
/// - [`GROTable`] for managing GRO state
/// - [`apply_tcp_coalesce_accounting`] for updating TCP headers after coalescing
pub fn handle_gro<B: ExpandBuffer>(
    bufs: &mut [B],
    offset: usize,
    tcp_table: &mut TcpGROTable,
    udp_table: &mut UdpGROTable,
    can_udp_gro: bool,
    to_write: &mut Vec<usize>,
) -> io::Result<()> {
    let bufs_len = bufs.len();
    for i in 0..bufs_len {
        if offset < VIRTIO_NET_HDR_LEN || offset > bufs[i].as_ref().len() - 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid offset",
            ));
        }

        let result = match packet_is_gro_candidate(&bufs[i].as_ref()[offset..], can_udp_gro) {
            GroCandidateType::Tcp4GRO => tcp_gro(bufs, offset, i, tcp_table, false),
            GroCandidateType::Tcp6GRO => tcp_gro(bufs, offset, i, tcp_table, true),
            GroCandidateType::Udp4GRO => udp_gro(bufs, offset, i, udp_table, false),
            GroCandidateType::Udp6GRO => udp_gro(bufs, offset, i, udp_table, true),
            GroCandidateType::NotGRO => GroResult::Noop,
        };

        match result {
            GroResult::Noop => {
                let hdr = VirtioNetHdr::default();
                hdr.encode(&mut bufs[i].as_mut()[offset - VIRTIO_NET_HDR_LEN..offset])?;
                // Fallthrough intended
                to_write.push(i);
            }
            GroResult::TableInsert => {
                to_write.push(i);
            }
            _ => {}
        }
    }

    let err_tcp = apply_tcp_coalesce_accounting(bufs, offset, tcp_table);
    let err_udp = apply_udp_coalesce_accounting(bufs, offset, udp_table);
    err_tcp?;
    err_udp?;
    Ok(())
}

/// Split a GSO (Generic Segmentation Offload) packet into multiple smaller packets.
///
/// When sending data with offload enabled, the application can provide large packets
/// that will be automatically segmented. This function performs the opposite operation:
/// splitting a large GSO packet into MTU-sized segments for transmission.
///
/// # Arguments
///
/// * `input` - The input buffer containing the large GSO packet (with virtio header).
/// * `hdr` - The virtio network header describing the GSO packet.
/// * `out_bufs` - Output buffers where segmented packets will be written.
/// * `sizes` - Output array where the size of each segmented packet will be written.
/// * `out_offset` - Offset in output buffers where packet data should start.
/// * `is_v6` - Whether this is an IPv6 packet (affects header offsets).
///
/// # Returns
///
/// Returns the number of output buffers populated (number of segments created),
/// or an error if segmentation fails.
///
/// # How GSO Splitting Works
///
/// For a large TCP packet with GSO enabled:
/// 1. The packet headers are parsed (IP + TCP)
/// 2. The payload is split into segments of size `hdr.gso_size`
/// 3. New packets are created with copied headers and updated fields:
///    - IP length field
///    - IP checksum (for IPv4)
///    - TCP sequence number (incremented for each segment)
///    - TCP checksum
///
/// # Example
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::{gso_split, VirtioNetHdr, VIRTIO_NET_HDR_LEN};
///
/// let mut large_packet = vec![0u8; 65536];
/// let hdr = VirtioNetHdr::default();
/// let mut out_bufs = vec![vec![0u8; 1500]; 128];
/// let mut sizes = vec![0; 128];
///
/// // Split the GSO packet
/// // let num_segments = gso_split(
/// //     &mut large_packet,
/// //     hdr,
/// //     &mut out_bufs,
/// //     &mut sizes,
/// //     VIRTIO_NET_HDR_LEN,
/// //     false  // IPv4
/// // )?;
///
/// // Now out_bufs[0..num_segments] contain the segmented packets
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Supported Protocols
///
/// - TCP over IPv4 (GSO type: [`VIRTIO_NET_HDR_GSO_TCPV4`])
/// - TCP over IPv6 (GSO type: [`VIRTIO_NET_HDR_GSO_TCPV6`])
/// - UDP (GSO type: [`VIRTIO_NET_HDR_GSO_UDP_L4`])
///
/// # Performance
///
/// GSO allows sending fewer, larger packets to the kernel, which then performs
/// efficient segmentation. This reduces:
/// - Number of system calls
/// - Per-packet processing overhead in the application
/// - Context switches
///
/// Typical performance improvement: 2-5x for bulk transfers.
pub fn gso_split<B: AsRef<[u8]> + AsMut<[u8]>>(
    input: &mut [u8],
    hdr: VirtioNetHdr,
    out_bufs: &mut [B],
    sizes: &mut [usize],
    out_offset: usize,
    is_v6: bool,
) -> io::Result<usize> {
    let iph_len = hdr.csum_start as usize;
    let (src_addr_offset, addr_len) = if is_v6 {
        (IPV6_SRC_ADDR_OFFSET, 16)
    } else {
        input[10] = 0;
        input[11] = 0; // clear IPv4 header checksum
        (IPV4_SRC_ADDR_OFFSET, 4)
    };

    let transport_csum_at = (hdr.csum_start + hdr.csum_offset) as usize;
    input[transport_csum_at] = 0;
    input[transport_csum_at + 1] = 0; // clear TCP/UDP checksum

    let (first_tcp_seq_num, protocol) =
        if hdr.gso_type == VIRTIO_NET_HDR_GSO_TCPV4 || hdr.gso_type == VIRTIO_NET_HDR_GSO_TCPV6 {
            (
                BigEndian::read_u32(&input[hdr.csum_start as usize + 4..]),
                IPPROTO_TCP,
            )
        } else {
            (0, IPPROTO_UDP)
        };

    let src_addr_bytes = &input[src_addr_offset..src_addr_offset + addr_len];
    let dst_addr_bytes = &input[src_addr_offset + addr_len..src_addr_offset + 2 * addr_len];
    let transport_header_len = (hdr.hdr_len - hdr.csum_start) as usize;

    let nonlast_segment_data_len = hdr.gso_size as usize;
    let nonlast_len_for_pseudo = (transport_header_len + nonlast_segment_data_len) as u16;
    let nonlast_total_len = hdr.hdr_len as usize + nonlast_segment_data_len;

    let nonlast_transport_csum_no_fold = pseudo_header_checksum_no_fold(
        protocol as u8,
        src_addr_bytes,
        dst_addr_bytes,
        nonlast_len_for_pseudo,
    );

    let mut next_segment_data_at = hdr.hdr_len as usize;
    let mut i = 0;

    while next_segment_data_at < input.len() {
        if i == out_bufs.len() {
            return Err(io::Error::other("ErrTooManySegments"));
        }

        let next_segment_end = next_segment_data_at + hdr.gso_size as usize;
        let (next_segment_end, segment_data_len, total_len, transport_csum_no_fold) =
            if next_segment_end > input.len() {
                let last_segment_data_len = input.len() - next_segment_data_at;
                let last_len_for_pseudo = (transport_header_len + last_segment_data_len) as u16;

                let last_total_len = hdr.hdr_len as usize + last_segment_data_len;
                let last_transport_csum_no_fold = pseudo_header_checksum_no_fold(
                    protocol as u8,
                    src_addr_bytes,
                    dst_addr_bytes,
                    last_len_for_pseudo,
                );
                (
                    input.len(),
                    last_segment_data_len,
                    last_total_len,
                    last_transport_csum_no_fold,
                )
            } else {
                (
                    next_segment_end,
                    hdr.gso_size as usize,
                    nonlast_total_len,
                    nonlast_transport_csum_no_fold,
                )
            };

        sizes[i] = total_len;
        let out = &mut out_bufs[i].as_mut()[out_offset..];

        out[..iph_len].copy_from_slice(&input[..iph_len]);

        if !is_v6 {
            // For IPv4 we are responsible for incrementing the ID field,
            // updating the total len field, and recalculating the header
            // checksum.
            if i > 0 {
                let id = BigEndian::read_u16(&out[4..]).wrapping_add(i as u16);
                BigEndian::write_u16(&mut out[4..6], id);
            }
            BigEndian::write_u16(&mut out[2..4], total_len as u16);
            let ipv4_csum = !checksum(&out[..iph_len], 0);
            BigEndian::write_u16(&mut out[10..12], ipv4_csum);
        } else {
            // For IPv6 we are responsible for updating the payload length field.
            // IPv6 extensions are not checksumed, but included in the payload length.
            const IPV6_FIXED_HDR_LEN: usize = 40;
            let payload_len = total_len - IPV6_FIXED_HDR_LEN;
            BigEndian::write_u16(&mut out[4..6], payload_len as u16);
        }

        out[hdr.csum_start as usize..hdr.hdr_len as usize]
            .copy_from_slice(&input[hdr.csum_start as usize..hdr.hdr_len as usize]);

        if protocol == IPPROTO_TCP {
            let tcp_seq = first_tcp_seq_num.wrapping_add(hdr.gso_size as u32 * i as u32);
            BigEndian::write_u32(
                &mut out[(hdr.csum_start + 4) as usize..(hdr.csum_start + 8) as usize],
                tcp_seq,
            );
            if next_segment_end != input.len() {
                out[hdr.csum_start as usize + TCP_FLAGS_OFFSET] &= !(TCP_FLAG_FIN | TCP_FLAG_PSH);
            }
        } else {
            let udp_len = (segment_data_len + (hdr.hdr_len - hdr.csum_start) as usize) as u16;
            BigEndian::write_u16(
                &mut out[(hdr.csum_start + 4) as usize..(hdr.csum_start + 6) as usize],
                udp_len,
            );
        }

        out[hdr.hdr_len as usize..total_len]
            .as_mut()
            .copy_from_slice(&input[next_segment_data_at..next_segment_end]);

        let transport_csum = !checksum(
            &out[hdr.csum_start as usize..total_len],
            transport_csum_no_fold,
        );
        BigEndian::write_u16(
            &mut out[transport_csum_at..transport_csum_at + 2],
            transport_csum,
        );

        next_segment_data_at += hdr.gso_size as usize;
        i += 1;
    }

    Ok(i)
}

/// Calculate checksum for packets without GSO.
///
/// This function computes and writes the transport layer (TCP/UDP) checksum for
/// packets that don't use Generic Segmentation Offload.
///
/// # Arguments
///
/// * `in_buf` - The packet buffer (mutable)
/// * `csum_start` - Offset where checksum calculation should begin
/// * `csum_offset` - Offset within the checksummed area where the checksum should be written
///
/// # Behavior
///
/// 1. Reads the initial checksum value (typically the pseudo-header checksum)
/// 2. Clears the checksum field
/// 3. Calculates the checksum over the transport header and data
/// 4. Writes the final checksum back to the buffer
///
/// This is used when [`VIRTIO_NET_HDR_F_NEEDS_CSUM`] flag is set but [`VIRTIO_NET_HDR_GSO_NONE`]
/// is the GSO type.
pub fn gso_none_checksum(in_buf: &mut [u8], csum_start: u16, csum_offset: u16) {
    let csum_at = (csum_start + csum_offset) as usize;
    // The initial value at the checksum offset should be summed with the
    // checksum we compute. This is typically the pseudo-header checksum.
    let initial = BigEndian::read_u16(&in_buf[csum_at..]);
    in_buf[csum_at] = 0;
    in_buf[csum_at + 1] = 0;
    let computed_checksum = checksum(&in_buf[csum_start as usize..], initial as u64);
    BigEndian::write_u16(&mut in_buf[csum_at..], !computed_checksum);
}

/// Generic Receive Offload (GRO) table for managing packet coalescing.
///
/// This structure maintains the state needed to coalesce multiple received packets
/// into larger segments, reducing per-packet processing overhead. It combines both
/// TCP and UDP GRO capabilities.
///
/// # Purpose
///
/// When receiving many small packets of the same flow, GRO can combine them into
/// fewer, larger packets. This provides significant performance benefits:
///
/// - Reduces the number of packets passed to the application
/// - Fewer context switches and system calls
/// - Better cache utilization
/// - Lower CPU usage per gigabit of traffic
///
/// # Usage
///
/// Create a `GROTable` and reuse it across multiple `recv_multiple` calls:
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use tun_rs::{DeviceBuilder, GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};
///
/// let dev = DeviceBuilder::new()
///     .offload(true)
///     .ipv4("10.0.0.1", 24, None)
///     .build_sync()?;
///
/// let mut gro_table = GROTable::default();
/// let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
/// let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
/// let mut sizes = vec![0; IDEAL_BATCH_SIZE];
///
/// loop {
///     let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;
///
///     // GRO table is automatically used by recv_multiple
///     // to coalesce packets
///     for i in 0..num {
///         println!("Packet: {} bytes", sizes[i]);
///     }
/// }
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Fields
///
/// - `tcp_gro_table`: State for TCP packet coalescing
/// - `udp_gro_table`: State for UDP packet coalescing (if supported by kernel)
/// - `to_write`: Internal buffer tracking which packets to emit
///
/// # Performance
///
/// The GRO table maintains internal state across calls, including:
/// - Hash map of active flows (preallocated for [`IDEAL_BATCH_SIZE`] flows)
/// - Memory pools to reduce allocations
/// - Per-flow coalescing state
///
/// Typical coalescing ratios:
/// - TCP bulk transfers: 5-20 packets coalesced into 1
/// - UDP: 2-5 packets coalesced into 1
/// - Interactive traffic: minimal coalescing (preserves latency)
///
/// # Thread Safety
///
/// `GROTable` is not thread-safe. Use one instance per thread or protect with a mutex.
#[derive(Default)]
pub struct GROTable {
    pub(crate) to_write: Vec<usize>,
    pub(crate) tcp_gro_table: TcpGROTable,
    pub(crate) udp_gro_table: UdpGROTable,
}

impl GROTable {
    pub fn new() -> GROTable {
        GROTable {
            to_write: Vec::with_capacity(IDEAL_BATCH_SIZE),
            tcp_gro_table: TcpGROTable::new(),
            udp_gro_table: UdpGROTable::new(),
        }
    }
    pub(crate) fn reset(&mut self) {
        self.to_write.clear();
        self.tcp_gro_table.reset();
        self.udp_gro_table.reset();
    }
}

/// A trait for buffers that can be expanded and resized for offload operations.
///
/// This trait extends basic buffer operations (`AsRef<[u8]>` and `AsMut<[u8]>`)
/// with methods needed for efficient packet processing with GRO/GSO offload support.
/// It allows buffers to grow dynamically as needed during packet coalescing and
/// segmentation operations.
///
/// # Required Methods
///
/// - `buf_capacity()` - Returns the current capacity of the buffer
/// - `buf_resize()` - Resizes the buffer to a new length, filling with a value
/// - `buf_extend_from_slice()` - Extends the buffer with data from a slice
///
/// # Implementations
///
/// This trait is implemented for:
/// - `BytesMut` - The primary buffer type for async operations
/// - `&mut BytesMut` - Mutable reference to BytesMut
/// - `Vec<u8>` - Standard Rust vector
/// - `&mut Vec<u8>` - Mutable reference to Vec
///
/// # Example
///
/// ```no_run
/// # #[cfg(target_os = "linux")]
/// # {
/// use bytes::BytesMut;
/// use tun_rs::ExpandBuffer;
///
/// let mut buffer = BytesMut::with_capacity(1500);
/// buffer.buf_resize(20, 0); // Resize to 20 bytes, filled with zeros
/// buffer.buf_extend_from_slice(b"packet data"); // Append data
/// assert!(buffer.buf_capacity() >= buffer.len());
/// # }
/// ```
pub trait ExpandBuffer: AsRef<[u8]> + AsMut<[u8]> {
    /// Returns the current capacity of the buffer in bytes.
    ///
    /// The capacity is the total amount of memory allocated, which may be
    /// greater than the current length of the buffer.
    fn buf_capacity(&self) -> usize;

    /// Resizes the buffer to the specified length, filling new space with the given value.
    ///
    /// If `new_len` is greater than the current length, the buffer is extended
    /// and new bytes are initialized to `value`. If `new_len` is less than the
    /// current length, the buffer is truncated.
    ///
    /// # Arguments
    ///
    /// * `new_len` - The new length of the buffer
    /// * `value` - The byte value to fill any new space with
    fn buf_resize(&mut self, new_len: usize, value: u8);

    /// Extends the buffer by appending data from a slice.
    ///
    /// This method appends all bytes from `src` to the end of the buffer,
    /// growing the buffer as necessary.
    ///
    /// # Arguments
    ///
    /// * `src` - The slice of bytes to append to the buffer
    fn buf_extend_from_slice(&mut self, src: &[u8]);
}

impl ExpandBuffer for BytesMut {
    fn buf_capacity(&self) -> usize {
        self.capacity()
    }

    fn buf_resize(&mut self, new_len: usize, value: u8) {
        self.resize(new_len, value)
    }

    fn buf_extend_from_slice(&mut self, extend: &[u8]) {
        self.extend_from_slice(extend)
    }
}

impl ExpandBuffer for &mut BytesMut {
    fn buf_capacity(&self) -> usize {
        self.capacity()
    }
    fn buf_resize(&mut self, new_len: usize, value: u8) {
        self.resize(new_len, value)
    }

    fn buf_extend_from_slice(&mut self, extend: &[u8]) {
        self.extend_from_slice(extend)
    }
}
impl ExpandBuffer for Vec<u8> {
    fn buf_capacity(&self) -> usize {
        self.capacity()
    }

    fn buf_resize(&mut self, new_len: usize, value: u8) {
        self.resize(new_len, value)
    }

    fn buf_extend_from_slice(&mut self, extend: &[u8]) {
        self.extend_from_slice(extend)
    }
}
impl ExpandBuffer for &mut Vec<u8> {
    fn buf_capacity(&self) -> usize {
        self.capacity()
    }

    fn buf_resize(&mut self, new_len: usize, value: u8) {
        self.resize(new_len, value)
    }

    fn buf_extend_from_slice(&mut self, extend: &[u8]) {
        self.extend_from_slice(extend)
    }
}
