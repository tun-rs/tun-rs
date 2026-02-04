#![cfg_attr(docsrs, feature(doc_cfg))]

/*!
# tun-rs: Cross-platform TUN/TAP Library

A high-performance, cross-platform Rust library for creating and managing TUN (Layer 3) and TAP (Layer 2)
network interfaces. This library provides both synchronous and asynchronous APIs with support for advanced
features like offload (TSO/GSO) on Linux and multi-queue support.

## Features

- **Multi-platform Support**: Windows, Linux, macOS, FreeBSD, OpenBSD, NetBSD, Android, iOS, tvOS, and OpenHarmony
- **TUN and TAP Modes**: Support for both Layer 3 (TUN) and Layer 2 (TAP) interfaces
- **Multiple IP Addresses**: Configure multiple IPv4 and IPv6 addresses on a single interface
- **Async Runtime Integration**: Optional integration with Tokio or async-io/async-std
- **Advanced Linux Features**:
  - Offload support (TSO/GSO) for improved performance
  - Multi-queue support for parallel packet processing
  - Generic Receive Offload (GRO) for packet coalescing
- **Platform Consistency**: Uniform packet format across platforms (optional packet information header)
- **Mobile Support**: Direct file descriptor support for iOS (PacketTunnelProvider) and Android (VpnService)

## Device Types

The library provides three main device types:

1. **`SyncDevice`**: Synchronous I/O operations, suitable for single-threaded or blocking code
2. **`AsyncDevice`**: Asynchronous I/O operations, requires the `async` feature flag
3. **`BorrowedDevice`**: Borrowed file descriptor variants that don't take ownership

## Quick Start

### Basic Synchronous Example

Create a TUN interface with IPv4 and IPv6 addresses:

```no_run
use tun_rs::DeviceBuilder;

let dev = DeviceBuilder::new()
    .name("utun7")
    .ipv4("10.0.0.12", 24, None)
    .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
    .mtu(1400)
    .build_sync()
    .unwrap();

let mut buf = [0; 65535];
loop {
    let len = dev.recv(&mut buf).unwrap();
    println!("Received packet: {:?}", &buf[..len]);
}
```

### Asynchronous Example (with Tokio)

Add to your `Cargo.toml`:
```toml
[dependencies]
tun-rs = { version = "2", features = ["async"] }
```

Then use async I/O:

```no_run
use tun_rs::DeviceBuilder;

# #[tokio::main]
# async fn main() -> std::io::Result<()> {
let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .build_async()?;

let mut buf = vec![0; 65536];
loop {
    let len = dev.recv(&mut buf).await?;
    println!("Received: {:?}", &buf[..len]);
}
# }
```

### Mobile Platforms (iOS/Android)

For iOS and Android, use the file descriptor from the system VPN APIs:

```no_run
#[cfg(unix)]
{
    use tun_rs::SyncDevice;
    // On iOS: from PacketTunnelProvider.packetFlow
    // On Android: from VpnService.Builder.establish()
    let fd = 7799; // exposition-only
    let dev = unsafe { SyncDevice::from_fd(fd).unwrap() };
    
    let mut buf = [0; 65535];
    loop {
        let len = dev.recv(&mut buf).unwrap();
        println!("Received packet: {:?}", &buf[..len]);
    }
}
```

## Advanced Features

### Multiple IP Addresses

You can add multiple IPv4 and IPv6 addresses to an interface:

```no_run
# use tun_rs::DeviceBuilder;
# #[tokio::main]
# async fn main() -> std::io::Result<()> {
let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .build_async()?;

dev.add_address_v4("10.1.0.1", 24)?;
dev.add_address_v4("10.2.0.1", 24)?;
dev.add_address_v6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)?;
# Ok(())
# }
```

### Linux Offload (TSO/GSO)

On Linux, enable offload for improved throughput:

```no_run
#[cfg(target_os = "linux")]
{
    use tun_rs::{DeviceBuilder, GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};
    
    let dev = DeviceBuilder::new()
        .offload(true)  // Enable TSO/GSO
        .ipv4("10.0.0.1", 24, None)
        .build_sync()?;
    
    let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
    let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
    let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    
    loop {
        let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;
        for i in 0..num {
            println!("Packet {}: {:?}", i, &bufs[i][..sizes[i]]);
        }
    }
}
# Ok(())
```

## Platform-Specific Notes

### Windows
- TUN mode requires the [Wintun driver](https://wintun.net/)
- TAP mode requires [tap-windows](https://build.openvpn.net/downloads/releases/)
- Administrator privileges required

### Linux
- Requires the `tun` kernel module (`modprobe tun`)
- Root privileges required for creating interfaces
- Supports advanced features: offload, multi-queue

### macOS
- TUN interfaces are named `utunN`
- TAP mode uses a pair of `feth` interfaces
- Routes are automatically configured

### BSD (FreeBSD, OpenBSD, NetBSD)
- Routes are automatically configured
- Platform-specific syscall interfaces

## Feature Flags

- **`async`** (alias for `async_tokio`): Enable async support with Tokio runtime
- **`async_tokio`**: Use Tokio for async I/O operations
- **`async_io`**: Use async-io for async operations (async-std, smol, etc.)
- **`async_framed`**: Enable framed I/O with futures
- **`interruptible`**: Enable interruptible I/O operations
- **`experimental`**: Enable experimental features (unstable)

## Safety

This library uses `unsafe` code in several places:
- File descriptor manipulation on Unix platforms
- FFI calls to platform-specific APIs (Windows, BSD)
- Direct memory access for performance-critical operations

All unsafe code is carefully audited and documented with safety invariants.

## Performance Considerations

- Use `recv_multiple`/`send_multiple` on Linux with offload enabled for best throughput
- Enable multi-queue on Linux for parallel packet processing across CPU cores
- Consider the async API for high-concurrency scenarios
- Adjust MTU based on your network requirements (default varies by platform)

## Error Handling

All I/O operations return `std::io::Result` with platform-specific error codes.
Common error scenarios include:
- Permission denied (need root/administrator)
- Device name conflicts
- Platform-specific driver issues
- Invalid configuration parameters
*/

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
pub use crate::builder::*;
pub use crate::platform::*;

#[cfg_attr(docsrs, doc(cfg(any(feature = "async_io", feature = "async_tokio"))))]
#[cfg(any(feature = "async_io", feature = "async_tokio"))]
mod async_device;

#[cfg_attr(docsrs, doc(cfg(any(feature = "async_io", feature = "async_tokio"))))]
#[cfg(any(feature = "async_io", feature = "async_tokio"))]
pub use async_device::*;

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
mod builder;
mod platform;

/// Length of the protocol information header used on some platforms.
///
/// On certain Unix-like platforms (macOS, iOS), TUN interfaces may include a 4-byte
/// protocol information header before each packet. This constant represents that header length.
/// 
/// When `packet_information` is enabled in [`DeviceBuilder`], packets will include this header.
/// The header typically contains the protocol family (e.g., AF_INET for IPv4, AF_INET6 for IPv6).
///
/// # Example
///
/// ```no_run
/// use tun_rs::PACKET_INFORMATION_LENGTH;
/// 
/// let mut buf = vec![0u8; PACKET_INFORMATION_LENGTH + 1500];
/// // Read packet with header
/// // let len = dev.recv(&mut buf)?;
/// // Skip the header to get the actual packet
/// // let packet = &buf[PACKET_INFORMATION_LENGTH..len];
/// ```
pub const PACKET_INFORMATION_LENGTH: usize = 4;
