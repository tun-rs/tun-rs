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
        let mut event_scope = EventScope::new()?;
        let device = self.inner.clone();
        let event_waiter = event_scope.waiter();
        blocking::unblock(move || {
            let result = device.wait_readable(event_waiter.handle().as_raw_handle());
            drop(device);
            event_waiter.complete();
            result
        })
        .await?;
        event_scope.forget();
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
        let mut scope_completion = ScopeCompletion::new()?;
        let waiter_completion = scope_completion.waiter();
        let result = blocking::unblock(move || {
            let result = device.send(&buf);
            drop(device);
            waiter_completion.complete();
            result
        })
        .await;
        scope_completion.forget();
        result
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_send(buf)
    }
}

pub(crate) struct EventScope {
    event: Arc<OwnedHandle>,
    completion: ScopeCompletion,
}

pub(crate) struct EventWaiter {
    event: Arc<OwnedHandle>,
    completion: WaiterCompletion,
}

impl EventWaiter {
    pub fn complete(&self) {
        self.completion.complete();
    }
    pub fn handle(&self) -> &OwnedHandle {
        &self.event
    }
}

impl EventScope {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            event: Arc::new(ffi::create_event()?),
            completion: ScopeCompletion::new()?,
        })
    }

    pub fn waiter(&self) -> EventWaiter {
        EventWaiter {
            event: self.event.clone(),
            completion: self.completion.waiter(),
        }
    }

    pub fn forget(&mut self) {
        self.completion.forget();
    }
}

impl Drop for EventScope {
    fn drop(&mut self) {
        if self.completion.armed {
            let _ = ffi::set_event(self.event.as_raw_handle());
        }
    }
}

struct ScopeCompletion {
    armed: bool,
    completed: Arc<AtomicBool>,
    completed_event: Arc<OwnedHandle>,
}

impl ScopeCompletion {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            armed: true,
            completed: Arc::new(AtomicBool::new(false)),
            completed_event: Arc::new(ffi::create_event()?),
        })
    }
    pub fn forget(&mut self) {
        self.armed = false;
    }
    pub fn waiter(&self) -> WaiterCompletion {
        WaiterCompletion {
            completed: self.completed.clone(),
            completed_event: self.completed_event.clone(),
        }
    }
}

struct WaiterCompletion {
    completed: Arc<AtomicBool>,
    completed_event: Arc<OwnedHandle>,
}

impl WaiterCompletion {
    pub fn complete(&self) {
        self.completed.store(true, Ordering::Relaxed);
        let _ = ffi::set_event(self.completed_event.as_raw_handle());
    }
}

impl Drop for ScopeCompletion {
    fn drop(&mut self) {
        if self.armed && !self.completed.load(Ordering::Relaxed) {
            let _ = ffi::wait_for_single_object(self.completed_event.as_raw_handle(), 10);
        }
    }
}
