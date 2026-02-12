use std::io;
use std::ops::Deref;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::task::{Context, Poll};

use crate::platform::DeviceImpl;
use ::tokio::io::unix::AsyncFd as TokioAsyncFd;
use ::tokio::io::Interest;

/// An async Tun/Tap device wrapper around a Tun/Tap device using the Tokio runtime.
///
/// This type does not provide a split method, because this functionality can be achieved by instead wrapping the socket in an Arc.
///
/// # Streams
///
/// If you need to produce a `Stream`, you can look at `DeviceFramed`.
///
/// **Note:** `DeviceFramed` is only available when the `async_framed` feature is enabled.
///
/// [`Stream`]: https://docs.rs/futures/0.3/futures/stream/trait.Stream.html
///
/// # Examples
///
/// ```no_run
/// use tun_rs::{TokioAsyncDevice, DeviceBuilder};
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     // Create a TUN device with basic configuration
///     let dev = DeviceBuilder::new()
///         .name("tun0")
///         .mtu(1500)
///         .ipv4("10.0.0.1", "255.255.255.0", None)
///         .build_async()?;
///
///     // Send a simple packet (Replace with real IP message)
///     let packet = b"[IP Packet: 10.0.0.1 -> 10.0.0.2] Hello, Async TUN!";
///     dev.send(packet).await?;
///
///     // Receive a packet
///     let mut buf = [0u8; 1500];
///     let n = dev.recv(&mut buf).await?;
///     println!("Received {} bytes: {:?}", n, &buf[..n]);
///
///     Ok(())
/// }
/// ```
pub struct TokioAsyncDevice(pub(crate) TokioAsyncFd<DeviceImpl>);
impl TokioAsyncDevice {
    /// Polls the I/O handle for readability.
    ///
    /// # Caveats
    ///
    /// Note that on multiple calls to a `poll_*` method in the `recv` direction, only the
    /// `Waker` from the `Context` passed to the most recent call will be scheduled to
    /// receive a wakeup.
    ///
    /// # Return value
    ///
    /// The function returns:
    ///
    /// * `Poll::Pending` if the device is not ready for reading.
    /// * `Poll::Ready(Ok(()))` if the device is ready for reading.
    /// * `Poll::Ready(Err(e))` if an error is encountered.
    ///
    /// # Errors
    ///
    /// This function may encounter any standard I/O error except `WouldBlock`.
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.poll_read_ready(cx).map_ok(|_| ())
    }
    /// Attempts to receive a single packet from the device
    ///
    /// # Caveats
    ///
    /// Note that on multiple calls to a `poll_*` method in the `recv` direction, only the
    /// `Waker` from the `Context` passed to the most recent call will be scheduled to
    /// receive a wakeup.
    ///
    /// # Return value
    ///
    /// The function returns:
    ///
    /// * `Poll::Pending` if the device is not ready to read
    /// * `Poll::Ready(Ok(()))` reads data `buf` if the device is ready
    /// * `Poll::Ready(Err(e))` if an error is encountered.
    ///
    /// # Errors
    ///
    /// This function may encounter any standard I/O error except `WouldBlock`.
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        loop {
            return match self.0.poll_read_ready(cx) {
                Poll::Ready(Ok(mut rs)) => {
                    let n = match rs.try_io(|dev| dev.get_ref().recv(buf)) {
                        Ok(rs) => rs?,
                        Err(_) => continue,
                    };
                    Poll::Ready(Ok(n))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            };
        }
    }

    /// Polls the I/O handle for writability.
    ///
    /// # Caveats
    ///
    /// Note that on multiple calls to a `poll_*` method in the send direction,
    /// only the `Waker` from the `Context` passed to the most recent call will
    /// be scheduled to receive a wakeup.
    ///
    /// # Return value
    ///
    /// The function returns:
    ///
    /// * `Poll::Pending` if the device is not ready for writing.
    /// * `Poll::Ready(Ok(()))` if the device is ready for writing.
    /// * `Poll::Ready(Err(e))` if an error is encountered.
    ///
    /// # Errors
    ///
    /// This function may encounter any standard I/O error except `WouldBlock`.
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.poll_write_ready(cx).map_ok(|_| ())
    }
    /// Attempts to send packet to the device
    ///
    /// # Caveats
    ///
    /// Note that on multiple calls to a `poll_*` method in the send direction,
    /// only the `Waker` from the `Context` passed to the most recent call will
    /// be scheduled to receive a wakeup.
    ///
    /// # Return value
    ///
    /// The function returns:
    ///
    /// * `Poll::Pending` if the device is not available to write
    /// * `Poll::Ready(Ok(n))` `n` is the number of bytes sent
    /// * `Poll::Ready(Err(e))` if an error is encountered.
    ///
    /// # Errors
    ///
    /// This function may encounter any standard I/O error except `WouldBlock`.
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            return match self.0.poll_write_ready(cx) {
                Poll::Ready(Ok(mut rs)) => {
                    let n = match rs.try_io(|dev| dev.get_ref().send(buf)) {
                        Ok(rs) => rs?,
                        Err(_) => continue,
                    };
                    Poll::Ready(Ok(n))
                }
                Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
                Poll::Pending => Poll::Pending,
            };
        }
    }
}

impl TokioAsyncDevice {
    #[allow(dead_code)]
    pub(crate) fn new(device: crate::SyncDevice) -> io::Result<TokioAsyncDevice> {
        TokioAsyncDevice::new_dev(device.0)
    }

