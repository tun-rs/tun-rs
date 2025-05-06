#[cfg(unix)]
pub(crate) mod unix;
#[cfg(all(
    unix,
    not(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd"
    ))
))]
pub use self::unix::DeviceImpl;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub(crate) mod linux;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub use self::linux::*;

#[cfg(target_os = "freebsd")]
pub(crate) mod freebsd;
#[cfg(target_os = "freebsd")]
pub use self::freebsd::DeviceImpl;

#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::DeviceImpl;

#[cfg(target_os = "windows")]
pub(crate) mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::DeviceImpl;

use getifaddrs::Interface;
#[cfg(unix)]
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
#[cfg(unix)]
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, IntoRawFd, RawFd};

#[allow(dead_code)]
pub(crate) const ETHER_ADDR_LEN: u8 = 6;

#[allow(dead_code)]
pub(crate) fn get_if_addrs_by_name(if_name: String) -> std::io::Result<Vec<Interface>> {
    let addrs = getifaddrs::getifaddrs()?;
    let ifs = addrs.filter(|v| v.name == if_name).collect();
    Ok(ifs)
}

/// A transparent wrapper around DeviceImpl, providing synchronous I/O operations.
#[repr(transparent)]
pub struct SyncDevice(pub(crate) DeviceImpl);

impl SyncDevice {
    /// Creates a new SyncDevice from a raw file descriptor.
    ///
    /// # Safety
    /// - The file descriptor (`fd`) must be an owned file descriptor.
    /// - It must be valid and open.
    ///
    /// This function is only available on Unix platforms.
    #[cfg(unix)]
    pub unsafe fn from_fd(fd: RawFd) -> Self {
        SyncDevice(DeviceImpl::from_fd(fd))
    }
    /// Receives data from the device into the provided buffer.
    ///
    /// Returns the number of bytes read, or an I/O error.
    pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.recv(buf)
    }
    /// Sends data from the provided buffer to the device.
    ///
    /// Returns the number of bytes written, or an I/O error.
    pub fn send(&self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.send(buf)
    }
    /// Attempts to receive data from the device in a non-blocking fashion.
    ///
    /// Returns the number of bytes read or an error if the operation would block.
    #[cfg(target_os = "windows")]
    pub fn try_recv(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.try_recv(buf)
    }
    /// Attempts to send data to the device in a non-blocking fashion.
    ///
    /// Returns the number of bytes written or an error if the operation would block.
    #[cfg(target_os = "windows")]
    pub fn try_send(&self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.try_send(buf)
    }
    /// Shuts down the device on Windows.
    ///
    /// This may close the device or signal that no further operations will occur.
    #[cfg(target_os = "windows")]
    pub fn shutdown(&self) -> std::io::Result<()> {
        self.0.shutdown()
    }
    /// Shuts down the device on Unix when the experimental feature is enabled.
    ///
    /// This method signals the device to stop operations.
    #[cfg(all(unix, feature = "experimental"))]
    pub fn shutdown(&self) -> std::io::Result<()> {
        self.0.shutdown()
    }
    /// Receives data from the device into multiple buffers using vectored I/O.
    ///
    /// **Note:** This method operates on a single packet only. It will only read data from one packet,
    /// even if multiple buffers are provided.
    ///
    /// Returns the total number of bytes read from the packet, or an error.
    #[cfg(unix)]
    pub fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> std::io::Result<usize> {
        self.0.recv_vectored(bufs)
    }
    /// Sends data to the device from multiple buffers using vectored I/O.
    ///
    /// **Note:** This method operates on a single packet only. It will only send the data contained in
    /// the provided buffers as one packet.
    ///
    /// Returns the total number of bytes written for the packet, or an error.
    #[cfg(unix)]
    pub fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> std::io::Result<usize> {
        self.0.send_vectored(bufs)
    }
    /// Checks whether the device is currently operating in nonblocking mode.
    ///
    /// Returns `true` if nonblocking mode is enabled, `false` otherwise, or an error.
    #[cfg(unix)]
    pub fn is_nonblocking(&self) -> std::io::Result<bool> {
        self.0.is_nonblocking()
    }

    /// Sets the nonblocking mode for the device.
    ///
    /// - `nonblocking`: Pass `true` to enable nonblocking mode, `false` to disable.
    ///
    /// Returns an empty result on success or an I/O error.
    #[cfg(unix)]
    pub fn set_nonblocking(&self, nonblocking: bool) -> std::io::Result<()> {
        self.0.set_nonblocking(nonblocking)
    }

    /// # Prerequisites
    /// - The `IFF_MULTI_QUEUE` flag must be enabled.
    /// - The system must support network interface multi-queue functionality.
    ///
    /// # Description
    /// When multi-queue is enabled, create a new queue by duplicating an existing one.
    #[cfg(target_os = "linux")]
    pub fn try_clone(&self) -> std::io::Result<SyncDevice> {
        let device_impl = self.0.try_clone()?;
        Ok(SyncDevice(device_impl))
    }
}

impl Deref for SyncDevice {
    type Target = DeviceImpl;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(unix)]
impl FromRawFd for SyncDevice {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        SyncDevice::from_fd(fd)
    }
}
#[cfg(unix)]
impl AsRawFd for SyncDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}
#[cfg(unix)]
impl AsFd for SyncDevice {
    fn as_fd(&self) -> BorrowedFd<'_> {
        unsafe { BorrowedFd::borrow_raw(self.as_raw_fd()) }
    }
}
#[cfg(unix)]
impl IntoRawFd for SyncDevice {
    fn into_raw_fd(self) -> RawFd {
        self.0.into_raw_fd()
    }
}

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
))]
#[cfg(test)]
mod test {
    use crate::DeviceBuilder;
    use std::net::Ipv4Addr;

    #[test]
    fn create() {
        let dev = DeviceBuilder::new()
            .name("utun6")
            .ipv4("192.168.50.1", 24, None)
            .mtu(1400)
            .build_sync()
            .unwrap();

        assert!(dev
            .addresses()
            .unwrap()
            .into_iter()
            .any(|v| v == "192.168.50.1".parse::<Ipv4Addr>().unwrap()));

        assert_eq!(1400, dev.mtu().unwrap());
        assert_eq!("utun6", dev.name().unwrap());
    }
}
