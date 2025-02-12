use std::io;
use std::task::{Context, Poll};

use crate::platform::DeviceImpl;
use ::tokio::io::unix::AsyncFd as TokioAsyncFd;
use ::tokio::io::Interest;

pub struct AsyncFd(TokioAsyncFd<DeviceImpl>);
impl AsyncFd {
    pub fn new(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self(TokioAsyncFd::new(device)?))
    }
    pub fn into_device(self) -> io::Result<DeviceImpl> {
        Ok(self.0.into_inner())
    }
    pub async fn readable(&self) -> io::Result<()> {
        _ = self.0.readable().await?;
        Ok(())
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.0.poll_read_ready(cx) {
            Poll::Ready(rs) => Poll::Ready(rs.map(|_| ())),
            Poll::Pending => Poll::Pending,
        }
    }
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
    pub async fn writable(&self) -> io::Result<()> {
        _ = self.0.writable().await?;
        Ok(())
    }
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.0.poll_write_ready(cx) {
            Poll::Ready(rs) => Poll::Ready(rs.map(|_| ())),
            Poll::Pending => Poll::Pending,
        }
    }
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
    pub async fn read_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0
            .async_io(Interest::READABLE.add(Interest::ERROR), |device| op(device))
            .await
    }
    pub async fn write_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0
            .async_io(Interest::WRITABLE, |device| op(device))
            .await
    }

    pub fn try_read_io<R>(&self, f: impl FnOnce(&DeviceImpl) -> io::Result<R>) -> io::Result<R> {
        self.0
            .try_io(Interest::READABLE.add(Interest::ERROR), |device| f(device))
    }

    pub fn try_write_io<R>(&self, f: impl FnOnce(&DeviceImpl) -> io::Result<R>) -> io::Result<R> {
        self.0.try_io(Interest::WRITABLE, |device| f(device))
    }

    pub fn get_ref(&self) -> &DeviceImpl {
        self.0.get_ref()
    }
}
