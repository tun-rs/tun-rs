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
    pub async fn readable(&self) -> io::Result<()> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.readable().await,
            AsyncModel::Select(dev) => dev.readable().await,
        }
    }
    pub async fn writable(&self) -> io::Result<()> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.writable().await,
            AsyncModel::Select(dev) => dev.writable().await,
        }
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_readable(cx),
            AsyncModel::Select(dev) => dev.poll_readable(cx),
        }
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_recv(cx, buf),
            AsyncModel::Select(dev) => dev.poll_recv(cx, buf),
        }
    }
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_writable(cx),
            AsyncModel::Select(dev) => dev.poll_writable(cx),
        }
    }
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.poll_send(cx, buf),
            AsyncModel::Select(dev) => dev.poll_send(cx, buf),
        }
    }
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.recv(buf).await,
            AsyncModel::Select(dev) => dev.recv(buf).await,
        }
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_recv(buf),
            AsyncModel::Select(dev) => dev.try_recv(buf),
        }
    }
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.send(buf).await,
            AsyncModel::Select(dev) => dev.send(buf).await,
        }
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_send(buf),
            AsyncModel::Select(dev) => dev.try_send(buf),
        }
    }
    pub async fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.recv_vectored(bufs).await,
            AsyncModel::Select(dev) => dev.recv_vectored(bufs).await,
        }
    }
    pub fn try_recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_recv_vectored(bufs),
            AsyncModel::Select(dev) => dev.try_recv_vectored(bufs),
        }
    }
    pub async fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.send_vectored(bufs).await,
            AsyncModel::Select(dev) => dev.send_vectored(bufs).await,
        }
    }
    pub fn try_send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        match &self.async_model {
            AsyncModel::Async(dev) => dev.try_send_vectored(bufs),
            AsyncModel::Select(dev) => dev.try_send_vectored(bufs),
        }
    }
}
