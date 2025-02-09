#[cfg(target_os = "linux")]
use crate::platform::offload::{handle_gro, VirtioNetHdr, VIRTIO_NET_HDR_LEN};
use crate::platform::DeviceImpl;
#[cfg(target_os = "linux")]
use crate::platform::GROTable;
use crate::SyncDevice;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::task::{Context, Poll};

#[cfg(feature = "async_tokio")]
mod tokio;
#[cfg(feature = "async_tokio")]
pub use self::tokio::*;

#[cfg(all(feature = "async_std", not(feature = "async_tokio")))]
mod async_std;
#[cfg(all(feature = "async_std", not(feature = "async_tokio")))]
pub use self::async_std::*;

#[cfg(all(feature = "async_tokio", feature = "async_std", not(doc)))]
compile_error! {"More than one asynchronous runtime is simultaneously specified in features"}

/// An async TUN device wrapper around a TUN device.
pub struct AsyncDevice {
    inner: AsyncFd,
}

impl FromRawFd for AsyncDevice {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        AsyncDevice::from_fd(fd).unwrap()
    }
}
impl IntoRawFd for AsyncDevice {
    fn into_raw_fd(self) -> RawFd {
        self.into_fd().unwrap()
    }
}
impl AsRawFd for AsyncDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.inner.get_ref().as_raw_fd()
    }
}

impl Deref for AsyncDevice {
    type Target = DeviceImpl;

    fn deref(&self) -> &Self::Target {
        self.inner.get_ref()
    }
}

impl AsyncDevice {
    pub fn new(device: SyncDevice) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(device.0)
    }
    /// Create a new `AsyncDevice` wrapping around a `Device`.
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<AsyncDevice> {
        Ok(AsyncDevice {
            inner: AsyncFd::new(device)?,
        })
    }

    /// # Safety
    /// This method is safe if the provided fd is valid
    /// Construct a AsyncDevice from an existing file descriptor
    pub unsafe fn from_fd(fd: RawFd) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(DeviceImpl::from_fd(fd))
    }
    pub fn into_fd(self) -> io::Result<RawFd> {
        Ok(self.inner.into_device()?.into_raw_fd())
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        loop {
            return match self.poll_readable(cx) {
                Poll::Ready(Ok(())) => match self.try_recv(buf) {
                    Ok(n) => Poll::Ready(Ok(n)),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => Poll::Ready(Err(e)),
                },
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            };
        }
    }
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            return match self.poll_writable(cx) {
                Poll::Ready(Ok(())) => match self.try_send(buf) {
                    Ok(n) => Poll::Ready(Ok(n)),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    Err(e) => Poll::Ready(Err(e)),
                },
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            };
        }
    }
    pub async fn readable(&self) -> io::Result<()> {
        self.inner.readable().await
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_readable(cx)
    }
    pub async fn writable(&self) -> io::Result<()> {
        self.inner.writable().await
    }
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.inner.poll_writable(cx)
    }
    /// Recv a packet from tun device
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.recv(buf).await
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.get_ref().recv(buf)
    }

    /// Send a packet to tun device
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.send(buf).await
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.get_ref().send(buf)
    }
    pub async fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.inner.send_vectored(bufs).await
    }
    pub async fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.inner.recv_vectored(bufs).await
    }
}

#[cfg(target_os = "linux")]
impl AsyncDevice {
    pub fn try_clone(&self) -> io::Result<Self> {
        AsyncDevice::new_dev(self.inner.get_ref().try_clone()?)
    }
    /// Recv a packet from tun device.
    /// If offload is enabled. This method can be used to obtain processed data.
    ///
    /// original_buffer is used to store raw data, including the VirtioNetHdr and the unsplit IP packet. The recommended size is 10 + 65535.
    /// bufs and sizes are used to store the segmented IP packets. bufs.len == sizes.len > 65535/MTU
    /// offset: Starting position
    #[cfg(target_os = "linux")]
    pub async fn recv_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &self,
        original_buffer: &mut [u8],
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        if bufs.is_empty() || bufs.len() != sizes.len() {
            return Err(io::Error::new(io::ErrorKind::Other, "bufs error"));
        }
        let tun = self.inner.get_ref();
        if tun.vnet_hdr {
            let len = self.recv(original_buffer).await?;
            if len <= VIRTIO_NET_HDR_LEN {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "length of packet ({len}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                    ),
                ))?
            }
            let hdr = VirtioNetHdr::decode(&original_buffer[..VIRTIO_NET_HDR_LEN])?;
            tun.handle_virtio_read(
                hdr,
                &mut original_buffer[VIRTIO_NET_HDR_LEN..len],
                bufs,
                sizes,
                offset,
            )
        } else {
            let len = self.recv(bufs[0].as_mut()).await?;
            sizes[0] = len;
            Ok(1)
        }
    }
    /// send multiple fragmented data packets.
    /// GROTable can be reused, as it is used to assist in data merging.
    /// Offset is the starting position of the data. Need to meet offset>10.
    #[cfg(target_os = "linux")]
    pub async fn send_multiple<B: crate::platform::ExpandBuffer>(
        &self,
        gro_table: &mut GROTable,
        bufs: &mut [B],
        mut offset: usize,
    ) -> io::Result<usize> {
        gro_table.reset();
        let tun = self.inner.get_ref();
        if tun.vnet_hdr {
            handle_gro(
                bufs,
                offset,
                &mut gro_table.tcp_gro_table,
                &mut gro_table.udp_gro_table,
                tun.udp_gso,
                &mut gro_table.to_write,
            )?;
            offset -= VIRTIO_NET_HDR_LEN;
        } else {
            for i in 0..bufs.len() {
                gro_table.to_write.push(i);
            }
        }

        let mut total = 0;
        let mut err = Ok(());
        for buf_idx in &gro_table.to_write {
            match self.send(&bufs[*buf_idx].as_ref()[offset..]).await {
                Ok(n) => {
                    total += n;
                }
                Err(e) => {
                    if let Some(code) = e.raw_os_error() {
                        if libc::EBADFD == code {
                            return Err(e);
                        }
                    }
                    err = Err(e)
                }
            }
        }
        err?;
        Ok(total)
    }
}
