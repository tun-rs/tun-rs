use super::{DeviceImpl, Fd, Tun};
use crate::builder::DeviceConfig;
use crate::{ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask};
use libc::c_int;
use std::ffi::CString;
use std::io::{self, ErrorKind};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

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
    /// Set once this device has plumbed the IPv6 stream, so `Drop` removes it.
    pub v6_plumbed: AtomicBool,
    /// Last requested MTU, applied to IPv6 when it is plumbed later.
    pub mtu: std::sync::atomic::AtomicU32,
}

impl Drop for State {
    fn drop(&mut self) {
        // The IPv6 stream is plumbed separately via ifconfig and is not
        // removed by I_PUNLINK, so unplumb it first.
        if self.v6_plumbed.load(Ordering::Relaxed) {
            let _ = Command::new("/sbin/ifconfig")
                .args([self.name.as_str(), "inet6", "unplumb"])
                .output();
        }
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
        self.illumos.get().map(|s| s.name.as_str()).ok_or_else(|| {
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
        // The driver only returns a ppa with no live data stream, so any
        // existing tun<ppa> plumbing is left over from a process that died
        // without running Drop (kill -9, crash). Remove it before linking.
        let stale = format!("tun{ppa}");
        for family in ["inet6", "inet"] {
            let _ = Command::new("/sbin/ifconfig")
                .args([stale.as_str(), family, "unplumb"])
                .output();
        }

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

        // The link is persistent: from here on, any early return must unlink
        // it. `State::drop` does the I_PUNLINK, so build it immediately.
        let name = format!("tun{ppa}");
        let state = State {
            name: name.clone(),
            ctl_fd: ctl,
            muxid,
            v6_plumbed: AtomicBool::new(false),
            mtu: std::sync::atomic::AtomicU32::new(0),
        };

        // 4. record muxid on the interface
        let mut req = LifReq::new(&name);
        req.set_ip_muxid(muxid);
        step(
            "SIOCSLIFMUXID",
            cvt(unsafe {
                libc::ioctl(
                    state.ctl_fd.as_raw_fd(),
                    SIOCSLIFMUXID,
                    &mut req as *mut LifReq,
                )
            }),
        )?;

        // 5. one packet per read: without message mode a read could merge
        // queued packets, so a failure here is fatal.
        step(
            "I_SRDOPT RMSGD",
            cvt(unsafe { libc::ioctl(data.as_raw_fd(), I_SRDOPT, RMSGD) }),
        )?;

        // 6. wrap the data fd
        let raw = data.into_raw_fd();
        let tun = Tun::new(unsafe { Fd::new_unchecked(raw) });
        let dev = Self::from_tun(tun)?;
        let _ = dev.illumos.set(state);
        Ok(dev)
    }

    pub fn name(&self) -> io::Result<String> {
        Ok(self.ifname()?.to_owned())
    }

    pub fn set_mtu(&self, mtu: u16) -> io::Result<()> {
        let _guard = self.op_lock.write().unwrap();
        let name = self.ifname()?;
        let m = mtu.to_string();
        // With no address family, ifconfig only touches IPv4.
        ifconfig(&[name, "inet", "mtu", &m])?;
        // IPv6 may not be plumbed yet; add_address_v6 applies the MTU later.
        let v6_plumbed = self
            .illumos
            .get()
            .is_some_and(|s| s.v6_plumbed.load(Ordering::Relaxed));
        if v6_plumbed {
            ifconfig(&[name, "inet6", "mtu", &m])?;
        }
        // Commit only after every command succeeded.
        if let Some(s) = self.illumos.get() {
            s.mtu.store(u32::from(mtu), Ordering::Relaxed);
        }
        Ok(())
    }

    pub fn enabled(&self, up: bool) -> io::Result<()> {
        // Hold the configuration lock for the whole function, so concurrent
        // enabled() calls cannot interleave their ifconfig commands.
        let _guard = self.op_lock.write().unwrap();

        let name = self.ifname()?;
        let flag = if up { "up" } else { "down" };

        // Propagate a failure to run ifconfig instead of treating it
        //  as "family not configured".
        let v4_output = Command::new("/sbin/ifconfig")
            .args([name, "inet"])
            .output()?;
        let v4_configured = v4_output.status.success()
            && String::from_utf8_lossy(&v4_output.stdout)
                .split_whitespace()
                .skip_while(|w| *w != "inet")
                .nth(1)
                .is_some_and(|a| a != "0.0.0.0");
        let v6_configured = Command::new("/sbin/ifconfig")
            .args([name, "inet6"])
            .output()?
            .status
            .success();

        // Nothing plumbed or configured: there is nothing to change.
        if !v4_configured && !v6_configured {
            return Ok(());
        }

        // Apply the state to every configured family and report the first failure.
        let mut result = Ok(());
        if v4_configured {
            result = ifconfig(&[name, "inet", flag]);
        }
        if v6_configured {
            let r6 = ifconfig(&[name, "inet6", flag]);
            if result.is_ok() {
                result = r6;
            }
        }
        result
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
                let no_peer = || {
                    io::Error::new(
                        ErrorKind::InvalidInput,
                        "cannot derive a default peer address",
                    )
                };
                let m = u32::from(mask);
                let a = u32::from(addr);
                let base = a & m;
                if m == 0xffff_fffe {
                    // /31 (RFC 3021): the peer is the other address in the pair.
                    std::net::Ipv4Addr::from(a ^ 1)
                } else {
                    // Point-to-point needs a peer distinct from our own address.
                    let mut peer = base.checked_add(1).ok_or_else(no_peer)?;
                    if peer == a {
                        peer = base.checked_add(2).ok_or_else(no_peer)?;
                    }
                    std::net::Ipv4Addr::from(peer)
                }
            }
        };

        ifconfig(&[
            name,
            "inet",
            &addr.to_string(),
            &dest.to_string(),
            "netmask",
            &mask.to_string(),
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
        // Mark before plumbing: if plumb fails because a stale stream already
        // exists, or the address step fails afterwards, Drop still cleans up.
        if let Some(s) = self.illumos.get() {
            s.v6_plumbed.store(true, Ordering::Relaxed);
        }
        let _ = ifconfig(&[name, "inet6", "plumb"]);
        // point-to-point: address is its own destination, as tailscale's tun0 does
        ifconfig(&[name, "inet6", &format!("{a}/{prefix}"), &a])?;
        if let Some(s) = self.illumos.get() {
            let m = s.mtu.load(Ordering::Relaxed);
            if m != 0 {
                ifconfig(&[name, "inet6", "mtu", &m.to_string()])?;
            }
        }
        Ok(())
    }
}
