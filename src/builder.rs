/*!
# Device Builder Module

This module provides the [`DeviceBuilder`] struct for configuring and creating TUN/TAP interfaces.

## Overview

The builder pattern allows you to specify all configuration parameters before creating a device.
Configuration includes:
- Device name
- IP addresses (IPv4 and IPv6, single or multiple)
- MTU (Maximum Transmission Unit)
- Operating layer (L2/TAP or L3/TUN)
- MAC address (for L2/TAP mode)
- Platform-specific options (offload, multi-queue, packet information, etc.)

## Basic Usage

```no_run
use tun_rs::DeviceBuilder;

// Create a TUN (L3) device
let dev = DeviceBuilder::new()
    .name("tun0")
    .ipv4("10.0.0.1", 24, None)
    .mtu(1400)
    .build_sync()?;
# Ok::<(), std::io::Error>(())
```

## Platform-Specific Configuration

The builder provides platform-specific methods accessible via the `with()` method:

### Linux Specific

```no_run
# #[cfg(target_os = "linux")]
# {
use tun_rs::DeviceBuilder;

let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .with(|builder| {
        builder
            .offload(true)        // Enable TSO/GSO offload
            .multi_queue(true)    // Enable multi-queue support
            .tx_queue_len(1000)   // Set transmit queue length
    })
    .build_sync()?;
# Ok::<(), std::io::Error>(())
# }
```

### Windows Specific

```no_run
# #[cfg(target_os = "windows")]
# {
use tun_rs::DeviceBuilder;

let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .with(|builder| {
        builder
            .ring_capacity(0x40_0000)  // 4MB ring buffer
            .wintun_log(true)          // Enable Wintun logging
            .description("My VPN")     // Set device description
    })
    .build_sync()?;
# Ok::<(), std::io::Error>(())
# }
```

### macOS Specific

```no_run
# #[cfg(target_os = "macos")]
# {
use tun_rs::DeviceBuilder;

let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .with(|builder| {
        builder
            .associate_route(true)        // Auto-configure routes
            .packet_information(false)    // Disable packet headers
    })
    .build_sync()?;
# Ok::<(), std::io::Error>(())
# }
```

## Layer Selection

Choose between Layer 2 (TAP) and Layer 3 (TUN):

```no_run
# #[cfg(any(target_os = "linux", target_os = "windows", target_os = "freebsd"))]
# {
use tun_rs::{DeviceBuilder, Layer};

// TAP interface (Layer 2)
let tap = DeviceBuilder::new()
    .name("tap0")
    .layer(Layer::L2)
    .mac_addr([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
    .build_sync()?;

// TUN interface (Layer 3, default)
let tun = DeviceBuilder::new()
    .name("tun0")
    .layer(Layer::L3)
    .ipv4("10.0.0.1", 24, None)
    .build_sync()?;
# Ok::<(), std::io::Error>(())
# }
```

## Multiple IP Addresses

You can configure multiple IPv6 addresses during device creation:

```no_run
use tun_rs::DeviceBuilder;

let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .ipv6("fd00::1", 64)
    .ipv6("fd00::2", 64)
    .build_sync()?;
# Ok::<(), std::io::Error>(())
```

Additional addresses can be added after creation using [`crate::SyncDevice::add_address_v4`]
and [`crate::SyncDevice::add_address_v6`] methods.

## Configuration Precedence

- Most settings have sensible defaults
- Unspecified values use platform defaults
- Some settings are mandatory (e.g., at least one IP address for routing)
- Platform-specific settings are ignored on other platforms

## Error Handling

The `build_sync()` and `build_async()` methods return `io::Result<Device>`.
Common errors include:
- Permission denied (need root/administrator privileges)
- Device name already exists
- Invalid IP address or netmask
- Platform-specific driver not found (e.g., Wintun on Windows)
*/

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::platform::{DeviceImpl, SyncDevice};

/// Represents the OSI layer at which the TUN/TAP interface operates.
///
/// This enum determines whether the interface works at the Data Link Layer (L2/TAP)
/// or the Network Layer (L3/TUN).
///
/// # Layer 2 (TAP) - Data Link Layer
///
/// TAP interfaces operate at the Ethernet frame level (Layer 2), handling complete
/// Ethernet frames including MAC addresses. This is useful for:
/// - Bridging networks
/// - Emulating full Ethernet connectivity
/// - Applications requiring MAC-level control
/// - Creating virtual switches
///
/// TAP mode requires setting a MAC address and can work with protocols like ARP.
///
/// **Platform availability**: Windows, Linux, FreeBSD, macOS, OpenBSD, NetBSD
///
/// # Layer 3 (TUN) - Network Layer
///
/// TUN interfaces operate at the IP packet level (Layer 3), handling IP packets
/// directly without Ethernet framing. This is the default and most common mode for:
/// - VPN implementations
/// - IP tunneling
/// - Point-to-point connections
/// - Routing between networks
///
/// TUN mode is simpler and more efficient than TAP when Ethernet-level features
/// are not needed.
///
/// **Platform availability**: All platforms
///
/// # Examples
///
/// Creating a TAP (L2) interface:
///
/// ```no_run
/// # #[cfg(any(target_os = "linux", target_os = "windows", target_os = "freebsd"))]
/// # {
/// use tun_rs::{DeviceBuilder, Layer};
///
/// let tap = DeviceBuilder::new()
///     .name("tap0")
///     .layer(Layer::L2)
///     .mac_addr([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
///     .build_sync()?;
/// # Ok::<(), std::io::Error>(())
/// # }
/// ```
///
/// Creating a TUN (L3) interface (default):
///
/// ```no_run
/// use tun_rs::{DeviceBuilder, Layer};
///
/// // L3 is the default, explicit specification is optional
/// let tun = DeviceBuilder::new()
///     .name("tun0")
///     .layer(Layer::L3)
///     .ipv4("10.0.0.1", 24, None)
///     .build_sync()?;
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Platform-Specific Notes
///
/// - On macOS, TAP mode uses a pair of `feth` (fake ethernet) interfaces
/// - On Windows, TAP mode requires the tap-windows driver
/// - On Linux, both modes use the kernel TUN/TAP driver
/// - Android and iOS only support TUN (L3) mode
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Layer {
    /// Data Link Layer (Ethernet frames with MAC addresses).
    ///
    /// TAP mode operates at Layer 2, handling complete Ethernet frames.
    /// Requires a MAC address to be configured.
    ///
    /// Available on: Windows, Linux, FreeBSD, macOS, OpenBSD, NetBSD
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos",
        target_os = "openbsd",
        target_os = "netbsd",
    ))]
    L2,

    /// Network Layer (IP packets).
    ///
    /// TUN mode operates at Layer 3, handling IP packets directly.
    /// This is the default and most common mode for VPN and tunneling applications.
    ///
    /// Available on: All platforms
    #[default]
    L3,
}

