use crate::DeviceImpl;
use std::future::Future;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

pub(crate) struct NonBlockingDevice {
    device: DeviceImpl,
    is_shutdown: AtomicBool,
    event_fd: EventFd,
}
impl NonBlockingDevice {
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<Self> {
        device.set_nonblocking(true)?;
        Ok(Self {
            device,
            is_shutdown: Default::default(),
            event_fd: EventFd::new()?,
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
        self.is_shutdown.store(true, Ordering::Relaxed);
        self.event_fd.wake()
    }
}

impl NonBlockingDevice {
    pub fn wait_write(&self) -> io::Result<()> {
        let readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut writefds: libc::fd_set = unsafe { std::mem::zeroed() };
        let fd = self.device.as_raw_fd();
        unsafe {
            libc::FD_SET(fd, &mut writefds);
        }
        self.wait(readfds, writefds)
    }
    pub fn wait_read(&self) -> io::Result<()> {
        let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let writefds: libc::fd_set = unsafe { std::mem::zeroed() };
        let fd = self.device.as_raw_fd();
        unsafe {
            libc::FD_SET(fd, &mut readfds);
        }
        self.wait(readfds, writefds)
    }
    fn wait(&self, mut readfds: libc::fd_set, mut writefds: libc::fd_set) -> io::Result<()> {
        let fd = self.device.as_raw_fd();
        let event_fd = self.event_fd.as_event_fd();
        let mut errorfds: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_SET(event_fd, &mut readfds);
        }
        let result = unsafe {
            libc::select(
                fd.max(event_fd) + 1,
                &mut readfds,
                &mut writefds,
                &mut errorfds,
                std::ptr::null_mut(),
            )
        };
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "close"));
        }
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
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
        blocking::unblock(move || device.wait_read()).await?;
        Ok(())
    }
    pub async fn writable(&self) -> io::Result<()> {
        let device = self.inner.clone();
        blocking::unblock(move || device.wait_write()).await?;
        Ok(())
    }
    pub fn poll_readable(&self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let mut guard = self.recv_task_lock.lock().unwrap();
        let mut task = if let Some(task) = guard.take() {
            task
        } else {
            let device = self.inner.clone();
            blocking::unblock(move || device.wait_read())
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
            blocking::unblock(move || device.wait_write())
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
