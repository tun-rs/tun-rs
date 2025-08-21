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
        timeout: Option<std::time::Duration>,
    ) -> io::Result<usize> {
        loop {
            self.wait_readable_interruptible(event, timeout)?;
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
            self.wait_readable_interruptible(event, None)?;
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
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;
        let event_fd = interrupted_event.as_event_fd();

        let mut fds = [
            libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: event_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let result = unsafe {
            libc::poll(
                fds.as_mut_ptr(),
                fds.len() as libc::nfds_t,
                timeout
                    .map(|t| t.as_millis().min(i32::MAX as _) as _)
                    .unwrap_or(-1),
            )
        };

        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if result == 0 {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        if fds[0].revents & libc::POLLIN != 0 {
            return Ok(());
        }

        if fds[1].revents & libc::POLLIN != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "trigger interrupt",
            ));
        }

        Err(io::Error::other("fd error"))
    }
    pub fn wait_writable_interruptible(
        &self,
        interrupted_event: &InterruptEvent,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;
        let event_fd = interrupted_event.as_event_fd();

        let mut fds = [
            libc::pollfd {
                fd,
                events: libc::POLLOUT,
                revents: 0,
            },
            libc::pollfd {
                fd: event_fd,
                events: libc::POLLIN,
                revents: 0,
            },
        ];

        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };

        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        if fds[0].revents & libc::POLLOUT != 0 {
            return Ok(());
        }

        if fds[1].revents & libc::POLLIN != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "trigger interrupt",
            ));
        }

        Err(io::Error::other("fd error"))
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
