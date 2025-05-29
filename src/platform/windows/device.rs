use getifaddrs::Interface;
use std::collections::HashSet;
use std::io;
use std::net::IpAddr;
use std::os::windows::io::OwnedHandle;

use crate::builder::DeviceConfig;
use crate::platform::windows::netsh;
use crate::platform::windows::tap::TapDevice;
use crate::platform::windows::tun::TunDevice;
use crate::platform::ETHER_ADDR_LEN;
use crate::{Layer, ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask};

pub(crate) enum Driver {
    Tun(TunDevice),
    Tap(TapDevice),
}

/// A TUN device using the wintun driver.
pub struct DeviceImpl {
    pub(crate) driver: Driver,
}

fn hash_name(input_str: &str) -> u128 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    8765028472139845610u64.hash(&mut hasher);
    input_str.hash(&mut hasher);
    let front = hasher.finish();

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    12874056902134875693u64.hash(&mut hasher);
    input_str.hash(&mut hasher);
    let back = hasher.finish();
    (u128::from(front) << 64) | u128::from(back)
}

impl DeviceImpl {
    /// Create a new `Device` for the given `Configuration`.
    pub(crate) fn new(config: DeviceConfig) -> io::Result<Self> {
        let layer = config.layer.unwrap_or(Layer::L3);
        let mut count = 0;
        let interfaces: HashSet<String> = Self::get_all_adapter_address()?
            .into_iter()
            .map(|v| v.name)
            .collect();
        let device = if layer == Layer::L3 {
            let wintun_file = config.wintun_file.as_deref().unwrap_or("wintun.dll");
            let ring_capacity = config.ring_capacity.unwrap_or(0x20_0000);
            let mut attempts = 0;
            let tun_device = loop {
                let default_name = format!("tun{count}");
                count += 1;
                let name = config.dev_name.as_deref().unwrap_or(&default_name);

                if interfaces.contains(name) {
                    if config.dev_name.is_none() {
                        continue;
                    }
                    Err(io::Error::other(format!(
                        "The network adapter [{name}] already exists."
                    )))?
                }
                let guid = config.device_guid.unwrap_or_else(|| hash_name(name));
                match TunDevice::create(wintun_file, name, name, guid, ring_capacity) {
                    Ok(tun_device) => break tun_device,
                    Err(e) => {
                        if attempts > 3 {
                            Err(e)?
                        }
                        attempts += 1;
                    }
                }
            };

            DeviceImpl {
                driver: Driver::Tun(tun_device),
            }
        } else if layer == Layer::L2 {
            const HARDWARE_ID: &str = "tap0901";
            let tap = loop {
                let default_name = format!("tap{count}");
                let name = config.dev_name.as_deref().unwrap_or(&default_name);
                if interfaces.contains(name) && config.dev_name.is_none() {
                    continue;
                }
                if let Ok(tap) = TapDevice::open(HARDWARE_ID, name) {
                    if config.dev_name.is_none() {
                        count += 1;
                        continue;
                    }
                    break tap;
                } else {
                    let tap = TapDevice::create(HARDWARE_ID)?;
                    if let Err(e) = tap.set_name(name) {
                        if config.dev_name.is_some() {
                            Err(e)?
                        }
                    }
                    break tap;
                }
            };
            DeviceImpl {
                driver: Driver::Tap(tap),
            }
        } else {
            panic!("unknown layer {:?}", layer);
        };
        Ok(device)
    }
    #[allow(dead_code)]
    pub(crate) fn wait_readable_cancelable(&self, cancel_event: &OwnedHandle) -> io::Result<()> {
        match &self.driver {
            Driver::Tap(tap) => tap.wait_readable_cancelable(cancel_event),
            Driver::Tun(tun) => tun.wait_readable_cancelable(cancel_event),
        }
    }
    /// Recv a packet from tun device
    pub(crate) fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self.driver {
            Driver::Tap(tap) => tap.read(buf),
            Driver::Tun(tun) => tun.recv(buf),
        }
    }
    pub(crate) fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self.driver {
            Driver::Tap(tap) => tap.try_read(buf),
            Driver::Tun(tun) => tun.try_recv(buf),
        }
    }

    /// Send a packet to tun device
    pub(crate) fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.driver {
            Driver::Tap(tap) => tap.write(buf),
            Driver::Tun(tun) => tun.send(buf),
        }
    }
    pub(crate) fn send_cancelable(
        &self,
        buf: &[u8],
        cancel_event: &OwnedHandle,
    ) -> io::Result<usize> {
        match &self.driver {
            Driver::Tap(tap) => tap.write_cancelable(buf, cancel_event),
            Driver::Tun(tun) => tun.send_cancelable(buf, cancel_event),
        }
    }
    pub(crate) fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.driver {
            Driver::Tap(tap) => tap.try_write(buf),
            Driver::Tun(tun) => tun.try_send(buf),
        }
    }
    pub(crate) fn shutdown(&self) -> io::Result<()> {
        match &self.driver {
            Driver::Tun(tun) => tun.shutdown(),
            Driver::Tap(tap) => tap.down(),
        }
    }
    fn get_all_adapter_address() -> io::Result<Vec<Interface>> {
        Ok(getifaddrs::getifaddrs()?.collect())
    }
    /// Retrieves the name of the device.
    ///
    /// Calls the appropriate method on the underlying driver (TUN or TAP) to obtain the device name.
    pub fn name(&self) -> io::Result<String> {
        match &self.driver {
            Driver::Tun(tun) => tun.get_name(),
            Driver::Tap(tap) => tap.get_name(),
        }
    }
    /// Sets a new name for the device.
    ///
    /// This method first checks if the current name is different from the desired one. If it is,
    /// it uses the `netsh` command to update the interface name.
    pub fn set_name(&self, value: &str) -> io::Result<()> {
        let name = self.name()?;
        if value == name {
            return Ok(());
        }
        netsh::set_interface_name(&name, value)
    }
    /// Retrieves the interface index (if_index) of the device.
    ///
    /// This is used for various network configuration commands.
    pub fn if_index(&self) -> io::Result<u32> {
        match &self.driver {
            Driver::Tun(tun) => Ok(tun.index()),
            Driver::Tap(tap) => Ok(tap.index()),
        }
    }
    /// Enables or disables the device.
    ///
    /// For a TUN device, disabling is not supported and will return an error.
    /// For a TAP device, this calls the appropriate method to set the device status.
    pub fn enabled(&self, value: bool) -> io::Result<()> {
        match &self.driver {
            Driver::Tun(_tun) => {
                if value {
                    Ok(())
                } else {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }
            }
            Driver::Tap(tap) => tap.set_status(value),
        }
    }
    /// Retrieves all IP addresses associated with this device.
    ///
    /// Filters the adapter addresses by matching the device's interface index.
    pub fn addresses(&self) -> io::Result<Vec<IpAddr>> {
        let index = self.if_index()?;
        let r = Self::get_all_adapter_address()?
            .into_iter()
            .filter(|v| v.index == Some(index))
            .map(|v| v.address)
            .collect();
        Ok(r)
    }
    /// Sets the IPv4 network address for the device.
    ///
    /// This method configures the IP address, netmask, and an optional destination for the interface
    /// using the `netsh` command.
    pub fn set_network_address<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
    ) -> io::Result<()> {
        netsh::set_interface_ip(
            self.if_index()?,
            address.ipv4()?.into(),
            netmask.netmask()?.into(),
            destination.map(|v| v.ipv4()).transpose()?.map(|v| v.into()),
        )
    }
    /// Removes the specified IP address from the device.
    pub fn remove_address(&self, addr: IpAddr) -> io::Result<()> {
        netsh::delete_interface_ip(self.if_index()?, addr)
    }
    /// Adds an IPv6 address to the device.
    ///
    /// Configures the IPv6 address and netmask (converted from prefix) for the interface.
    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        let mask = netmask.netmask()?;
        netsh::set_interface_ip(self.if_index()?, addr.ipv6()?.into(), mask.into(), None)
    }
    /// Retrieves the MTU for the device (IPv4).
    ///
    /// This method uses a Windows-specific FFI function to query the MTU by interface index.
    pub fn mtu(&self) -> io::Result<u16> {
        let index = self.if_index()?;
        let mtu = crate::platform::windows::ffi::get_mtu_by_index(index, true)?;
        Ok(mtu as _)
    }
    /// Retrieves the MTU for the device (IPv6).
    ///
    /// This method uses a Windows-specific FFI function to query the IPv6 MTU by interface index.
    pub fn mtu_v6(&self) -> io::Result<u16> {
        let index = self.if_index()?;
        let mtu = crate::platform::windows::ffi::get_mtu_by_index(index, false)?;
        Ok(mtu as _)
    }
    /// Sets the MTU for the device (IPv4) using the `netsh` command.
    pub fn set_mtu(&self, mtu: u16) -> io::Result<()> {
        netsh::set_interface_mtu(self.if_index()?, mtu as _)
    }
    /// Sets the MTU for the device (IPv6) using the `netsh` command.
    pub fn set_mtu_v6(&self, mtu: u16) -> io::Result<()> {
        netsh::set_interface_mtu_v6(self.if_index()?, mtu as _)
    }
    /// Sets the MAC address for the device.
    ///
    /// This operation is only supported for TAP devices; attempting to set a MAC address on a TUN device
    /// will result in an error.
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> io::Result<()> {
        match &self.driver {
            Driver::Tun(_tun) => Err(io::Error::from(io::ErrorKind::Unsupported)),
            Driver::Tap(tap) => tap.set_mac(&eth_addr),
        }
    }
    /// Retrieves the MAC address of the device.
    ///
    /// This operation is only supported for TAP devices.
    pub fn mac_address(&self) -> io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        match &self.driver {
            Driver::Tun(_tun) => Err(io::Error::from(io::ErrorKind::Unsupported)),
            Driver::Tap(tap) => tap.get_mac(),
        }
    }
    /// Sets the interface metric (routing cost) using the `netsh` command.
    pub fn set_metric(&self, metric: u16) -> io::Result<()> {
        netsh::set_interface_metric(self.if_index()?, metric)
    }
    /// Retrieves the version of the underlying driver.
    ///
    /// For TUN devices, this directly queries the driver version.
    /// For TAP devices, the version is composed of several components joined by dots.
    pub fn version(&self) -> io::Result<String> {
        match &self.driver {
            Driver::Tun(tun) => tun.version(),
            Driver::Tap(tap) => tap.get_version().map(|v| {
                v.iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<String>>()
                    .join(".")
            }),
        }
    }
}