/// Configuration for a TUN/TAP interface.
///
/// This structure stores settings such as the device name, operating layer,
/// and platform-specific parameters (e.g., GUID, wintun file, ring capacity on Windows).
#[derive(Clone, Default, Debug)]
pub(crate) struct DeviceConfig {
    /// The name of the device/interface.
    pub(crate) dev_name: Option<String>,
    /// The description of the device/interface.
    #[cfg(windows)]
    pub(crate) description: Option<String>,
    /// Available with Layer::L2; creates a pair of feth devices, with peer_feth as the IO interface name.
    #[cfg(target_os = "macos")]
    pub(crate) peer_feth: Option<String>,
    /// If true (default), the program will automatically add or remove routes on macOS or FreeBSD to provide consistent routing behavior across all platforms.
    /// If false, the program will not modify or manage routes in any way, allowing the system to handle all routing natively.
    /// Set this to be false to obtain the platform's default routing behavior.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub(crate) associate_route: Option<bool>,
    /// If true (default), the existing device with the given name will be used if possible.
    /// If false, an error will be returned if a device with the specified name already exists.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "netbsd"))]
    pub(crate) reuse_dev: Option<bool>,
    /// If true, the feth device will be kept after the program exits;
    /// if false (default), the device will be destroyed automatically.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub(crate) persist: Option<bool>,
    /// Specifies whether the interface operates at L2 or L3.
    #[allow(dead_code)]
    pub(crate) layer: Option<Layer>,
    /// Device GUID on Windows.
    #[cfg(windows)]
    pub(crate) device_guid: Option<u128>,
    #[cfg(windows)]
    pub(crate) wintun_log: Option<bool>,
    /// Path to the wintun file on Windows.
    #[cfg(windows)]
    pub(crate) wintun_file: Option<String>,
    /// Capacity of the ring buffer on Windows.
    #[cfg(windows)]
    pub(crate) ring_capacity: Option<u32>,
    /// Whether to call WintunDeleteDriver to remove the driver.
    /// Default: false.
    #[cfg(windows)]
    pub(crate) delete_driver: Option<bool>,
    #[cfg(windows)]
    pub(crate) mac_address: Option<String>,
    /// switch of Enable/Disable packet information for network driver
    #[cfg(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub(crate) packet_information: Option<bool>,
    /// Enable/Disable TUN offloads.
    /// After enabling, use `recv_multiple`/`send_multiple` for data transmission.
    #[cfg(target_os = "linux")]
    pub(crate) offload: Option<bool>,
    /// Enable multi queue support
    #[cfg(target_os = "linux")]
    pub(crate) multi_queue: Option<bool>,
}
type IPV4 = (
    io::Result<Ipv4Addr>,
    io::Result<u8>,
    Option<io::Result<Ipv4Addr>>,
);
/// A builder for configuring a TUN/TAP interface.
///
/// This builder allows you to set parameters such as device name, MTU,
/// IPv4/IPv6 addresses, MAC address, and other platform-specific options.
///
/// # Examples
///
/// Creating a basic IPv4 TUN interface:
///
/// ````no_run
/// use std::net::Ipv4Addr;
/// use tun_rs::DeviceBuilder;
///
/// fn main() -> std::io::Result<()> {
///     let tun = DeviceBuilder::new()
///         .name("my-tun")
///         .mtu(1500)
///         .ipv4(Ipv4Addr::new(10, 0, 0, 1), 24, None)
///         .build_sync()?;
///     Ok(())
/// }
/// ````
///
/// Creating an IPv6 TUN interface:
///
/// ````no_run
/// use std::net::Ipv6Addr;
/// use tun_rs::DeviceBuilder;
///
/// fn main() -> std::io::Result<()> {
///     let tun = DeviceBuilder::new()
///         .name("my-tun6")
///         .mtu(1500)
///         .ipv6(Ipv6Addr::new(0xfd00, 0, 0, 0, 0, 0, 0, 1), 64)
///         .build_sync()?;
///     Ok(())
/// }
/// ````
///
/// Creating an L2 TAP interface (platform-dependent):
///
/// ````no_run
/// #[cfg(any(
///     target_os = "windows",
///     all(target_os = "linux", not(target_env = "ohos")),
///     target_os = "freebsd",
///     target_os = "macos",
///     target_os = "openbsd",
///     target_os = "netbsd"
/// ))]
/// use tun_rs::{DeviceBuilder, Layer};
///
/// #[cfg(any(
///     target_os = "windows",
///     all(target_os = "linux", not(target_env = "ohos")),
///     target_os = "freebsd",
///     target_os = "macos",
///     target_os = "openbsd",
///     target_os = "netbsd"
/// ))]
/// fn main() -> std::io::Result<()> {
///     let tap = DeviceBuilder::new()
///         .name("my-tap")
///         .layer(Layer::L2)
///         .mac_addr([0x00, 0x11, 0x22, 0x33, 0x44, 0x55])
///         .mtu(1500)
///         .build_sync()?;
///     Ok(())
/// }
/// ````
#[doc(hidden)]
pub struct DeviceBuilderGuard<'a>(&'a mut DeviceBuilder);

