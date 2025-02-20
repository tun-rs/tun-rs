use crate::platform::DeviceImpl;
use ::async_io::Async;
use std::io;
use std::task::{Context, Poll};

/// An async Tun/Tap device wrapper around a Tun/Tap device.
///
/// This type does not provide a split method, because this functionality can be achieved by instead wrapping the socket in an Arc.
///
/// # Streams
///
/// If you need to produce a [`Stream`], you can look at [`DeviceFramed`](crate::async_framed::DeviceFramed).
///
/// **Note:** `DeviceFramed` is only available when the `async_framed` feature is enabled.
///
/// [`Stream`]: https://docs.rs/futures/0.3/futures/stream/trait.Stream.html
pub struct AsyncDevice(pub(crate) Async<DeviceImpl>);
impl AsyncDevice {
    /// Polls the I/O handle for readability.
    ///
    /// When this method returns [`Poll::Ready`], that means the OS has delivered an event
    /// indicating readability since the last time this task has called the method and received
    /// [`Poll::Pending`].
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
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
        self.0.poll_readable(cx)
    }

    /// Attempts to receive a single packet from the device
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
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
    /// Polls the I/O handle for writability.
    ///
    /// When this method returns [`Poll::Ready`], that means the OS has delivered an event
    /// indicating writability since the last time this task has called the method and received
    /// [`Poll::Pending`].
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
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
        self.0.poll_writable(cx)
    }
    /// Attempts to send packet to the device
    ///
    /// # Caveats
    ///
    /// Two different tasks should not call this method concurrently. Otherwise, conflicting tasks
    /// will just keep waking each other in turn, thus wasting CPU time.
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
}
impl AsyncDevice {
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        Ok(Self(Async::new(device)?))
    }
    pub(crate) fn into_device(self) -> io::Result<DeviceImpl> {
        self.0.into_inner()
    }

    pub(crate) async fn read_with<R>(
        &self,
        op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.read_with(op).await
    }
    pub(crate) async fn write_with<R>(
        &self,
        op: impl FnMut(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        self.0.write_with(op).await
    }
    pub(crate) fn try_read_io<R>(
        &self,
        f: impl FnOnce(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        f(self.0.get_ref())
    }
    pub(crate) fn try_write_io<R>(
        &self,
        f: impl FnOnce(&DeviceImpl) -> io::Result<R>,
    ) -> io::Result<R> {
        f(self.0.get_ref())
    }

    pub(crate) fn get_ref(&self) -> &DeviceImpl {
        self.0.get_ref()
    }
}
