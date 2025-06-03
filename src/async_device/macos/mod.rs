use crate::async_device::unix;
use crate::{DeviceImpl, SyncDevice};
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::task::{Context, Poll};

mod select_io;

pub struct AsyncDevice {
    async_model: AsyncModel,
}
impl Deref for AsyncDevice {
    type Target = DeviceImpl;
    fn deref(&self) -> &Self::Target {
        self.async_model.as_device()
    }
}
enum AsyncModel {
    Async(unix::AsyncDevice),
    Select(select_io::AsyncDevice),
}

impl AsyncModel {
    fn as_device(&self) -> &DeviceImpl {
        match &self {
            AsyncModel::Async(dev) => dev,
            AsyncModel::Select(dev) => dev,
        }
    }
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
        self.async_model.as_device().as_raw_fd()
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
        match self.async_model {
            AsyncModel::Async(dev) => Ok(dev.into_device()?.into_raw_fd()),
            AsyncModel::Select(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "into_raw_fd operation is not supported for feth/bpf devices",
            )),
        }
    }
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        let async_model = if device.tun.is_tun() {
            AsyncModel::Async(unix::AsyncDevice::new_dev(device)?)
        } else {
            AsyncModel::Select(select_io::AsyncDevice::new_dev(device)?)
        };
        Ok(Self { async_model })
    }
}
impl AsyncDevice {
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.readable().await,
            AsyncModel::Select(dev) => dev.readable().await,
        }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.writable().await,
            AsyncModel::Select(dev) => dev.writable().await,
        }
    }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_readable(cx),
            AsyncModel::Select(dev) => dev.poll_readable(cx),
        }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_recv(cx, buf),
            AsyncModel::Select(dev) => dev.poll_recv(cx, buf),
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_writable(cx),
            AsyncModel::Select(dev) => dev.poll_writable(cx),
        }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_send(cx, buf),
            AsyncModel::Select(dev) => dev.poll_send(cx, buf),
        }
    }
    /// Receives a single packet from the device.
    /// On success, returns the number of bytes read.
    ///
    /// The function must be called with valid byte array `buf` of sufficient
    /// size to hold the message bytes. If a message is too long to fit in the
    /// supplied buffer, excess bytes may be discarded.
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.recv(buf).await,
            AsyncModel::Select(dev) => dev.recv(buf).await,
        }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_recv(buf),
            AsyncModel::Select(dev) => dev.try_recv(buf),
        }
    }
    /// Send a packet to the device
    ///
    /// # Return
    /// On success, the number of bytes sent is returned, otherwise, the encountered error is returned.
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.send(buf).await,
            AsyncModel::Select(dev) => dev.send(buf).await,
        }
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
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_send(buf),
            AsyncModel::Select(dev) => dev.try_send(buf),
        }
    }

    /// Receives a packet into multiple buffers (scatter read).
    /// **Processes single packet per call**.
    pub async fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.recv_vectored(bufs).await,
            AsyncModel::Select(dev) => dev.recv_vectored(bufs).await,
        }
    }
    /// Non-blocking version of `recv_vectored`.
    pub fn try_recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_recv_vectored(bufs),
            AsyncModel::Select(dev) => dev.try_recv_vectored(bufs),
        }
    }
    /// Sends multiple buffers as a single packet (gather write).
    pub async fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.send_vectored(bufs).await,
            AsyncModel::Select(dev) => dev.send_vectored(bufs).await,
        }
    }
    /// Non-blocking version of `send_vectored`.
    pub fn try_send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_send_vectored(bufs),
            AsyncModel::Select(dev) => dev.try_send_vectored(bufs),
        }
    }
}
