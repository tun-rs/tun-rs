use crate::platform::DeviceImpl;
use ::async_io::Async;
use std::io;
use std::task::{Context, Poll};

pub struct AsyncFd(Async<DeviceImpl>);
impl AsyncFd {
    pub fn new(device: DeviceImpl) -> io::Result<Self> {
        Ok(Self(Async::new(device)?))
    }
    pub fn into_device(self) -> io::Result<DeviceImpl> {
        self.0.into_inner()
    }
    pub async fn readable(&self) -> io::Result<()> {
        self.0.readable().await
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.poll_readable(cx)
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.0.get_ref().recv(buf) {
            Err(e) => if e.kind() == io::ErrorKind::WouldBlock {},
            rs => return Poll::Ready(rs),
        }
        match self.0.poll_readable(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(self.0.get_ref().recv(buf)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    pub async fn writable(&self) -> io::Result<()> {
        self.0.writable().await
    }
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.0.poll_writable(cx)
    }
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.0.get_ref().send(buf) {
            Err(e) => if e.kind() == io::ErrorKind::WouldBlock {},
            rs => return Poll::Ready(rs),
        }
        match self.0.poll_writable(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(self.0.get_ref().send(buf)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }
    pub async fn read_with<R>(
        &self,
        op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.read_with(op).await
    }
    pub async fn write_with<R>(
        &self,
        op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.write_with(op).await
    }
    pub fn try_read_io<R>(&self, f: impl FnOnce(&DeviceImpl) -> io::Result<R>) -> io::Result<R> {
        f(self.0.get_ref())
    }
    pub fn try_write_io<R>(&self, f: impl FnOnce(&DeviceImpl) -> io::Result<R>) -> io::Result<R> {
        f(self.0.get_ref())
    }

    pub fn get_ref(&self) -> &DeviceImpl {
        self.0.get_ref()
    }
}
