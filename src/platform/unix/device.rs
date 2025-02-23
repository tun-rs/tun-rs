use crate::platform::unix::{Fd, Tun};
use crate::platform::DeviceImpl;
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
use libc::{AF_INET, AF_INET6, SOCK_DGRAM};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};

impl FromRawFd for DeviceImpl {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        DeviceImpl::from_fd(fd)
    }
}
impl AsRawFd for DeviceImpl {
    fn as_raw_fd(&self) -> RawFd {
        self.tun.as_raw_fd()
    }
}
impl AsFd for DeviceImpl {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.as_raw_fd()) }
    }
}

impl IntoRawFd for DeviceImpl {
    fn into_raw_fd(self) -> RawFd {
        self.tun.into_raw_fd()
    }
}
impl DeviceImpl {
    /// # Safety
    /// The fd passed in must be an owned file descriptor; in particular, it must be open.
    pub(crate) unsafe fn from_fd(fd: RawFd) -> Self {
        let tun = Fd::new_unchecked(fd);
        DeviceImpl::from_tun(Tun::new(tun))
    }
    pub(crate) fn is_nonblocking(&self) -> io::Result<bool> {
        self.tun.is_nonblocking()
    }
    /// Moves this Device into or out of nonblocking mode.
    pub(crate) fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
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
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
impl DeviceImpl {
    /// Retrieves the interface index for the network interface.
    ///
    /// This function converts the interface name (obtained via `self.name()`) into a
    /// C-compatible string (CString) and then calls the libc function `if_nametoindex`
    /// to retrieve the corresponding interface index.
    pub fn if_index(&self) -> io::Result<u32> {
        let if_name = std::ffi::CString::new(self.name()?)?;
        unsafe { Ok(libc::if_nametoindex(if_name.as_ptr())) }
    }
    /// Retrieves all IP addresses associated with the network interface.
    ///
    /// This function calls `getifaddrs` with the interface name,
    /// then iterates over the returned list of interface addresses, extracting and collecting
    /// the IP addresses into a vector.
    pub fn addresses(&self) -> io::Result<Vec<std::net::IpAddr>> {
        Ok(crate::platform::get_if_addrs_by_name(self.name()?)?
            .iter()
            .map(|v| v.address)
            .collect())
    }
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
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
        self.tun.ignore_packet_info()
    }
    /// Sets whether the TUN device should ignore packet information (PI).
    ///
    /// When `ignore_packet_info` is set to `true`, the TUN device does not
    /// prepend the `struct tun_pi` header to packets. This can be useful
    /// if the additional metadata is not needed.
    ///
    /// # Parameters
    /// * `ign` - If `true`, the TUN device will ignore packet information.
    ///           If `false`, it will include packet information.
    pub fn set_ignore_packet_info(&self, ign: bool) {
        self.tun.set_ignore_packet_info(ign)
    }
}
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
pub(crate) unsafe fn ctl() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET, SOCK_DGRAM, 0))
}
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
pub(crate) unsafe fn ctl_v6() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET6, SOCK_DGRAM, 0))
}
