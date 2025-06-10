use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use crate::platform::{DeviceImpl, SyncDevice};

/// Represents the OSI layer at which the TUN interface operates.
///
/// - **L2**: Data Link Layer (available on Windows, Linux, and FreeBSD; used for TAP interfaces).
/// - **L3**: Network Layer (default for TUN interfaces).
#[derive(Clone, Copy, Default, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Layer {
    #[cfg(any(
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "macos"
    ))]
    L2,
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
    pub dev_name: Option<String>,
    /// The description of the device/interface.
    #[cfg(windows)]
    pub description: Option<String>,
    /// Available with Layer::L2; creates a pair of feth devices, with peer_feth as the IO interface name.
    #[cfg(target_os = "macos")]
    pub peer_feth: Option<String>,
    /// If true (default), the program will automatically add or remove routes on macOS or FreeBSD to provide consistent routing behavior across all platforms.
    /// If false, the program will not modify or manage routes in any way, allowing the system to handle all routing natively.
    /// Set this to be false to obtain the platform's default routing behavior.
    #[cfg(target_os = "macos")]
    pub associate_route: Option<bool>,
    /// If true (default), the existing device with the given name will be used if possible.
    /// If false, an error will be returned if a device with the specified name already exists.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub reuse_dev: Option<bool>,
    /// If true, the feth device will be kept after the program exits;
    /// if false (default), the device will be destroyed automatically.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub persist: Option<bool>,
    /// Specifies whether the interface operates at L2 or L3.
    #[allow(dead_code)]
    pub layer: Option<Layer>,
    /// Device GUID on Windows.
    #[cfg(windows)]
    pub device_guid: Option<u128>,
    /// Path to the wintun file on Windows.
    #[cfg(windows)]
    pub wintun_file: Option<String>,
    /// Capacity of the ring buffer on Windows.
    #[cfg(windows)]
    pub ring_capacity: Option<u32>,
    /// Whether to call WintunDeleteDriver to remove the driver.
    /// Default: false.
    #[cfg(windows)]
    pub delete_driver: Option<bool>,
    #[cfg(windows)]
    pub mac_address: Option<String>,
    /// switch of Enable/Disable packet information for network driver
    #[cfg(any(
        target_os = "tvos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "macos",
        target_os = "linux"
    ))]
    pub packet_information: Option<bool>,
    /// Enable/Disable TUN offloads.
    /// After enabling, use `recv_multiple`/`send_multiple` for data transmission.
    #[cfg(target_os = "linux")]
    pub offload: Option<bool>,
    /// Enable multi queue support
    #[cfg(target_os = "linux")]
    pub multi_queue: Option<bool>,
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
#[derive(Default)]
pub struct DeviceBuilder {
    dev_name: Option<String>,
    #[cfg(windows)]
    description: Option<String>,
    #[cfg(target_os = "macos")]
    peer_feth: Option<String>,
    #[cfg(target_os = "macos")]
    associate_route: Option<bool>,
    #[cfg(any(target_os = "macos", target_os = "windows"))]
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
        target_os = "macos"
    ))]
    mac_addr: Option<[u8; 6]>,
    #[cfg(windows)]
    device_guid: Option<u128>,
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
        target_os = "ios",
        target_os = "tvos",
        target_os = "macos",
        target_os = "linux"
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
        Self::default()
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
        target_os = "macos"
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
    pub fn ipv4<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        mut self,
        address: IPv4,
        mask: Netmask,
        destination: Option<IPv4>,
    ) -> Self {
        self.ipv4 = Some((address.ipv4(), mask.prefix(), destination.map(|v| v.ipv4())));
        self
    }
    /// Configures an IPv6 address for the device.
    ///
    /// - `address`: The IPv6 address.
    /// - `mask`: The subnet mask or prefix length.
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
    /// Sets the wintun file path on Windows.
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
    /// Sets the routing metric on Windows.
    #[cfg(windows)]
    pub fn metric(mut self, metric: u16) -> Self {
        self.metric = Some(metric);
        self
    }
    /// Whether to call WintunDeleteDriver to remove the driver.
    /// Default: false.
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
    /// Enables or disables packet information for the network driver
    /// on iOS, macOS, and Linux.
    ///
    /// This option is disabled by default (`false`).
    #[cfg(any(
        target_os = "ios",
        target_os = "tvos",
        target_os = "macos",
        target_os = "linux"
    ))]
    pub fn packet_information(mut self, packet_information: bool) -> Self {
        self.packet_information = Some(packet_information);
        self
    }
    /// Available with Layer::L2;
    /// creates a pair of feth devices, with peer_feth as the IO interface name.
    #[cfg(target_os = "macos")]
    pub fn peer_feth<S: Into<String>>(mut self, peer_feth: S) -> Self {
        self.peer_feth = Some(peer_feth.into());
        self
    }
    /// If true (default), the program will automatically add or remove routes on macOS or FreeBSD to provide consistent routing behavior across all platforms.
    /// If false, the program will not modify or manage routes in any way, allowing the system to handle all routing natively.
    /// Set this to false to obtain the platform's default routing behavior.
    #[cfg(target_os = "macos")]
    pub fn associate_route(mut self, associate_route: bool) -> Self {
        self.associate_route = Some(associate_route);
        self
    }
    /// Only effective in TAP mode.
    /// If true (default), the existing device with the given name will be used if possible.
    /// If false, an error will be returned if a device with the specified name already exists.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn reuse_dev(mut self, reuse: bool) -> Self {
        self.reuse_dev = Some(reuse);
        self
    }
    /// Only effective in TAP mode.
    /// If true, the feth device will be kept after the program exits;
    /// if false (default), the device will be destroyed automatically.
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    pub fn persist(mut self, persist: bool) -> Self {
        self.persist = Some(persist);
        self
    }
    /// Enables or disables the device.
    /// Defaults to enabled.
    pub fn enable(mut self, enable: bool) -> Self {
        self.enabled = Some(enable);
        self
    }
    pub(crate) fn build_config(&mut self) -> DeviceConfig {
        DeviceConfig {
            dev_name: self.dev_name.take(),
            #[cfg(windows)]
            description: self.description.take(),
            #[cfg(target_os = "macos")]
            peer_feth: self.peer_feth.take(),
            #[cfg(target_os = "macos")]
            associate_route: self.associate_route,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            reuse_dev: self.reuse_dev,
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            persist: self.persist,
            layer: self.layer.take(),
            #[cfg(windows)]
            device_guid: self.device_guid.take(),
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
                        write!(&mut s, "{:02X}", b).unwrap();
                        s
                    })
            }),
            #[cfg(any(
                target_os = "ios",
                target_os = "tvos",
                target_os = "macos",
                target_os = "linux"
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
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "macos"))]
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
        device.enabled(self.enabled.unwrap_or(true))?;
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
    /// This method is available only when the async_io or async_tokio features are enabled.
    #[cfg(any(feature = "async_io", feature = "async_tokio"))]
    pub fn build_async(self) -> io::Result<crate::AsyncDevice> {
        let sync_device = self.build_sync()?;
        let device = crate::AsyncDevice::new_dev(sync_device.0)?;
        Ok(device)
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
