//! Readiness tests for the unix `AsyncDevice` (tokio backend).
//!
//! Two failure modes are pinned here.
//!
//! * A device that only reports an error condition (`EPOLLERR` without
//!   `EPOLLIN`, which is what a deleted Linux TUN/TAP interface looks like) must
//!   wake every kind of read waiter and surface the error, repeatedly.
//! * Waiting for that error must not turn into a busy loop: combined interests
//!   given to `AsyncFd::try_io` report an empty readiness, never clear it and
//!   make `readable()` return forever.
//!
//! The error-only shape is reproduced without privileges with a pipe whose read
//! end is closed after registration: the write end then reports `EPOLLERR`
//! (plus `EPOLLOUT` while it has room). The `privileged_*` tests at the bottom
//! use real TUN/TAP devices and `ip link del`.

use super::test_util::*;
use super::AsyncDevice;
use crate::platform::DeviceImpl;
use bytes::buf::UninitSlice;
use futures::future::join_all;
use futures::poll;
use std::future::{poll_fn, Future};
use std::io::{self, IoSliceMut};
use std::os::fd::{AsRawFd, RawFd};
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

fn async_dev(fd: RawFd) -> AsyncDevice {
    AsyncDevice::new_dev(DeviceImpl::from_fd_for_test(fd)).unwrap()
}

/// Runs `scenario` on a multi-thread and on a current-thread runtime.
macro_rules! on_both_runtimes {
    ($scenario:ident) => {
        mod $scenario {
            #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
            async fn multi_thread() {
                super::$scenario().await
            }
            #[tokio::test(flavor = "current_thread")]
            async fn current_thread() {
                super::$scenario().await
            }
        }
    };
}

async fn within<F: Future>(what: &str, fut: F) -> F::Output {
    tokio::time::timeout(HANG_GUARD, fut)
        .await
        .unwrap_or_else(|_| panic!("{what} hung"))
}

/// Idle, then healthy: a datagram device and its peer descriptor.
fn data_device() -> (Arc<AsyncDevice>, RawFd) {
    let (a, b) = socketpair(libc::SOCK_DGRAM);
    (Arc::new(async_dev(a)), b)
}

/// A device that is idle until the returned descriptor is closed, and then
/// reports only an error. Returns the device and the read end to close.
fn error_only_device() -> (Arc<AsyncDevice>, RawFd) {
    let (read_end, write_end) = pipe();
    (Arc::new(async_dev(write_end)), read_end)
}

fn assert_read_error(err: io::Error) {
    assert_ne!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
}

// ---------------------------------------------------------------------------
// Error-only readiness: every read entry point wakes and keeps failing.
// ---------------------------------------------------------------------------

async fn recv_wakes_on_error_only_readiness() {
    let (dev, read_end) = error_only_device();
    let mut buf = [0u8; 16];
    {
        let mut recv = pin!(dev.recv(&mut buf));
        assert!(poll!(recv.as_mut()).is_pending());
        close(read_end);
        assert_read_error(within("recv", recv).await.unwrap_err());
    }
    for _ in 0..5 {
        assert_read_error(
            within("repeated recv", dev.recv(&mut buf))
                .await
                .unwrap_err(),
        );
    }
}
on_both_runtimes!(recv_wakes_on_error_only_readiness);

async fn recv_reports_error_present_before_registration() {
    let (read_end, write_end) = pipe();
    close(read_end);
    let dev = async_dev(write_end);
    let mut buf = [0u8; 16];
    assert_read_error(within("recv", dev.recv(&mut buf)).await.unwrap_err());
}
on_both_runtimes!(recv_reports_error_present_before_registration);

