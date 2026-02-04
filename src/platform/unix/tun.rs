use crate::platform::unix::Fd;
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
use crate::PACKET_INFORMATION_LENGTH as PIL;
use std::io::{self, IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
use std::sync::atomic::{AtomicBool, Ordering};

/// Infer the protocol based on the first nibble in the packet buffer.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
pub(crate) fn is_ipv6(buf: &[u8]) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind::InvalidData};
    if buf.is_empty() {
        return Err(Error::new(InvalidData, "Zero-length data"));
    }
    match buf[0] >> 4 {
        4 => Ok(false),
        6 => Ok(true),
        p => Err(Error::new(InvalidData, format!("IP version {p}"))),
    }
}
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
pub(crate) fn generate_packet_information(_ipv6: bool) -> [u8; PIL] {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const TUN_PROTO_IP6: [u8; PIL] = (libc::ETH_P_IPV6 as u32).to_be_bytes();
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const TUN_PROTO_IP4: [u8; PIL] = (libc::ETH_P_IP as u32).to_be_bytes();

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    const TUN_PROTO_IP6: [u8; PIL] = (libc::AF_INET6 as u32).to_be_bytes();
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    const TUN_PROTO_IP4: [u8; PIL] = (libc::AF_INET as u32).to_be_bytes();

    if _ipv6 {
        TUN_PROTO_IP6
    } else {
        TUN_PROTO_IP4
    }
}

pub(crate) struct Tun {
    pub(crate) fd: Fd,
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    ignore_packet_information: AtomicBool,
}

