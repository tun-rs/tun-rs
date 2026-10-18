use super::{DeviceImpl, Fd, Tun};
use crate::builder::DeviceConfig;
use crate::{ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask};
use libc::c_int;
use std::ffi::CString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::process::Command;

// Values verified on this machine (headers + C program)
const STR: c_int = (b'S' as c_int) << 8;
const I_PUSH: c_int = STR | 0o02;
const I_SRDOPT: c_int = STR | 0o06;
const I_STR: c_int = STR | 0o10;
const I_PLINK: c_int = STR | 0o26;
const I_PUNLINK: c_int = STR | 0o27;
const RMSGD: c_int = 0x001;
const TUNNEWPPA: c_int = (('T' as c_int) << 16) | 0x0001;
const IF_UNITSEL: c_int = 0x8004_7336u32 as c_int;
const SIOCSLIFMUXID: c_int = 0x8078_6984u32 as c_int;
const LIFREQ_SIZE: usize = 376;
const OFF_IP_MUXID: usize = 40;
const LIFNAMSIZ: usize = 32;

#[repr(C)]
struct StrIoctl {
    ic_cmd: c_int,
    ic_timout: c_int,
    ic_len: c_int,
    ic_dp: *mut libc::c_char,
}

#[repr(C)]
struct LifReq([u8; LIFREQ_SIZE]);

impl LifReq {
    fn new(name: &str) -> Self {
        debug_assert!(name.len() < LIFNAMSIZ);
        let mut r = LifReq([0; LIFREQ_SIZE]);
        r.0[..name.len()].copy_from_slice(name.as_bytes());
        r
    }
    fn set_ip_muxid(&mut self, id: c_int) {
        self.0[OFF_IP_MUXID..OFF_IP_MUXID + 4].copy_from_slice(&id.to_ne_bytes());
    }
}

pub(crate) struct State {
    pub name: String,
    pub ctl_fd: OwnedFd,
    pub muxid: c_int,
}

impl Drop for State {
    fn drop(&mut self) {
        unsafe { libc::ioctl(self.ctl_fd.as_raw_fd(), I_PUNLINK, self.muxid) };
    }
}

fn cvt(r: c_int) -> io::Result<c_int> {
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(r)
    }
}

fn step<T>(what: &str, r: io::Result<T>) -> io::Result<T> {
    r.map_err(|e| io::Error::new(e.kind(), format!("illumos tun step '{what}' failed: {e}")))
}

fn open_rw(path: &str) -> io::Result<OwnedFd> {
    let c = CString::new(path).unwrap();
    let fd = cvt(unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) })?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn ifconfig(args: &[&str]) -> io::Result<()> {
    let st = Command::new("/sbin/ifconfig").args(args).status()?;
    if st.success() {
        Ok(())
    } else {
        Err(io::Error::new(
            ErrorKind::Other,
            format!("ifconfig {args:?}: {st}"),
        ))
    }
}

impl DeviceImpl {
    fn ifname(&self) -> io::Result<&str> {
        self.illumos
            .get()
            .map(|s| s.name.as_str())
            .ok_or_else(|| {
                io::Error::new(ErrorKind::NotConnected, "illumos device not initialised")
            })
    }

    pub(crate) fn new(config: DeviceConfig) -> io::Result<Self> {
        // Layer::L2 is not available on this target, so only L3 can be requested.
        let mut ppa: c_int = match config.dev_name.as_deref() {
            Some(n) => n
                .strip_prefix("tun")
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| io::Error::new(ErrorKind::InvalidInput, "name must be tunN"))?,
            None => -1,
        };

        // 1. data stream + allocate PPA
        let data = step("open /dev/tun (data)", open_rw("/dev/tun"))?;
        let mut sio = StrIoctl {
            ic_cmd: TUNNEWPPA,
            ic_timout: 0,
            ic_len: std::mem::size_of::<c_int>() as c_int,
            ic_dp: &mut ppa as *mut c_int as *mut libc::c_char,
        };
        ppa = step(
            "TUNNEWPPA",
            cvt(unsafe { libc::ioctl(data.as_raw_fd(), I_STR, &mut sio as *mut StrIoctl) }),
        )?;

