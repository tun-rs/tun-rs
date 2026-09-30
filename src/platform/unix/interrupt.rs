/*!
# Interruptible I/O Module

This module provides support for interruptible I/O operations on Unix platforms.

## Overview

Interruptible I/O allows you to cancel blocking read/write operations on a TUN/TAP device
from another thread. This is useful for graceful shutdown, implementing timeouts, or
responding to signals.

## Availability

This module is only available when the `interruptible` feature flag is enabled:

```toml
[dependencies]
tun-rs = { version = "2", features = ["interruptible"] }
```

## How It Works

The implementation uses a pipe-based signaling mechanism:
- An `InterruptEvent` creates a pipe internally
- When triggered, it writes to the pipe
- I/O operations use `poll()` to wait on both the device fd and the pipe
- If the pipe becomes readable, the I/O operation returns with `ErrorKind::Interrupted`

## Usage

```no_run
# #[cfg(all(unix, feature = "interruptible"))]
# {
use tun_rs::{DeviceBuilder, InterruptEvent};
use std::sync::Arc;
use std::thread;

let dev = DeviceBuilder::new()
    .ipv4("10.0.0.1", 24, None)
    .build_sync()?;

let event = Arc::new(InterruptEvent::new()?);
let event_clone = event.clone();

// Spawn a thread that will read from the device
let handle = thread::spawn(move || {
    let mut buf = vec![0u8; 1500];
    match dev.recv_intr(&mut buf, &event_clone) {
        Ok(n) => println!("Received {} bytes", n),
        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
            println!("Read was interrupted");
        }
        Err(e) => eprintln!("Error: {}", e),
    }
});

// From the main thread, trigger the interrupt
thread::sleep(std::time::Duration::from_secs(1));
event.trigger()?;

handle.join().unwrap();
# }
# Ok::<(), std::io::Error>(())
```

## Performance Considerations

- Interruptible I/O has slightly more overhead than regular I/O due to the additional poll() fd
- The pipe is created once and reused across all operations
- Non-blocking mode is set on the pipe fds to prevent deadlocks

## Platform Support

- **Linux**: Uses `poll()` with two file descriptors
- **macOS**: Uses `poll()` like other Unix targets
- **FreeBSD/OpenBSD/NetBSD**: Uses `poll()` like Linux
- **Windows**: Not supported (would need IOCP or overlapped I/O)

## Thread Safety

`InterruptEvent` is thread-safe and can be shared across threads using `Arc`.
*/

use crate::platform::unix::Fd;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::AsRawFd;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn poll_timeout_ms(timeout: Option<Duration>) -> libc::c_int {
    let Some(timeout) = timeout else {
        return -1;
    };
    let whole_millis = timeout.as_millis();
    let has_fractional_millisecond = timeout.subsec_nanos() % 1_000_000 != 0;
    let rounded_up = whole_millis.saturating_add(u128::from(has_fractional_millisecond));
    libc::c_int::try_from(rounded_up).unwrap_or(libc::c_int::MAX)
}

fn timeout_expired(started: Instant, timeout: Option<Duration>) -> bool {
    match timeout {
        Some(limit) => started.elapsed() >= limit,
        None => false,
    }
}

impl Fd {
    pub(crate) fn read_interruptible(
        &self,
        buf: &mut [u8],
        event: &InterruptEvent,
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        let started = Instant::now();
        loop {
            self.wait_readable_interruptible(
                event,
                timeout.map(|limit| limit.saturating_sub(started.elapsed())),
            )?;
            match self.read(buf) {
                Err(ref error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if timeout_expired(started, timeout) {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                }
                result => return result,
            }
        }
    }
    pub(crate) fn readv_interruptible(
        &self,
        bufs: &mut [IoSliceMut<'_>],
        event: &InterruptEvent,
        timeout: Option<Duration>,
    ) -> io::Result<usize> {
        let started = Instant::now();
        loop {
            self.wait_readable_interruptible(
                event,
                timeout.map(|limit| limit.saturating_sub(started.elapsed())),
            )?;
            match self.readv(bufs) {
                Err(ref error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if timeout_expired(started, timeout) {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                }
                result => return result,
            }
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
        timeout: Option<Duration>,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd() as libc::c_int;
        let event_fd = interrupted_event.as_event_fd();
        let started = Instant::now();

        loop {
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

            let poll_timeout =
                poll_timeout_ms(timeout.map(|limit| limit.saturating_sub(started.elapsed())));
            let result =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, poll_timeout) };

            if result == -1 {
                return Err(io::Error::last_os_error());
            }
            if result == 0 {
                if timeout_expired(started, timeout) {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                continue;
            }

            if fds[1].revents & libc::POLLIN != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "trigger interrupt",
                ));
            }
            if fds[0].revents & libc::POLLIN != 0 {
                return Ok(());
            }
            return Err(io::Error::other("fd error"));
        }
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
        if fds[1].revents & libc::POLLIN != 0 {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "trigger interrupt",
            ));
        }
        if fds[0].revents & libc::POLLOUT != 0 {
            return Ok(());
        }

        Err(io::Error::other("fd error"))
    }
}

