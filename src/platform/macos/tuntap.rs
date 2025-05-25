use crate::platform::macos::tap::Tap;
use crate::platform::unix::Tun;
use libc::{c_char, socklen_t, SYSPROTO_CONTROL, UTUN_OPT_IFNAME};
use std::ffi::{c_void, CStr};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};

pub enum TunTap {
    Tun(Tun),
    Tap(Tap),
}

impl TunTap {
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
            TunTap::Tap(_tap) => {
                // tap not supported IntoRawFd
                -1
            }
        }
    }
}
