use crate::{
    builder::{DeviceConfig, Layer},
    platform::openbsd::sys::*,
    platform::{
        unix::{sockaddr_union, Fd, Tun},
        ETHER_ADDR_LEN,
    },
    ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask,
};

use crate::platform::unix::device::{ctl, ctl_v6};
use libc::{self, c_char, c_short, ifreq, AF_LINK, IFF_RUNNING, IFF_UP, IFNAMSIZ, O_RDWR};
use mac_address::mac_address_by_name;
use std::io::ErrorKind;
use std::os::fd::{FromRawFd, IntoRawFd, RawFd};
use std::os::unix::fs::MetadataExt;
use std::sync::Mutex;
use std::{io, mem, net::IpAddr, os::unix::io::AsRawFd, ptr};

/// A TUN device using the TUN/TAP Linux driver.
pub struct DeviceImpl {
    name: String,
    pub(crate) tun: Tun,
    pub(crate) op_lock: Mutex<bool>,
}
impl IntoRawFd for DeviceImpl {
    fn into_raw_fd(mut self) -> RawFd {
        let fd = self.tun.fd.inner;
        self.tun.fd.inner = -1;
        fd
    }
}
impl Drop for DeviceImpl {
    fn drop(&mut self) {
        if self.tun.fd.inner < 0 {
            return;
        }
        unsafe {
            if let (Ok(ctl), Ok(req)) = (ctl(), self.request()) {
                libc::close(self.tun.fd.inner);
                self.tun.fd.inner = -1;
                _ = siocifdestroy(ctl.as_raw_fd(), &req);
            }
        }
    }
}
impl DeviceImpl {
    /// Create a new `Device` for the given `Configuration`.
    pub(crate) fn new(config: DeviceConfig) -> io::Result<Self> {
        let layer = config.layer.unwrap_or(Layer::L3);
        let associate_route = if layer == Layer::L3 {
            config.associate_route.unwrap_or(true)
        } else {
            false
        };
        if let Some(dev_name) = config.dev_name.as_ref() {
            Self::check_name(layer, dev_name)?;
        }
        let (dev_fd, name) = Self::create_tuntap(layer, config.dev_name)?;

        let tun = Tun::new(dev_fd);
        if layer == Layer::L2 {
            tun.set_ignore_packet_info(false);
        }
        Ok(DeviceImpl {
            name,
            tun,
            op_lock: Mutex::new(associate_route),
        })
    }
    fn create_tuntap(layer: Layer, dev_name: Option<String>) -> io::Result<(Fd, String)> {
        let device_prefix = match layer {
            Layer::L2 => "tap",
            Layer::L3 => "tun",
        };
        if let Some(dev_name) = dev_name {
            if !dev_name.starts_with(&device_prefix) {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    format!("device name must start with {device_prefix}"),
                ));
            }
            let if_index = dev_name[3..]
                .parse::<u32>()
                .map(|v| v)
                .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
            let device_path = format!("/dev/{device_prefix}{if_index}\0");
            let fd = Self::open_create_dev(&dev_name, &device_path)?;
            Ok((fd, dev_name))
        } else {
            for index in 0..256 {
                let dev_name = format!("{device_prefix}{index}");
                let device_path = format!("/dev/{device_prefix}{index}\0");
                match Self::open_create_dev(&dev_name, &device_path) {
                    Ok(dev) => {
                        return Ok((dev, dev_name));
                    }
                    Err(e) => {
                        if e.raw_os_error() != Some(libc::EBUSY) {
                            return Err(e);
                        }
                    }
                }
            }
            Err(io::Error::last_os_error())
        }
    }
    fn check_name(layer: Layer, dev_name: &str) -> io::Result<()> {
        if dev_name.len() > IFNAMSIZ {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "device name too long",
            ));
        }
        let device_prefix = match layer {
            Layer::L2 => "tap",
            Layer::L3 => "tun",
        };
        if !dev_name.starts_with(device_prefix) {
            Err(io::Error::new(
                ErrorKind::InvalidInput,
                format!("device name must start with {device_prefix}"),
            ))
        } else {
            Ok(())
        }
    }
    fn open_create_dev(dev_name: &str, device_path: &str) -> io::Result<Fd> {
        let fd = unsafe { libc::open(device_path.as_ptr() as *const _, O_RDWR | libc::O_CLOEXEC) };
        match Fd::new(fd) {
            Ok(dev) => Ok(dev),
            Err(ref e) if e.kind() == ErrorKind::NotFound => {
                if let Err(e) = DeviceImpl::create_dev(&dev_name) {
                    if e.kind() != ErrorKind::AlreadyExists {
                        return Err(e);
                    }
                }
                Self::open_and_makedev_dev(dev_name, device_path)
            }
            Err(e) => Err(e),
        }
    }
    fn open_and_makedev_dev(dev_name: &str, device_path: &str) -> io::Result<Fd> {
        let fd = unsafe { libc::open(device_path.as_ptr() as *const _, O_RDWR | libc::O_CLOEXEC) };
        match Fd::new(fd) {
            Ok(fd) => Ok(fd),
            Err(ref e) if e.kind() == ErrorKind::NotFound => {
                DeviceImpl::makedev_dev(dev_name)?;
                let fd = unsafe {
                    libc::open(device_path.as_ptr() as *const _, O_RDWR | libc::O_CLOEXEC)
                };
                Ok(Fd::new(fd)?)
            }
            Err(e) => Err(e),
        }
    }
    fn makedev_dev(name: &str) -> io::Result<()> {
        let status = std::process::Command::new("sh")
            .arg("MAKEDEV")
            .arg(name)
            .current_dir("/dev")
            .status()?;

        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other(format!(
                "MAKEDEV {} failed with status {:?}",
                name,
                status.code()
            )))
        }
    }
    fn create_dev(name: &str) -> io::Result<()> {
        unsafe {
            let mut req: ifreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                name.as_ptr() as *const c_char,
                req.ifr_name.as_mut_ptr(),
                name.len(),
            );
            if let Err(err) = siocifcreate(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    pub(crate) fn from_tun(tun: Tun) -> io::Result<Self> {
        let name = Self::name_of_fd(&tun)?;
        if name.starts_with("tap") {
            // Tap does not have PI
            tun.set_ignore_packet_info(false)
        }
        Ok(Self {
            name,
            tun,
            op_lock: Mutex::new(true),
        })
    }

    fn calc_dest_addr(&self, addr: IpAddr, netmask: IpAddr) -> io::Result<IpAddr> {
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        Ok(ipnet::IpNet::new(addr, prefix_len)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?
            .broadcast())
    }

    /// Set the IPv4 alias of the device.
    fn set_alias(
        &self,
        addr: IpAddr,
        dest: IpAddr,
        mask: IpAddr,
        associate_route: bool,
    ) -> io::Result<()> {
        unsafe {
            match addr {
                IpAddr::V4(_) => {
                    let ctl = ctl()?;
                    let mut req: ifaliasreq = mem::zeroed();
                    let tun_name = self.name_impl()?;
                    ptr::copy_nonoverlapping(
                        tun_name.as_ptr() as *const c_char,
                        req.ifra_name.as_mut_ptr(),
                        tun_name.len(),
                    );

                    req.ifra_ifrau.ifrau_addr =
                        crate::platform::unix::sockaddr_union::from((addr, 0)).addr;
                    req.ifra_dstaddr = crate::platform::unix::sockaddr_union::from((dest, 0)).addr;
                    req.ifra_mask = crate::platform::unix::sockaddr_union::from((mask, 0)).addr;

                    if let Err(err) = siocaifaddr(ctl.as_raw_fd(), &req) {
                        return Err(io::Error::from(err));
                    }
                }
                IpAddr::V6(_) => {
                    let IpAddr::V6(_) = mask else {
                        return Err(std::io::Error::from(ErrorKind::InvalidInput));
                    };
                    let tun_name = self.name_impl()?;
                    let mut req: in6_aliasreq = mem::zeroed();
                    ptr::copy_nonoverlapping(
                        tun_name.as_ptr() as *const c_char,
                        req.ifra_name.as_mut_ptr(),
                        tun_name.len(),
                    );
                    req.ifra_ifrau.ifrau_addr = sockaddr_union::from((addr, 0)).addr6;
                    req.ifra_prefixmask = sockaddr_union::from((mask, 0)).addr6;
                    req.ifra_lifetime.ia6t_vltime = 0xffffffff_u32;
                    req.ifra_lifetime.ia6t_pltime = 0xffffffff_u32;
                    req.ifra_flags = IN6_IFF_NODAD;
                    if let Err(err) = siocaifaddr_in6(ctl_v6()?.as_raw_fd(), &req) {
                        return Err(io::Error::from(err));
                    }
                }
            }

            if let Err(e) = self.add_route(addr, mask, associate_route) {
                log::warn!("{e:?}");
            }

            Ok(())
        }
    }

    /// Prepare a new request.
    unsafe fn request(&self) -> io::Result<ifreq> {
        let mut req: ifreq = mem::zeroed();
        let tun_name = self.name_impl()?;
        ptr::copy_nonoverlapping(
            tun_name.as_ptr() as *const c_char,
            req.ifr_name.as_mut_ptr(),
            tun_name.len(),
        );

        Ok(req)
    }

    /// # Safety
    unsafe fn request_v6(&self) -> io::Result<in6_ifreq> {
        let tun_name = self.name_impl()?;
        let mut req: in6_ifreq = mem::zeroed();
        ptr::copy_nonoverlapping(
            tun_name.as_ptr() as *const c_char,
            req.ifra_name.as_mut_ptr(),
            tun_name.len(),
        );
        req.ifr_ifru.ifru_flags = IN6_IFF_NODAD as _;
        Ok(req)
    }
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
    fn add_route(&self, addr: IpAddr, netmask: IpAddr, associate_route: bool) -> io::Result<()> {
        if !associate_route {
            return Ok(());
        }
        let if_index = self.if_index_impl()?;
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        let mut manager = route_manager::RouteManager::new()?;
        let route = route_manager::Route::new(addr, prefix_len).with_if_index(if_index);
        manager.add(&route)?;
        Ok(())
    }

    /// Retrieves the name of the network interface.
    pub(crate) fn name_impl(&self) -> io::Result<String> {
        Ok(self.name.clone())
    }
    fn name_of_fd(tun: &Tun) -> io::Result<String> {
        let file = unsafe { std::fs::File::from_raw_fd(tun.as_raw_fd()) };
        let metadata = file.metadata()?;
        let rdev = metadata.rdev();
        let index = rdev % 256;
        std::mem::forget(file); // prevent fd being closed
        Ok(format!("tun{index}"))
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
        let addr = address.ipv4()?.into();
        let netmask = netmask.netmask()?.into();
        let default_dest = self.calc_dest_addr(addr, netmask)?;
        let dest = destination
            .map(|d| d.ipv4())
            .transpose()?
            .map(|v| v.into())
            .unwrap_or(default_dest);
        self.remove_all_address_v4()?;
        self.set_alias(addr, dest, netmask, associate_route)?;
        Ok(())
    }
}

//Public User Interface
impl DeviceImpl {
    /// Returns whether the TUN device is set to ignore packet information (PI).
    ///
    /// When enabled, the device does not prepend the `struct tun_pi` header
    /// to packets, which can simplify packet processing in some cases.
    ///
    /// # Returns
    /// * `true` - The TUN device ignores packet information.
    /// * `false` - The TUN device includes packet information.
    pub fn ignore_packet_info(&self) -> bool {
        let _guard = self.op_lock.lock().unwrap();
        self.tun.ignore_packet_info()
    }
    /// Sets whether the TUN device should ignore packet information (PI).
    ///
    /// When `ignore_packet_info` is set to `true`, the TUN device does not
    /// prepend the `struct tun_pi` header to packets. This can be useful
    /// if the additional metadata is not needed.
    ///
    /// # Parameters
    /// * `ign`
    ///     - If `true`, the TUN device will ignore packet information.
    ///     - If `false`, it will include packet information.
    pub fn set_ignore_packet_info(&self, ign: bool) {
        let _guard = self.op_lock.lock().unwrap();
        if let Ok(name) = self.name_impl() {
            if name.starts_with("tun") {
                self.tun.set_ignore_packet_info(ign)
            }
        }
    }
    /// Enables or disables the network interface.
    pub fn enabled(&self, value: bool) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req = self.request()?;
            let ctl = ctl()?;

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
            let mut req: ifreq_mtu = mem::zeroed();
            let tun_name = self.name_impl()?;
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifr_name.as_mut_ptr(),
                tun_name.len(),
            );
            if let Err(err) = siocgifmtu(ctl()?.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            let r: u16 = req.mtu.try_into().map_err(io::Error::other)?;
            Ok(r)
        }
    }
    /// Sets the MTU (Maximum Transmission Unit) for the interface.
    pub fn set_mtu(&self, value: u16) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req: ifreq_mtu = mem::zeroed();
            let tun_name = self.name_impl()?;
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifr_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.mtu = value as _;

            if let Err(err) = siocsifmtu(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
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
        let guard = self.op_lock.lock().unwrap();
        self.set_network_address_impl(address, netmask, None, *guard)
    }
    /// Removes an IP address from the interface.
    pub fn remove_address(&self, addr: IpAddr) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            match addr {
                IpAddr::V4(addr) => {
                    let mut req_v4 = self.request()?;
                    req_v4.ifr_ifru.ifru_addr = sockaddr_union::from((addr, 0)).addr;
                    if let Err(err) = siocdifaddr(ctl()?.as_raw_fd(), &req_v4) {
                        return Err(io::Error::from(err));
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
    /// Adds an IPv6 address to the interface.
    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        let guard = self.op_lock.lock().unwrap();
        let addr = addr.ipv6()?;
        unsafe {
            let tun_name = self.name_impl()?;
            let mut req: in6_aliasreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifra_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.ifra_ifrau.ifrau_addr = sockaddr_union::from((addr, 0)).addr6;
            let network_addr = ipnet::IpNet::new(addr.into(), netmask.prefix()?)
                .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
            let mask = network_addr.netmask();
            req.ifra_prefixmask = sockaddr_union::from((mask, 0)).addr6;
            req.ifra_lifetime.ia6t_vltime = 0xffffffff_u32;
            req.ifra_lifetime.ia6t_pltime = 0xffffffff_u32;
            req.ifra_flags = IN6_IFF_NODAD;
            if let Err(err) = siocaifaddr_in6(ctl_v6()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            if let Err(e) = self.add_route(addr.into(), mask, *guard) {
                log::warn!("{e:?}");
            }
        }
        Ok(())
    }
    /// Sets the MAC (hardware) address for the interface.
    ///
    /// This function constructs an interface request and copies the provided MAC address
    /// into the hardware address field. It then applies the change via a system call.
    /// This operation is typically supported only for TAP devices.
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req = self.request()?;
            req.ifr_ifru.ifru_addr.sa_len = ETHER_ADDR_LEN;
            req.ifr_ifru.ifru_addr.sa_family = AF_LINK as u8;
            req.ifr_ifru.ifru_addr.sa_data[0..ETHER_ADDR_LEN as usize]
                .copy_from_slice(eth_addr.map(|c| c as i8).as_slice());
            if let Err(err) = siocsiflladdr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }
    /// Retrieves the name of the network interface.
    pub fn name(&self) -> io::Result<String> {
        let _guard = self.op_lock.lock().unwrap();
        self.name_impl()
    }
    /// Retrieves the MAC (hardware) address of the interface.
    ///
    /// This function queries the MAC address by the interface name using a helper function.
    /// An error is returned if the MAC address cannot be found.
    pub fn mac_address(&self) -> io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        let _guard = self.op_lock.lock().unwrap();
        let mac = mac_address_by_name(&self.name_impl()?)
            .map_err(|e| io::Error::other(e.to_string()))?
            .ok_or(io::Error::new(
                ErrorKind::InvalidInput,
                "invalid mac address",
            ))?;
        Ok(mac.bytes())
    }
}

impl From<Layer> for c_short {
    fn from(layer: Layer) -> Self {
        match layer {
            Layer::L2 => 2,
            Layer::L3 => 3,
        }
    }
}
