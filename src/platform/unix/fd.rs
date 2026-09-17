use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};

use bytes::buf::UninitSlice;
use libc::{self, fcntl, F_GETFL, O_NONBLOCK};

/// POSIX file descriptor support for `io` traits.
pub(crate) struct Fd {
    pub(crate) inner: RawFd,
    borrow: bool,
}

impl Fd {
    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
    ))]
    pub(crate) fn new(value: RawFd) -> io::Result<Self> {
        if value < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { Self::new_unchecked(value) })
    }
    pub(crate) unsafe fn new_unchecked(value: RawFd) -> Self {
        Fd::new_unchecked_with_borrow(value, false)
    }
    pub(crate) unsafe fn new_unchecked_with_borrow(value: RawFd, borrow: bool) -> Self {
        Fd {
            inner: value,
            borrow,
        }
    }
    #[inline]
    pub(crate) const fn should_drop_cleanup(&self) -> bool {
        self.inner >= 0 && !self.borrow
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
    #[cfg(target_os = "macos")]
    pub(crate) fn set_cloexec(&self) -> io::Result<()> {
        unsafe {
            let flags = fcntl(self.inner, libc::F_GETFD);
            if flags < 0 {
                return Err(io::Error::last_os_error());
            }
            if fcntl(self.inner, libc::F_SETFD, flags | libc::FD_CLOEXEC) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
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
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        let amount = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn read_uninit(&self, buf: &mut UninitSlice) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        let amount = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if amount < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(amount as usize)
    }
    #[inline]
    pub fn readv(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
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
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    pub(crate) fn readv_raw(&self, bufs: &mut [libc::iovec]) -> io::Result<usize> {
        if bufs.len() > max_iov() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let amount =
            unsafe { libc::readv(self.as_raw_fd(), bufs.as_ptr(), bufs.len() as libc::c_int) };
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
    target_os = "openbsd",
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
        if self.should_drop_cleanup() {
            unsafe { libc::close(self.inner) };
            self.inner = -1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Fd;
    use std::fs::File;
    use std::os::fd::AsRawFd;

    #[test]
    fn should_drop_cleanup_matches_ownership() {
        let owned = unsafe { Fd::new_unchecked(1) };
        assert!(owned.should_drop_cleanup());

        let borrowed = unsafe { Fd::new_unchecked_with_borrow(1, true) };
        assert!(!borrowed.should_drop_cleanup());

        let invalid = unsafe { Fd::new_unchecked_with_borrow(-1, false) };
        assert!(!invalid.should_drop_cleanup());
    }

    #[test]
    fn borrowed_fd_drop_leaves_descriptor_open() {
        let file = File::open("/dev/null").unwrap();
        let raw_fd = file.as_raw_fd();

        let fd = unsafe { Fd::new_unchecked_with_borrow(raw_fd, true) };
        drop(fd);

        assert!(unsafe { libc::fcntl(raw_fd, libc::F_GETFD) } >= 0);
    }

    #[test]
    fn owned_fd_drop_closes_descriptor() {
        let file = File::open("/dev/null").unwrap();
        let raw_fd = unsafe { libc::dup(file.as_raw_fd()) };
        assert!(raw_fd >= 0);

        let fd = unsafe { Fd::new_unchecked(raw_fd) };
        drop(fd);

        assert_eq!(unsafe { libc::fcntl(raw_fd, libc::F_GETFD) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EBADF)
        );
    }
}