#[cfg(target_os = "macos")]
impl Fd {
    pub fn wait_writable(
        &self,
        interrupt_event: Option<&InterruptEvent>,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        self.wait(libc::POLLOUT, interrupt_event, timeout)
    }
    pub fn wait_readable(
        &self,
        interrupt_event: Option<&InterruptEvent>,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        self.wait(libc::POLLIN, interrupt_event, timeout)
    }
    fn wait(
        &self,
        device_events: libc::c_short,
        interrupt_event: Option<&InterruptEvent>,
        timeout: Option<Duration>,
    ) -> io::Result<()> {
        let fd = self.as_raw_fd();
        let started = Instant::now();

        loop {
            let mut fds = Vec::with_capacity(if interrupt_event.is_some() { 2 } else { 1 });
            fds.push(libc::pollfd {
                fd,
                events: device_events,
                revents: 0,
            });
            if let Some(interrupt_event) = interrupt_event {
                fds.push(libc::pollfd {
                    fd: interrupt_event.as_event_fd(),
                    events: libc::POLLIN,
                    revents: 0,
                });
            }

            let timeout_ms =
                poll_timeout_ms(timeout.map(|limit| limit.saturating_sub(started.elapsed())));
            let result =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
            if result < 0 {
                return Err(io::Error::last_os_error());
            }
            if result == 0 {
                if timeout_expired(started, timeout) {
                    return Err(io::Error::from(io::ErrorKind::TimedOut));
                }
                continue;
            }
            if interrupt_event.is_some() && fds[1].revents & libc::POLLIN != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "trigger interrupt",
                ));
            }
            if fds[0].revents & device_events != 0 {
                return Ok(());
            }
            return Err(io::Error::other("fd error"));
        }
    }
}
/// Event object for interrupting blocking I/O operations.
///
/// `InterruptEvent` provides a mechanism to cancel blocking read/write operations
/// from another thread. It uses a pipe-based signaling mechanism internally.
///
/// # Thread Safety
///
/// This type is thread-safe and can be shared across threads using `Arc<InterruptEvent>`.
///
/// # Examples
///
/// Basic usage with interruptible read:
///
/// ```no_run
/// # #[cfg(all(unix, feature = "interruptible"))]
/// # {
/// use std::sync::Arc;
/// use std::thread;
/// use std::time::Duration;
/// use tun_rs::{DeviceBuilder, InterruptEvent};
///
/// let device = DeviceBuilder::new()
///     .ipv4("10.0.0.1", 24, None)
///     .build_sync()?;
///
/// let event = Arc::new(InterruptEvent::new()?);
/// let event_clone = event.clone();
///
/// let reader = thread::spawn(move || {
///     let mut buf = vec![0u8; 1500];
///     device.recv_intr(&mut buf, &event_clone)
/// });
///
/// // Give the reader time to start blocking
/// thread::sleep(Duration::from_millis(100));
///
/// // Trigger the interrupt
/// event.trigger()?;
///
/// match reader.join().unwrap() {
///     Ok(n) => println!("Read {} bytes", n),
///     Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
///         println!("Successfully interrupted!");
///     }
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// Using with a timeout:
///
/// ```no_run
/// # #[cfg(all(unix, feature = "interruptible"))]
/// # {
/// use std::time::Duration;
/// use tun_rs::{DeviceBuilder, InterruptEvent};
///
/// let device = DeviceBuilder::new()
///     .ipv4("10.0.0.1", 24, None)
///     .build_sync()?;
///
/// let event = InterruptEvent::new()?;
/// let mut buf = vec![0u8; 1500];
///
/// // Will return an error if no data arrives within 5 seconds
/// match device.recv_intr_timeout(&mut buf, &event, Some(Duration::from_secs(5))) {
///     Ok(n) => println!("Read {} bytes", n),
///     Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
///         println!("Timed out waiting for data");
///     }
///     Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {
///         println!("Operation was interrupted");
///     }
///     Err(e) => eprintln!("Error: {}", e),
/// }
/// # }
/// # Ok::<(), std::io::Error>(())
/// ```
///
/// # Implementation Details
///
/// - Uses a Unix pipe for signaling
/// - Both read and write ends are set to non-blocking mode
/// - State is protected by a mutex to prevent race conditions
/// - Once triggered, the event remains triggered until reset
pub struct InterruptEvent {
    state: Mutex<i32>,
    read_fd: Fd,
    write_fd: Fd,
}
impl InterruptEvent {
    /// Creates a new `InterruptEvent`.
    ///
    /// This allocates a Unix pipe for signaling and sets both ends to non-blocking mode.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The pipe creation fails (e.g., out of file descriptors)
    /// - Setting non-blocking mode fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use tun_rs::InterruptEvent;
    ///
    /// let event = InterruptEvent::new()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn new() -> io::Result<Self> {
        let mut fds: [libc::c_int; 2] = [0; 2];