#[doc(hidden)]
impl DeviceBuilderGuard<'_> {
    /// Sets the device description (effective only on Windows L3 mode).
    #[cfg(windows)]
    pub fn description<S: Into<String>>(&mut self, description: S) -> &mut Self {
        self.0.description = Some(description.into());
        self
    }

    /// Sets the IPv4 MTU specifically for Windows.
    #[cfg(windows)]
    pub fn mtu_v4(&mut self, mtu: u16) -> &mut Self {
        self.0.mtu = Some(mtu);
        self
    }
    /// Sets the IPv6 MTU specifically for Windows.
    #[cfg(windows)]
    pub fn mtu_v6(&mut self, mtu: u16) -> &mut Self {
        self.0.mtu_v6 = Some(mtu);
        self
    }
    /// Sets the MAC address for the device (effective only in L2 mode).
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "macos",
        target_os = "netbsd"
    ))]
    pub fn mac_addr(&mut self, mac_addr: [u8; 6]) -> &mut Self {
        self.0.mac_addr = Some(mac_addr);
        self
    }

    /// Sets the device GUID on Windows.
    /// By default, GUID is chosen by the system at random.
    #[cfg(windows)]
    pub fn device_guid(&mut self, device_guid: u128) -> &mut Self {
        self.0.device_guid = Some(device_guid);
        self
    }
    /// Enables or disables Wintun logging.
    ///
    /// By default, logging is disabled.
    #[cfg(windows)]
    pub fn wintun_log(&mut self, wintun_log: bool) -> &mut Self {
        self.0.wintun_log = Some(wintun_log);
        self
    }
    /// Sets the `wintun.dll` file path on Windows.
    #[cfg(windows)]
    pub fn wintun_file(&mut self, wintun_file: String) -> &mut Self {
        self.0.wintun_file = Some(wintun_file);
        self
    }
    /// Sets the ring capacity on Windows.
    /// This specifies the capacity of the packet ring buffer in bytes.
    /// By default, the ring capacity is set to `0x20_0000` (2 MB).
    #[cfg(windows)]
    pub fn ring_capacity(&mut self, ring_capacity: u32) -> &mut Self {
        self.0.ring_capacity = Some(ring_capacity);
        self
    }
    /// Sets the routing metric (routing cost) for the interface on Windows.
    ///
    /// The metric determines the priority of this interface when multiple routes exist
    /// to the same destination. Lower metric values have higher priority.
    ///
    /// # Arguments
    ///
    /// * `metric` - The metric value (lower values = higher priority)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "windows")]
    /// # {
    /// use tun_rs::DeviceBuilder;
    ///
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.metric(10)  // Set lower metric for higher priority
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// Windows only.
    #[cfg(windows)]
    pub fn metric(&mut self, metric: u16) -> &mut Self {
        self.0.metric = Some(metric);
        self
    }
    /// Whether to call `WintunDeleteDriver` to remove the driver.
    /// Default: false.
    /// # Note
    /// The clean-up work closely depends on whether the destructor can be normally executed
    #[cfg(windows)]
    pub fn delete_driver(&mut self, delete_driver: bool) -> &mut Self {
        self.0.delete_driver = Some(delete_driver);
        self
    }
    /// Sets the transmit queue length for the network interface on Linux.
    ///
    /// The transmit queue length controls how many packets can be queued for
    /// transmission by the network stack. A larger queue can help with bursty
    /// traffic but may increase latency.
    ///
    /// # Arguments
    ///
    /// * `tx_queue_len` - The queue length in packets (typical values: 100-10000)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "linux")]
    /// # {
    /// use tun_rs::DeviceBuilder;
    ///
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.tx_queue_len(1000)  // Set queue length to 1000 packets
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// Linux only.
    #[cfg(target_os = "linux")]
    pub fn tx_queue_len(&mut self, tx_queue_len: u32) -> &mut Self {
        self.0.tx_queue_len = Some(tx_queue_len);
        self
    }
    /// Enables Generic Segmentation Offload (GSO) and Generic Receive Offload (GRO) on Linux.
    ///
    /// When enabled, the TUN device can handle larger packets, allowing the kernel to perform
    /// segmentation and coalescing for improved network throughput. This is particularly beneficial
    /// for high-bandwidth TCP and UDP traffic.
    ///
    /// After enabling offload, you should use [`recv_multiple`](crate::SyncDevice::recv_multiple)
    /// and [`send_multiple`](crate::SyncDevice::send_multiple) for optimal performance.
    ///
    /// # Arguments
    ///
    /// * `offload` - `true` to enable offload, `false` to disable (default: false)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "linux")]
    /// # {
    /// use tun_rs::{DeviceBuilder, GROTable, VIRTIO_NET_HDR_LEN, IDEAL_BATCH_SIZE};
    ///
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.offload(true)  // Enable TSO/GSO/GRO
    ///     })
    ///     .build_sync()?;
    ///
    /// // Use with recv_multiple for best performance
    /// let mut gro_table = GROTable::default();
    /// let mut original_buffer = vec![0u8; VIRTIO_NET_HDR_LEN + 65535];
    /// let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
    /// let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    ///
    /// let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;
    /// println!("Received {} packets", num);
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Performance
    ///
    /// Enabling offload can provide 2-10x throughput improvement for TCP traffic.
    ///
    /// # Platform
    ///
    /// Linux only. Requires kernel support for IFF_VNET_HDR (Linux 2.6.32+).
    #[cfg(target_os = "linux")]
    pub fn offload(&mut self, offload: bool) -> &mut Self {
        self.0.offload = Some(offload);
        self
    }
    /// Enables multi-queue support for parallel packet processing on Linux.
    ///
    /// When enabled, the TUN device can be cloned to create multiple queues, allowing
    /// packet processing to be distributed across multiple CPU cores for improved performance.
    /// Each queue can be used independently in separate threads.
    ///
    /// # Arguments
    ///
    /// * `multi_queue` - `true` to enable multi-queue support (default: false)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "linux")]
    /// # {
    /// use std::thread;
    /// use tun_rs::DeviceBuilder;
    ///
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.multi_queue(true)  // Enable multi-queue
    ///     })
    ///     .build_sync()?;
    ///
    /// // Clone the device for use in another thread
    /// let dev_clone = dev.try_clone()?;
    ///
    /// thread::spawn(move || {
    ///     let mut buf = [0u8; 1500];
    ///     loop {
    ///         if let Ok(n) = dev_clone.recv(&mut buf) {
    ///             println!("Thread 2: {} bytes", n);
    ///         }
    ///     }
    /// });
    ///
    /// // Process in main thread
    /// let mut buf = [0u8; 1500];
    /// loop {
    ///     let n = dev.recv(&mut buf)?;
    ///     println!("Thread 1: {} bytes", n);
    /// }
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Performance
    ///
    /// Multi-queue allows parallel packet processing across CPU cores, improving throughput
    /// for multi-core systems.
    ///
    /// # Platform
    ///
    /// Linux only. Requires kernel support for IFF_MULTI_QUEUE.
    #[cfg(target_os = "linux")]
    pub fn multi_queue(&mut self, multi_queue: bool) -> &mut Self {
        self.0.multi_queue = Some(multi_queue);
        self
    }
    /// Enables or disables packet information for the network driver(TUN)
    /// on macOS, Linux, freebsd, openbsd, netbsd.
    ///
    /// This option is disabled by default (`false`).
    /// # Note
    /// There is no native way to enable/disable packet information on macOS.
    /// The elimination of the packet information on macOS according to this setting
    /// is processed by this library.
    /// The set value `v` can be retrieved by `ignore_packet_info`, the returned value is `!v`.
    #[cfg(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub fn packet_information(&mut self, packet_information: bool) -> &mut Self {
        self.0.packet_information = Some(packet_information);
        self
    }
    /// Creates a pair of `feth` devices for TAP mode on macOS.
    ///
    /// On macOS, TAP mode (Layer 2) is implemented using a pair of fake Ethernet (`feth`)
    /// devices. One device is used for I/O operations, and the other (specified by `peer_feth`)
    /// is the peer interface. Both devices must be configured and brought up for proper operation.
    ///
    /// # Arguments
    ///
    /// * `peer_feth` - The name of the peer interface (e.g., "feth1")
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "macos")]
    /// # {
    /// use tun_rs::{DeviceBuilder, Layer};
    ///
    /// // Create a TAP interface with a peer device
    /// let dev = DeviceBuilder::new()
    ///     .name("feth0")
    ///     .layer(Layer::L2)
    ///     .with(|builder| {
    ///         builder.peer_feth("feth1")  // Specify the peer interface name
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// macOS only, Layer 2 (TAP) mode only.
    #[cfg(target_os = "macos")]
    pub fn peer_feth<S: Into<String>>(&mut self, peer_feth: S) -> &mut Self {
        self.0.peer_feth = Some(peer_feth.into());
        self
    }
    /// Controls automatic route management on BSD and macOS platforms.
    ///
    /// When enabled (the default), the library automatically adds or removes routes
    /// to provide consistent routing behavior across all platforms. When disabled,
    /// the system's native routing behavior is used.
    ///
    /// # Arguments
    ///
    /// * `associate_route` - `true` to enable automatic route management (default),
    ///   `false` to use native system routing
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(any(target_os = "macos", target_os = "freebsd"))]
    /// # {
    /// use tun_rs::DeviceBuilder;
    ///
    /// // Use native system routing without automatic route management
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.associate_route(false)  // Disable automatic route management
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// macOS, FreeBSD, OpenBSD, NetBSD only.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub fn associate_route(&mut self, associate_route: bool) -> &mut Self {
        self.0.associate_route = Some(associate_route);
        self
    }
    /// Controls whether to reuse an existing device with the same name.
    ///
    /// In TAP mode, if a device with the specified name already exists:
    /// - When `true` (default): The existing device will be reused
    /// - When `false`: An error will be returned
    ///
    /// # Arguments
    ///
    /// * `reuse` - `true` to reuse existing devices (default), `false` to error on conflicts
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(any(target_os = "macos", target_os = "windows"))]
    /// # {
    /// use tun_rs::{DeviceBuilder, Layer};
    ///
    /// // Don't reuse existing device - fail if it already exists
    /// let dev = DeviceBuilder::new()
    ///     .name("tap0")
    ///     .layer(Layer::L2)
    ///     .with(|builder| {
    ///         builder.reuse_dev(false)  // Error if tap0 already exists
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// macOS, Windows, NetBSD only. TAP mode (Layer 2) only.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "netbsd"))]
    pub fn reuse_dev(&mut self, reuse: bool) -> &mut Self {
        self.0.reuse_dev = Some(reuse);
        self
    }
    /// Controls whether the TAP device persists after the program exits.
    ///
    /// In TAP mode:
    /// - When `true`: The device remains after the program terminates
    /// - When `false` (default): The device is automatically destroyed on exit
    ///
    /// # Arguments
    ///
    /// * `persist` - `true` to keep the device after exit, `false` to auto-destroy (default)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(any(target_os = "macos", target_os = "windows"))]
    /// # {
    /// use tun_rs::{DeviceBuilder, Layer};
    ///
    /// // Create a persistent TAP device that survives program exit
    /// let dev = DeviceBuilder::new()
    ///     .name("tap0")
    ///     .layer(Layer::L2)
    ///     .with(|builder| {
    ///         builder.persist(true)  // Keep device after program exits
    ///     })
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// macOS, Windows only. TAP mode (Layer 2) only.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn persist(&mut self, persist: bool) -> &mut Self {
        self.0.persist = Some(persist);
        self
    }
}
/// This is a unified constructor of a device for various platforms. The specification of every API can be found by looking at
/// the documentation of the concrete platform.
#[derive(Default)]
pub struct DeviceBuilder {
    dev_name: Option<String>,
    #[cfg(windows)]
    description: Option<String>,
    #[cfg(target_os = "macos")]
    peer_feth: Option<String>,
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    associate_route: Option<bool>,
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "netbsd"))]
    reuse_dev: Option<bool>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    persist: Option<bool>,
    enabled: Option<bool>,
    mtu: Option<u16>,
    #[cfg(windows)]
    mtu_v6: Option<u16>,
    ipv4: Option<IPV4>,
    ipv6: Option<Vec<(io::Result<Ipv6Addr>, io::Result<u8>)>>,
    layer: Option<Layer>,
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "macos",
        target_os = "netbsd"
    ))]
    mac_addr: Option<[u8; 6]>,
    #[cfg(windows)]
    device_guid: Option<u128>,
    #[cfg(windows)]
    wintun_log: Option<bool>,
    #[cfg(windows)]
    wintun_file: Option<String>,
    #[cfg(windows)]
    ring_capacity: Option<u32>,
    #[cfg(windows)]
    metric: Option<u16>,
    #[cfg(windows)]
    delete_driver: Option<bool>,
    /// switch of Enable/Disable packet information for network driver
    #[cfg(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    packet_information: Option<bool>,
    #[cfg(target_os = "linux")]
    tx_queue_len: Option<u32>,
    /// Enable/Disable TUN offloads.
    /// After enabling, use `recv_multiple`/`send_multiple` for data transmission.
    #[cfg(target_os = "linux")]
    offload: Option<bool>,
    /// Enable multi queue support
    #[cfg(target_os = "linux")]
    multi_queue: Option<bool>,
}

