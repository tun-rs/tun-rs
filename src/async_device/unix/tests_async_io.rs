//! Error-only readiness tests for the async-io backend (see `tests.rs` for the
//! tokio counterparts). `async-io` reports `EPOLLERR` as readable, so a read on
//! a device that only reports an error must complete with that error.

use super::AsyncDevice;
use crate::platform::DeviceImpl;
use futures::future::{select, Either};
use std::io;
use std::time::Duration;

fn error_only_device() -> (AsyncDevice, impl FnOnce()) {
    let mut fds = [0; 2];
    let rc = unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC | libc::O_NONBLOCK) };
    assert_eq!(rc, 0, "{}", io::Error::last_os_error());
    let dev = AsyncDevice::new_dev(DeviceImpl::from_fd_for_test(fds[1])).unwrap();
    let read_end = fds[0];
    (dev, move || {
        unsafe { libc::close(read_end) };
    })
}

async fn within<F: std::future::Future>(d: Duration, f: F) -> Option<F::Output> {
    let f = std::pin::pin!(f);
    match select(f, std::pin::pin!(async_io::Timer::after(d))).await {
        Either::Left((v, _)) => Some(v),
        Either::Right(_) => None,
    }
}

#[test]
fn recv_returns_error_on_error_only_readiness() {
    let (dev, close_peer) = error_only_device();
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        close_peer();
    });
    let res = async_io::block_on(within(Duration::from_secs(5), async {
        let mut buf = [0u8; 16];
        dev.recv(&mut buf).await
    }))
    .expect("recv hung after the device reported only an error");
    closer.join().unwrap();
    assert!(res.is_err(), "expected the read error, got {res:?}");
}

#[test]
fn readable_try_recv_surfaces_error_only_readiness() {
    let (dev, close_peer) = error_only_device();
    let closer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        close_peer();
    });
    let mut iterations = 0;
    let res = async_io::block_on(within(Duration::from_secs(5), async {
        let mut buf = [0u8; 16];
        loop {
            dev.readable().await?;
            iterations += 1;
            assert!(iterations <= 3, "spin");
            match dev.try_recv(&mut buf) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                other => return other,
            }
        }
    }))
    .expect("readable() hung after the device reported only an error");
    closer.join().unwrap();
    assert!(res.is_err());
    assert_ne!(res.unwrap_err().kind(), io::ErrorKind::WouldBlock);
}
