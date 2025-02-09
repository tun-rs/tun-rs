use crate::{
    builder::DeviceConfig,
    platform::{macos::sys::*, unix::sockaddr_union},
    ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask,
};

//const OVERWRITE_SIZE: usize = std::mem::size_of::<libc::__c_anonymous_ifr_ifru>();

use crate::platform::unix::device::{ctl, ctl_v6};
use crate::platform::unix::Tun;
use getifaddrs::{self, Interface};
use libc::{
    self, c_char, c_short, c_uint, c_void, sockaddr, socklen_t, AF_SYSTEM, AF_SYS_CONTROL,
    IFF_RUNNING, IFF_UP, IFNAMSIZ, PF_SYSTEM, SOCK_DGRAM, SYSPROTO_CONTROL, UTUN_OPT_IFNAME,
};
use std::io::ErrorKind;
use std::net::Ipv4Addr;
use std::{ffi::CStr, io, mem, net::IpAddr, os::unix::io::AsRawFd, ptr, sync::Mutex};
#[derive(Clone, Copy, Debug)]
struct Route {
    addr: IpAddr,
    netmask: IpAddr,
    #[allow(dead_code)]
    dest: IpAddr,
}

/// A TUN device using the TUN macOS driver.
pub struct DeviceImpl {
    pub(crate) tun: Tun,
    alias_lock: Mutex<()>,
}

impl DeviceImpl {
    /// Create a new `Device` for the given `Configuration`.
    pub(crate) fn new(config: DeviceConfig) -> io::Result<Self> {
        let id = if let Some(tun_name) = config.dev_name.as_ref() {
            if tun_name.len() > IFNAMSIZ {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "device name too long",
                ));
            }