    /// # Safety
    /// This method is safe if the provided fd is valid
    /// Construct a TokioAsyncDevice from an existing file descriptor
    pub unsafe fn from_fd(fd: RawFd) -> io::Result<TokioAsyncDevice> {
        TokioAsyncDevice::new_dev(DeviceImpl::from_fd(fd)?)
    }

    /// # Safety
    /// The fd passed in must be a valid, open file descriptor.
    /// Unlike [`from_fd`], this function does **not** take ownership of `fd`,
    /// and therefore will not close it when dropped.  
    /// The caller is responsible for ensuring the lifetime and eventual closure of `fd`.
    #[allow(dead_code)]
    pub(crate) unsafe fn borrow_raw(fd: RawFd) -> io::Result<Self> {
        TokioAsyncDevice::new_dev(DeviceImpl::borrow_raw(fd)?)
    }

    pub fn into_fd(self) -> io::Result<RawFd> {
        Ok(self.into_device()?.into_raw_fd())
    }

    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self(TokioAsyncFd::new(device)?))
    }
    pub(crate) fn into_device(self) -> io::Result<DeviceImpl> {
        Ok(self.0.into_inner())
    }

    pub(crate) async fn read_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0
            .async_io(Interest::READABLE, |device| op(device))
            .await
    }
    pub(crate) async fn write_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0
            .async_io(Interest::WRITABLE, |device| op(device))
            .await
    }

    pub(crate) fn try_read_io<R>(
        &self,
        f: impl FnOnce(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.try_io(Interest::READABLE, |device| f(device))
    }

    pub(crate) fn try_write_io<R>(
        &self,
        f: impl FnOnce(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.try_io(Interest::WRITABLE, |device| f(device))
    }

    pub(crate) fn get_ref(&self) -> &DeviceImpl {
        self.0.get_ref()
    }

    /// Waits for the device to become readable.
    pub async fn readable(&self) -> io::Result<()> {
        self.read_with(|_| Ok(())).await
    }

    /// Waits for the device to become writable.
    pub async fn writable(&self) -> io::Result<()> {
        self.write_with(|_| Ok(())).await
    }

    /// Receives a single packet from the device.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|device| device.recv(buf)).await
    }

    /// Tries to receive a single packet from the device.
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.try_read_io(|device| device.recv(buf))
    }

    /// Send a packet to the device
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.write_with(|device| device.send(buf)).await
    }

    /// Tries to send packet to the device.
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.try_write_io(|device| device.send(buf))
    }

    /// Receives a packet into multiple buffers (scatter read).
    pub async fn recv_vectored(&self, bufs: &mut [std::io::IoSliceMut<'_>]) -> io::Result<usize> {
        self.read_with(|device| device.recv_vectored(bufs)).await
    }

    /// Non-blocking version of `recv_vectored`.
    pub fn try_recv_vectored(&self, bufs: &mut [std::io::IoSliceMut<'_>]) -> io::Result<usize> {
        self.try_read_io(|device| device.recv_vectored(bufs))
    }

    /// Sends multiple buffers as a single packet (gather write).
    pub async fn send_vectored(&self, bufs: &[std::io::IoSlice<'_>]) -> io::Result<usize> {
        self.write_with(|device| device.send_vectored(bufs)).await
    }

    /// Non-blocking version of `send_vectored`.
    pub fn try_send_vectored(&self, bufs: &[std::io::IoSlice<'_>]) -> io::Result<usize> {
        self.try_write_io(|device| device.send_vectored(bufs))
    }
}

impl FromRawFd for TokioAsyncDevice {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        TokioAsyncDevice::from_fd(fd).unwrap()
    }
}

impl IntoRawFd for TokioAsyncDevice {
    fn into_raw_fd(self) -> RawFd {
        self.into_device().unwrap().into_raw_fd()
    }
}

impl AsRawFd for TokioAsyncDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.get_ref().as_raw_fd()
    }
}

impl Deref for TokioAsyncDevice {
    type Target = DeviceImpl;

    fn deref(&self) -> &Self::Target {
        self.get_ref()
    }
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
impl TokioAsyncDevice {
    /// # Prerequisites
    /// - The `IFF_MULTI_QUEUE` flag must be enabled.
    /// - The system must support network interface multi-queue functionality.
    ///
    /// # Description
    /// When multi-queue is enabled, create a new queue by duplicating an existing one.
    pub fn try_clone(&self) -> io::Result<Self> {
        TokioAsyncDevice::new_dev(self.get_ref().try_clone()?)
    }

    /// Recv a packet from the device.
    /// If offload is enabled. This method can be used to obtain processed data.
    pub async fn recv_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &self,
        original_buffer: &mut [u8],
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        use crate::platform::offload::{VirtioNetHdr, VIRTIO_NET_HDR_LEN};
        
        if bufs.is_empty() || bufs.len() != sizes.len() {
            return Err(io::Error::other("bufs error"));
        }
        let tun = self.get_ref();
        if tun.vnet_hdr {
            let len = self.recv(original_buffer).await?;
            if len <= VIRTIO_NET_HDR_LEN {
                Err(io::Error::other(format!(
                    "length of packet ({len}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                )))?
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
            let len = self.recv(&mut bufs[0].as_mut()[offset..]).await?;
            sizes[0] = len;
            Ok(1)
        }
    }

    /// send multiple fragmented data packets.
    pub async fn send_multiple<B: crate::platform::ExpandBuffer>(
        &self,
        gro_table: &mut crate::platform::GROTable,
        bufs: &mut [B],
        mut offset: usize,
    ) -> io::Result<usize> {
        use crate::platform::offload::{handle_gro, VIRTIO_NET_HDR_LEN};
        
        gro_table.reset();
        let tun = self.get_ref();
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
