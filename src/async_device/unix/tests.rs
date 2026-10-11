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