async fn readable_then_try_recv_surfaces_error_only_readiness() {
    let (dev, read_end) = error_only_device();
    let mut buf = [0u8; 16];
    {
        let mut readable = pin!(dev.readable());
        assert!(poll!(readable.as_mut()).is_pending());
        close(read_end);
        within("readable", readable).await.unwrap();
    }
    // The first attempt must already return the real error: reporting
    // `WouldBlock` here without clearing the readiness is the busy loop.
    for _ in 0..5 {
        assert_read_error(dev.try_recv(&mut buf).unwrap_err());
        within("readable", dev.readable()).await.unwrap();
    }
    let mut slice = [IoSliceMut::new(&mut buf)];
    assert_read_error(dev.try_recv_vectored(&mut slice).unwrap_err());
}
on_both_runtimes!(readable_then_try_recv_surfaces_error_only_readiness);

async fn poll_recv_wakes_on_error_only_readiness() {
    let (dev, read_end) = error_only_device();
    let mut buf = [0u8; 16];
    {
        let mut recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
        assert!(poll!(recv.as_mut()).is_pending());
        close(read_end);
        assert_read_error(within("poll_recv", recv).await.unwrap_err());
    }
    for _ in 0..5 {
        let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
        let Poll::Ready(Err(err)) = polled else {
            panic!("repeated poll_recv: {polled:?}");
        };
        assert_read_error(err);
    }
}
on_both_runtimes!(poll_recv_wakes_on_error_only_readiness);

async fn poll_recv_uninit_wakes_on_error_only_readiness() {
    let (dev, read_end) = error_only_device();
    let mut buf = [0u8; 16];
    let mut recv = pin!(poll_fn(
        |cx| dev.poll_recv_uninit(cx, UninitSlice::new(&mut buf))
    ));
    assert!(poll!(recv.as_mut()).is_pending());
    close(read_end);
    assert_read_error(within("poll_recv_uninit", recv).await.unwrap_err());
}
on_both_runtimes!(poll_recv_uninit_wakes_on_error_only_readiness);

async fn poll_readable_wakes_on_error_only_readiness() {
    let (dev, read_end) = error_only_device();
    let mut readable = pin!(poll_fn(|cx| dev.poll_readable(cx)));
    assert!(poll!(readable.as_mut()).is_pending());
    close(read_end);
    within("poll_readable", readable).await.unwrap();
    let mut buf = [0u8; 16];
    let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
    assert!(matches!(polled, Poll::Ready(Err(_))), "{polled:?}");
}
on_both_runtimes!(poll_readable_wakes_on_error_only_readiness);

/// A socket error recorded by the kernel (`EPOLLERR` without `EPOLLIN`) is the
/// same error-only shape from a source that is not a pipe. The read consumes
/// the error, so each entry point gets its own device.
async fn pending_readers_wake_on_socket_error() {
    let refused = |err: io::Error| assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);

    let fd = refused_udp_socket();
    let dev = async_dev(fd);
    let mut buf = [0u8; 16];
    {
        let mut recv = pin!(dev.recv(&mut buf));
        assert!(poll!(recv.as_mut()).is_pending());
        provoke_socket_error(fd);
        refused(within("recv", recv).await.unwrap_err());
    }

    let fd = refused_udp_socket();
    let dev = async_dev(fd);
    let mut readable = pin!(dev.readable());
    assert!(poll!(readable.as_mut()).is_pending());
    provoke_socket_error(fd);
    within("readable", readable).await.unwrap();
    refused(dev.try_recv(&mut buf).unwrap_err());

    let fd = refused_udp_socket();
    let dev = async_dev(fd);
    let mut poll_recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
    assert!(poll!(poll_recv.as_mut()).is_pending());
    provoke_socket_error(fd);
    refused(within("poll_recv", poll_recv).await.unwrap_err());
}
on_both_runtimes!(pending_readers_wake_on_socket_error);