        unsafe {
            if libc::pipe(fds.as_mut_ptr()) == -1 {
                return Err(io::Error::last_os_error());
            }
            let read_fd = Fd::new_unchecked(fds[0]);
            let write_fd = Fd::new_unchecked(fds[1]);
            write_fd.set_nonblocking(true)?;
            read_fd.set_nonblocking(true)?;
            Ok(Self {
                state: Mutex::new(0),
                read_fd,
                write_fd,
            })
        }
    }

    /// Triggers the interrupt event with value 1.
    ///
    /// This will cause any blocking I/O operations waiting on this event to return
    /// with `ErrorKind::Interrupted`.
    ///
    /// Calling `trigger()` multiple times before `reset()` has no additional effect -
    /// the event remains triggered.
    ///
    /// # Errors
    ///
    /// Returns an error if writing to the internal pipe fails, though this is rare
    /// in practice (pipe write errors are usually `WouldBlock`, which is handled).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use std::sync::Arc;
    /// use std::thread;
    /// use tun_rs::{DeviceBuilder, InterruptEvent};
    ///
    /// let event = Arc::new(InterruptEvent::new()?);
    /// let event_clone = event.clone();
    ///
    /// thread::spawn(move || {
    ///     // ... blocking I/O with event_clone ...
    /// });
    ///
    /// // Interrupt the operation
    /// event.trigger()?;
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn trigger(&self) -> io::Result<()> {
        self.trigger_value(1)
    }

    /// Triggers the interrupt event with a specific value.
    ///
    /// Similar to [`trigger()`](Self::trigger), but allows specifying a custom value
    /// that can be retrieved with [`value()`](Self::value).
    ///
    /// # Arguments
    ///
    /// * `val` - The value to store (must be non-zero)
    ///
    /// # Errors
    ///
    /// - Returns `ErrorKind::InvalidInput` if `val` is 0
    /// - Returns an error if writing to the pipe fails
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use tun_rs::InterruptEvent;
    ///
    /// let event = InterruptEvent::new()?;
    ///
    /// // Trigger with a custom value (e.g., signal number)
    /// event.trigger_value(15)?; // SIGTERM
    ///
    /// // Later, check what value was used
    /// assert_eq!(event.value(), 15);
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn trigger_value(&self, val: i32) -> io::Result<()> {
        if val == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "value cannot be 0",
            ));
        }
        let mut guard = self.state.lock().unwrap();
        if *guard != 0 {
            return Ok(());
        }
        *guard = val;
        let buf: [u8; 8] = 1u64.to_ne_bytes();
        let res = unsafe {
            libc::write(
                self.write_fd.as_raw_fd(),
                buf.as_ptr() as *const _,
                buf.len(),
            )
        };
        if res == -1 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::WouldBlock {
                return Ok(());
            }
            Err(e)
        } else {
            Ok(())
        }
    }
    /// Checks if the event has been triggered.
    ///
    /// Returns `true` if the event is currently in the triggered state (value is non-zero).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use tun_rs::InterruptEvent;
    ///
    /// let event = InterruptEvent::new()?;
    /// assert!(!event.is_trigger());
    ///
    /// event.trigger()?;
    /// assert!(event.is_trigger());
    ///
    /// event.reset()?;
    /// assert!(!event.is_trigger());
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn is_trigger(&self) -> bool {
        *self.state.lock().unwrap() != 0
    }

    /// Returns the current trigger value.
    ///
    /// Returns 0 if the event is not triggered, or the value passed to
    /// [`trigger_value()`](Self::trigger_value) if triggered.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use tun_rs::InterruptEvent;
    ///
    /// let event = InterruptEvent::new()?;
    /// assert_eq!(event.value(), 0);
    ///
    /// event.trigger_value(42)?;
    /// assert_eq!(event.value(), 42);
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn value(&self) -> i32 {
        *self.state.lock().unwrap()
    }

    /// Resets the event to the non-triggered state.
    ///
    /// This drains any pending data from the internal pipe and sets the state back to 0.
    /// After calling `reset()`, the event can be triggered again.
    ///
    /// # Errors
    ///
    /// Returns an error if reading from the pipe fails (other than `WouldBlock`).
    ///
    /// # Example
    ///
    /// ```no_run
    /// # #[cfg(all(unix, feature = "interruptible"))]
    /// # {
    /// use tun_rs::InterruptEvent;
    ///
    /// let event = InterruptEvent::new()?;
    ///
    /// event.trigger()?;
    /// assert!(event.is_trigger());
    ///
    /// event.reset()?;
    /// assert!(!event.is_trigger());
    ///
    /// // Can trigger again after reset
    /// event.trigger()?;
    /// assert!(event.is_trigger());
    /// # }
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn reset(&self) -> io::Result<()> {
        let mut buf = [0; 8];
        let mut guard = self.state.lock().unwrap();
        *guard = 0;
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

#[cfg(test)]
mod timeout_tests {
    use super::{poll_timeout_ms, Fd, InterruptEvent};
    use std::io;
    use std::time::Duration;

    fn pipe_pair() -> io::Result<(Fd, Fd)> {
        let mut raw = [-1; 2];
        if unsafe { libc::pipe(raw.as_mut_ptr()) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok((Fd::new(raw[0])?, Fd::new(raw[1])?))
    }

    #[test]
    fn poll_timeout_rounds_positive_sub_millisecond_durations_up() {
        assert_eq!(poll_timeout_ms(None), -1);
        assert_eq!(poll_timeout_ms(Some(Duration::ZERO)), 0);
        assert_eq!(poll_timeout_ms(Some(Duration::from_nanos(1))), 1);
        assert_eq!(poll_timeout_ms(Some(Duration::from_micros(999))), 1);
        assert_eq!(poll_timeout_ms(Some(Duration::from_millis(1))), 1);
        assert_eq!(
            poll_timeout_ms(Some(Duration::from_millis(1) + Duration::from_nanos(1))),
            2
        );
        assert_eq!(poll_timeout_ms(Some(Duration::MAX)), libc::c_int::MAX);
    }

    #[test]
    fn interrupt_wins_when_device_is_also_ready() -> io::Result<()> {
        let (reader, writer) = pipe_pair()?;
        let event = InterruptEvent::new()?;
        assert_eq!(writer.write(b"x")?, 1);
        event.trigger()?;

        let read_error = reader
            .wait_readable_interruptible(&event, Some(Duration::from_secs(1)))
            .expect_err("readable device won over cancellation");
        assert_eq!(read_error.kind(), io::ErrorKind::Interrupted);

        let write_error = writer
            .wait_writable_interruptible(&event)
            .expect_err("writable device won over cancellation");
        assert_eq!(write_error.kind(), io::ErrorKind::Interrupted);
        Ok(())
    }

    #[test]
    fn wait_readable_distinguishes_timeout_interrupt_and_readiness() -> io::Result<()> {
        let (reader, writer) = pipe_pair()?;
        let event = InterruptEvent::new()?;

        let timeout = reader
            .wait_readable_interruptible(&event, Some(Duration::from_millis(1)))
            .expect_err("empty pipe unexpectedly became readable");
        assert_eq!(timeout.kind(), io::ErrorKind::TimedOut);

        event.trigger()?;
        let interrupted = reader
            .wait_readable_interruptible(&event, Some(Duration::from_secs(1)))
            .expect_err("triggered event did not interrupt the wait");
        assert_eq!(interrupted.kind(), io::ErrorKind::Interrupted);
        event.reset()?;

        assert_eq!(writer.write(b"x")?, 1);
        reader.wait_readable_interruptible(&event, Some(Duration::from_secs(1)))?;
        Ok(())
    }

    #[test]
    fn read_interruptible_keeps_one_overall_timeout() -> io::Result<()> {
        let (reader, writer) = pipe_pair()?;
        let event = InterruptEvent::new()?;
        assert_eq!(writer.write(b"abc")?, 3);

        let mut buf = [0u8; 8];
        let read = reader.read_interruptible(&mut buf, &event, Some(Duration::from_secs(1)))?;
        assert_eq!(read, 3);
        assert_eq!(&buf[..read], b"abc");

        let timeout = reader
            .read_interruptible(&mut buf, &event, Some(Duration::from_millis(1)))
            .expect_err("empty pipe read did not time out");
        assert_eq!(timeout.kind(), io::ErrorKind::TimedOut);
        Ok(())
    }
}