impl DeviceBuilder {
    /// Creates a new DeviceBuilder instance with default settings.
    pub fn new() -> Self {
        Self::default().enable(true)
    }
    /// Sets the device name.
    pub fn name<S: Into<String>>(mut self, dev_name: S) -> Self {
        self.dev_name = Some(dev_name.into());
        self
    }
    /// Sets the device description (effective only on Windows L3 mode).
    #[cfg(windows)]
    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }
    /// Sets the device MTU (Maximum Transmission Unit).
    pub fn mtu(mut self, mtu: u16) -> Self {
        self.mtu = Some(mtu);
        #[cfg(windows)]
        {
            // On Windows, also set the MTU for IPv6.
            self.mtu_v6 = Some(mtu);
        }
        self
    }
    /// Sets the IPv4 MTU specifically for Windows.
    #[cfg(windows)]
    pub fn mtu_v4(mut self, mtu: u16) -> Self {
        self.mtu = Some(mtu);
        self
    }
    /// Sets the IPv6 MTU specifically for Windows.
    #[cfg(windows)]
    pub fn mtu_v6(mut self, mtu: u16) -> Self {
        self.mtu_v6 = Some(mtu);
        self
    }
    /// Sets the MAC address for the device (effective only in L2 mode).
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "macos",
        target_os = "netbsd"
    ))]
    pub fn mac_addr(mut self, mac_addr: [u8; 6]) -> Self {
        self.mac_addr = Some(mac_addr);
        self
    }
    /// Configures the IPv4 address for the device.
    ///
    /// - `address`: The IPv4 address of the device.
    /// - `mask`: The subnet mask or prefix length.
    /// - `destination`: Optional destination address for point-to-point links.
    /// # Example
    /// ```
    /// use std::net::Ipv4Addr;
    /// use tun_rs::DeviceBuilder;
    /// DeviceBuilder::new().ipv4(Ipv4Addr::new(10, 0, 0, 12), 24, None);
    /// ```
    pub fn ipv4<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        mut self,
        address: IPv4,
        mask: Netmask,
        destination: Option<IPv4>,
    ) -> Self {
        self.ipv4 = Some((address.ipv4(), mask.prefix(), destination.map(|v| v.ipv4())));
        self
    }
    /// Configures a single IPv6 address for the device.
    ///
    /// - `address`: The IPv6 address.
    /// - `mask`: The subnet mask or prefix length.
    /// # Example
    /// ```
    /// use tun_rs::DeviceBuilder;
    /// DeviceBuilder::new().ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64);
    /// ```
    pub fn ipv6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        mut self,
        address: IPv6,
        mask: Netmask,
    ) -> Self {
        if let Some(v) = &mut self.ipv6 {
            v.push((address.ipv6(), mask.prefix()));
        } else {
            self.ipv6 = Some(vec![(address.ipv6(), mask.prefix())]);
        }

        self
    }
    /// Configures multiple IPv6 addresses in batch.
    ///
    /// Accepts a slice of (IPv6 address, netmask) tuples.
    /// # Example
    /// ```rust
    /// use tun_rs::DeviceBuilder;
    /// DeviceBuilder::new().ipv6_tuple(&[
    ///     ("CDCD:910A:2222:5498:8475:1111:3900:2022", 64),
    ///     ("CDCD:910A:2222:5498:8475:1111:3900:2023", 64),
    /// ]);
    /// ```
    pub fn ipv6_tuple<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        mut self,
        addrs: &[(IPv6, Netmask)],
    ) -> Self {
        if let Some(v) = &mut self.ipv6 {
            for (address, mask) in addrs {
                v.push((address.ipv6(), mask.prefix()));
            }
        } else {
            self.ipv6 = Some(
                addrs
                    .iter()
                    .map(|(ip, mask)| (ip.ipv6(), mask.prefix()))
                    .collect(),
            );
        }
        self
    }
    /// Sets the operating layer (L2 or L3) for the device.
    ///
    /// * L2 corresponds to TAP
    /// * L3 corresponds to TUN
    pub fn layer(mut self, layer: Layer) -> Self {
        self.layer = Some(layer);
        self
    }
    /// Sets the device GUID on Windows.
    /// By default, GUID is chosen by the system at random.
    #[cfg(windows)]
    pub fn device_guid(mut self, device_guid: u128) -> Self {
        self.device_guid = Some(device_guid);
        self
    }
    /// Enables or disables Wintun logging.
    ///
    /// By default, logging is disabled.
    #[cfg(windows)]
    pub fn wintun_log(mut self, wintun_log: bool) -> Self {
        self.wintun_log = Some(wintun_log);
        self
    }
    /// Sets the `wintun.dll` file path on Windows.
    #[cfg(windows)]
    pub fn wintun_file(mut self, wintun_file: String) -> Self {
        self.wintun_file = Some(wintun_file);
        self
    }
    /// Sets the ring capacity on Windows.
    /// This specifies the capacity of the packet ring buffer in bytes.
    /// By default, the ring capacity is set to `0x20_0000` (2 MB).
    #[cfg(windows)]
    pub fn ring_capacity(mut self, ring_capacity: u32) -> Self {
        self.ring_capacity = Some(ring_capacity);
        self
    }
    /// Sets the routing metric (routing cost) for the interface on Windows.
    ///
    /// The metric determines the priority of this interface when multiple routes exist
    /// to the same destination. Lower metric values have higher priority.
    ///
    /// # Arguments
    ///
    /// * `metric` - The metric value (lower values = higher priority)
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(target_os = "windows")]
    /// # {
    /// use tun_rs::DeviceBuilder;
    ///
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .metric(10)  // Set lower metric for higher priority
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # Platform
    ///
    /// Windows only.
    #[cfg(windows)]
    pub fn metric(mut self, metric: u16) -> Self {
        self.metric = Some(metric);
        self
    }
    /// Whether to call `WintunDeleteDriver` to remove the driver.
    /// Default: false.
    /// # Note
    /// The clean-up work closely depends on whether the destructor can be normally executed
    #[cfg(windows)]
    pub fn delete_driver(mut self, delete_driver: bool) -> Self {
        self.delete_driver = Some(delete_driver);
        self
    }
    /// Sets the transmit queue length on Linux.
    #[cfg(target_os = "linux")]
    pub fn tx_queue_len(mut self, tx_queue_len: u32) -> Self {
        self.tx_queue_len = Some(tx_queue_len);
        self
    }
    /// Enables TUN offloads on Linux.
    /// After enabling, use `recv_multiple`/`send_multiple` for data transmission.
    #[cfg(target_os = "linux")]
    pub fn offload(mut self, offload: bool) -> Self {
        self.offload = Some(offload);
        self
    }
    /// Enables multi-queue support on Linux.
    #[cfg(target_os = "linux")]
    pub fn multi_queue(mut self, multi_queue: bool) -> Self {
        self.multi_queue = Some(multi_queue);
        self
    }
    /// Enables or disables packet information for the network driver(TUN)
    /// on macOS, Linux, freebsd, openbsd, netbsd.
    ///
    /// This option is disabled by default (`false`).
    /// # Note
    /// There is no native way to enable/disable packet information on macOS.
    /// The elimination of the packet information on macOS according to this setting
    /// is processed by this library.
    /// The set value `v` can be retrieved by `ignore_packet_info`, the returned value is `!v`.
    #[cfg(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub fn packet_information(mut self, packet_information: bool) -> Self {
        self.packet_information = Some(packet_information);
        self
    }
    /// Available on Layer::L2;
    /// creates a pair of `feth` devices, with `peer_feth` as the IO interface name.
    #[cfg(target_os = "macos")]
    pub fn peer_feth<S: Into<String>>(mut self, peer_feth: S) -> Self {
        self.peer_feth = Some(peer_feth.into());
        self
    }
    /// If true (default), the program will automatically add or remove routes on macOS or FreeBSD to provide consistent routing behavior across all platforms.
    /// If false, the program will not modify or manage routes in any way, allowing the system to handle all routing natively.
    /// Set this to be false to obtain the platform's default routing behavior.
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub fn associate_route(mut self, associate_route: bool) -> Self {
        self.associate_route = Some(associate_route);
        self
    }
    /// Only works in TAP mode.
    /// If true (default), the existing device with the given name will be used if possible.
    /// If false, an error will be returned if a device with the specified name already exists.
    #[cfg(any(target_os = "macos", target_os = "windows", target_os = "netbsd"))]
    pub fn reuse_dev(mut self, reuse: bool) -> Self {
        self.reuse_dev = Some(reuse);
        self
    }
    /// Only works in TAP mode.
    /// If true, the `feth` device will be kept after the program exits;
    /// if false (default), the device will be destroyed automatically.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn persist(mut self, persist: bool) -> Self {
        self.persist = Some(persist);
        self
    }
    /// Enables or disables the network interface upon creation.
    ///
    /// By default, newly created TUN/TAP devices are enabled (brought up).
    /// Use this method to control whether the device should be automatically enabled
    /// or left in a disabled state.
    ///
    /// # Arguments
    ///
    /// * `enable` - `true` to enable the device (default), `false` to leave it disabled
    ///
    /// # Example
    ///
    /// ```no_run
    /// use tun_rs::DeviceBuilder;
    ///
    /// // Create a device but leave it disabled initially
    /// let dev = DeviceBuilder::new()
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .enable(false)  // Don't enable the device yet
    ///     .build_sync()?;
    ///
    /// // Later, enable it manually using platform-specific methods
    /// // dev.enabled(true)?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # See Also
    ///
    /// - [`inherit_enable_state`](Self::inherit_enable_state) - Preserve existing device state
    pub fn enable(mut self, enable: bool) -> Self {
        self.enabled = Some(enable);
        self
    }

    /// Preserves the existing enable state of the network interface.
    ///
    /// When reusing an existing device, this method prevents the builder from
    /// explicitly setting the device's enable/disable state. The device will
    /// retain whatever state it currently has in the system.
    ///
    /// This is particularly useful when:
    /// - Reconnecting to an existing TUN/TAP device
    /// - You want to preserve the current system configuration
    /// - Avoiding unnecessary state changes that might disrupt existing connections
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    /// # {
    /// use tun_rs::DeviceBuilder;
    ///
    /// // Reuse an existing device and keep its current enabled state
    /// let dev = DeviceBuilder::new()
    ///     .name("tun0")
    ///     .ipv4("10.0.0.1", 24, None)
    ///     .with(|builder| {
    ///         builder.reuse_dev(true)
    ///     })
    ///     .inherit_enable_state()  // Don't change the existing enable state
    ///     .build_sync()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    ///
    /// # See Also
    ///
    /// - [`enable`](Self::enable) - Explicitly enable or disable the device
    pub fn inherit_enable_state(mut self) -> Self {
        self.enabled = None;
        self
    }
    pub(crate) fn build_config(&mut self) -> DeviceConfig {
        DeviceConfig {
            dev_name: self.dev_name.take(),
            #[cfg(windows)]
            description: self.description.take(),
            #[cfg(target_os = "macos")]
            peer_feth: self.peer_feth.take(),
            #[cfg(any(
                target_os = "macos",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            associate_route: self.associate_route,
            #[cfg(any(target_os = "macos", target_os = "windows", target_os = "netbsd"))]
            reuse_dev: self.reuse_dev,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            persist: self.persist,
            layer: self.layer.take(),
            #[cfg(windows)]
            device_guid: self.device_guid.take(),
            #[cfg(windows)]
            wintun_log: self.wintun_log.take(),
            #[cfg(windows)]
            wintun_file: self.wintun_file.take(),
            #[cfg(windows)]
            ring_capacity: self.ring_capacity.take(),
            #[cfg(windows)]
            delete_driver: self.delete_driver.take(),
            #[cfg(windows)]
            mac_address: self.mac_addr.map(|v| {
                use std::fmt::Write;
                v.iter()
                    .fold(String::with_capacity(v.len() * 2), |mut s, b| {
                        write!(&mut s, "{b:02X}").unwrap();
                        s
                    })
            }),
            #[cfg(any(
                target_os = "macos",
                target_os = "linux",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            packet_information: self.packet_information.take(),
            #[cfg(target_os = "linux")]
            offload: self.offload.take(),
            #[cfg(target_os = "linux")]
            multi_queue: self.multi_queue.take(),
        }
    }
    pub(crate) fn config(self, device: &DeviceImpl) -> io::Result<()> {
        if let Some(mtu) = self.mtu {
            device.set_mtu(mtu)?;
        }
        #[cfg(windows)]
        if let Some(mtu) = self.mtu_v6 {
            device.set_mtu_v6(mtu)?;
        }
        #[cfg(windows)]
        if let Some(metric) = self.metric {
            device.set_metric(metric)?;
        }
        #[cfg(target_os = "linux")]
        if let Some(tx_queue_len) = self.tx_queue_len {
            device.set_tx_queue_len(tx_queue_len)?;
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "macos",
            target_os = "openbsd",
            target_os = "netbsd"
        ))]
        if let Some(mac_addr) = self.mac_addr {
            device.set_mac_address(mac_addr)?;
        }

        if let Some((address, prefix, destination)) = self.ipv4 {
            let prefix = prefix?;
            let address = address?;
            let destination = destination.transpose()?;
            device.set_network_address(address, prefix, destination)?;
        }
        if let Some(ipv6) = self.ipv6 {
            for (address, prefix) in ipv6 {
                let prefix = prefix?;
                let address = address?;
                device.add_address_v6(address, prefix)?;
            }
        }
        if let Some(enabled) = self.enabled {
            device.enabled(enabled)?;
        }
        Ok(())
    }
    /// Builds a synchronous device instance and applies all configuration parameters.
    pub fn build_sync(mut self) -> io::Result<SyncDevice> {
        let device = DeviceImpl::new(self.build_config())?;
        self.config(&device)?;
        Ok(SyncDevice(device))
    }
    /// Builds an asynchronous device instance.
    ///
    /// This method is available only when either async_io or async_tokio feature is enabled.
    ///
    /// # Note
    /// Both async runtimes can now be enabled simultaneously. When both are enabled,
    /// this method returns a device compatible with the Tokio runtime (for backward compatibility).
    /// Use `build_tokio_async()` or `build_async_io()` to explicitly choose a runtime.
    #[cfg(any(feature = "async_io", feature = "async_tokio"))]
    pub fn build_async(self) -> io::Result<crate::AsyncDevice> {
        let sync_device = self.build_sync()?;
        let device = crate::AsyncDevice::new_dev(sync_device.0)?;
        Ok(device)
    }

    /// Build a TUN/TAP device with Tokio async runtime support.
    ///
    /// This method explicitly creates a device for the Tokio runtime.
    #[cfg(feature = "async_tokio")]
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn build_tokio_async(self) -> io::Result<crate::TokioAsyncDevice> {
        let sync_device = self.build_sync()?;
        let device = crate::TokioAsyncDevice::new_dev(sync_device.0)?;
        Ok(device)
    }

    /// Build a TUN/TAP device with async-io runtime support.
    ///
    /// This method explicitly creates a device for the async-io runtime (async-std, smol, etc.).
    #[cfg(feature = "async_io")]
    #[cfg(all(unix, not(target_os = "macos")))]
    pub fn build_async_io(self) -> io::Result<crate::AsyncIoDevice> {
        let sync_device = self.build_sync()?;
        let device = crate::AsyncIoDevice::new_dev(sync_device.0)?;
        Ok(device)
    }
    /// To conveniently set the platform-specific parameters without breaking the calling chain.
    /// # Ergonomic
    ///
    /// For example:
    /// ````no_run
    /// use tun_rs::DeviceBuilder;
    /// let builder = DeviceBuilder::new().name("tun1");
    /// #[cfg(target_os = "macos")]
    /// let builder = builder.associate_route(false);
    /// #[cfg(windows)]
    /// let builder = builder.wintun_log(false);
    /// let dev = builder.build_sync().unwrap();
    /// ````
    /// This is tedious and breaks the calling chain.
    ///
    /// With `with`, we can just set platform-specific parameters as follows without breaking the calling chain:
    /// ````no_run
    /// use tun_rs::DeviceBuilder;
    /// let dev = DeviceBuilder::new().name("tun1").with(|opt|{
    ///    #[cfg(windows)]
    ///    opt.wintun_log(false);
    ///    #[cfg(target_os = "macos")]
    ///    opt.associate_route(false).packet_information(false);
    /// }).build_sync().unwrap();
    /// ````
    pub fn with<F: FnMut(&mut DeviceBuilderGuard)>(mut self, mut f: F) -> Self {
        let mut borrow = DeviceBuilderGuard(&mut self);
        f(&mut borrow);
        self
    }
}

