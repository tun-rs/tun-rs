use crate::DeviceImpl;
use std::cmp::Ordering;
use std::future::Future;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

pub(crate) struct NonBlockingDevice {
    device: DeviceImpl,
    shutdown_event: EventFd,
}
impl NonBlockingDevice {
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self {
            device,
            shutdown_event: EventFd::new()?,
        })
    }

    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.device.recv(buf)
    }

    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.device.send(buf)
    }
    pub fn try_recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.device.recv_vectored(bufs)
    }
    pub fn try_send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.device.send_vectored(bufs)
    }

    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.shutdown_event.wake()
    }
}

impl NonBlockingDevice {
    pub fn wait_writable(&self, cancel_event: Option<libc::c_int>) -> io::Result<()> {
        let readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut writefds: libc::fd_set = unsafe { std::mem::zeroed() };
        let fd = self.device.as_raw_fd();
        unsafe {
            libc::FD_SET(fd, &mut writefds);
        }
        self.wait(readfds, writefds, cancel_event)
    }
    pub fn wait_readable(&self, cancel_event: Option<libc::c_int>) -> io::Result<()> {
        let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let writefds: libc::fd_set = unsafe { std::mem::zeroed() };
        let fd = self.device.as_raw_fd();
        unsafe {
            libc::FD_SET(fd, &mut readfds);
        }
        self.wait(readfds, writefds, cancel_event)
    }
    fn wait(
        &self,
        mut readfds: libc::fd_set,
        mut writefds: libc::fd_set,
        cancel_event: Option<libc::c_int>,
    ) -> io::Result<()> {
        let fd = self.device.as_raw_fd();
        let event_fd = self.shutdown_event.as_event_fd();
        let mut errorfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut nfds = fd.max(event_fd);
        unsafe {
            libc::FD_SET(event_fd, &mut readfds);
            if let Some(cancel_event) = cancel_event {
                libc::FD_SET(cancel_event, &mut readfds);
                nfds = nfds.max(cancel_event);
            }
        }
        let result = unsafe {
            libc::select(
                nfds + 1,
                &mut readfds,
                &mut writefds,
                &mut errorfds,
                std::ptr::null_mut(),
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        unsafe {
            if libc::FD_ISSET(event_fd, &readfds) {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "close"));
            }
            if let Some(cancel_event) = cancel_event {
                if libc::FD_ISSET(cancel_event, &readfds) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "cancel"));
                }
            }
        }
        Ok(())
    }
}

struct EventFd(libc::c_int, libc::c_int);
impl EventFd {
    fn new() -> io::Result<Self> {
        let mut fds = [0 as libc::c_int; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let read_fd = fds[0];
        let write_fd = fds[1];
        Ok(Self(read_fd, write_fd))
    }
    fn wake(&self) -> io::Result<()> {
        let buf: [u8; 8] = 2u64.to_ne_bytes();
        let res = unsafe { libc::write(self.1, buf.as_ptr() as *const libc::c_void, buf.len()) };
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    fn wait_timeout(&self, timeout: Duration) -> io::Result<()> {
        let mut readfds = unsafe {
            let mut set = std::mem::zeroed::<libc::fd_set>();
            libc::FD_ZERO(&mut set);
            libc::FD_SET(self.0, &mut set);
            set
        };
        let mut tv = libc::timeval {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_usec: timeout.subsec_micros() as libc::suseconds_t,
        };
        let res = unsafe {
            libc::select(
                self.0 + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut tv,
            )
        };
        match res.cmp(&0) {
            Ordering::Less => Err(io::Error::last_os_error()),
            Ordering::Equal => Err(io::Error::from(io::ErrorKind::TimedOut)),
            Ordering::Greater => Ok(()),
        }
    }
    fn as_event_fd(&self) -> libc::c_int {
        self.0
    }
}
impl Drop for EventFd {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::close(self.0);
            let _ = libc::close(self.1);
        }
    }
}

pub struct AsyncDevice {
    inner: Arc<NonBlockingDevice>,
    recv_task_lock: Arc<Mutex<Option<blocking::Task<io::Result<()>>>>>,
    send_task_lock: Arc<Mutex<Option<blocking::Task<io::Result<()>>>>>,
}

