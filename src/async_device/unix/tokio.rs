use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
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
/// `AsyncFd::try_io` must not be given this combined interest: it maps only a
/// single interest to a readiness mask, so it sees an empty readiness, never
/// runs the operation and never clears the event, which makes a
/// `readable()` + `try_recv()` loop spin. `ready`, `async_io` and the guard's
/// `try_io` handle it correctly.
#[cfg(any(target_os = "linux", target_os = "android"))]
const READ_INTEREST: Interest = Interest::READABLE.add(Interest::ERROR);
/// Interest used when waiting for the device to become readable; only
/// `READABLE` where a deleted device is not reported as an error condition.
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
pub struct AsyncDevice(pub(crate) Arc<TokioAsyncFd<DeviceImpl>>, ErrorWait);

/// Parked wait for the error readiness, used by the `poll_*` read methods.
///
/// `AsyncFd::poll_read_ready` only observes `READABLE`, and tokio offers no
/// poll-style call for `ERROR`, so the `ready(ERROR)` future is kept between
/// polls to hold the task's waker. The future owns an `Arc` of the descriptor.
type ErrorWait = Mutex<Option<Pin<Box<dyn Future<Output = io::Result<()>> + Send>>>>;

/// Readiness helpers for the poll-style read methods.
impl AsyncDevice {
    /// Polls for an error condition on the device and consumes the parked wait
    /// once it fires. Never completes where `READ_INTEREST` has no `ERROR`.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn poll_error_ready(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut slot = self
            .1
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let wait = slot.get_or_insert_with(|| {
            let fd = self.0.clone();
            Box::pin(async move { fd.ready(Interest::ERROR).await.map(|_| ()) })
        });
        let polled = wait.as_mut().poll(cx);
        if polled.is_ready() {
            *slot = None;
        }
        polled
    }
    /// Platforms without error-only readiness have nothing to wait for.
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn poll_error_ready(&self, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Pending
    }

    /// Poll-style read: runs `op` when the device is readable, or when it only
    /// reports an error so that the operation can return it.
    fn poll_read_with<R>(
        &self,
        cx: &mut Context<'_>,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> Poll<io::Result<R>> {
        loop {
            match self.0.poll_read_ready(cx) {
                Poll::Ready(Ok(mut guard)) => match guard.try_io(|fd| op(fd.get_ref())) {
                    Ok(result) => return Poll::Ready(result),
                    Err(_would_block) => continue,
                },
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => {}
            }
            match self.poll_error_ready(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Ready(Ok(())) => match self.try_read_io(&mut op) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => continue,
                    result => return Poll::Ready(result),
                },
            }
        }
    }
}

impl AsyncDevice {
    /// Polls the I/O handle for readability.
    ///
    /// On Linux and Android it also completes when the device only reports an
    /// error (for example a TUN/TAP interface that was deleted), so the
    /// following read can return that error.
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
        match self.0.poll_read_ready(cx) {
            Poll::Ready(result) => Poll::Ready(result.map(|_| ())),
            Poll::Pending => self.poll_error_ready(cx),
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
        self.poll_read_with(cx, |dev| dev.recv(buf))
    }

    /// Like [`poll_recv`](Self::poll_recv) but reads into uninitialised memory.
    #[allow(dead_code)]
    pub(crate) fn poll_recv_uninit(
        &self,
        cx: &mut Context<'_>,
        buf: &mut UninitSlice,
    ) -> Poll<io::Result<usize>> {
        self.poll_read_with(cx, |dev| dev.recv_uninit(buf))
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
    /// Switches the device to non-blocking mode and registers it with tokio.
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self(Arc::new(TokioAsyncFd::new(device)?), Mutex::new(None)))
    }
    /// Deregisters and returns the device; fails if the descriptor is still
    /// shared.
    pub(crate) fn into_device(self) -> io::Result<DeviceImpl> {
        // The parked error wait holds the only other reference.
        drop(self.1);
        Arc::try_unwrap(self.0)
            .map(TokioAsyncFd::into_inner)
            .map_err(|_| io::Error::other("device is still shared"))
    }

    /// Waits until the device is readable or, on Linux, in an error state.
    ///
    /// The readiness is intentionally not cleared; the following read does it.
    pub(crate) async fn wait_readable(&self) -> io::Result<()> {
        self.0.ready(READ_INTEREST).await.map(|_| ())
    }

    /// Runs `op` once the device is readable (or, on Linux, in an error state).
    pub(crate) async fn read_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.async_io(READ_INTEREST, |device| op(device)).await
    }
    pub(crate) async fn write_with<R>(
        &self,
        mut op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0
            .async_io(Interest::WRITABLE, |device| op(device))
            .await
    }

    /// Runs `f` if the device is currently readable (or in an error state),
    /// clearing the readiness when `f` returns `WouldBlock`; otherwise returns
    /// `WouldBlock` without waiting.
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
