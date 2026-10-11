//! Descriptor and waker helpers shared by the backend readiness tests.
//!
//! The tests run on descriptors that are not TUN devices so they need no
//! privileges. `AsyncDevice` only relies on `read(2)`, `write(2)` and the
//! readiness the kernel reports for them.

use futures::task::AtomicWaker;
use std::future::{poll_fn, Future};
use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::Duration;

/// Upper bound for operations that must complete; only a hang guard, never
/// used for synchronisation.
pub(super) const HANG_GUARD: Duration = Duration::from_secs(10);

/// Time given to a device that must stay idle before asserting on it.
pub(super) const IDLE_WINDOW: Duration = Duration::from_millis(150);

fn check(rc: libc::c_int) {
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
}

pub(super) fn socketpair(kind: libc::c_int) -> (RawFd, RawFd) {
    let mut fds = [0; 2];
    check(unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            kind | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    });
    (fds[0], fds[1])
}

/// A non-blocking pipe: `(read end, write end)`.
pub(super) fn pipe() -> (RawFd, RawFd) {
    let mut fds = [0; 2];
    check(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) });
    (fds[0], fds[1])
}

/// A non-blocking UDP socket connected to a closed local port. Idle (reads give
/// `WouldBlock`) until [`provoke_socket_error`] makes the kernel record one
/// `ECONNREFUSED`, which then shows up as `EPOLLERR` without `EPOLLIN`; the
/// next read returns the error once.
pub(super) fn refused_udp_socket() -> RawFd {
    use std::net::UdpSocket;
    use std::os::fd::IntoRawFd;
    let closed_port = UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
    socket.connect(("127.0.0.1", closed_port)).unwrap();
    socket.set_nonblocking(true).unwrap();
    socket.into_raw_fd()
}

pub(super) fn provoke_socket_error(fd: RawFd) {
    send_all(fd, b"x");
}

pub(super) fn close(fd: RawFd) {
    check(unsafe { libc::close(fd) });
}

pub(super) fn send_all(fd: RawFd, payload: &[u8]) {
    let n = unsafe { libc::send(fd, payload.as_ptr().cast(), payload.len(), 0) };
    assert_eq!(n, payload.len() as isize, "{}", io::Error::last_os_error());
}

/// Reads one packet straight from the descriptor, bypassing the async layer.
pub(super) fn raw_recv(fd: RawFd, buf: &mut [u8]) -> isize {
    unsafe { libc::recv(fd, buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) }
}

/// Writes to a non-blocking pipe until it is full.
pub(super) fn fill_pipe(write_end: RawFd) {
    let chunk = [0u8; 4096];
    loop {
        let n = unsafe { libc::write(write_end, chunk.as_ptr().cast(), chunk.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
            return;
        }
    }
}

/// A waker that counts wakeups and forwards them to the task awaiting
/// [`WakeCounter::wait_wakes`].
#[derive(Default)]
pub(super) struct WakeCounter {
    count: AtomicUsize,
    waiter: AtomicWaker,
}

impl WakeCounter {
    pub(super) fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }

    pub(super) async fn wait_wakes(&self, at_least: usize) {
        poll_fn(|cx| {
            self.waiter.register(cx.waker());
            if self.count() >= at_least {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await
    }
}

impl Wake for WakeCounter {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.count.fetch_add(1, Ordering::SeqCst);
        self.waiter.wake();
    }
}

pub(super) fn counting_waker() -> (Waker, Arc<WakeCounter>) {
    let counter = Arc::new(WakeCounter::default());
    (Waker::from(counter.clone()), counter)
}

/// Drives one read entry point through its whole wakeup contract.
///
/// `poll_once` is polled with a counting waker. It must be pending, stay
/// silent for `idle`, be woken a bounded number of times once `trigger` fires
/// and then complete with an error exactly when `expect_error` is set. A
/// missed wakeup hangs (callers wrap this in a hang guard); a busy loop shows
/// up as wakeups while idle or as an unbounded wake count.
pub(super) async fn assert_idle_then_wakes(
    what: &str,
    mut poll_once: impl FnMut(&mut Context<'_>) -> Poll<io::Result<()>>,
    idle: impl Future<Output = ()>,
    trigger: impl FnOnce(),
    expect_error: bool,
) {
    let (waker, wakes) = counting_waker();
    let mut cx = Context::from_waker(&waker);
    assert!(poll_once(&mut cx).is_pending(), "{what}: ready while idle");
    idle.await;
    assert_eq!(wakes.count(), 0, "{what}: woken while idle");
    trigger();
    wakes.wait_wakes(1).await;
    match poll_once(&mut cx) {
        Poll::Ready(result) => assert_eq!(
            result.is_err(),
            expect_error,
            "{what}: unexpected result {result:?}"
        ),
        Poll::Pending => panic!("{what}: woken but still pending"),
    }
    assert!(
        wakes.count() <= 2,
        "{what}: {} wakeups for a single event",
        wakes.count()
    );
}