            if !tun_name.starts_with("utun") {
                return Err(io::Error::new(
                    ErrorKind::InvalidInput,
                    "device name must start with utun",
                ));
            }
            tun_name[4..]
                .parse::<u32>()
                .map_err(|e| std::io::Error::new(ErrorKind::InvalidInput, e))?
                + 1_u32
        } else {
            0_u32
        };

        let device = unsafe {
            let fd = libc::socket(PF_SYSTEM, SOCK_DGRAM, SYSPROTO_CONTROL);
            let tun = crate::platform::unix::Fd::new(fd)?;

            let mut info = ctl_info {
                ctl_id: 0,
                ctl_name: {
                    let mut buffer = [0; 96];
                    for (i, o) in UTUN_CONTROL_NAME.as_bytes().iter().zip(buffer.iter_mut()) {
                        *o = *i as _;
                    }
                    buffer
                },
            };

            if let Err(err) = ctliocginfo(tun.inner, &mut info as *mut _ as *mut _) {
                return Err(io::Error::from(err));
            }

            let addr = libc::sockaddr_ctl {
                sc_id: info.ctl_id,
                sc_len: mem::size_of::<libc::sockaddr_ctl>() as _,
                sc_family: AF_SYSTEM as _,
                ss_sysaddr: AF_SYS_CONTROL as _,
                sc_unit: id as c_uint,
                sc_reserved: [0; 5],
            };

            let address = &addr as *const libc::sockaddr_ctl as *const sockaddr;
            if libc::connect(tun.inner, address, mem::size_of_val(&addr) as socklen_t) < 0 {
                return Err(io::Error::last_os_error());
            }

            let mut tun_name = [0u8; 64];
            let mut name_len: socklen_t = 64;

            let optval = &mut tun_name as *mut _ as *mut c_void;
            let optlen = &mut name_len as *mut socklen_t;
            if libc::getsockopt(tun.inner, SYSPROTO_CONTROL, UTUN_OPT_IFNAME, optval, optlen) < 0 {
                return Err(io::Error::last_os_error());
            }

            DeviceImpl {
                tun: Tun::new(tun),
                alias_lock: Mutex::new(()),
            }
        };
        device
            .tun
            .set_ignore_packet_info(!config.packet_information.unwrap_or(false));
        Ok(device)
    }
    pub(crate) fn from_tun(tun: Tun) -> Self {
        Self {
            tun,
            alias_lock: Mutex::new(()),
        }
    }
    /// Prepare a new request.
    /// # Safety
    unsafe fn request(&self) -> std::io::Result<libc::ifreq> {
        let tun_name = self.name()?;
        let mut req: libc::ifreq = mem::zeroed();
        ptr::copy_nonoverlapping(
            tun_name.as_ptr() as *const c_char,
            req.ifr_name.as_mut_ptr(),
            tun_name.len(),
        );

        Ok(req)
    }
    /// # Safety
    unsafe fn request_v6(&self) -> std::io::Result<in6_ifreq> {
        let tun_name = self.name()?;
        let mut req: in6_ifreq = mem::zeroed();
        ptr::copy_nonoverlapping(
            tun_name.as_ptr() as *const c_char,
            req.ifra_name.as_mut_ptr(),
            tun_name.len(),
        );
        req.ifr_ifru.ifru_flags = IN6_IFF_NODAD as _;
        Ok(req)
    }

    fn current_route(&self) -> Option<Route> {
        let addr = crate::platform::get_if_addrs_by_name(self.name().ok()?).ok()?;
        let addr = addr
            .into_iter()
            .filter(|v| v.address.is_ipv4())
            .collect::<Vec<Interface>>();
        let addr = addr.first()?;
        let addr_ = addr.address;
        let netmask = addr.netmask?;
        let dest = addr
            .associated_address
            .unwrap_or(self.calc_dest_addr(addr_, netmask).ok()?);
        Some(Route {
            addr: addr_,
            netmask,
            dest,
        })
    }

    pub(crate) fn calc_dest_addr(&self, addr: IpAddr, netmask: IpAddr) -> std::io::Result<IpAddr> {
        let prefix_len = ipnet::ip_mask_to_prefix(netmask)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        Ok(ipnet::IpNet::new(addr, prefix_len)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
            .broadcast())
    }

    /// Set the IPv4 alias of the device.
    fn set_alias(&self, addr: Ipv4Addr, dest: Ipv4Addr, mask: Ipv4Addr) -> std::io::Result<()> {
        let _guard = self.alias_lock.lock().unwrap();
        let old_route = self.current_route();
        let tun_name = self.name()?;
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
                dest: dest.into(),
            };
            if let Err(e) = self.set_route(old_route, new_route) {
                log::warn!("{e:?}");
            }
            Ok(())
        }
    }

    fn remove_route(&self, route: Route) -> std::io::Result<()> {
        let if_name = self.name()?;
        let prefix_len = ipnet::ip_mask_to_prefix(route.netmask)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let network = ipnet::IpNet::new(route.addr, prefix_len)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
            .network();
        // command: route -n delete -net 10.0.0.0/24 10.0.0.1
        let args = [
            "-n",
            "delete",
            if route.addr.is_ipv4() {
                "-net"
            } else {
                "-inet6"
            },
            &format!("{}/{}", network, prefix_len),
            "-iface",
            &if_name,
        ];
        if crate::run_command("route", &args).is_err() {
            log::error!("route {}", args.join(" "));
        } else {
            log::info!("route {}", args.join(" "));
        }
        Ok(())
    }

    fn add_route(&self, route: Route) -> std::io::Result<()> {
        let if_name = self.name()?;
        // command: route -n add -net 10.0.0.9/24 10.0.0.1
        let prefix_len = ipnet::ip_mask_to_prefix(route.netmask)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let args = [
            "-n",
            "add",
            if route.addr.is_ipv4() {
                "-net"
            } else {
                "-inet6"
            },
            &format!("{}/{}", route.addr, prefix_len),
            "-iface",
            &if_name,
        ];
        crate::run_command("route", &args)?;
        log::info!("route {}", args.join(" "));
        Ok(())
    }

    fn set_route(&self, old_route: Option<Route>, new_route: Route) -> std::io::Result<()> {
        let if_name = self.name()?;
        if let Some(v) = old_route {
            let prefix_len = ipnet::ip_mask_to_prefix(v.netmask)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            let network = ipnet::IpNet::new(v.addr, prefix_len)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?
                .network();
            // command: route -n delete -net 10.0.0.0/24 10.0.0.1
            let args = [
                "-n",
                "delete",
                if v.addr.is_ipv4() { "-net" } else { "-inet6" },
                &format!("{}/{}", network, prefix_len),
                "-iface",
                &if_name,
            ];
            if crate::run_command("route", &args).is_err() {
                log::error!("route {}", args.join(" "));
            } else {
                log::info!("route {}", args.join(" "));
            }
        }

        // command: route -n add -net 10.0.0.9/24 10.0.0.1
        let prefix_len = ipnet::ip_mask_to_prefix(new_route.netmask)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        let args = [
            "-n",
            "add",
            if new_route.addr.is_ipv4() {
                "-net"
            } else {
                "-inet6"
            },
            &format!("{}/{}", new_route.addr, prefix_len),
            "-iface",
            &if_name,
        ];
        crate::run_command("route", &args)?;
        log::info!("route {}", args.join(" "));
        Ok(())
    }

    // fn set_address(&self, value: IpAddr) -> Result<()> {
    //     let IpAddr::V4(value) = value else {
    //         unimplemented!("do not support IPv6 yet")
    //     };
    //     let ctl = self.ctl.as_ref().ok_or(Error::InvalidConfig)?;
    //     unsafe {
    //         let mut req = self.request()?;
    //         ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_addr, OVERWRITE_SIZE);
    //         if let Err(err) = siocsifaddr(ctl.as_raw_fd(), &req) {
    //             return Err(io::Error::from(err).into());
    //         }
    //         let route = { *self.route.lock().unwrap() };
    //         if let Some(mut route) = route {
    //             route.addr = value;
    //             self.set_route(route)?;
    //         }
    //         Ok(())
    //     }
    // }
    // fn set_netmask(&self, value: IpAddr) -> Result<()> {
    //     let IpAddr::V4(value) = value else {
    //         unimplemented!("do not support IPv6 yet")
    //     };
    //     let ctl = self.ctl.as_ref().ok_or(Error::InvalidConfig)?;
    //     unsafe {
    //         let mut req = self.request()?;
    //         // Note: Here should be `ifru_netmask`, but it is not defined in `ifreq`.
    //         ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_addr, OVERWRITE_SIZE);
    //         if let Err(err) = siocsifnetmask(ctl.as_raw_fd(), &req) {
    //             return Err(io::Error::from(err).into());
    //         }
    //         let route = { *self.route.lock().unwrap() };
    //         if let Some(mut route) = route {
    //             route.netmask = value;
    //             self.set_route(route)?;
    //         }
    //         Ok(())
    //     }
    // }

    // fn set_destination<A: IntoAddress>(&self, value: A) -> Result<()> {
    //     let value = value.into_address()?;
    //     let IpAddr::V4(value) = value else {
    //         unimplemented!("do not support IPv6 yet")
    //     };
    //     let ctl = self.ctl.as_ref().ok_or(Error::InvalidConfig)?;
    //     unsafe {
    //         let mut req = self.request()?;
    //         ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_dstaddr, OVERWRITE_SIZE);
    //         if let Err(err) = siocsifdstaddr(ctl.as_raw_fd(), &req) {
    //             return Err(io::Error::from(err).into());
    //         }
    //         let route = { *self.route.lock().unwrap() };
    //         if let Some(mut route) = route {
    //             route.dest = value;
    //             self.set_route(route)?;
    //         }
    //         Ok(())
    //     }
    // }

    pub fn name(&self) -> std::io::Result<String> {
        let mut tun_name = [0u8; 64];
        let mut name_len: socklen_t = 64;

        let optval = &mut tun_name as *mut _ as *mut c_void;
        let optlen = &mut name_len as *mut socklen_t;
        unsafe {
            if libc::getsockopt(
                self.tun.as_raw_fd(),
                SYSPROTO_CONTROL,
                UTUN_OPT_IFNAME,
                optval,
                optlen,
            ) < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(CStr::from_ptr(tun_name.as_ptr() as *const c_char)
                .to_string_lossy()
                .into())
        }
    }

    pub fn enabled(&self, value: bool) -> std::io::Result<()> {
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

    // /// Question on macOS
    // fn broadcast(&self) -> Result<IpAddr> {
    //     unsafe {
    //         let ctl = ctl()?;
    //         let mut req = self.request()?;
    //         if let Err(err) = siocgifbrdaddr(ctl.as_raw_fd(), &mut req) {
    //             return Err(io::Error::from(err).into());
    //         }
    //         let sa = sockaddr_union::from(req.ifr_ifru.ifru_broadaddr);
    //         Ok(std::net::SocketAddr::try_from(sa)?.ip())
    //     }
    // }

    // /// Question on macOS
    // fn set_broadcast<A: IntoAddress>(&self, value: A) -> Result<()> {
    //     let value = value.into_address()?;
    //     let IpAddr::V4(value) = value else {
    //         unimplemented!("do not support IPv6 yet")
    //     };
    //     unsafe {
    //         let ctl = ctl()?;
    //         let mut req = self.request()?;
    //         ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_broadaddr, OVERWRITE_SIZE);
    //         if let Err(err) = siocsifbrdaddr(ctl.as_raw_fd(), &req) {
    //             return Err(io::Error::from(err).into());
    //         }
    //         Ok(())
    //     }
    // }

    pub fn mtu(&self) -> std::io::Result<u16> {
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;

            if let Err(err) = siocgifmtu(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            let r: u16 = req
                .ifr_ifru
                .ifru_mtu
                .try_into()
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            Ok(r)
        }
    }

    pub fn set_mtu(&self, value: u16) -> std::io::Result<()> {
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;
            req.ifr_ifru.ifru_mtu = value as i32;

            if let Err(err) = siocsifmtu(ctl.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }

    pub fn set_network_address<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
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
        self.set_alias(address, dest, netmask)?;
        Ok(())
    }

    pub fn remove_address(&self, addr: IpAddr) -> io::Result<()> {
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
            if let Ok(addrs) = crate::platform::get_if_addrs_by_name(self.name()?) {
                addrs.iter().filter(|v| v.address == addr).for_each(|v| {
                    let Some(netmask) = v.netmask else {
                        return;
                    };
                    let Some(dest) = v.associated_address else {
                        return;
                    };
                    if let Err(e) = self.remove_route(Route {
                        addr,
                        netmask,
                        dest,
                    }) {
                        log::warn!("{e:?}");
                    }
                })
            }
            Ok(())
        }
    }

    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        let addr = addr.ipv6()?;
        unsafe {
            let tun_name = self.name()?;
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
            let Ok(dest) = self.calc_dest_addr(addr.into(), mask) else {
                return Ok(());
            };
            if let Err(e) = self.add_route(Route {
                addr: addr.into(),
                netmask: mask,
                dest,
            }) {
                log::warn!("{e:?}");
            }
        }
        Ok(())
    }

    pub fn ignore_packet_info(&self) -> bool {
        self.tun.ignore_packet_info()
    }

    pub fn set_ignore_packet_info(&self, ign: bool) {
        self.tun.set_ignore_packet_info(ign)
    }
}
