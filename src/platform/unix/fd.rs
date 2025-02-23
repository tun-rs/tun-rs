use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};
#[cfg(feature = "experimental")]
use std::sync::atomic::{AtomicBool, Ordering};

use libc::{self, fcntl, F_GETFL, O_NONBLOCK};

/// POSIX file descriptor support for `io` traits.
pub(crate) struct Fd {
    pub(crate) inner: RawFd,
    #[cfg(feature = "experimental")]
    is_shutdown: AtomicBool,
    #[cfg(feature = "experimental")]
    event_fd: EventFd,
}

impl Fd {
    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd"
    ))]
    pub(crate) fn new(value: RawFd) -> io::Result<Self> {
        if value < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { Self::new_unchecked(value) })
    }
    pub(crate) unsafe fn new_unchecked(value: RawFd) -> Self {
        Fd {
            inner: value,
            #[cfg(feature = "experimental")]
            is_shutdown: AtomicBool::new(false),
            #[cfg(feature = "experimental")]
            event_fd: EventFd::new().expect("failed to create event fd"),
        }
    }
    pub(crate) fn is_nonblocking(&self) -> io::Result<bool> {
        unsafe {
            let flags = fcntl(self.inner, F_GETFL);
            if flags == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok((flags & O_NONBLOCK) != 0)
        }
    }
    /// Enable non-blocking mode
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        let mut nonblocking = nonblocking as libc::c_int;
        match unsafe { libc::ioctl(self.as_raw_fd(), libc::FIONBIO, &mut nonblocking) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    #[inline]
    fn read0(&self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        let amount = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }
    #[inline]
    fn readv0(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        if bufs.len() > max_iov() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let amount = unsafe {
            libc::readv(
                self.as_raw_fd(),
                bufs.as_mut_ptr() as *mut libc::iovec as *const libc::iovec,
                bufs.len() as libc::c_int,
            )
        };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }

    #[inline]
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        let amount = unsafe { libc::write(fd, buf.as_ptr() as *const _, buf.len()) };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }
    #[inline]
    pub fn writev(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        if bufs.len() > max_iov() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let amount = unsafe {
            libc::writev(
                self.as_raw_fd(),
                bufs.as_ptr() as *const libc::iovec,
                bufs.len() as libc::c_int,
            )
        };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }
}
#[cfg(any(
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_vendor = "apple",
))]
pub(crate) const fn max_iov() -> usize {
    libc::IOV_MAX as usize
}

#[cfg(any(
    target_os = "android",
    target_os = "emscripten",
    target_os = "linux",
    target_os = "nto",
))]
pub(crate) const fn max_iov() -> usize {
    libc::UIO_MAXIOV as usize
}
#[cfg(not(feature = "experimental"))]
impl Fd {
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.read0(buf)
    }
    pub fn readv(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        self.readv0(bufs)
    }
}
#[cfg(feature = "experimental")]
impl Fd {
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "close"));
        }
        if self.is_nonblocking()? {
            return self.read0(buf);
        }
        self.wait()?;
        self.read0(buf)
    }
    pub fn readv(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "close"));
        }
        if self.is_nonblocking()? {
            return self.readv0(bufs);
        }
        self.wait()?;
        self.readv0(bufs)
    }
    fn wait(&self) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;

        let event_fd = self.event_fd.as_event_fd();
        let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_SET(fd, &mut readfds);
            libc::FD_SET(event_fd, &mut readfds);
        }
        let result = unsafe {
            libc::select(
                fd.max(event_fd) + 1,
                &mut readfds,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if self.is_shutdown.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::ConnectionAborted, "close"));
        }
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        Ok(())
    }
    pub fn shutdown(&self) -> io::Result<()> {
        self.is_shutdown.store(true, Ordering::Relaxed);
        self.event_fd.wake()
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
#[cfg(feature = "experimental")]
struct EventFd(std::fs::File);
#[cfg(feature = "experimental")]
#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
impl EventFd {
    fn new() -> io::Result<Self> {
        #[cfg(not(target_os = "espidf"))]
        let flags = libc::EFD_CLOEXEC | libc::EFD_NONBLOCK;
        // ESP-IDF is EFD_NONBLOCK by default and errors if you try to pass this flag.
        #[cfg(target_os = "espidf")]
        let flags = 0;
        let event_fd = unsafe { libc::eventfd(0, flags) };
        if event_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        use std::os::fd::FromRawFd;
        let file = unsafe { std::fs::File::from_raw_fd(event_fd) };
        Ok(Self(file))
    }
    fn wake(&self) -> io::Result<()> {
        use std::io::Write;
        let buf: [u8; 8] = 1u64.to_ne_bytes();
        match (&self.0).write_all(&buf) {
            Ok(_) => Ok(()),
            Err(ref err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(err) => Err(err),
        }
    }
    fn as_event_fd(&self) -> libc::c_int {
        self.0.as_raw_fd() as _
    }
}
#[cfg(any(target_os = "macos", target_os = "ios"))]
#[cfg(feature = "experimental")]
struct EventFd(libc::c_int, libc::c_int);
#[cfg(feature = "experimental")]
#[cfg(any(target_os = "macos", target_os = "ios"))]
impl EventFd {
    fn new() -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let read_fd = fds[0];
        let write_fd = fds[1];
        Ok(Self(read_fd, write_fd))
    }
    fn wake(&self) -> io::Result<()> {
        let buf: [u8; 8] = 1u64.to_ne_bytes();
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
#[cfg(feature = "experimental")]
#[cfg(any(target_os = "macos", target_os = "ios"))]
impl Drop for EventFd {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::close(self.0);
            let _ = libc::close(self.1);
        }
    }
}
impl AsRawFd for Fd {
    fn as_raw_fd(&self) -> RawFd {
        self.inner
    }
}

impl IntoRawFd for Fd {
    fn into_raw_fd(mut self) -> RawFd {
        let fd = self.inner;
        self.inner = -1;
        fd
    }
}

impl Drop for Fd {
    fn drop(&mut self) {
        if self.inner >= 0 {
            unsafe { libc::close(self.inner) };
        }
    }
}
