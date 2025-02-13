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

#[cfg(feature = "async_tokio")]
mod tokio;
#[cfg(feature = "async_tokio")]
pub use self::tokio::AsyncDevice;

#[cfg(all(feature = "async_std", not(feature = "async_tokio")))]
mod async_std;
#[cfg(all(feature = "async_std", not(feature = "async_tokio")))]
pub use self::async_std::AsyncDevice;

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
        self.get_ref().as_raw_fd()
    }
}

impl Deref for AsyncDevice {
    type Target = DeviceImpl;

    fn deref(&self) -> &Self::Target {
        self.get_ref()
    }
}

impl AsyncDevice {
    pub fn new(device: SyncDevice) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(device.0)
    }

    /// # Safety
    /// This method is safe if the provided fd is valid
    /// Construct a AsyncDevice from an existing file descriptor
    pub unsafe fn from_fd(fd: RawFd) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(DeviceImpl::from_fd(fd))
    }
    pub fn into_fd(self) -> io::Result<RawFd> {
        Ok(self.into_device()?.into_raw_fd())
    }
    /// Waits for the device to become readable.
    ///
    /// This function is usually paired with `try_recv()`.
    ///
    /// The function may complete without the device being readable. This is a
    /// false-positive and attempting a `try_recv()` will return with
    /// `io::ErrorKind::WouldBlock`.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. Once a readiness event occurs, the method
    /// will continue to return immediately until the readiness event is
    /// consumed by an attempt to read that fails with `WouldBlock` or
    /// `Poll::Pending`.
    pub async fn readable(&self) -> io::Result<()> {
        self.0.readable().await.map(|_| ())
    }
    /// Waits for the device to become writable.
    ///
    /// This function is usually paired with `try_send()`.
    ///
    /// The function may complete without the device being writable. This is a
    /// false-positive and attempting a `try_send()` will return with
    /// `io::ErrorKind::WouldBlock`.
    ///
    /// # Cancel safety
    ///
    /// This method is cancel safe. Once a readiness event occurs, the method
    /// will continue to return immediately until the readiness event is
    /// consumed by an attempt to write that fails with `WouldBlock` or
    /// `Poll::Pending`.
    pub async fn writable(&self) -> io::Result<()> {
        self.0.writable().await.map(|_| ())
    }
    /// Receives a single packet from the device.
    /// On success, returns the number of bytes read.
    ///
    /// The function must be called with valid byte array `buf` of sufficient
    /// size to hold the message bytes. If a message is too long to fit in the
    /// supplied buffer, excess bytes may be discarded.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_with(|device| device.recv(buf)).await
    }
    /// Tries to receive a single packet from the device.
    /// On success, returns the number of bytes read.
    ///
    /// This method must be called with valid byte array `buf` of sufficient size
    /// to hold the message bytes. If a message is too long to fit in the
    /// supplied buffer, excess bytes may be discarded.
    ///
    /// When there is no pending data, `Err(io::ErrorKind::WouldBlock)` is
    /// returned. This function is usually paired with `readable()`.
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.try_read_io(|device| device.recv(buf))
    }

    /// Send a packet to the device
    ///
    /// # Return
    /// On success, the number of bytes sent is returned, otherwise, the encountered error is returned.
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.write_with(|device| device.send(buf)).await
    }
    /// Tries to send packet to the device.
    ///
    /// When the device buffer is full, `Err(io::ErrorKind::WouldBlock)` is
    /// returned. This function is usually paired with `writable()`.
    ///
    /// # Returns
    ///
    /// If successful, `Ok(n)` is returned, where `n` is the number of bytes
    /// sent. If the device is not ready to send data,
    /// `Err(ErrorKind::WouldBlock)` is returned.
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.try_write_io(|device| device.send(buf))
    }
    /// Receives a packet into multiple buffers (scatter read).
    /// **Processes single packet per call**.
    pub async fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.read_with(|device| device.recv_vectored(bufs)).await
    }
    /// Non-blocking version of `recv_vectored`.
    pub fn try_recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.try_read_io(|device| device.recv_vectored(bufs))
    }
    /// Sends multiple buffers as a single packet (gather write).
    pub async fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.write_with(|device| device.send_vectored(bufs)).await
    }
    /// Non-blocking version of `send_vectored`.
    pub fn try_send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.try_write_io(|device| device.send_vectored(bufs))
    }
}

#[cfg(target_os = "linux")]
impl AsyncDevice {
    pub fn try_clone(&self) -> io::Result<Self> {
        AsyncDevice::new_dev(self.get_ref().try_clone()?)
    }
    /// Recv a packet from the device.
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
        let tun = self.get_ref();
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
