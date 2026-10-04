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
use bytes::buf::UninitSlice;
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

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
fn strip_packet_info_read_len(len: usize) -> io::Result<usize> {
    len.checked_sub(PIL).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "TUN/TAP read returned fewer bytes than the packet-information header",
        )
    })
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "openbsd",
    target_os = "freebsd",
    target_os = "netbsd",
))]
fn strip_packet_info_write_len(len: usize) -> io::Result<usize> {
    len.checked_sub(PIL).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::WriteZero,
            "TUN/TAP write consumed fewer bytes than the packet-information header",
        )
    })
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
            return strip_packet_info_write_len(len);
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
            let mut iov_block = Vec::with_capacity(bufs.len() + 1);
            iov_block.push(IoSlice::new(&head));
            iov_block.extend(bufs.iter().copied());
            let len = self.fd.writev(&iov_block)?;
            strip_packet_info_write_len(len)
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
    #[cfg(not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )))]
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn recv_uninit(&self, buf: &mut UninitSlice) -> io::Result<usize> {
        self.fd.read_uninit(buf)
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
            strip_packet_info_read_len(len)
        } else {
            self.fd.read(buf)
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
    #[allow(dead_code)]
    pub(crate) fn recv_uninit(&self, buf: &mut UninitSlice) -> io::Result<usize> {
        if self.ignore_packet_info() {
            let mut head = [0u8; PIL];
            let mut bufs = [
                libc::iovec {
                    iov_base: head.as_mut_ptr() as *mut _,
                    iov_len: head.len(),
                },
                libc::iovec {
                    iov_base: buf.as_mut_ptr() as *mut _,
                    iov_len: buf.len(),
                },
            ];
            let len = self.fd.readv_raw(&mut bufs)?;
            strip_packet_info_read_len(len)
        } else {
            self.fd.read_uninit(buf)
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
            let mut head = [0u8; PIL];
            let mut iov_block = Vec::with_capacity(bufs.len() + 1);
            iov_block.push(IoSliceMut::new(&mut head));
            for buf in bufs.iter_mut() {
                iov_block.push(IoSliceMut::new(buf.as_mut()));
            }
            let len = self.fd.readv(&mut iov_block)?;
            strip_packet_info_read_len(len)
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
            strip_packet_info_read_len(len)
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
            let mut head = [0u8; PIL];
            let mut iov_block = Vec::with_capacity(bufs.len() + 1);
            iov_block.push(IoSliceMut::new(&mut head));
            for buf in bufs.iter_mut() {
                iov_block.push(IoSliceMut::new(buf.as_mut()));
            }
            let len = self
                .fd
                .readv_interruptible(&mut iov_block, event, timeout)?;
            strip_packet_info_read_len(len)
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
            strip_packet_info_write_len(len)
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
            let mut iov_block = Vec::with_capacity(bufs.len() + 1);
            iov_block.push(IoSlice::new(&head));
            iov_block.extend(bufs.iter().copied());
            let len = self.fd.writev_interruptible(&iov_block, event)?;
            strip_packet_info_write_len(len)
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

#[cfg(all(
    test,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "openbsd",
        target_os = "freebsd",
        target_os = "netbsd",
    )
))]
mod packet_information_length_tests {
    use super::{strip_packet_info_read_len, strip_packet_info_write_len, PIL};
    use std::io;

    #[test]
    fn short_packet_information_io_is_an_error() {
        assert_eq!(strip_packet_info_read_len(PIL).unwrap_or(usize::MAX), 0);
        assert_eq!(
            strip_packet_info_write_len(PIL + 7).unwrap_or(usize::MAX),
            7
        );
        assert!(matches!(
            strip_packet_info_read_len(PIL - 1),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof
        ));
        assert!(matches!(
            strip_packet_info_write_len(PIL - 1),
            Err(error) if error.kind() == io::ErrorKind::WriteZero
        ));
    }
}
