use crate::platform::unix::Fd;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::AsRawFd;
use std::sync::Mutex;

impl Fd {
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &InterruptEvent,
    ) -> io::Result<usize> {
        loop {
            self.wait_readable_interruptible(event)?;
            return match self.read(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                rs => rs,
            };
        }
    }
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &InterruptEvent,
    ) -> io::Result<usize> {
        loop {
            self.wait_readable_interruptible(event)?;
            return match self.readv(bufs) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }

                rs => rs,
            };
        }
    }
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        event: &InterruptEvent,
    ) -> io::Result<usize> {
        loop {
            self.wait_writable_interruptible(event)?;
            return match self.write(buf) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                rs => rs,
            };
        }
    }
    pub fn writev_interruptible(
        &self,
        bufs: &[IoSlice<'_>],
        event: &InterruptEvent,
    ) -> io::Result<usize> {
        loop {
            self.wait_writable_interruptible(event)?;
            return match self.writev(bufs) {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    continue;
                }
                rs => rs,
            };
        }
    }
    pub fn wait_readable_interruptible(
        &self,
        interrupted_event: &InterruptEvent,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;

        let event_fd = interrupted_event.as_event_fd();
        let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut errorfds: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_SET(fd, &mut readfds);
            libc::FD_SET(fd, &mut errorfds);
            libc::FD_SET(event_fd, &mut readfds);
        }
        let result = unsafe {
            libc::select(
                fd.max(event_fd) + 1,
                &mut readfds,
                std::ptr::null_mut(),
                &mut errorfds,
                std::ptr::null_mut(),
            )
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            if libc::FD_ISSET(event_fd, &readfds) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "trigger interrupt",
                ));
            }
        }
        Ok(())
    }
    pub fn wait_writable_interruptible(
        &self,
        interrupted_event: &InterruptEvent,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;

        let event_fd = interrupted_event.as_event_fd();
        let mut readfds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut writefds: libc::fd_set = unsafe { std::mem::zeroed() };
        let mut errorfds: libc::fd_set = unsafe { std::mem::zeroed() };
        unsafe {
            libc::FD_SET(fd, &mut writefds);
            libc::FD_SET(fd, &mut errorfds);
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
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        unsafe {
            if libc::FD_ISSET(event_fd, &readfds) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "trigger interrupt",
                ));
            }
        }
        Ok(())
    }
}

pub struct InterruptEvent {
    state: Mutex<bool>,
    read_fd: Fd,
    write_fd: Fd,
}
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];

        unsafe {
            if libc::pipe(fds.as_mut_ptr()) == -1 {
                return Err(io::Error::last_os_error());
            }
            let read_fd = Fd::new_unchecked(fds[0]);
            let write_fd = Fd::new_unchecked(fds[1]);
            read_fd.set_nonblocking(true)?;
            Ok(Self {
                state: Default::default(),
                read_fd,
                write_fd,
            })
        }
    }
    pub fn trigger(&self) -> io::Result<()> {
        let mut guard = self.state.lock().unwrap();
        *guard = true;
        let buf: [u8; 8] = 1u64.to_ne_bytes();
        let res = unsafe {
            libc::write(
                self.write_fd.as_raw_fd(),
                buf.as_ptr() as *const _,
                buf.len(),
            )
        };
        if res == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
    pub fn is_trigger(&self) -> bool {
        *self.state.lock().unwrap()
    }
    pub fn reset(&self) -> io::Result<()> {
        let mut buf = [0; 8];
        let mut guard = self.state.lock().unwrap();
        *guard = false;
        loop {
            unsafe {
                let res = libc::read(
                    self.read_fd.as_raw_fd(),
                    buf.as_mut_ptr() as *mut _,
                    buf.len(),
                );
                if res == -1 {
                    let error = io::Error::last_os_error();
                    return if error.kind() == io::ErrorKind::WouldBlock {
                        Ok(())
                    } else {
                        Err(error)
                    };
                }
            }
        }
    }
    fn as_event_fd(&self) -> libc::c_int {
        self.read_fd.as_raw_fd()
    }
}
