use crate::platform::posix::Fd;
use crate::platform::{Device, Tun};
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
use libc::{AF_INET, AF_INET6, SOCK_DGRAM};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};

impl FromRawFd for Device {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        Device::from_fd(fd)
    }
}
impl AsRawFd for Device {
    fn as_raw_fd(&self) -> RawFd {
        self.tun.as_raw_fd()
    }
}
impl AsFd for Device {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.as_raw_fd()) }
    }
}

impl IntoRawFd for Device {
    fn into_raw_fd(self) -> RawFd {
        self.tun.into_raw_fd()
    }
}
impl Device {
    /// # Safety
    /// The fd passed in must be an owned file descriptor; in particular, it must be open.
    pub(crate) unsafe fn from_fd(fd: RawFd) -> Self {
        let tun = Fd::new_unchecked(fd);
        Device::from_tun(Tun::new(tun))
    }
    pub fn is_nonblocking(&self) -> io::Result<bool> {
        self.tun.is_nonblocking()
    }
    /// Moves this Device into or out of nonblocking mode.
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.tun.set_nonblocking(nonblocking)
    }

    /// Recv a packet from tun device
    pub(crate) fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.tun.recv(buf)
    }
    pub(crate) fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.tun.recv_vectored(bufs)
    }

    /// Send a packet to tun device
    pub(crate) fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.tun.send(buf)
    }
    pub(crate) fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.tun.send_vectored(bufs)
    }
    #[cfg_attr(docsrs, doc(cfg(feature = "experimental")))]
    #[cfg(feature = "experimental")]
    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.tun.shutdown()
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
impl Device {
    pub fn if_index(&self) -> io::Result<u32> {
        let if_name = std::ffi::CString::new(self.name()?)?;
        unsafe { Ok(libc::if_nametoindex(if_name.as_ptr())) }
    }
    pub fn addresses(&self) -> io::Result<Vec<std::net::IpAddr>> {
        Ok(crate::device::get_if_addrs_by_name(self.name()?)?
            .iter()
            .map(|v| v.address)
            .collect())
    }
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
pub(crate) unsafe fn ctl() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET, SOCK_DGRAM, 0))
}
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "freebsd"))]
pub(crate) unsafe fn ctl_v6() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET6, SOCK_DGRAM, 0))
}
