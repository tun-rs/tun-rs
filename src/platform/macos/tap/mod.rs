/*
link https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/sockio.h
link https://github.com/apple-oss-distributions/xnu/blob/main/bsd/net/if_fake.c
link https://www.zerotier.com/blog/how-zerotier-eliminated-kernel-extensions-on-macos/
 */
/*
* link https://github.com/zerotier/ZeroTierOne/blob/dev/osdep/MacEthernetTapAgent.c
*
* This creates a pair of feth devices with the lower numbered device
* being the virtual interface and the other being the device
* used to actually read and write packets. The latter gets no IP config
* and is only used for I/O. The behavior of feth is similar to the
* veth pairs that exist on Linux.
*
* The feth device has only existed since MacOS Sierra, but that's fairly
* long ago in Mac terms.
*
* I/O with feth must be done using two different sockets. The BPF socket
* is used to receive packets, while an AF_NDRV (low-level network driver
* access) socket must be used to inject. AF_NDRV can't read IP frames
* since BSD doesn't forward packets out the NDRV tap if they've already
* been handled, and while BPF can inject its MTU for injected packets
* is limited to 2048.
*
* All this stuff is basically undocumented. A lot of tracing through
* the Darwin/XNU kernel source was required to figure out how to make
* this actually work.
﻿
*
* See also:
*
* https://apple.stackexchange.com/questions/337715/fake-ethernet-interfaces-feth-if-fake-anyone-ever-seen-this
*
*/
use crate::builder::DeviceConfig;
use crate::platform::macos::sys::siocifcreate;
use crate::platform::unix::Fd;
use bytes::BytesMut;
use libc::{ifreq, IFNAMSIZ};
use nix::errno::Errno;
use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};
use std::sync::Mutex;

const FETH: &str = "feth";
const BUFFER_LEN: usize = 131072;
pub(crate) fn run_command(command: &str, args: &[&str]) -> io::Result<()> {
    let out = std::process::Command::new(command).args(args).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(if out.stderr.is_empty() {
            &out.stdout
        } else {
            &out.stderr
        });
        let info = format!("{} failed with: \"{}\"", command, err);
        return Err(io::Error::other(info));
    }
    Ok(())
}