/// Trait for converting various types into an IPv4 address.
pub trait ToIpv4Address {
    /// Attempts to convert the implementing type into an `Ipv4Addr`.
    /// Returns the IPv4 address on success or an error on failure.
    fn ipv4(&self) -> io::Result<Ipv4Addr>;
}
impl ToIpv4Address for Ipv4Addr {
    fn ipv4(&self) -> io::Result<Ipv4Addr> {
        Ok(*self)
    }
}
impl ToIpv4Address for IpAddr {
    fn ipv4(&self) -> io::Result<Ipv4Addr> {
        match self {
            IpAddr::V4(ip) => Ok(*ip),
            IpAddr::V6(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid address",
            )),
        }
    }
}
impl ToIpv4Address for String {
    fn ipv4(&self) -> io::Result<Ipv4Addr> {
        self.as_str().ipv4()
    }
}
impl ToIpv4Address for &str {
    fn ipv4(&self) -> io::Result<Ipv4Addr> {
        match Ipv4Addr::from_str(self) {
            Ok(ip) => Ok(ip),
            Err(_e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IPv4 str",
            )),
        }
    }
}

/// Trait for converting various types into an IPv6 address.
pub trait ToIpv6Address {
    /// Attempts to convert the implementing type into an `Ipv6Addr`.
    /// Returns the IPv6 address on success or an error on failure.
    fn ipv6(&self) -> io::Result<Ipv6Addr>;
}