        // 2. IP-side stream
        let if_fd = step("open /dev/tun (ip side)", open_rw("/dev/tun"))?;
        step(
            "I_PUSH ip",
            cvt(unsafe { libc::ioctl(if_fd.as_raw_fd(), I_PUSH, b"ip\0".as_ptr()) }),
        )?;
        step(
            "IF_UNITSEL",
            cvt(unsafe { libc::ioctl(if_fd.as_raw_fd(), IF_UNITSEL, &ppa as *const c_int) }),
        )?;

        // 3. link under /dev/udp
        let ctl = step("open /dev/udp", open_rw("/dev/udp"))?;
        let muxid = step(
            "I_PLINK",
            cvt(unsafe { libc::ioctl(ctl.as_raw_fd(), I_PLINK, if_fd.as_raw_fd()) }),
        )?;
        drop(if_fd);

        // 4. record muxid on the interface
        let name = format!("tun{ppa}");
        let mut req = LifReq::new(&name);
        req.set_ip_muxid(muxid);
        step(
            "SIOCSLIFMUXID",
            cvt(unsafe { libc::ioctl(ctl.as_raw_fd(), SIOCSLIFMUXID, &mut req as *mut LifReq) }),
        )?;

        // 5. one packet per read (non fatal if it fails)
        unsafe { libc::ioctl(data.as_raw_fd(), I_SRDOPT, RMSGD) };

        // 6. wrap the data fd
        let raw = data.into_raw_fd();
        let tun = Tun::new(unsafe { Fd::new_unchecked(raw) });
        let dev = Self::from_tun(tun)?;
        let _ = dev.illumos.set(State { name, ctl_fd: ctl, muxid });
        Ok(dev)
    }

    pub fn name(&self) -> io::Result<String> {
        Ok(self.ifname()?.to_owned())
    }

    pub fn set_mtu(&self, mtu: u16) -> io::Result<()> {
        ifconfig(&[self.ifname()?, "mtu", &mtu.to_string()])
    }

    pub fn enabled(&self, up: bool) -> io::Result<()> {
        let name = self.ifname()?;
        let flag = if up { "up" } else { "down" };
        // Try IPv4 first; if only IPv6 is plumbed, fall back to inet6.
        match ifconfig(&[name, "inet", flag]) {
            Ok(()) => Ok(()),
            Err(_) => ifconfig(&[name, "inet6", flag]),
        }
    }

    pub fn set_network_address<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
    ) -> io::Result<()> {
        let _guard = self.op_lock.write().unwrap();
        let name = self.ifname()?;
        let addr = address.ipv4()?;
        let mask = netmask.netmask()?;

        let dest = match destination {
            Some(d) => d.ipv4()?,
            None => {
                // Point-to-point needs a peer distinct from our own address.
                // Use the network address + 1, or + 2 if that is us.
                let m = u32::from(mask);
                let base = u32::from(addr) & m;
                let mut peer = base + 1;
                if peer == u32::from(addr) {
                    peer = base + 2;
                }
                std::net::Ipv4Addr::from(peer)
            }
        };

        ifconfig(&[
            name,
            "inet",
            &addr.to_string(),
            &dest.to_string(),
            "netmask",
            &mask.to_string(),
            "up",
        ])
    }

    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        let _guard = self.op_lock.write().unwrap();
        let name = self.ifname()?;
        let a = addr.ipv6()?.to_string();
        let prefix = u128::from(netmask.netmask()?).leading_ones().to_string();
        // plumb may already have been done; ignore its error
        let _ = ifconfig(&[name, "inet6", "plumb"]);
        // point-to-point: address is its own destination, as tailscale's tun0 does
        ifconfig(&[name, "inet6", &format!("{a}/{prefix}"), &a, "up"])
    }
}