pub struct Tap {
    s_bpf_fd: Fd,
    s_ndrv_fd: Fd,
    dev_feth: Feth,
    #[allow(dead_code)]
    peer_feth: Feth,
    buffer: Mutex<VecDeque<BytesMut>>,
}
struct Feth {
    is_drop: bool,
    name: String,
}
impl Drop for Feth {
    fn drop(&mut self) {
        if self.is_drop {
            _ = run_command("ifconfig", &[&self.name, "destroy"]);
            self.is_drop = false;
        }
    }
}
impl IntoRawFd for Tap {
    fn into_raw_fd(mut self) -> RawFd {
        self.peer_feth.is_drop = false;
        self.dev_feth.is_drop = false;
        self.s_bpf_fd.into_raw_fd()
    }
}
impl Tap {
    pub fn new(config: &DeviceConfig) -> io::Result<Tap> {
        unsafe {
            let s_ndrv_fd = libc::socket(libc::AF_NDRV, libc::SOCK_RAW, 0);
            let s_ndrv_fd = Fd::new(s_ndrv_fd)?;
            let mut ifr = new_ifreq(config.dev_name.as_ref())?;
            if let Err(e) = siocifcreate(s_ndrv_fd.inner, &mut ifr) {
                if e != Errno::EEXIST || (e == Errno::EEXIST && !config.reuse_dev.unwrap_or(true)) {
                    return Err(e.into());
                }
            }

            let dev_name = CStr::from_ptr(ifr.ifr_name.as_ptr())
                .to_string_lossy()
                .into_owned();
            let dev_feth = Feth {
                is_drop: !config.persist.unwrap_or(false),
                name: dev_name,
            };
            std::thread::sleep(std::time::Duration::from_millis(1));
            let mut peer_ifr = new_ifreq(config.peer_feth.as_ref())?;
            if let Err(e) = siocifcreate(s_ndrv_fd.inner, &mut peer_ifr) {
                if e != Errno::EEXIST || (e == Errno::EEXIST && !config.reuse_dev.unwrap_or(true)) {
                    return Err(e.into());
                }
            }
            let peer_name = CStr::from_ptr(peer_ifr.ifr_name.as_ptr())
                .to_string_lossy()
                .into_owned();
            let peer_feth = Feth {
                is_drop: !config.persist.unwrap_or(false),
                name: peer_name,
            };
            std::thread::sleep(std::time::Duration::from_millis(1));
            run_command("ifconfig", &[&peer_feth.name, "peer", &dev_feth.name])?;
            let mut nd: libc::sockaddr_ndrv = std::mem::zeroed();
            nd.snd_len = size_of::<libc::sockaddr_ndrv>() as u8;
            nd.snd_family = libc::AF_NDRV as u8;
            nd.snd_name[..peer_feth.name.len()].copy_from_slice(peer_feth.name.as_bytes());
            if libc::bind(
                s_ndrv_fd.inner,
                &nd as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_ndrv>() as u32,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            if libc::connect(
                s_ndrv_fd.inner,
                &nd as *const _ as *const libc::sockaddr,
                size_of::<libc::sockaddr_ndrv>() as u32,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            let s_bpf_fd = open_bpf()?;
            let mut buffer_len = BUFFER_LEN;
            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCSBLEN, &mut buffer_len);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }
            let mut enable = 1i32;
            let mut disable = 0i32;
            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCIMMEDIATE, &mut enable);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }
            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCSSEESENT, &mut disable);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }

            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCSETIF, &mut peer_ifr);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }
            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCSHDRCMPLT, &mut enable);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }
            let rs = libc::ioctl(s_bpf_fd.inner, libc::BIOCPROMISC as u64, &mut enable);
            if rs != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                s_bpf_fd,
                s_ndrv_fd,
                dev_feth,
                peer_feth,
                buffer: Default::default(),
            })
        }
    }
    // pub fn as_s_ndrv_fd(&self) -> RawFd {
    //     self.s_ndrv_fd.as_raw_fd()
    // }
    // pub fn as_s_bpf_fd(&self) -> RawFd {
    //     self.s_bpf_fd.as_raw_fd()
    // }
    pub fn name(&self) -> &String {
        &self.dev_feth.name
    }
    pub fn peer_name(&self) -> &String {
        &self.peer_feth.name
    }
    pub fn is_nonblocking(&self) -> io::Result<bool> {
        self.s_bpf_fd.is_nonblocking()
    }
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.s_bpf_fd.set_nonblocking(nonblocking)?;
        self.s_ndrv_fd.set_nonblocking(nonblocking)?;
        Ok(())
    }
    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.s_ndrv_fd.write(buf)
    }
    pub fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.s_ndrv_fd.writev(bufs)
    }
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self.buffer.lock().unwrap();
        if guard.is_empty() {
            self.recv_to_buffer(&mut guard)?;
        }

        let Some(buffer) = guard.pop_front() else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "recv buffer is empty",
            ));
        };
        if buf.len() < buffer.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "buffer too small",
            ));
        }
        buf[..buffer.len()].copy_from_slice(&buffer);
        Ok(buffer.len())
    }
    fn recv_to_buffer(&self, bufs: &mut VecDeque<BytesMut>) -> io::Result<()> {
        let mut buffer = [0; BUFFER_LEN];
        let len = self.s_bpf_fd.read(&mut buffer)?;
        if len > 0 {
            let mut p = 0;
            unsafe {
                while p < len {
                    let hdr = buffer.as_ptr().add(p) as *const libc::bpf_hdr;
                    let bh_caplen = (*hdr).bh_caplen as usize;
                    let bh_hdrlen = (*hdr).bh_hdrlen as usize;
                    if bh_caplen > 0 && p + bh_hdrlen + bh_caplen <= len {
                        let buf = &buffer[p + bh_hdrlen..p + bh_hdrlen + bh_caplen];
                        bufs.push_back(buf.into());
                    }
                    p += ((*hdr).bh_hdrlen as usize + bh_caplen + 3) & !3;
                }
            }
        }
        Ok(())
    }
    #[allow(dead_code)]
    pub fn recv_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &self,
        bufs: &mut [B],
        sizes: &mut [usize],
    ) -> io::Result<usize> {
        let mut buffer = [0; BUFFER_LEN];
        let len = self.s_bpf_fd.read(&mut buffer)?;
        let mut num = 0;
        if len > 0 {
            let mut p = 0;
            unsafe {
                while p < len {
                    let hdr = buffer.as_ptr().add(p) as *const libc::bpf_hdr;
                    let bh_caplen = (*hdr).bh_caplen as usize;
                    let bh_hdrlen = (*hdr).bh_hdrlen as usize;
                    if bh_caplen > 0 && p + bh_hdrlen + bh_caplen <= len {
                        let buf = &buffer[p + bh_hdrlen..p + bh_hdrlen + bh_caplen];
                        if let Some(dst) = bufs.get_mut(num) {
                            let dst = dst.as_mut();
                            if dst.len() < buf.len() {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "buffer too small",
                                ));
                            }
                            dst[..buf.len()].copy_from_slice(buf);
                            sizes[num] = buf.len();
                            num += 1;
                        } else {
                            break;
                        }
                    }
                    p += ((*hdr).bh_hdrlen as usize + bh_caplen + 3) & !3;
                }
            }
        }
        Ok(num)
    }
    pub fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        let mut guard = self.buffer.lock().unwrap();
        if guard.is_empty() {
            self.recv_to_buffer(&mut guard)?;
        }

        let Some(buf) = guard.pop_front() else {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "recv buffer is empty",
            ));
        };
        let len: usize = bufs.iter().map(|v| v.len()).sum();
        if len < buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "buffer too small",
            ));
        }
        let mut pos = 0;
        for b in bufs {
            let n = b.len().min(buf.len() - pos);
            if n == 0 {
                break;
            }
            b[..n].copy_from_slice(&buf[pos..pos + n]);
            pos += n;
            if pos == buf.len() {
                break;
            }
        }
        Ok(pos)
    }
    #[cfg(feature = "experimental")]
    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.s_bpf_fd.shutdown()
    }
}
impl AsRawFd for Tap {
    fn as_raw_fd(&self) -> RawFd {
        self.s_bpf_fd.as_raw_fd()
    }
}