/// Once a read has consumed the socket error the readiness it left behind is
/// stale: every entry point must find `WouldBlock`, clear it and go back to
/// sleep until the next error, however often that repeats.
async fn consumed_socket_error_goes_back_to_sleep() {
    let fd = refused_udp_socket();
    let dev = async_dev(fd);
    let mut buf = [0u8; 16];
    for _ in 0..3 {
        {
            let mut recv = pin!(dev.recv(&mut buf));
            assert!(poll!(recv.as_mut()).is_pending());
            provoke_socket_error(fd);
            let err = within("recv", recv).await.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        }
        let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
        assert!(polled.is_pending(), "stale error surfaced: {polled:?}");
        assert!(poll!(pin!(dev.readable())).is_pending());
        assert_eq!(
            dev.try_recv(&mut buf).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }
}
on_both_runtimes!(consumed_socket_error_goes_back_to_sleep);

/// Several tasks may wait on one device; the error wakes all of them.
async fn many_waiters_all_wake_on_error() {
    const WAITERS: usize = 16;
    let (dev, read_end) = error_only_device();
    let mut waiters: Vec<Pin<Box<dyn Future<Output = io::Result<usize>>>>> = Vec::new();
    for i in 0..WAITERS {
        let dev = dev.clone();
        waiters.push(Box::pin(async move {
            let mut buf = [0u8; 16];
            // Mix the entry points so that all waiter kinds are covered.
            match i % 3 {
                0 => dev.recv(&mut buf).await,
                1 => {
                    dev.readable().await?;
                    dev.try_recv(&mut buf)
                }
                _ => poll_fn(|cx| dev.poll_recv(cx, &mut buf)).await,
            }
        }));
    }
    for waiter in &mut waiters {
        assert!(poll!(waiter.as_mut()).is_pending());
    }
    close(read_end);
    let results = within("waiters", join_all(waiters)).await;
    assert_eq!(results.len(), WAITERS);
    for result in results {
        assert_read_error(result.unwrap_err());
    }
}
on_both_runtimes!(many_waiters_all_wake_on_error);

// ---------------------------------------------------------------------------
// Send side
// ---------------------------------------------------------------------------

async fn send_reports_closed_peer() {
    let (read_end, write_end) = pipe();
    let dev = async_dev(write_end);
    close(read_end);
    let err = within("send", dev.send(b"x")).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    within("writable", dev.writable()).await.unwrap();
    assert_eq!(
        dev.try_send(b"x").unwrap_err().kind(),
        io::ErrorKind::BrokenPipe
    );
}
on_both_runtimes!(send_reports_closed_peer);

/// A full pipe is not writable; closing the reader reports only an error
/// (no `EPOLLOUT`), which must still wake the pending writers.
async fn pending_send_wakes_when_peer_closes() {
    let (read_end, write_end) = pipe();
    fill_pipe(write_end);
    let dev = async_dev(write_end);
    let mut send = pin!(dev.send(b"x"));
    let mut writable = pin!(dev.writable());
    let mut poll_send = pin!(poll_fn(|cx| dev.poll_send(cx, b"x")));
    assert!(poll!(send.as_mut()).is_pending());
    assert!(poll!(writable.as_mut()).is_pending());
    assert!(poll!(poll_send.as_mut()).is_pending());
    close(read_end);
    let err = within("send", send).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    within("writable", writable).await.unwrap();
    // `poll_send` shares the write waker with `writable`; only the most
    // recently polled one is guaranteed to be woken, so poll it afresh.
    let err = within("poll_send", poll_fn(|cx| dev.poll_send(cx, b"x")))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}
on_both_runtimes!(pending_send_wakes_when_peer_closes);

// ---------------------------------------------------------------------------
// Cancellation safety
// ---------------------------------------------------------------------------

async fn dropped_futures_do_not_lose_the_error() {
    let (dev, read_end) = error_only_device();
    let mut buf = [0u8; 16];
    for _ in 0..3 {
        {
            let mut recv = pin!(dev.recv(&mut buf));
            assert!(poll!(recv.as_mut()).is_pending());
        }
        {
            let mut readable = pin!(dev.readable());
            assert!(poll!(readable.as_mut()).is_pending());
        }
        {
            let mut poll_recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
            assert!(poll!(poll_recv.as_mut()).is_pending());
        }
    }
    close(read_end);
    assert_read_error(within("recv", dev.recv(&mut buf)).await.unwrap_err());
}
on_both_runtimes!(dropped_futures_do_not_lose_the_error);

async fn dropped_futures_do_not_lose_data() {
    let (dev, peer) = data_device();
    let mut buf = [0u8; 16];
    for _ in 0..3 {
        {
            let mut recv = pin!(dev.recv(&mut buf));
            assert!(poll!(recv.as_mut()).is_pending());
        }
        {
            let mut poll_recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
            assert!(poll!(poll_recv.as_mut()).is_pending());
        }
    }
    send_all(peer, b"packet");
    let n = within("recv", dev.recv(&mut buf)).await.unwrap();
    assert_eq!(&buf[..n], b"packet");
    close(peer);
}
on_both_runtimes!(dropped_futures_do_not_lose_data);

// ---------------------------------------------------------------------------
// Data and errors together, spurious readiness
// ---------------------------------------------------------------------------

/// Queued data and a pending socket error make the descriptor `EPOLLIN |
/// EPOLLERR`. The data must be delivered first and the error afterwards.
async fn data_pending_then_error() {
    let (a, b) = socketpair(libc::SOCK_STREAM);
    // Unread data in the peer's queue turns its close into a reset.
    send_all(a, b"unread");
    let dev = async_dev(a);
    let mut buf = [0u8; 16];
    let n = {
        let mut first = pin!(dev.recv(&mut buf));
        assert!(poll!(first.as_mut()).is_pending());
        send_all(b, b"payload!");
        close(b);
        within("first recv", first).await.unwrap()
    };
    assert_eq!(&buf[..n], b"payload!");
    let err = within("second recv", dev.recv(&mut buf)).await.unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
}
on_both_runtimes!(data_pending_then_error);

/// Readiness left over from data that was consumed elsewhere must resolve to
/// `WouldBlock` and then go back to waiting, not to an error or a spin.
async fn spurious_readiness_keeps_would_block_semantics() {
    let (dev, peer) = data_device();
    let raw = dev.get_ref().as_raw_fd();
    let mut buf = [0u8; 16];

    send_all(peer, b"stolen");
    within("readable", dev.readable()).await.unwrap();
    assert_eq!(raw_recv(raw, &mut buf), 6);

    // The stale readiness is still set: a plain attempt must report
    // `WouldBlock`, clear it, and the waiters must go back to sleep. Draining
    // the datagram also signals writability, a newer event that tokio keeps
    // the old attempt from clearing, so a second round may be needed.
    let mut settled = false;
    for _ in 0..3 {
        assert_eq!(
            dev.try_recv(&mut buf).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        if poll!(pin!(dev.readable())).is_pending() {
            settled = true;
            break;
        }
    }
    assert!(settled, "stale readiness was never cleared");
    assert!(poll!(pin!(poll_fn(|cx| dev.poll_readable(cx)))).is_pending());

    send_all(peer, b"stolen");
    within("readable", dev.readable()).await.unwrap();
    assert_eq!(raw_recv(raw, &mut buf), 6);
    let n = {
        let mut recv = pin!(dev.recv(&mut buf));
        assert!(poll!(recv.as_mut()).is_pending());
        send_all(peer, b"real");
        within("recv", recv).await.unwrap()
    };
    assert_eq!(&buf[..n], b"real");
    close(peer);
}
on_both_runtimes!(spurious_readiness_keeps_would_block_semantics);

// ---------------------------------------------------------------------------
// Idle devices stay quiet; events cause a bounded number of wakeups.
// ---------------------------------------------------------------------------

type Trigger = Box<dyn FnOnce()>;

fn data_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, peer) = data_device();
    (dev, Box::new(move || send_all(peer, b"x")))
}

