use crate::builder::DeviceConfig;
use crate::platform::macos::sys::{
    ctl_info, ctliocginfo, in6_ifreq, siocgiflladdr, siocsiflladdr, siocsifmtu, IN6_IFF_NODAD,
    UTUN_CONTROL_NAME,
};
use crate::platform::macos::tap::Tap;
use crate::platform::unix::device::ctl;
use crate::platform::unix::Tun;
use crate::platform::ETHER_ADDR_LEN;
use crate::Layer;
use libc::{
    c_char, c_uint, sockaddr, socklen_t, AF_SYSTEM, AF_SYS_CONTROL, IFNAMSIZ, PF_SYSTEM,
    SOCK_DGRAM, SYSPROTO_CONTROL, UTUN_OPT_IFNAME,
};
use std::ffi::{c_void, CStr};
use std::io::{ErrorKind, IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::{io, mem, ptr};

pub enum TunTap {
    Tun(Tun),
    Tap(Tap),
}

impl TunTap {
    pub fn new(config: DeviceConfig) -> io::Result<Self> {
        let layer = config.layer.unwrap_or(Layer::L3);
        let packet_information = config.packet_information.unwrap_or(false);
        match layer {
            Layer::L2 => Ok(TunTap::Tap(Tap::new(&config)?)),
            Layer::L3 => {
                let id = config
                    .dev_name
                    .as_ref()
                    .map(|tun_name| {
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
                            .map(|v| v + 1)
                            .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e))
                    })
                    .transpose()?
                    .unwrap_or(0);

                unsafe {
                    let fd = libc::socket(PF_SYSTEM, SOCK_DGRAM, SYSPROTO_CONTROL);
                    let tun = crate::platform::unix::Fd::new(fd)?;

                    let mut info = ctl_info {
                        ctl_id: 0,
                        ctl_name: {
                            let mut buffer = [0; 96];
                            for (i, o) in UTUN_CONTROL_NAME.as_bytes().iter().zip(buffer.iter_mut())
                            {
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
                    if libc::getsockopt(
                        tun.inner,
                        SYSPROTO_CONTROL,
                        UTUN_OPT_IFNAME,
                        optval,
                        optlen,
                    ) < 0
                    {
                        return Err(io::Error::last_os_error());
                    }
                    let tun = Tun::new(tun);
                    tun.set_ignore_packet_info(!packet_information);
                    Ok(TunTap::Tun(tun))
                }
            }
        }
    }
    pub fn name(&self) -> io::Result<String> {
        match &self {
            TunTap::Tun(tun) => {
                let mut tun_name = [0u8; 64];
                let mut name_len: socklen_t = 64;

                let optval = &mut tun_name as *mut _ as *mut c_void;
                let optlen = &mut name_len as *mut socklen_t;
                unsafe {
                    if libc::getsockopt(
                        tun.as_raw_fd(),
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
            TunTap::Tap(tap) => Ok(tap.name().to_string()),
        }
    }
    pub(crate) fn is_tun(&self) -> bool {
        match &self {
            TunTap::Tun(_) => true,
            TunTap::Tap(_) => false,
        }
    }
    pub fn is_nonblocking(&self) -> io::Result<bool> {
        match &self {
            TunTap::Tun(tun) => tun.is_nonblocking(),
            TunTap::Tap(tap) => tap.is_nonblocking(),
        }
    }
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        match &self {
            TunTap::Tun(tun) => tun.set_nonblocking(nonblocking),
            TunTap::Tap(tap) => tap.set_nonblocking(nonblocking),
        }
    }
    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.send(buf),
            TunTap::Tap(tap) => tap.send(buf),
        }
    }
    pub fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.send_vectored(bufs),
            TunTap::Tap(tap) => tap.send_vectored(bufs),
        }
    }
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.recv(buf),
            TunTap::Tap(tap) => tap.recv(buf),
        }
    }
    pub fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.recv_vectored(bufs),
            TunTap::Tap(tap) => tap.recv_vectored(bufs),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.read_interruptible(buf, event),
            TunTap::Tap(tap) => tap.read_interruptible(buf, event),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.readv_interruptible(bufs, event),
            TunTap::Tap(tap) => tap.readv_interruptible(bufs, event),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_readable_interruptible(
        &self,
        event: &crate::InterruptEvent,
    ) -> io::Result<()> {
        match &self {
            TunTap::Tun(tun) => tun.wait_readable_interruptible(event),
            TunTap::Tap(tap) => tap.wait_readable_interruptible(event),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.write_interruptible(buf, event),
            TunTap::Tap(tap) => tap.write_interruptible(buf, event),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn writev_interruptible(
        &self,
        bufs: &[IoSlice<'_>],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        match &self {
            TunTap::Tun(tun) => tun.writev_interruptible(bufs, event),
            TunTap::Tap(tap) => tap.writev_interruptible(bufs, event),
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_writable_interruptible(
        &self,
        event: &crate::InterruptEvent,
    ) -> io::Result<()> {
        match &self {
            TunTap::Tun(tun) => tun.wait_writable_interruptible(event),
            TunTap::Tap(tap) => tap.wait_writable_interruptible(event),
        }
    }
    pub fn request(&self) -> io::Result<libc::ifreq> {
        let tun_name = self.name()?;
        unsafe {
            let mut req: libc::ifreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifr_name.as_mut_ptr(),
                tun_name.len(),
            );
            Ok(req)
        }
    }
    pub fn request_peer(&self) -> Option<libc::ifreq> {
        let name = match &self {
            TunTap::Tun(_) => {
                return None;
            }
            TunTap::Tap(tap) => tap.peer_name(),
        };
        unsafe {
            let mut req: libc::ifreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                name.as_ptr() as *const c_char,
                req.ifr_name.as_mut_ptr(),
                name.len(),
            );
            Some(req)
        }
    }
    pub fn request_v6(&self) -> io::Result<in6_ifreq> {
        let tun_name = self.name()?;
        unsafe {
            let mut req: in6_ifreq = mem::zeroed();
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifra_name.as_mut_ptr(),
                tun_name.len(),
            );
            req.ifr_ifru.ifru_flags = IN6_IFF_NODAD as _;
            Ok(req)
        }
    }
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> io::Result<()> {
        match &self {
            TunTap::Tun(_) => Err(io::Error::from(io::ErrorKind::Unsupported)),
            TunTap::Tap(_) => {
                let mut ifr = self.request()?;
                unsafe {
                    ifr.ifr_ifru.ifru_addr.sa_family = libc::AF_LINK as _;
                    ifr.ifr_ifru.ifru_addr.sa_len = ETHER_ADDR_LEN;
                    for (i, v) in eth_addr.iter().enumerate() {
                        ifr.ifr_ifru.ifru_addr.sa_data[i] = *v as _;
                    }
                    siocsiflladdr(ctl()?.inner, &ifr)?;
                }
                Ok(())
            }
        }
    }
    pub fn mac_address(&self) -> io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        match &self {
            TunTap::Tun(_) => Err(io::Error::from(io::ErrorKind::Unsupported)),
            TunTap::Tap(_) => {
                let mut ifr = self.request()?;
                unsafe {
                    ifr.ifr_ifru.ifru_addr.sa_family = libc::AF_LINK as _;
                    ifr.ifr_ifru.ifru_addr.sa_len = ETHER_ADDR_LEN;

                    siocgiflladdr(ctl()?.inner, &mut ifr)?;
                    let mut eth_addr = [0; ETHER_ADDR_LEN as usize];
                    for (i, v) in eth_addr.iter_mut().enumerate() {
                        *v = ifr.ifr_ifru.ifru_addr.sa_data[i] as _;
                    }
                    Ok(eth_addr)
                }
            }
        }
    }
    #[inline]
    pub(crate) fn ignore_packet_info(&self) -> bool {
        match &self {
            TunTap::Tun(tun) => tun.ignore_packet_info(),
            TunTap::Tap(_) => true,
        }
    }
    pub(crate) fn set_ignore_packet_info(&self, ign: bool) {
        match &self {
            TunTap::Tun(tun) => tun.set_ignore_packet_info(ign),
            TunTap::Tap(_) => {}
        }
    }
    pub fn set_mtu(&self, value: u16) -> io::Result<()> {
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;
            req.ifr_ifru.ifru_mtu = value as i32;
            if let Err(err) = siocsifmtu(ctl.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            // peer feth
            if let Some(mut req) = self.request_peer() {
                req.ifr_ifru.ifru_mtu = value as i32;
                if let Err(err) = siocsifmtu(ctl.as_raw_fd(), &req) {
                    return Err(io::Error::from(err));
                }
            }
            Ok(())
        }
    }
}
impl AsRawFd for TunTap {
    fn as_raw_fd(&self) -> RawFd {
        match &self {
            TunTap::Tun(tun) => tun.as_raw_fd(),
            TunTap::Tap(tap) => tap.as_raw_fd(),
        }
    }
}
impl IntoRawFd for TunTap {
    fn into_raw_fd(self) -> RawFd {
        match self {
            TunTap::Tun(tun) => tun.into_raw_fd(),
            TunTap::Tap(tap) => tap.into_raw_fd(),
        }
    }
}
