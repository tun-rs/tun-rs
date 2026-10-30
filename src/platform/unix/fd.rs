use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::unix::io::{AsRawFd, IntoRawFd, RawFd};

use bytes::buf::UninitSlice;
use libc::{self, fcntl, F_GETFL, O_NONBLOCK};

fn syscall_result_len(amount: libc::ssize_t) -> io::Result<usize> {
    if amount < 0 {
        return Err(io::Error::last_os_error());
    }
    usize::try_from(amount).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "successful Unix I/O byte count does not fit usize",
        )
    })
}

fn checked_iov_count(len: usize) -> io::Result<libc::c_int> {
    libc::c_int::try_from(len).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))
}

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
        // SAFETY: this function has the same raw-fd ownership contract with borrow=false.
        unsafe { Fd::new_unchecked_with_borrow(value, false) }
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
        // SAFETY: self owns or validly borrows this live descriptor.
        unsafe {
            let flags = fcntl(self.inner, F_GETFL);
            if flags == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok((flags & O_NONBLOCK) != 0)
        }
    }
    /// Adds `FD_CLOEXEC` while preserving the descriptor's existing flags.
    #[cfg(any(
        target_os = "macos",
        all(
            feature = "interruptible",
            not(any(
                target_os = "linux",
                target_os = "android",
                target_os = "freebsd",
                target_os = "openbsd",
                target_os = "netbsd",
            ))
        )
    ))]
    pub(crate) fn set_cloexec(&self) -> io::Result<()> {
        // SAFETY: `self.inner` is a live descriptor tracked by `Fd`; F_GETFD only
        // reads descriptor flags and has no pointer arguments.
        let flags = unsafe { fcntl(self.inner, libc::F_GETFD) };
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: the same live descriptor is used, and F_SETFD consumes only the
        // integer flag value returned by F_GETFD with FD_CLOEXEC added.
        if unsafe { fcntl(self.inner, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    /// Enable non-blocking mode
    pub fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
        let mut nonblocking = libc::c_int::from(nonblocking);
        // SAFETY: FIONBIO expects a pointer to an int and self provides a live descriptor.
        match unsafe { libc::ioctl(self.as_raw_fd(), libc::FIONBIO, &mut nonblocking) } {
            0 => Ok(()),
            _ => Err(io::Error::last_os_error()),
        }
    }

    #[inline]
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        // SAFETY: buf provides writable storage for buf.len() bytes and fd is live.
        let amount = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        syscall_result_len(amount)
    }
    #[inline]
    #[allow(dead_code)]
    pub(crate) fn read_uninit(&self, buf: &mut UninitSlice) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        // SAFETY: UninitSlice exposes writable storage for buf.len() bytes and fd is live.
        let amount = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        syscall_result_len(amount)
    }
    #[inline]
    pub fn readv(&self, bufs: &mut [IoSliceMut<'_>]) -> io::Result<usize> {
        if bufs.len() > max_iov() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let iov_count = checked_iov_count(bufs.len())?;
        // SAFETY: IoSliceMut uses the platform iovec representation; bufs remains live for
        // the duration of readv, and iov_count is the checked slice length.
        let amount = unsafe {
            libc::readv(
                self.as_raw_fd(),
                bufs.as_ptr().cast::<libc::iovec>(),
                iov_count,
            )
        };
        syscall_result_len(amount)
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
        let iov_count = checked_iov_count(bufs.len())?;
        // SAFETY: bufs is a live array of libc::iovec values and iov_count is its checked length.
        let amount = unsafe { libc::readv(self.as_raw_fd(), bufs.as_ptr(), iov_count) };
        syscall_result_len(amount)
    }

    #[inline]
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let fd = self.as_raw_fd();
        // SAFETY: buf provides readable storage for buf.len() bytes and fd is live.
        let amount = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        syscall_result_len(amount)
    }
    #[inline]
    pub fn writev(&self, bufs: &[IoSlice<'_>]) -> io::Result<usize> {
        if bufs.len() > max_iov() {
            return Err(io::Error::from(io::ErrorKind::InvalidInput));
        }
        let iov_count = checked_iov_count(bufs.len())?;
        // SAFETY: IoSlice uses the platform iovec representation; bufs remains live for the
        // duration of writev, and iov_count is the checked slice length.
        let amount = unsafe {
            libc::writev(
                self.as_raw_fd(),
                bufs.as_ptr().cast::<libc::iovec>(),
                iov_count,
            )
        };
        syscall_result_len(amount)
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

// illumos has neither `IOV_MAX` nor `UIO_MAXIOV` as a compile-time constant in `libc` --
// unlike Linux/BSD, it doesn't fix this value at build time, so it's only available via
// `sysconf(_SC_IOV_MAX)` at runtime (this is itself standard POSIX behavior for platforms
// that don't hardcode an IOV_MAX). Not `const fn` for this platform, since `sysconf` is a
// real runtime call; none of the call sites require a compile-time constant. Falls back to
// `_XOPEN_IOV_MAX` (16), the POSIX-mandated minimum, in the (unexpected) case the sysconf
// query itself fails, rather than treating that as fatal.
#[cfg(target_os = "illumos")]
pub(crate) fn max_iov() -> usize {
    const XOPEN_IOV_MAX_FALLBACK: usize = 16;
    let n = unsafe { libc::sysconf(libc::_SC_IOV_MAX) };
    if n > 0 {
        n as usize
    } else {
        XOPEN_IOV_MAX_FALLBACK
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
        if self.should_drop_cleanup() {
            // SAFETY: should_drop_cleanup guarantees this is an owned live descriptor.
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

#[cfg(test)]
mod conversion_tests {
    use super::{checked_iov_count, syscall_result_len};

    #[test]
    fn syscall_result_len_rejects_negative_results() {
        assert!(matches!(syscall_result_len(0), Ok(0)));
        assert!(syscall_result_len(-1).is_err());
    }

    #[test]
    fn iov_count_rejects_values_outside_c_int_range() {
        assert!(matches!(checked_iov_count(0), Ok(0)));
        assert!(checked_iov_count(usize::MAX).is_err());
    }
}