impl Tun {
    pub(crate) fn new(fd: Fd) -> Self {
        Self {
            fd,
            #[cfg(any(
                target_os = "macos",
                target_os = "ios",
                target_os = "tvos",
                target_os = "openbsd",
                target_os = "freebsd",
                target_os = "netbsd",
            ))]
            ignore_packet_information: AtomicBool::new(true),
        }
    }
    pub fn is_nonblocking(&self) -> io::Result<bool> {
        self.fd.is_nonblocking()
    }
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        self.fd.set_nonblocking(nonblocking)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )))]
    #[inline]
    pub(crate) fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.fd.write(buf)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    #[inline]
    pub(crate) fn send(&self, buf: &[u8]) -> io::Result<usize> {
        if self.ignore_packet_info() {
            let ipv6 = is_ipv6(buf)?;
            let header = generate_packet_information(ipv6);
            let len = self
                .fd
                .writev(&[IoSlice::new(&header), IoSlice::new(buf)])?;
            return Ok(len.saturating_sub(PIL));
        }
        self.fd.write(buf)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )))]
    #[inline]
    pub(crate) fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.fd.writev(bufs)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    #[inline]
    pub(crate) fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        if self.ignore_packet_info() {
            if crate::platform::unix::fd::max_iov() - 1 < bufs.len() {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let buf = bufs
                .iter()
                .find(|b| !b.is_empty())
                .map_or(&[][..], |b| &**b);
            let ipv6 = is_ipv6(buf)?;
            let head = generate_packet_information(ipv6);
            let mut iov_block = [IoSlice::new(&head); crate::platform::unix::fd::max_iov()];
            for (index, buf) in bufs.iter().enumerate() {
                iov_block[index + 1] = *buf
            }
            let len = self.fd.writev(&iov_block[..bufs.len() + 1])?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.writev(bufs)
        }
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )))]
    #[inline]
    pub(crate) fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.fd.read(buf)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    #[inline]
    pub(crate) fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        if self.ignore_packet_info() {
            let mut head = [0u8; PIL];
            let bufs = &mut [IoSliceMut::new(&mut head), IoSliceMut::new(buf)];
            let len = self.fd.readv(bufs)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.read(buf)
        }
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )))]
    #[inline]
    pub(crate) fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.fd.readv(bufs)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    #[inline]
    pub(crate) fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        if self.ignore_packet_info() {
            if crate::platform::unix::fd::max_iov() - 1 < bufs.len() {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let offset = bufs.len() + 1;
            let mut head = [0u8; PIL];
            let mut iov_block =
                [const { std::mem::MaybeUninit::uninit() }; crate::platform::unix::fd::max_iov()];
            iov_block[0] = std::mem::MaybeUninit::new(IoSliceMut::new(&mut head));
            for (index, buf) in bufs.iter_mut().enumerate() {
                iov_block[index + 1] = std::mem::MaybeUninit::new(IoSliceMut::new(buf.as_mut()));
            }
            let part: &mut [IoSliceMut] = unsafe {
                std::slice::from_raw_parts_mut(iov_block.as_mut_ptr() as *mut IoSliceMut, offset)
            };
            let len = self.fd.readv(part)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.readv(bufs)
        }
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    #[inline]
    pub(crate) fn ignore_packet_info(&self) -> bool {
        self.ignore_packet_information.load(Ordering::Relaxed)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    ))]
    pub(crate) fn set_ignore_packet_info(&self, ign: bool) {
        self.ignore_packet_information.store(ign, Ordering::Relaxed);
    }
    #[cfg(all(
        feature = "interruptible",
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        ))
    ))]
    #[inline]
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        self.fd.read_interruptible(buf, event, timeout)
    }
    #[cfg(all(
        feature = "interruptible",
        any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        )
    ))]
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        if self.ignore_packet_info() {
            let mut head = [0u8; PIL];
            let bufs = &mut [IoSliceMut::new(&mut head), IoSliceMut::new(buf)];
            let len = self.fd.readv_interruptible(bufs, event, timeout)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.read_interruptible(buf, event, timeout)
        }
    }
    #[cfg(all(
        feature = "interruptible",
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        ))
    ))]
    #[inline]
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        self.fd.readv_interruptible(bufs, event, timeout)
    }
    #[cfg(all(
        feature = "interruptible",
        any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        )
    ))]
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        if self.ignore_packet_info() {
            if crate::platform::unix::fd::max_iov() - 1 < bufs.len() {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let offset = bufs.len() + 1;
            let mut head = [0u8; PIL];
            let mut iov_block =
                [const { std::mem::MaybeUninit::uninit() }; crate::platform::unix::fd::max_iov()];
            iov_block[0] = std::mem::MaybeUninit::new(IoSliceMut::new(&mut head));
            for (index, buf) in bufs.iter_mut().enumerate() {
                iov_block[index + 1] = std::mem::MaybeUninit::new(IoSliceMut::new(buf.as_mut()));
            }
            let part: &mut [IoSliceMut] = unsafe {
                std::slice::from_raw_parts_mut(iov_block.as_mut_ptr() as *mut IoSliceMut, offset)
            };
            let len = self.fd.readv_interruptible(part, event, timeout)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.readv_interruptible(bufs, event, timeout)
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_readable_interruptible(
        &self,
        event: &crate::InterruptEvent,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        self.fd.wait_readable_interruptible(event, timeout)
    }
    #[cfg(all(
        feature = "interruptible",
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        ))
    ))]
    #[inline]
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        self.fd.write_interruptible(buf, event)
    }
    #[cfg(all(
        feature = "interruptible",
        any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        )
    ))]
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        if self.ignore_packet_info() {
            let ipv6 = is_ipv6(buf)?;
            let head = generate_packet_information(ipv6);
            let len = self
                .fd
                .writev_interruptible(&[IoSlice::new(&head), IoSlice::new(buf)], event)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.write_interruptible(buf, event)
        }
    }
    #[cfg(all(
        feature = "interruptible",
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        ))
    ))]
    #[inline]
    pub(crate) fn writev_interruptible(
        &self,
        bufs: &[IoSlice<'_>],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        self.fd.writev_interruptible(bufs, event)
    }
    #[cfg(all(
        feature = "interruptible",
        any(
            target_os = "macos",
            target_os = "ios",
            target_os = "tvos",
            target_os = "openbsd",
            target_os = "freebsd",
            target_os = "netbsd",
        )
    ))]
    pub(crate) fn writev_interruptible(
        &self,
        bufs: &[IoSlice<'_>],
        event: &crate::InterruptEvent,
    ) -> io::Result<usize> {
        if self.ignore_packet_info() {
            if crate::platform::unix::fd::max_iov() - 1 < bufs.len() {
                return Err(io::Error::from(io::ErrorKind::InvalidInput));
            }
            let buf = bufs
                .iter()
                .find(|b| !b.is_empty())
                .map_or(&[][..], |b| &**b);
            let ipv6 = is_ipv6(buf)?;
            let head = generate_packet_information(ipv6);
            let mut iov_block = [IoSlice::new(&head); crate::platform::unix::fd::max_iov()];
            for (index, buf) in bufs.iter().enumerate() {
                iov_block[index + 1] = *buf;
            }
            let len = self
                .fd
                .writev_interruptible(&iov_block[..bufs.len() + 1], event)?;
            Ok(len.saturating_sub(PIL))
        } else {
            self.fd.writev_interruptible(bufs, event)
        }
    }
    #[cfg(feature = "interruptible")]
    #[inline]
    pub(crate) fn wait_writable_interruptible(
        &self,
        event: &crate::InterruptEvent,
    ) -> io::Result<()> {
        self.fd.wait_writable_interruptible(event)
    }
}

impl AsRawFd for Tun {
    fn as_raw_fd(&self) -> RawFd {
        self.fd.as_raw_fd()
    }
}

impl IntoRawFd for Tun {
    fn into_raw_fd(self) -> RawFd {
        self.fd.into_raw_fd()
    }
}
