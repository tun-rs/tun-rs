use crate::platform::windows::ffi;
use crate::platform::DeviceImpl;
use crate::SyncDevice;
use std::future::Future;
use std::io;
use std::ops::Deref;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
pub struct AsyncDevice {
    inner: Arc<DeviceImpl>,
    recv_task_lock: Arc<Mutex<Option<RecvTask>>>,
    send_task_lock: Arc<Mutex<Option<SendTask>>>,
}
type RecvTask = blocking::Task<io::Result<(Vec<u8>, usize)>>;
type SendTask = blocking::Task<io::Result<usize>>;
impl Deref for AsyncDevice {
    type Target = DeviceImpl;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl Drop for AsyncDevice {
    fn drop(&mut self) {
        _ = self.inner.shutdown();
    }
}
impl AsyncDevice {
    pub fn new(device: SyncDevice) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(device.0)
    }
    /// Create a new `AsyncDevice` wrapping around a `Device`.
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<AsyncDevice> {
        let inner = Arc::new(device);

        Ok(AsyncDevice {
            inner,
            recv_task_lock: Arc::new(Mutex::new(None)),
            send_task_lock: Arc::new(Mutex::new(None)),
        })
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
    pub fn poll_recv(&self, cx: &mut Context<'_>, mut buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.try_recv(buf) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return Poll::Ready(rs),
        }
        let mut guard = self.recv_task_lock.lock().unwrap();
        let mut task = if let Some(task) = guard.take() {
            task
        } else {
            let device = self.inner.clone();
            let size = buf.len();
            blocking::unblock(move || {
                let mut in_buf = vec![0; size];
                let n = device.recv(&mut in_buf)?;
                Ok((in_buf, n))
            })
        };
        match Pin::new(&mut task).poll(cx) {
            Poll::Ready(rs) => {
                drop(guard);
                match rs {
                    Ok((packet, n)) => {
                        let mut packet: &[u8] = &packet[..n];
                        match io::copy(&mut packet, &mut buf) {
                            Ok(n) => Poll::Ready(Ok(n as usize)),
                            Err(e) => Poll::Ready(Err(e)),
                        }
                    }
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
            Poll::Pending => {
                guard.replace(task);
                Poll::Pending
            }
        }
    }
    /// Attempts to send packet to the device
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
    /// * `Poll::Pending` if the device is not available to write
    /// * `Poll::Ready(Ok(n))` `n` is the number of bytes sent
    /// * `Poll::Ready(Err(e))` if an error is encountered.
    ///
    /// # Errors
    ///
    /// This function may encounter any standard I/O error except `WouldBlock`.
    pub fn poll_send(&self, cx: &mut Context<'_>, src: &[u8]) -> Poll<io::Result<usize>> {
        match self.try_send(src) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return Poll::Ready(rs),
        }
        let mut guard = self.send_task_lock.lock().unwrap();
        loop {
            if let Some(mut task) = guard.take() {
                match Pin::new(&mut task).poll(cx) {
                    Poll::Ready(rs) => {
                        // If the previous write was successful, continue.
                        // Otherwise, error.
                        rs?;
                        continue;
                    }
                    Poll::Pending => {
                        guard.replace(task);
                        return Poll::Pending;
                    }
                }
            } else {
                let device = self.inner.clone();
                let buf = src.to_vec();
                let task = blocking::unblock(move || device.send(&buf));
                guard.replace(task);
                drop(guard);
                return Poll::Ready(Ok(src.len()));
            };
        }
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
        let (canceller, cancel_event_handle) = Canceller::new_cancelable()?;
        let device = self.inner.clone();
        let (drop_guard, exit_guard) = canceller.guard();
        blocking::unblock(move || {
            let _exit_guard = exit_guard;
            let result = device.wait_readable(cancel_event_handle.as_raw_handle());
            drop(device);
            result
        })
        .await?;
        std::mem::forget(drop_guard);
        Ok(())
    }

    /// Recv a packet from the device
    pub async fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.try_recv(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return rs,
            }
            self.readable().await?;
        }
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.try_recv(buf)
    }

    /// Send a packet to the device
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.try_send(buf) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return rs,
        }
        let buf = buf.to_vec();
        let device = self.inner.clone();
        let cancel_guard = Canceller::new()?;
        let (drop_guard, exit_guard) = cancel_guard.guard();
        let result = blocking::unblock(move || {
            let _exit_guard = exit_guard;
            let result = device.send(&buf);
            drop(device);
            result
        })
        .await;
        std::mem::forget(drop_guard);
        result
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_send(buf)
    }
}

struct ExitGuard {
    is_finished: Arc<AtomicBool>,
    exit_event: Arc<OwnedHandle>,
}
impl Drop for ExitGuard {
    fn drop(&mut self) {
        self.is_finished.store(true, Ordering::Relaxed);
        _ = ffi::set_event(self.exit_event.as_raw_handle());
    }
}

struct Canceller {
    exit_event_handle: Arc<OwnedHandle>,
    cancel_event_handle: Option<Arc<OwnedHandle>>,
    is_finished: Arc<AtomicBool>,
}

impl Canceller {
    fn new() -> io::Result<Self> {
        Ok(Self {
            exit_event_handle: Arc::new(ffi::create_event()?),
            cancel_event_handle: None,
            is_finished: Arc::new(AtomicBool::new(false)),
        })
    }

    fn new_cancelable() -> io::Result<(Self, Arc<OwnedHandle>)> {
        let event = Arc::new(ffi::create_event()?);
        Ok((
            Self {
                exit_event_handle: Arc::new(ffi::create_event()?),
                cancel_event_handle: Some(event.clone()),
                is_finished: Arc::new(AtomicBool::new(false)),
            },
            event,
        ))
    }

    fn guard(&self) -> (DropGuard<'_>, ExitGuard) {
        (
            DropGuard {
                exit_event_handle: &self.exit_event_handle,
                cancel_event_handle: &self.cancel_event_handle,
                is_finished: &self.is_finished,
            },
            ExitGuard {
                exit_event: self.exit_event_handle.clone(),
                is_finished: self.is_finished.clone(),
            },
        )
    }
}

struct DropGuard<'a> {
    exit_event_handle: &'a Arc<OwnedHandle>,
    cancel_event_handle: &'a Option<Arc<OwnedHandle>>,
    is_finished: &'a Arc<AtomicBool>,
}

impl Drop for DropGuard<'_> {
    fn drop(&mut self) {
        if let Some(cancel_event) = self.cancel_event_handle {
            _ = ffi::set_event(cancel_event.as_raw_handle());
        }
        if !self.is_finished.load(Ordering::Relaxed) {
            _ = ffi::wait_for_single_object(self.exit_event_handle.as_raw_handle(), 10);
        }
    }
}