impl ToIpv6Address for Ipv6Addr {
    fn ipv6(&self) -> io::Result<Ipv6Addr> {
        Ok(*self)
    }
}
impl ToIpv6Address for IpAddr {
    fn ipv6(&self) -> io::Result<Ipv6Addr> {
        match self {
            IpAddr::V4(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid address",
            )),
            IpAddr::V6(ip) => Ok(*ip),
        }
    }
}
impl ToIpv6Address for String {
    fn ipv6(&self) -> io::Result<Ipv6Addr> {
        self.as_str().ipv6()
    }
}
impl ToIpv6Address for &str {
    fn ipv6(&self) -> io::Result<Ipv6Addr> {
        match Ipv6Addr::from_str(self) {
            Ok(ip) => Ok(ip),
            Err(_e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IPv6 str",
            )),
        }
    }
}
/// Trait for converting various types into an IPv4 netmask (prefix length).
pub trait ToIpv4Netmask {
    /// Returns the prefix length (i.e., the number of consecutive 1s in the netmask).
    fn prefix(&self) -> io::Result<u8>;
    /// Computes the IPv4 netmask based on the prefix length.
    fn netmask(&self) -> io::Result<Ipv4Addr> {
        let ip = u32::MAX
            .checked_shl(32 - self.prefix()? as u32)
            .unwrap_or(0);
        Ok(Ipv4Addr::from(ip))
    }
}

