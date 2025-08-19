use crate::{
    builder::DeviceConfig,
    platform::{macos::sys::*, unix::sockaddr_union},
    ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask,
};

//const OVERWRITE_SIZE: usize = std::mem::size_of::<libc::__c_anonymous_ifr_ifru>();

use crate::platform::macos::tuntap::TunTap;
use crate::platform::unix::device::{ctl, ctl_v6};
use crate::platform::unix::Tun;
use crate::platform::ETHER_ADDR_LEN;
use getifaddrs::{self, Interface};
use libc::{self, c_char, c_short, IFF_RUNNING, IFF_UP};
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::{io, mem, net::IpAddr, os::unix::io::AsRawFd, ptr, sync::Mutex};

#[derive(Clone, Copy, Debug)]
struct Route {
    addr: IpAddr,
    netmask: IpAddr,
}

/// A TUN device using the TUN macOS driver.
pub struct DeviceImpl {
    pub(crate) tun: TunTap,
    pub(crate) op_lock: Mutex<bool>,
}

impl DeviceImpl {
    /// Create a new `Device` for the given `Configuration`.
    pub(crate) fn new(config: DeviceConfig) -> io::Result<Self> {
        let associate_route = config.associate_route;
        let tun_tap = TunTap::new(config)?;
        let associate_route = if tun_tap.is_tun() {
            associate_route.unwrap_or(true)
        } else {
            false
        };
        let device_impl = DeviceImpl {
            tun: tun_tap,
            op_lock: Mutex::new(associate_route),
        };
        Ok(device_impl)
    }
    pub(crate) fn from_tun(tun: Tun) -> io::Result<Self> {
        Ok(Self {
            tun: TunTap::Tun(tun),
            op_lock: Mutex::new(true),
        })
    }
    /// Prepare a new request.
    fn request(&self) -> io::Result<libc::ifreq> {
        self.tun.request()
    }
    fn request_v6(&self) -> io::Result<in6_ifreq> {
        self.tun.request_v6()
    }

    fn current_route(&self) -> Option<Route> {
        let addr = crate::platform::get_if_addrs_by_name(self.name_impl().ok()?).ok()?;
        let addr = addr
            .into_iter()
            .filter(|v| v.address.is_ipv4())
            .collect::<Vec<Interface>>();
        let addr = addr.first()?;
        let addr_ = addr.address;
        let netmask = addr.netmask?;
        Some(Route {
            addr: addr_,
            netmask,
        })
    }

    pub(crate) fn calc_dest_addr(&self, addr: IpAddr, netmask: IpAddr) -> io::Result<IpAddr> {
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        Ok(ipnet::IpNet::new(addr, prefix_len)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?
            .broadcast())
    }

    /// Set the IPv4 alias of the device.
    fn add_address(
        &self,
        addr: Ipv4Addr,
        dest: Ipv4Addr,
        mask: Ipv4Addr,
        associate_route: bool,
    ) -> io::Result<()> {
        let old_route = self.current_route();
        let tun_name = self.name_impl()?;
        unsafe {
            let mut req: ifaliasreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifra_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.ifra_addr = sockaddr_union::from((addr, 0)).addr;
            req.ifra_broadaddr = sockaddr_union::from((dest, 0)).addr;
            req.ifra_mask = sockaddr_union::from((mask, 0)).addr;

            if let Err(err) = siocaifaddr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            let new_route = Route {
                addr: addr.into(),
                netmask: mask.into(),
            };
            if let Err(e) = self.set_route(old_route, new_route, associate_route) {
                log::warn!("{e:?}");
            }
            Ok(())
        }
    }
    fn remove_route(&self, addr: IpAddr, netmask: IpAddr, associate_route: bool) -> io::Result<()> {
        if !associate_route {
            return Ok(());
        }
        let if_index = self.if_index_impl()?;
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        let mut manager = route_manager::RouteManager::new()?;
        let route = route_manager::Route::new(addr, prefix_len).with_if_index(if_index);
        manager.delete(&route)?;
        Ok(())
    }