fn error_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, read_end) = error_only_device();
    (dev, Box::new(move || close(read_end)))
}

async fn idle_then_wakes(case: fn() -> (Arc<AsyncDevice>, Trigger), expect_error: bool) {
    {
        let (dev, trigger) = case();
        let mut buf = [0u8; 64];
        within(
            "poll_recv",
            assert_idle_then_wakes(
                "poll_recv",
                |cx| dev.poll_recv(cx, &mut buf).map_ok(|_| ()),
                tokio::time::sleep(IDLE_WINDOW),
                trigger,
                expect_error,
            ),
        )
        .await;
    }
    {
        let (dev, trigger) = case();
        within(
            "poll_readable",
            assert_idle_then_wakes(
                "poll_readable",
                |cx| dev.poll_readable(cx),
                tokio::time::sleep(IDLE_WINDOW),
                trigger,
                false,
            ),
        )
        .await;
    }
    for what in ["recv", "readable"] {
        let (dev, trigger) = case();
        let mut fut: Pin<Box<dyn Future<Output = io::Result<()>>>> = if what == "recv" {
            Box::pin(async move {
                let mut buf = [0u8; 64];
                dev.recv(&mut buf).await.map(|_| ())
            })
        } else {
            Box::pin(async move { dev.readable().await })
        };
        within(
            what,
            assert_idle_then_wakes(
                what,
                |cx| fut.as_mut().poll(cx),
                tokio::time::sleep(IDLE_WINDOW),
                trigger,
                expect_error && what == "recv",
            ),
        )
        .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_data_device_wakes_once_per_event_multi_thread() {
    idle_then_wakes(data_case, false).await;
}

#[tokio::test(flavor = "current_thread")]
async fn idle_data_device_wakes_once_per_event_current_thread() {
    idle_then_wakes(data_case, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_error_device_wakes_once_per_event_multi_thread() {
    idle_then_wakes(error_case, true).await;
}

#[tokio::test(flavor = "current_thread")]
async fn idle_error_device_wakes_once_per_event_current_thread() {
    idle_then_wakes(error_case, true).await;
}

// ---------------------------------------------------------------------------
// readable() + try_recv() under traffic must not busy-loop.
// ---------------------------------------------------------------------------

/// Receives `packets` datagrams with the `readable()` + `try_recv()` idiom
/// while `traffic` writes them, and bounds the loop iterations. Healthy
/// behaviour is about one iteration per packet plus one `WouldBlock` that
/// clears the readiness each time the queue runs dry.
async fn assert_no_spin(packets: usize, size: usize, traffic: fn(RawFd, usize, usize)) {
    let (dev, peer) = data_device();
    let cap = packets * 3 + 5;
    let iterations = Arc::new(AtomicUsize::new(0));
    let reader = {
        let dev = dev.clone();
        let iterations = iterations.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; size.max(8)];
            let mut next = 0u64;
            while (next as usize) < packets {
                dev.readable().await.unwrap();
                if iterations.fetch_add(1, Ordering::Relaxed) >= cap {
                    // Leave instead of spinning the runtime forever.
                    return Err(format!("busy loop after {cap} iterations"));
                }
                match dev.try_recv(&mut buf) {
                    Ok(n) => {
                        assert_eq!(n, size.max(8));
                        let seq = u64::from_le_bytes(buf[..8].try_into().unwrap());
                        assert_eq!(seq, next, "packets reordered or lost");
                        next += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(())
        })
    };
    let writer = std::thread::spawn(move || {
        traffic(peer, packets, size.max(8));
        peer
    });
    let outcome = within("reader", reader).await.unwrap();
    close(writer.join().unwrap());
    if let Err(e) = outcome {
        panic!("{e}");
    }
    let used = iterations.load(Ordering::Relaxed);
    assert!(used <= cap, "{used} iterations for {packets} packets");
}

fn packet(seq: usize, size: usize) -> Vec<u8> {
    let mut p = vec![0xAB; size];
    p[..8].copy_from_slice(&(seq as u64).to_le_bytes());
    p
}

fn burst(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
    }
}

fn trickle(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn bursts_with_gaps(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
        if seq % 10 == 9 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

async fn burst_does_not_spin() {
    assert_no_spin(500, 16, burst).await;
}
on_both_runtimes!(burst_does_not_spin);

async fn trickle_does_not_spin() {
    assert_no_spin(100, 8, trickle).await;
}
on_both_runtimes!(trickle_does_not_spin);

async fn bursts_with_gaps_do_not_spin() {
    assert_no_spin(200, 64, bursts_with_gaps).await;
}
on_both_runtimes!(bursts_with_gaps_do_not_spin);

async fn large_packets_do_not_spin() {
    assert_no_spin(64, 60_000, burst).await;
}
on_both_runtimes!(large_packets_do_not_spin);

// ---------------------------------------------------------------------------
// Privileged: real TUN/TAP devices removed from under a pending reader.
//
// Run with CAP_NET_ADMIN, the `ip` tool and /dev/net/tun:
//   sudo -E env "PATH=$PATH" cargo test --lib --features async_tokio \
//     -- --ignored privileged_ --test-threads=1
// ---------------------------------------------------------------------------

mod privileged {
    use super::*;
    use crate::{DeviceBuilder, Layer};

    fn build(name: &str, layer: Layer, multi_queue: bool) -> AsyncDevice {
        DeviceBuilder::new()
            .name(name)
            .layer(layer)
            .multi_queue(multi_queue)
            .enable(false)
            .build_async()
            .expect("create the device (needs CAP_NET_ADMIN and /dev/net/tun)")
    }

    fn delete_link(name: &str) {
        let status = std::process::Command::new("ip")
            .args(["link", "del", name])
            .status()
            .expect("run ip");
        assert!(status.success(), "ip link del {name} failed");
    }

    async fn recv_fails_after_delete(name: &str, layer: Layer) {
        let dev = build(name, layer, false);
        let mut buf = vec![0u8; 2048];
        {
            let mut recv = pin!(dev.recv(&mut buf));
            assert!(poll!(recv.as_mut()).is_pending());
            delete_link(name);
            assert_read_error(within("recv", recv).await.unwrap_err());
        }
        for _ in 0..3 {
            assert_read_error(
                within("repeated recv", dev.recv(&mut buf))
                    .await
                    .unwrap_err(),
            );
        }
        assert_read_error(dev.try_recv(&mut buf).unwrap_err());
        within("send", dev.send(&[0u8; 20])).await.unwrap_err();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_tun_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_tun_recv", Layer::L3).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_tap_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_tap_recv", Layer::L2).await;
    }

    #[tokio::test(flavor = "current_thread")]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_tun_recv_fails_after_link_delete_current_thread() {
        recv_fails_after_delete("trs_tun_cur", Layer::L3).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_readable_try_recv_fails_after_link_delete() {
        let name = "trs_tun_rdbl";
        let dev = build(name, Layer::L3, false);
        let mut buf = vec![0u8; 2048];
        let mut readable = pin!(dev.readable());
        assert!(poll!(readable.as_mut()).is_pending());
        delete_link(name);
        within("readable", readable).await.unwrap();
        assert_read_error(dev.try_recv(&mut buf).unwrap_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_poll_recv_fails_after_link_delete() {
        let name = "trs_tun_poll";
        let dev = build(name, Layer::L3, false);
        let mut buf = vec![0u8; 2048];
        let mut recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
        assert!(poll!(recv.as_mut()).is_pending());
        delete_link(name);
        assert_read_error(within("poll_recv", recv).await.unwrap_err());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    async fn privileged_every_queue_fails_after_link_delete() {
        let name = "trs_tun_mq";
        let first = build(name, Layer::L3, true);
        let second = first.try_clone().expect("second queue");
        let mut buf_a = vec![0u8; 2048];
        let mut buf_b = vec![0u8; 2048];
        let mut recv_a = pin!(first.recv(&mut buf_a));
        let mut recv_b = pin!(second.recv(&mut buf_b));
        assert!(poll!(recv_a.as_mut()).is_pending());
        assert!(poll!(recv_b.as_mut()).is_pending());
        delete_link(name);
        let (a, b) = within("queues", async { futures::join!(recv_a, recv_b) }).await;
        assert_read_error(a.unwrap_err());
        assert_read_error(b.unwrap_err());
    }
}