impl Deref for AsyncDevice {
    type Target = DeviceImpl;
    fn deref(&self) -> &Self::Target {
        &self.inner.device
    }
}
impl Drop for AsyncDevice {
    fn drop(&mut self) {
        _ = self.inner.shutdown();
    }
}
impl AsyncDevice {
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<AsyncDevice> {
        let inner = Arc::new(NonBlockingDevice::new_dev(device)?);
        Ok(AsyncDevice {
            inner,
            recv_task_lock: Arc::new(Mutex::new(None)),
            send_task_lock: Arc::new(Mutex::new(None)),
        })
    }
}
impl AsyncDevice {
    pub async fn readable(&self) -> io::Result<()> {
        let device = self.inner.clone();
        let mut canceller = Canceller::new_cancelable()?;
        let (cancel_guard, exit_guard) = canceller.guard(device);
        blocking::unblock(move || {
            exit_guard
                .call(|device, cancel_event| device.wait_readable(Some(cancel_event.as_event_fd())))
        })
        .await?;
        std::mem::forget(cancel_guard);
        Ok(())
    }
    pub async fn writable(&self) -> io::Result<()> {
        let device = self.inner.clone();
        let mut canceller = Canceller::new_cancelable()?;
        let (cancel_guard, exit_guard) = canceller.guard(device);
        blocking::unblock(move || {
            exit_guard
                .call(|device, cancel_event| device.wait_writable(Some(cancel_event.as_event_fd())))
        })
        .await?;
        std::mem::forget(cancel_guard);
        Ok(())
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut guard = self.recv_task_lock.lock().unwrap();
        let mut task = if let Some(task) = guard.take() {
            task
        } else {
            let device = self.inner.clone();
            blocking::unblock(move || device.wait_readable(None))
        };
        match Pin::new(&mut task).poll(cx) {
            Poll::Ready(rs) => {
                drop(guard);
                match rs {
                    Ok(_) => Poll::Ready(Ok(())),
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
            Poll::Pending => {
                guard.replace(task);
                Poll::Pending
            }
        }
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        loop {
            match self.try_recv(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return Poll::Ready(rs),
            }
            match self.poll_readable(cx)? {
                Poll::Ready(_) => {}
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
    pub fn poll_writable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut guard = self.send_task_lock.lock().unwrap();
        let mut task = if let Some(task) = guard.take() {
            task
        } else {
            let device = self.inner.clone();
            blocking::unblock(move || device.wait_writable(None))
        };
        match Pin::new(&mut task).poll(cx) {
            Poll::Ready(rs) => match rs {
                Ok(_) => Poll::Ready(Ok(())),
                Err(e) => Poll::Ready(Err(e)),
            },
            Poll::Pending => {
                guard.replace(task);
                Poll::Pending
            }
        }
    }
    pub fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        loop {
            match self.try_send(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return Poll::Ready(rs),
            }
            match self.poll_writable(cx)? {
                Poll::Ready(_) => {}
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
    }
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
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match self.try_send(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return rs,
            }
            self.writable().await?;
        }
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_send(buf)
    }
    pub async fn recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        loop {
            match self.try_recv_vectored(bufs) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return rs,
            }
            self.readable().await?;
        }
    }
    pub fn try_recv_vectored(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.inner.try_recv_vectored(bufs)
    }
    pub async fn send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        loop {
            match self.try_send_vectored(bufs) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                rs => return rs,
            }
            self.writable().await?;
        }
    }
    pub fn try_send_vectored(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        self.inner.try_send_vectored(bufs)
    }
}

struct ExitSignalGuard {
    device: Option<Arc<NonBlockingDevice>>,
    cancel_event_handle: Arc<EventFd>,
    exit_event: Arc<EventFd>,
}
impl Drop for ExitSignalGuard {
    fn drop(&mut self) {
        drop(self.device.take());
        _ = self.exit_event.wake();
    }
}
impl ExitSignalGuard {
    pub fn call<R>(
        &self,
        mut op: impl FnMut(&NonBlockingDevice, &EventFd) -> io::Result<R>,
    ) -> io::Result<R> {
        if let Some(device) = &self.device {
            op(device, &self.cancel_event_handle)
        } else {
            unreachable!()
        }
    }
}

struct Canceller {
    exit_event_handle: Arc<EventFd>,
    cancel_event_handle: Arc<EventFd>,
}

impl Canceller {
    fn new_cancelable() -> io::Result<Self> {
        Ok(Self {
            exit_event_handle: Arc::new(EventFd::new()?),
            cancel_event_handle: Arc::new(EventFd::new()?),
        })
    }

    fn guard(
        &mut self,
        device_impl: Arc<NonBlockingDevice>,
    ) -> (CancelWaitGuard<'_>, ExitSignalGuard) {
        (
            CancelWaitGuard {
                exit_event_handle: &self.exit_event_handle,
                cancel_event_handle: &self.cancel_event_handle,
            },
            ExitSignalGuard {
                device: Some(device_impl),
                exit_event: self.exit_event_handle.clone(),
                cancel_event_handle: self.cancel_event_handle.clone(),
            },
        )
    }
}

struct CancelWaitGuard<'a> {
    exit_event_handle: &'a Arc<EventFd>,
    cancel_event_handle: &'a Arc<EventFd>,
}

impl Drop for CancelWaitGuard<'_> {
    fn drop(&mut self) {
        if self.cancel_event_handle.wake().is_ok() {
            _ = self
                .exit_event_handle
                .wait_timeout(Duration::from_millis(1))
        }
    }
}
