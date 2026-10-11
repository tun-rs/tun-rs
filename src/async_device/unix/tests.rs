//! Readiness regression tests for the unix `AsyncDevice` (tokio backend).
//!
//! The tests run on descriptors that are not TUN devices (socketpair / pipe)
//! so they need no privileges; `AsyncDevice` only relies on `read(2)`,
//! `write(2)` and epoll readiness here.

use super::AsyncDevice;
use crate::platform::DeviceImpl;
use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn socketpair() -> (RawFd, RawFd) {
    let mut fds = [0; 2];
    let rc = unsafe {
        libc::socketpair(
            libc::AF_UNIX,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC,
            0,
            fds.as_mut_ptr(),
        )
    };
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
    (fds[0], fds[1])
}

fn async_dev(fd: RawFd) -> AsyncDevice {
    AsyncDevice::new_dev(DeviceImpl::from_fd_for_test(fd)).unwrap()
}

/// Issue #89: `readable()` + `try_recv()` under a multi-thread runtime with
/// real traffic must not spin. Counts loop iterations per received packet.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn readable_try_recv_does_not_spin() {
    const PACKETS: usize = 200;
    let (a, b) = socketpair();
    let dev = Arc::new(async_dev(a));
    let iterations = Arc::new(AtomicUsize::new(0));

    let reader = {
        let dev = dev.clone();
        let iterations = iterations.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 64];
            let mut received = 0;
            while received < PACKETS {
                dev.readable().await.unwrap();
                // Bail out of a spin instead of hanging the runtime shutdown.
                if iterations.fetch_add(1, Ordering::Relaxed) > PACKETS * 50 {
                    return;
                }
                match dev.try_recv(&mut buf) {
                    Ok(_) => received += 1,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => panic!("{e}"),
                }
            }
        })
    };
    let writer = std::thread::spawn(move || {
        for i in 0..PACKETS {
            let n = unsafe { libc::send(b, [i as u8; 8].as_ptr().cast(), 8, 0) };
            assert_eq!(n, 8);
            std::thread::sleep(Duration::from_millis(1));
        }
        b
    });
    tokio::time::timeout(Duration::from_secs(20), reader)
        .await
        .expect("reader finished")
        .unwrap();
    let b = writer.join().unwrap();
    unsafe { libc::close(b) };
    let it = iterations.load(Ordering::Relaxed);
    eprintln!("readable_try_recv: {it} iterations for {PACKETS} packets");
    // Healthy behaviour is ~2 iterations per packet (the packet plus one
    // WouldBlock that clears readiness); a spin is orders of magnitude more.
    assert!(it <= PACKETS * 3, "busy loop: {it} iterations");
}

/// A pipe whose read end is closed after registration: the write end then
/// reports `EPOLLERR` (and `EPOLLOUT`) but never `EPOLLIN`, the same
/// error-only shape a detached TUN file gives (`tun_chr_poll`).
/// Returns the registered write-end device and a closer for the read end.
fn error_only_device() -> (Arc<AsyncDevice>, impl FnOnce()) {
    let mut fds = [0; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
    let dev = Arc::new(async_dev(fds[1]));
    let read_end = fds[0];
    (dev, move || {
        unsafe { libc::close(read_end) };
    })
}

const ERROR_WAIT: Duration = Duration::from_secs(5);

/// Issue #178: `recv` must return an error instead of hanging when the
/// device only reports an error condition.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recv_returns_error_on_error_only_readiness() {
    let (dev, close_peer) = error_only_device();
    let task = {
        let dev = dev.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 16];
            dev.recv(&mut buf).await
        })
    };
    // Let the task park on the (still healthy) device first.
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !task.is_finished(),
        "recv must wait while the device is idle"
    );
    close_peer();
    let res = tokio::time::timeout(ERROR_WAIT, task)
        .await
        .expect("recv hung after the device reported only an error")
        .unwrap();
    assert!(res.is_err(), "expected the read error, got {res:?}");
}

/// `readable()` + `try_recv()` surfaces the error (not `WouldBlock`) and does
/// so without spinning.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn readable_try_recv_surfaces_error_only_readiness() {
    let (dev, close_peer) = error_only_device();
    let iterations = Arc::new(AtomicUsize::new(0));
    let task = {
        let dev = dev.clone();
        let iterations = iterations.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 16];
            loop {
                dev.readable().await?;
                if iterations.fetch_add(1, Ordering::Relaxed) > 100 {
                    return Err(io::Error::other("spin"));
                }
                match dev.try_recv(&mut buf) {
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    other => return other,
                }
            }
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    close_peer();
    let res = tokio::time::timeout(ERROR_WAIT, task)
        .await
        .expect("readable() hung after the device reported only an error")
        .unwrap();
    let it = iterations.load(Ordering::Relaxed);
    eprintln!("error_only: {it} iterations");
    let err = res.expect_err("try_recv must surface the error");
    assert_ne!(err.kind(), io::ErrorKind::WouldBlock);
    assert_ne!(err.to_string(), "spin");
    assert!(it <= 3, "unexpected extra wakeups: {it}");
}

/// A healthy idle device must stay pending on both entry points.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_device_stays_pending() {
    let (a, b) = socketpair();
    let dev = async_dev(a);
    let mut buf = [0u8; 16];
    assert!(
        tokio::time::timeout(Duration::from_millis(100), dev.recv(&mut buf))
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), dev.readable())
            .await
            .is_err()
    );
    assert_eq!(
        dev.try_recv(&mut buf).unwrap_err().kind(),
        io::ErrorKind::WouldBlock
    );
    unsafe { libc::close(b) };
}

/// Privileged end-to-end check of issue #178 with a real TUN device.
///
/// Run: `sudo -E env "PATH=$PATH" cargo test --lib --features async_tokio \
///   -- --ignored recv_returns_error_after_link_deleted`
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs CAP_NET_ADMIN and the `ip` tool"]
async fn recv_returns_error_after_link_deleted() {
    let name = "tunrs_dead0";
    let dev = Arc::new(
        crate::DeviceBuilder::new()
            .name(name)
            .build_async()
            .expect("create TUN device (needs CAP_NET_ADMIN)"),
    );
    let task = {
        let dev = dev.clone();
        tokio::spawn(async move {
            let mut buf = [0u8; 2048];
            dev.recv(&mut buf).await
        })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let status = std::process::Command::new("ip")
        .args(["link", "del", name])
        .status()
        .expect("run ip");
    assert!(status.success(), "ip link del failed");
    let res = tokio::time::timeout(ERROR_WAIT, task)
        .await
        .expect("recv hung after `ip link del`")
        .unwrap();
    assert!(res.is_err(), "expected an error, got {res:?}");
}