    fn set_route(
        &self,
        old_route: Option<Route>,
        new_route: Route,
        associate_route: bool,
    ) -> io::Result<()> {
        if !associate_route {
            return Ok(());
        }
        let if_index = self.if_index_impl()?;
        let mut manager = route_manager::RouteManager::new()?;
        if let Some(old_route) = old_route {
            let prefix_len = ipnet::ip_mask_to_prefix(old_route.netmask)
                .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
            let route =
                route_manager::Route::new(old_route.addr, prefix_len).with_if_index(if_index);
            let result = manager.delete(&route);
            if let Err(e) = result {
                log::warn!("route {route:?} {e:?}");
            }
        }

        let prefix_len = ipnet::ip_mask_to_prefix(new_route.netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        let route = route_manager::Route::new(new_route.addr, prefix_len).with_if_index(if_index);
        manager.add(&route)?;
        Ok(())
    }
    fn remove_all_address_v4(&self) -> io::Result<()> {
        unsafe {
            let req_v4 = self.request()?;
            loop {
                if let Err(err) = siocdifaddr(ctl()?.as_raw_fd(), &req_v4) {
                    if err == nix::errno::Errno::EADDRNOTAVAIL {
                        break;
                    }
                    return Err(io::Error::from(err));
                }
            }
        }
        Ok(())
    }
    /// Sets the IPv4 network address, netmask, and an optional destination address.
    /// Remove all previous set IPv4 addresses and set the specified address.
    fn set_network_address_impl<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
        associate_route: bool,
    ) -> io::Result<()> {
        let netmask = netmask.netmask()?;
        let address = address.ipv4()?;
        let default_dest = self.calc_dest_addr(address.into(), netmask.into())?;
        let IpAddr::V4(default_dest) = default_dest else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "invalid destination for address/netmask",
            ));
        };
        let dest = destination
            .map(|v| v.ipv4())
            .transpose()?
            .unwrap_or(default_dest);
        self.remove_all_address_v4()?;
        self.add_address(address, dest, netmask, associate_route)?;
        Ok(())
    }
    pub(crate) fn name_impl(&self) -> io::Result<String> {
        self.tun.name()
    }
}

// Public User Interface
impl DeviceImpl {
    /// Retrieves the name of the network interface.
    pub fn name(&self) -> io::Result<String> {
        let _guard = self.op_lock.lock().unwrap();
        self.name_impl()
    }
    /// System behavior:
    /// On macOS, adding an IP to a feth interface will automatically add a route,
    /// while adding an IP to an utun interface will not.
    ///
    /// If false, the program will not modify or manage routes in any way, allowing the system to handle all routing natively.
    /// If true (default), the program will automatically add or remove routes to provide consistent routing behavior across all platforms.
    /// Set this to be false to obtain the platform's default routing behavior.
    pub fn set_associate_route(&self, associate_route: bool) {
        *self.op_lock.lock().unwrap() = associate_route;
    }
    /// Retrieve whether route is associated with the IP setting interface, see [`DeviceImpl::set_associate_route`]
    pub fn associate_route(&self) -> bool {
        *self.op_lock.lock().unwrap()
    }
    /// Enables or disables the network interface.
    ///
    /// If `value` is true, the interface is enabled by setting the IFF_UP and IFF_RUNNING flags.
    /// If false, the IFF_UP flag is cleared. The change is applied using a system call.
    pub fn enabled(&self, value: bool) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;

