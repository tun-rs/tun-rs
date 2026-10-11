use std::future::Future;
use std::io;
use std::task::{Context, Poll, Waker};

use crate::platform::DeviceImpl;
use ::tokio::io::unix::AsyncFd as TokioAsyncFd;
use ::tokio::io::Interest;
use bytes::buf::UninitSlice;

/// Interest used when waiting for the device to become readable.
///
/// On Linux a TUN/TAP file whose interface was deleted (`ip link del`) reports
/// only `EPOLLERR` (`tun_chr_poll`), which tokio maps to `Ready::ERROR` and
/// which a `READABLE`-only waiter never observes. Waiting for `ERROR` too lets
/// the following read surface the error instead of hanging forever.
///
/// Only `AsyncFd::ready` and the guard's `try_io` understand a combined
/// interest; never pass it to `AsyncFd::try_io` / `async_io`.
#[cfg(any(target_os = "linux", target_os = "android"))]
const READ_INTEREST: Interest = Interest::READABLE.add(Interest::ERROR);
#[cfg(not(any(target_os = "linux", target_os = "android")))]
const READ_INTEREST: Interest = Interest::READABLE;

/// An async Tun/Tap device wrapper around a Tun/Tap device.
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
/// use tun_rs::{AsyncDevice, DeviceBuilder};
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
pub struct AsyncDevice(pub(crate) TokioAsyncFd<DeviceImpl>);
impl AsyncDevice {
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

    #[allow(dead_code)]
    pub(crate) fn poll_recv_uninit(
        &self,
        cx: &mut Context<'_>,
        buf: &mut UninitSlice,
    ) -> Poll<io::Result<usize>> {
        loop {
            return match self.0.poll_read_ready(cx) {
                Poll::Ready(Ok(mut rs)) => {
                    let n = match rs.try_io(|dev| dev.get_ref().recv_uninit(buf)) {
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

impl AsyncDevice {
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self(TokioAsyncFd::new(device)?))
    }
    pub(crate) fn into_device(self) -> io::Result<DeviceImpl> {
        Ok(self.0.into_inner())
    }

    /// Waits until the device is readable or, on Linux, in an error state.
    ///
    /// The readiness is intentionally not cleared; the following read does it.
    pub(crate) async fn wait_readable(&self) -> io::Result<()> {
        self.0.ready(READ_INTEREST).await.map(|_| ())
    }

    /// Runs `op` once the device is readable (or, on Linux, in an error state).
    ///
    /// This uses `ready` + the guard's `try_io` instead of `AsyncFd::async_io`
    /// / `AsyncFd::try_io`: those take a single interest and, given a combined
    /// one, see an empty readiness (the operation is never attempted and the
    /// readiness is never cleared), which busy-loops callers.
    pub(crate) async fn read_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        loop {
            let mut guard = self.0.ready(READ_INTEREST).await?;
            if let Ok(result) = guard.try_io(|fd| op(fd.get_ref())) {
                return result;
            }
        }
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
        // Poll the readiness once with a no-op waker: a pending result means
        // "not ready" (WouldBlock) without registering a wakeup.
        let mut ready = std::pin::pin!(self.0.ready(READ_INTEREST));
        let mut cx = Context::from_waker(Waker::noop());
        match ready.as_mut().poll(&mut cx) {
            Poll::Ready(Ok(mut guard)) => match guard.try_io(|fd| f(fd.get_ref())) {
                Ok(result) => result,
                Err(_would_block) => Err(io::ErrorKind::WouldBlock.into()),
            },
            Poll::Ready(Err(e)) => Err(e),
            Poll::Pending => Err(io::ErrorKind::WouldBlock.into()),
        }
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
}