impl ToIpv4Netmask for u8 {
    fn prefix(&self) -> io::Result<u8> {
        if *self > 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IP prefix length",
            ));
        }
        Ok(*self)
    }
}

impl ToIpv4Netmask for Ipv4Addr {
    fn prefix(&self) -> io::Result<u8> {
        let ip = u32::from_be_bytes(self.octets());
        // Validate that the netmask is contiguous (all 1s followed by all 0s).
        if ip.leading_ones() != ip.count_ones() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid netmask",
            ));
        }
        Ok(ip.leading_ones() as u8)
    }
}
impl ToIpv4Netmask for String {
    fn prefix(&self) -> io::Result<u8> {
        ToIpv4Netmask::prefix(&self.as_str())
    }
}
impl ToIpv4Netmask for &str {
    fn prefix(&self) -> io::Result<u8> {
        match Ipv4Addr::from_str(self) {
            Ok(ip) => ip.prefix(),
            Err(_e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid netmask str",
            )),
        }
    }
}
/// Trait for converting various types into an IPv6 netmask (prefix length).
pub trait ToIpv6Netmask {
    /// Returns the prefix length.
    fn prefix(&self) -> io::Result<u8>;
    /// Computes the IPv6 netmask based on the prefix length.
    fn netmask(&self) -> io::Result<Ipv6Addr> {
        let ip = u128::MAX
            .checked_shl(128 - self.prefix()? as u32)
            .unwrap_or(0);
        Ok(Ipv6Addr::from(ip))
    }
}

impl ToIpv6Netmask for u8 {
    fn prefix(&self) -> io::Result<u8> {
        if *self > 128 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid IP prefix length",
            ));
        }
        Ok(*self)
    }
}

impl ToIpv6Netmask for Ipv6Addr {
    fn prefix(&self) -> io::Result<u8> {
        let ip = u128::from_be_bytes(self.octets());
        if ip.leading_ones() != ip.count_ones() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid netmask",
            ));
        }
        Ok(ip.leading_ones() as u8)
    }
}
impl ToIpv6Netmask for String {
    fn prefix(&self) -> io::Result<u8> {
        ToIpv6Netmask::prefix(&self.as_str())
    }
}
impl ToIpv6Netmask for &str {
    fn prefix(&self) -> io::Result<u8> {
        match Ipv6Addr::from_str(self) {
            Ok(ip) => ip.prefix(),
            Err(_e) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid netmask str",
            )),
        }
    }
}