fn open_bpf() -> io::Result<Fd> {
    for i in 1..5000 {
        let path = CString::new(format!("/dev/bpf{}", i).into_bytes())?;
        let bpf_fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
        match Fd::new(bpf_fd) {
            Ok(fd) => {
                return Ok(fd);
            }
            Err(e) => {
                if e.raw_os_error() == Some(libc::EBUSY) {
                    continue;
                } else {
                    return Err(e);
                }
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "No available /dev/bpf",
    ))
}
fn new_ifreq(name: Option<&String>) -> io::Result<ifreq> {
    if let Some(name) = name {
        new_ifreq_str(name.as_str())
    } else {
        new_ifreq_str(FETH)
    }
}
fn new_ifreq_str(name: &str) -> io::Result<ifreq> {
    let bytes = name.as_bytes();
    if bytes.len() >= IFNAMSIZ {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
    }
    if bytes.len() < 4 || &bytes[..4] != FETH.as_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "The prefix of the network card name must be 'fech'",
        ));
    }
    let mut ifr: ifreq = unsafe { std::mem::zeroed() };
    for (i, &b) in bytes.iter().enumerate() {
        ifr.ifr_name[i] = b as libc::c_char;
    }
    ifr.ifr_name[bytes.len()] = 0;
    Ok(ifr)
}
