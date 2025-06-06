use crate::platform::unix::Fd;
use std::io;
use std::io::IoSliceMut;
use std::os::fd::AsRawFd;

impl Fd {
    pub fn read_interruptible(&self, buf: &mut [u8], event: &InterruptEvent) -> io::Result<usize> {
        self.wait(event)?;
        self.read(buf)
    }
    pub fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &InterruptEvent,
    ) -> io::Result<usize> {
        self.wait(event)?;
        self.readv(bufs)
    }
    fn wait(&self, interrupted_event: &InterruptEvent) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;

        let event_fd = interrupted_event.as_event_fd();
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
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        unsafe {
            if libc::FD_ISSET(event_fd, &readfds) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancel"));
            }
        }
        Ok(())
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
pub struct InterruptEvent(std::fs::File);
#[cfg(any(target_os = "linux", target_os = "android", target_os = "freebsd"))]
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
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
    pub fn wake(&self) -> io::Result<()> {
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
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
pub struct EventFd(libc::c_int, libc::c_int);
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        let read_fd = fds[0];
        let write_fd = fds[1];
        Ok(Self(read_fd, write_fd))
    }
    pub fn wake(&self) -> io::Result<()> {
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
#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos"))]
impl Drop for InterruptEvent {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::close(self.0);
            let _ = libc::close(self.1);
        }
    }
}
