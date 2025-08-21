use crate::{
    builder::{DeviceConfig, Layer},
    platform::freebsd::sys::*,
    platform::{
        unix::{sockaddr_union, Fd, Tun},
        ETHER_ADDR_LEN,
    },
    ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask,
};

use crate::platform::unix::device::{ctl, ctl_v6};
use libc::{
    self, c_char, c_short, fcntl, ifreq, kinfo_file, AF_LINK, F_KINFO, IFF_RUNNING, IFF_UP,
    IFNAMSIZ, KINFO_FILE_SIZE, O_RDWR,
};
use mac_address::mac_address_by_name;
use std::io::ErrorKind;
use std::os::fd::{IntoRawFd, RawFd};
use std::{ffi::CStr, io, mem, net::IpAddr, os::unix::io::AsRawFd, ptr, sync::Mutex};

/// A TUN device using the TUN/TAP Linux driver.
pub struct DeviceImpl {
    pub(crate) tun: Tun,
    pub op_lock: Mutex<bool>,
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
        let device_prefix = if layer == Layer::L3 {
            "tun".to_string()
        } else {
            "tap".to_string()
        };
        let dev_index = match config.dev_name.as_ref() {
            Some(tun_name) => {
                if tun_name.len() > IFNAMSIZ {
                    return Err(io::Error::new(
                        ErrorKind::InvalidInput,
                        "device name too long",
                    ));
                }
                match layer {
                    Layer::L2 => {
                        if !tun_name.starts_with("tap") {
                            return Err(io::Error::new(
                                ErrorKind::InvalidInput,
                                "device name must start with tap",
                            ));
                        }
                    }
                    Layer::L3 => {
                        if !tun_name.starts_with("tun") {
                            return Err(io::Error::new(
                                ErrorKind::InvalidInput,
                                "device name must start with tun",
                            ));
                        }
                    }
                }
                Some(
                    tun_name[3..]
                        .parse::<u32>()
                        .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?,
                )
            }
            None => None,
        };
        let tun = unsafe {
            if let Some(name_index) = dev_index.as_ref() {
                let device_path = format!("/dev/{device_prefix}{name_index}\0");
                let fd = libc::open(device_path.as_ptr() as *const _, O_RDWR | libc::O_CLOEXEC);
                Fd::new(fd)?
            } else {
                'End: {
                    for i in 0..256 {
                        let device_path = format!("/dev/{device_prefix}{i}\0");
                        let fd =
                            libc::open(device_path.as_ptr() as *const _, O_RDWR | libc::O_CLOEXEC);
                        match Fd::new(fd) {
                            Ok(tun) => {
                                break 'End tun;
                            }
                            Err(e) => {
                                if e.raw_os_error() != Some(libc::EBUSY) {
                                    return Err(e);
                                }
                            }
                        }
                    }
                    return Err(io::Error::new(
                        ErrorKind::AlreadyExists,
                        "no available file descriptor",
                    ));
                }
            }
        };
        let tun = Tun::new(tun);
        if matches!(layer, Layer::L3) {
            Self::enable_tunsifhead_impl(&tun.fd)?;
            tun.set_ignore_packet_info(!config.packet_information.unwrap_or(false));
        } else {
            tun.set_ignore_packet_info(false);
        }
        let device = DeviceImpl {
            tun,
            op_lock: Mutex::new(associate_route),
        };
        device.disable_deafult_sys_local_ipv6()?;
        Ok(device)
    }
    pub(crate) fn from_tun(tun: Tun) -> io::Result<Self> {
        let name = Self::name_of_fd(tun.as_raw_fd())?;
        if name.starts_with("tap") {
            // Tap does not have PI
            tun.set_ignore_packet_info(false);
        } else {
            tun.set_ignore_packet_info(true);
        }
        let dev = Self {
            tun,
            op_lock: Mutex::new(true),
        };
        Ok(dev)
    }

    fn disable_deafult_sys_local_ipv6(&self) -> std::io::Result<()> {
        unsafe {
            let tun_name = self.name_impl()?;
            let mut req: in6_ndireq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifra_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.ndi.flags &= !(ND6_IFF_AUTO_LINKLOCAL as u32);
            if let Err(err) = siocsifinfoin6(ctl_v6()?.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }

    // https://forums.freebsd.org/threads/ping6-address-family-not-supported-by-protocol-family.51467/
    // https://man.freebsd.org/cgi/man.cgi?query=tun&sektion=4&manpath=FreeBSD+5.3-RELEASE
    // https://web.mit.edu/freebsd/head/sys/net/if_tun.h
    // If the TUNSIFHEAD ioctl has been set, the address family must
    // be prepended, otherwise the packet is assumed to	be  of	type  AF_INET.
    // IPv6 needs AF_INET6.
    // The argument	should be a pointer to an int; a  non-zero value turns off "link-layer" mode, and enables "multi-af"
    // mode, where every packet is preceded	with a four byte ad-dress family.
    fn enable_tunsifhead_impl(device_fd: &Fd) -> std::io::Result<()> {
        unsafe {
            if let Err(err) = sioctunsifhead(device_fd.as_raw_fd(), &1 as *const _) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }

    fn calc_dest_addr(&self, addr: IpAddr, netmask: IpAddr) -> std::io::Result<IpAddr> {
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        Ok(ipnet::IpNet::new(addr, prefix_len)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
            .broadcast())
    }

    /// Set the IPv4 alias of the device.
    fn add_address(
        &self,
        addr: IpAddr,
        mask: IpAddr,
        dest: Option<IpAddr>,
        associate_route: bool,
    ) -> std::io::Result<()> {
        unsafe {
            match addr {
                IpAddr::V4(_) => {
                    let ctl = ctl()?;
                    let mut req: ifaliasreq = mem::zeroed();
                    let tun_name = self.name_impl()?;
                    ptr::copy_nonoverlapping(
                        tun_name.as_ptr() as *const c_char,
                        req.ifran.as_mut_ptr(),
                        tun_name.len(),
                    );

                    req.addr = crate::platform::unix::sockaddr_union::from((addr, 0)).addr;
                    if let Some(dest) = dest {
                        req.dstaddr = crate::platform::unix::sockaddr_union::from((dest, 0)).addr;
                    }
                    req.mask = crate::platform::unix::sockaddr_union::from((mask, 0)).addr;

                    if let Err(err) = siocaifaddr(ctl.as_raw_fd(), &req) {
                        return Err(io::Error::from(err));
                    }
                    if let Err(e) = self.add_route(addr, mask, associate_route) {
                        log::warn!("{e:?}");
                    }
                }
                IpAddr::V6(_) => {
                    let IpAddr::V6(_) = mask else {
                        return Err(std::io::Error::from(ErrorKind::InvalidInput));
                    };
                    let tun_name = self.name_impl()?;
                    let mut req: in6_ifaliasreq = mem::zeroed();
                    ptr::copy_nonoverlapping(
                        tun_name.as_ptr() as *const c_char,
                        req.ifra_name.as_mut_ptr(),
                        tun_name.len(),
                    );
                    req.ifra_addr = sockaddr_union::from((addr, 0)).addr6;
                    req.ifra_prefixmask = sockaddr_union::from((mask, 0)).addr6;
                    req.in6_addrlifetime.ia6t_vltime = 0xffffffff_u32;
                    req.in6_addrlifetime.ia6t_pltime = 0xffffffff_u32;
                    req.ifra_flags = IN6_IFF_NODAD;
                    if let Err(err) = siocaifaddr_in6(ctl_v6()?.as_raw_fd(), &req) {
                        return Err(io::Error::from(err));
                    }
                }
            }

            Ok(())
        }
    }

    /// Prepare a new request.
    unsafe fn request(&self) -> std::io::Result<ifreq> {
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
    unsafe fn request_v6(&self) -> std::io::Result<in6_ifreq> {
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
    fn add_route(&self, addr: IpAddr, netmask: IpAddr, associate_route: bool) -> io::Result<()> {
        if !associate_route {
            return Ok(());
        }
        let if_index = self.if_index_impl()?;
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))?;
        let mut manager = route_manager::RouteManager::new()?;
        let route = route_manager::Route::new(addr, prefix_len)
            .with_pref_source(addr)
            .with_if_index(if_index);
        manager.add(&route)?;
        Ok(())
    }
    fn name_of_fd(tun: &Tun) -> io::Result<String> {
        use std::path::PathBuf;
        unsafe {
            let mut path_info: kinfo_file = std::mem::zeroed();
            path_info.kf_structsize = KINFO_FILE_SIZE;
            if fcntl(tun.as_raw_fd(), F_KINFO, &mut path_info as *mut _) < 0 {
                return Err(io::Error::last_os_error());
            }
            let dev_path = CStr::from_ptr(path_info.kf_path.as_ptr() as *const c_char)
                .to_string_lossy()
                .into_owned();
            let path = PathBuf::from(dev_path);
            let device_name = path
                .file_name()
                .ok_or(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid device name",
                ))?
                .to_string_lossy()
                .to_string();
            Ok(device_name)
        }
    }
    /// Retrieves the name of the network interface.
    pub(crate) fn name_impl(&self) -> std::io::Result<String> {
        Self::name_of_fd(&self.tun)
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
        self.add_address(addr, netmask, Some(dest), associate_route)?;
        Ok(())
    }
}

// Public User Interface
impl DeviceImpl {
    /// Retrieves the name of the network interface.
    pub fn name(&self) -> std::io::Result<String> {
        let _guard = self.op_lock.lock().unwrap();
        self.name_impl()
    }
    /// Sets a new name for the network interface.
    pub fn set_name(&self, value: &str) -> std::io::Result<()> {
        use std::ffi::CString;
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            if value.len() > IFNAMSIZ {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "device name too long",
                ));
            }
            let mut req = self.request()?;
            let tun_name = CString::new(value)?;
            let mut tun_name: Vec<c_char> = tun_name
                .into_bytes_with_nul()
                .into_iter()
                .map(|c| c as _)
                .collect::<_>();
            req.ifr_ifru.ifru_data = tun_name.as_mut_ptr();
            if let Err(err) = siocsifname(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }

            Ok(())
        }
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

    /// Returns whether the TUN device is set to ignore packet information (PI).
    ///
    /// When enabled, the device does not prepend the `struct tun_pi` header
    /// to packets, which can simplify packet processing in some cases.
    ///
    /// # Returns
    /// * `true` - The TUN device ignores packet information.
    /// * `false` - The TUN device includes packet information.
    /// # Note
    /// Retrieve whether the packet is ignored for the TUN Device; The TAP device always returns `false`.
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
    /// # Note
    /// This only works for a TUN device; The invocation will be ignored if the device is a TAP.
    pub fn set_ignore_packet_info(&self, ign: bool) {
        let _guard = self.op_lock.lock().unwrap();
        if let Ok(name) = self.name_impl() {
            if name.starts_with("tun") {
                self.tun.set_ignore_packet_info(ign)
            }
        }
    }
    /// Enables or disables the network interface.
    pub fn enabled(&self, value: bool) -> std::io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req = self.request()?;
            let ctl = ctl()?;
            if let Err(err) = siocgifflags(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            if value {
                req.ifr_ifru.ifru_flags[0] |= (IFF_UP | IFF_RUNNING) as c_short;
            } else {
                req.ifr_ifru.ifru_flags[0] &= !(IFF_UP as c_short);
            }

            if let Err(err) = siocsifflags(ctl.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }

            Ok(())
        }
    }
    /// Retrieves the current MTU (Maximum Transmission Unit) for the interface.
    pub fn mtu(&self) -> std::io::Result<u16> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req = self.request()?;

            if let Err(err) = siocgifmtu(ctl()?.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            let r: u16 = req.ifr_ifru.ifru_mtu.try_into().map_err(io::Error::other)?;
            Ok(r)
        }
    }
    /// Sets the MTU (Maximum Transmission Unit) for the interface.
    pub fn set_mtu(&self, value: u16) -> std::io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        unsafe {
            let mut req = self.request()?;
            req.ifr_ifru.ifru_mtu = value as i32;

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
        let addr = address.ipv4()?.into();
        let netmask = netmask.netmask()?.into();
        let default_dest = self.calc_dest_addr(addr, netmask)?;
        self.add_address(addr, netmask, Some(default_dest), *guard)
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
        self.add_address(addr.ipv6()?.into(), netmask.netmask()?.into(), None, *guard)
    }
    /// Sets the MAC (hardware) address for the interface.
    ///
    /// This function constructs an interface request and copies the provided MAC address
    /// into the hardware address field. It then applies the change via a system call.
    /// This operation is typically supported only for TAP devices.
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> std::io::Result<()> {
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
    /// Retrieves the MAC (hardware) address of the interface.
    ///
    /// This function queries the MAC address by the interface name using a helper function.
    /// An error is returned if the MAC address cannot be found.
    pub fn mac_address(&self) -> std::io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        let _guard = self.op_lock.lock().unwrap();
        let mac = mac_address_by_name(&self.name_impl()?)
            .map_err(|e| io::Error::other(e.to_string()))?
            .ok_or(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid mac address",
            ))?;
        Ok(mac.bytes())
    }
    /// In Layer3(i.e. TUN mode), we need to put the tun interface into "multi_af" mode, which will prepend the address
    /// family to all packets (same as NetBSD).
    /// If this is not enabled, the kernel silently drops all IPv6 packets on output and gets confused on input.
    pub fn enable_tunsifhead(&self) -> io::Result<()> {
        let _guard = self.op_lock.lock().unwrap();
        Self::enable_tunsifhead_impl(&self.tun.fd)
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