            if let Err(err) = siocgifflags(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            if value {
                req.ifr_ifru.ifru_flags |= (IFF_UP | IFF_RUNNING) as c_short;
            } else {
                req.ifr_ifru.ifru_flags &= !(IFF_UP as c_short);
            }

            if let Err(err) = siocsifflags(ctl.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }

            Ok(())
        }
    }
    /// Retrieves the current MTU (Maximum Transmission Unit) for the interface.
    pub fn mtu(&self) -> io::Result<u16> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;

            if let Err(err) = siocgifmtu(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            let r: u16 = req.ifr_ifru.ifru_mtu.try_into().map_err(io::Error::other)?;
            Ok(r)
        }
    }
    /// Sets the MTU (Maximum Transmission Unit) for the interface.
    pub fn set_mtu(&self, value: u16) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        self.tun.set_mtu(value)
    }
    /// Sets the IPv4 network address, netmask, and an optional destination address.
    /// Remove all previous set IPv4 addresses and set the specified address.
    pub fn set_network_address<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
    ) -> io::Result<()> {
        let guard = self.op_lock.lock().unwrap();
        self.set_network_address_impl(address, netmask, destination, *guard)
    }
    /// Add IPv4 network address, netmask
    pub fn add_address_v4<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
    ) -> io::Result<()> {
        let associate_route = self.op_lock.lock().unwrap();
        let netmask = netmask.netmask()?;
        let address = address.ipv4()?;
        let default_dest = self.calc_dest_addr(address.into(), netmask.into())?;
        let IpAddr::V4(default_dest) = default_dest else {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "invalid destination for address/netmask",
            ));
        };
        self.add_address(address, default_dest, netmask, *associate_route)?;
        Ok(())
    }
    /// Remove an IP address from the interface.
    pub fn remove_address(&self, addr: IpAddr) -> io::Result<()> {
        let guard = self.op_lock.lock().unwrap();
        let is_associate_route = *guard;
        unsafe {
            match addr {
                IpAddr::V4(addr_v4) => {
                    let mut req_v4 = self.request()?;
                    req_v4.ifr_ifru.ifru_addr = sockaddr_union::from((addr_v4, 0)).addr;
                    if let Err(err) = siocdifaddr(ctl()?.as_raw_fd(), &req_v4) {
                        return Err(io::Error::from(err));
                    }
                    if let Ok(addrs) = crate::platform::get_if_addrs_by_name(self.name_impl()?) {
                        for v in addrs.iter().filter(|v| v.address == addr) {
                            let Some(netmask) = v.netmask else {
                                continue;
                            };
                            if let Err(e) = self.remove_route(addr, netmask, is_associate_route) {
                                log::warn!("remove_route {addr}-{netmask},{e}")
                            }
                        }
                    }
                }
                IpAddr::V6(addr) => {
                    let mut req_v6 = self.request_v6()?;
                    req_v6.ifr_ifru.ifru_addr = sockaddr_union::from((addr, 0)).addr6;
                    if let Err(err) = siocdifaddr_in6(ctl_v6()?.as_raw_fd(), &req_v6) {
                        return Err(io::Error::from(err));
                    }
                }
            }

            Ok(())
        }
    }
    /// Add an IPv6 address to the interface.
    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        let addr = addr.ipv6()?;
        unsafe {
            let tun_name = self.name_impl()?;
            let mut req: in6_ifaliasreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifra_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.ifra_addr = sockaddr_union::from((addr, 0)).addr6;
            let network_addr = ipnet::IpNet::new(addr.into(), netmask.prefix()?)
                .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
            let mask = network_addr.netmask();
            req.ifra_prefixmask = sockaddr_union::from((mask, 0)).addr6;
            req.in6_addrlifetime.ia6t_vltime = 0xffffffff_u32;
            req.in6_addrlifetime.ia6t_pltime = 0xffffffff_u32;
            req.ifra_flags = IN6_IFF_NODAD;
            if let Err(err) = siocaifaddr_in6(ctl_v6()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    /// Set MAC address on L2 layer
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        self.tun.set_mac_address(eth_addr)
    }
    /// Retrieve MAC address for the device
    pub fn mac_address(&self) -> io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        let _guard = self.op_lock.lock().unwrap();
        self.tun.mac_address()
    }
}
