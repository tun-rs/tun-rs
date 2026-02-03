use crate::platform::unix::{Fd, Tun};
use crate::platform::DeviceImpl;
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
use libc::{AF_INET, AF_INET6, SOCK_DGRAM};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, RawFd};

impl FromRawFd for DeviceImpl {
    /// # Safety
    ///
    /// The caller must ensure that `fd` is a valid, open file descriptor for a TUN/TAP device.
    ///
    /// # Panics
    ///
    /// This function will panic if the provided file descriptor is invalid or cannot be used
    /// to create a TUN/TAP device. This is acceptable because providing an invalid fd violates
    /// the safety contract of `FromRawFd`.
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        // If this panics, the caller violated the safety contract by providing an invalid fd
        DeviceImpl::from_fd(fd).expect(
            "Failed to create device from file descriptor. \
                                         The provided fd must be a valid, open file descriptor \
                                         for a TUN/TAP device.",
        )
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
#[cfg(not(any(target_os = "freebsd", target_os = "netbsd", target_os = "openbsd")))]
impl std::os::unix::io::IntoRawFd for DeviceImpl {
    fn into_raw_fd(self) -> RawFd {
        self.tun.into_raw_fd()
    }
}
impl DeviceImpl {
    /// # Safety
    /// The fd passed in must be an owned file descriptor; in particular, it must be open.
    pub(crate) unsafe fn from_fd(fd: RawFd) -> io::Result<Self> {
        let tun = Fd::new_unchecked(fd);
        DeviceImpl::from_tun(Tun::new(tun))
    }
    /// # Safety
    /// The fd passed in must be a valid, open file descriptor.
    /// Unlike [`from_fd`], this function does **not** take ownership of `fd`,
    /// and therefore will not close it when dropped.  
    /// The caller is responsible for ensuring the lifetime and eventual closure of `fd`.
    pub(crate) unsafe fn borrow_raw(fd: RawFd) -> io::Result<Self> {
        let tun = Fd::new_unchecked_with_borrow(fd, true);
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
    #[cfg(feature = "interruptible")]
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        self.tun.read_interruptible(buf, event, timeout)
    }
    #[cfg(feature = "interruptible")]
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        self.tun.readv_interruptible(bufs, event, timeout)
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_readable_interruptible(
        &self,
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        self.tun.wait_readable_interruptible(event, timeout)
    }
    #[cfg(feature = "interruptible")]
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        self.tun.write_interruptible(buf, event)
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn writev_interruptible(
        &self,
        bufs: &[IoSlice<'_>],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        self.tun.writev_interruptible(bufs, event)
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_writable_interruptible(
        &self,
        event: &crate::InterruptEvent,
    ) -> io::Result<()> {
        self.tun.wait_writable_interruptible(event)
    }
}
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
impl DeviceImpl {
    /// Retrieves the interface index for the network interface.
    ///
    /// This function converts the interface name (obtained via `self.name()`) into a
    /// C-compatible string (CString) and then calls the libc function `if_nametoindex`
    /// to retrieve the corresponding interface index.
    pub fn if_index(&self) -> io::Result<u32> {
        let _guard = self.op_lock.lock().unwrap();
        self.if_index_impl()
    }
    pub(crate) fn if_index_impl(&self) -> io::Result<u32> {
        let if_name = std::ffi::CString::new(self.name_impl()?)?;
        unsafe { Ok(libc::if_nametoindex(if_name.as_ptr())) }
    }
    /// Retrieves all IP addresses associated with the network interface.
    ///
    /// This function calls `getifaddrs` with the interface name,
    /// then iterates over the returned list of interface addresses, extracting and collecting
    /// the IP addresses into a vector.
    pub fn addresses(&self) -> io::Result<Vec<std::net::IpAddr>> {
        Ok(crate::platform::get_if_addrs_by_name(self.name_impl()?)?
            .iter()
            .map(|v| v.address)
            .collect())
    }
}
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos",))]
impl DeviceImpl {
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
    /// * `ign` - If `true`, the TUN device will ignore packet information.
    ///   `  ` If `false`, it will include packet information.
    /// # Note
    /// This only works for a TUN device; The invocation will be ignored if the device is a TAP.
    pub fn set_ignore_packet_info(&self, ign: bool) {
        let _guard = self.op_lock.lock().unwrap();
        self.tun.set_ignore_packet_info(ign)
    }
}
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
pub(crate) unsafe fn ctl() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET, SOCK_DGRAM | libc::SOCK_CLOEXEC, 0))
}
#[cfg(target_os = "macos")]
pub(crate) unsafe fn ctl() -> io::Result<Fd> {
    let fd = Fd::new(libc::socket(AF_INET, SOCK_DGRAM, 0))?;
    _ = fd.set_cloexec();
    Ok(fd)
}
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
pub(crate) unsafe fn ctl_v6() -> io::Result<Fd> {
    Fd::new(libc::socket(AF_INET6, SOCK_DGRAM | libc::SOCK_CLOEXEC, 0))
}
#[cfg(target_os = "macos")]
pub(crate) unsafe fn ctl_v6() -> io::Result<Fd> {
    let fd = Fd::new(libc::socket(AF_INET6, SOCK_DGRAM, 0))?;
    _ = fd.set_cloexec();
    Ok(fd)
}
