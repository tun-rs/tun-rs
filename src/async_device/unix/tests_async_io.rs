//! Readiness tests for the unix `AsyncDevice` (async-io backend); the tokio
//! counterparts and the rationale live in `tests.rs`.
//!
//! `async-io` reports `EPOLLERR` as both readable and writable, so a device
//! that only reports an error must complete every waiter with that error. These
//! tests lock that behaviour in.

use super::test_util::*;
use super::AsyncDevice;
use crate::platform::DeviceImpl;
use async_io::{block_on, Timer};
use bytes::buf::UninitSlice;
use futures::future::{select, Either};
use futures::poll;
use std::future::{poll_fn, Future};
use std::io::{self, IoSliceMut};
use std::os::fd::{AsRawFd, RawFd};
use std::pin::{pin, Pin};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};
use std::task::Poll;
use std::time::Duration;

/// Wraps an arbitrary non-blocking descriptor in an `AsyncDevice`.
fn async_dev(fd: RawFd) -> AsyncDevice {
    AsyncDevice::new_dev(DeviceImpl::from_fd_for_test(fd)).unwrap()
}

/// Awaits `fut`, panicking with `what` if it takes longer than [`HANG_GUARD`].
async fn within<F: Future>(what: &str, fut: F) -> F::Output {
    match select(pin!(fut), pin!(Timer::after(HANG_GUARD))).await {
        Either::Left((value, _)) => value,
        Either::Right(_) => panic!("{what} hung"),
    }
}

/// Blocks on `fut` under the process watchdog and the [`HANG_GUARD`] timeout.
fn run<F: Future>(what: &str, fut: F) -> F::Output {
    let _watchdog = Watchdog::arm();
    block_on(within(what, fut))
}

/// Sleeps for [`IDLE_WINDOW`] so a device that must stay quiet can be checked.
async fn idle() {
    Timer::after(IDLE_WINDOW).await;
}

/// Idle, then healthy: a datagram device and its peer descriptor.
fn data_device() -> (Arc<AsyncDevice>, RawFd) {
    let (a, b) = socketpair(libc::SOCK_DGRAM);
    (Arc::new(async_dev(a)), b)
}

/// A descriptor whose reads always fail (the write end of a pipe). `async-io`
/// attempts the read before waiting, so this covers only the immediate error
/// path and that errors keep being reported.
fn failing_device() -> Arc<AsyncDevice> {
    let (read_end, write_end) = pipe();
    close(read_end);
    Arc::new(async_dev(write_end))
}

/// Idle (reads give `WouldBlock`) until the returned descriptor is passed to
/// `provoke_socket_error`; the device then reports only `EPOLLERR` and the next
/// read returns the error. This is the shape a deleted TUN device has while a
/// reader is parked on it.
fn error_only_device() -> (Arc<AsyncDevice>, RawFd) {
    let fd = refused_udp_socket();
    (Arc::new(async_dev(fd)), fd)
}

/// Asserts the error is the `ECONNREFUSED` provoked by [`provoke_socket_error`].
fn assert_connection_refused(err: io::Error) {
    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused, "{err}");
}

/// Asserts that a read surfaced a real error rather than `WouldBlock`.
fn assert_read_error(err: io::Error) {
    assert_ne!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
}

// ---------------------------------------------------------------------------
// Error-only readiness: every read entry point wakes and keeps failing.
// ---------------------------------------------------------------------------

/// A pending `recv` wakes with the socket error.
#[test]
fn recv_wakes_on_error_only_readiness() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut buf = [0u8; 16];
        let mut recv = pin!(dev.recv(&mut buf));
        assert!(poll!(recv.as_mut()).is_pending());
        provoke_socket_error(fd);
        assert_connection_refused(recv.await.unwrap_err());
    });
}

/// A permanently failing descriptor reports its error on every read entry
/// point, repeatedly.
#[test]
fn failing_device_keeps_reporting_the_error() {
    run("test", async {
        let dev = failing_device();
        let mut buf = [0u8; 16];
        for _ in 0..5 {
            assert_read_error(dev.recv(&mut buf).await.unwrap_err());
            assert_read_error(dev.try_recv(&mut buf).unwrap_err());
            let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
            assert!(matches!(polled, Poll::Ready(Err(_))), "{polled:?}");
        }
    });
}

/// `readable()` wakes on error-only readiness and `try_recv` /
/// `try_recv_vectored` return the error instead of `WouldBlock`.
#[test]
fn readable_then_try_recv_surfaces_error_only_readiness() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut buf = [0u8; 16];
        {
            let mut readable = pin!(dev.readable());
            assert!(poll!(readable.as_mut()).is_pending());
            provoke_socket_error(fd);
            readable.await.unwrap();
        }
        assert_connection_refused(dev.try_recv(&mut buf).unwrap_err());

        let failing = failing_device();
        let mut slice = [IoSliceMut::new(&mut buf)];
        assert_read_error(failing.try_recv_vectored(&mut slice).unwrap_err());
    });
}

/// A pending `poll_recv` wakes with the socket error.
#[test]
fn poll_recv_wakes_on_error_only_readiness() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut buf = [0u8; 16];
        let mut recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
        assert!(poll!(recv.as_mut()).is_pending());
        provoke_socket_error(fd);
        assert_connection_refused(recv.await.unwrap_err());
    });
}

/// A pending `poll_recv_uninit` wakes with the socket error.
#[test]
fn poll_recv_uninit_wakes_on_error_only_readiness() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut buf = [0u8; 16];
        let mut recv = pin!(poll_fn(
            |cx| dev.poll_recv_uninit(cx, UninitSlice::new(&mut buf))
        ));
        assert!(poll!(recv.as_mut()).is_pending());
        provoke_socket_error(fd);
        assert_connection_refused(recv.await.unwrap_err());
    });
}

/// A pending `poll_readable` wakes on error-only readiness and the following
/// `poll_recv` reports the error.
#[test]
fn poll_readable_wakes_on_error_only_readiness() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut readable = pin!(poll_fn(|cx| dev.poll_readable(cx)));
        assert!(poll!(readable.as_mut()).is_pending());
        provoke_socket_error(fd);
        readable.await.unwrap();
        let mut buf = [0u8; 16];
        let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
        let Poll::Ready(Err(err)) = polled else {
            panic!("poll_recv after poll_readable: {polled:?}");
        };
        assert_connection_refused(err);
    });
}

/// Once a read has consumed the socket error nothing is left to read: the
/// entry points must go back to waiting until the next error.
#[test]
fn consumed_socket_error_goes_back_to_sleep() {
    run("test", async {
        let (dev, fd) = error_only_device();
        let mut buf = [0u8; 16];
        for _ in 0..3 {
            {
                let mut recv = pin!(dev.recv(&mut buf));
                assert!(poll!(recv.as_mut()).is_pending());
                provoke_socket_error(fd);
                assert_connection_refused(recv.await.unwrap_err());
            }
            let polled = poll!(pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf))));
            assert!(polled.is_pending(), "stale error surfaced: {polled:?}");
            assert_eq!(
                dev.try_recv(&mut buf).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
        }
    });
}

/// A reader running on another OS thread, with its own executor, is woken by
/// an error raised from this one.
#[test]
fn reader_on_another_thread_wakes_on_error() {
    let (dev, fd) = error_only_device();
    let parked = Arc::new(Barrier::new(2));
    let reader = {
        let parked = parked.clone();
        std::thread::spawn(move || {
            run("threaded recv", async move {
                let mut buf = [0u8; 16];
                let mut recv = pin!(dev.recv(&mut buf));
                assert!(poll!(recv.as_mut()).is_pending());
                parked.wait();
                recv.await
            })
        })
    };
    parked.wait();
    provoke_socket_error(fd);
    assert_connection_refused(reader.join().unwrap().unwrap_err());
}

// ---------------------------------------------------------------------------
// Send side
// ---------------------------------------------------------------------------

/// Writing to a pipe whose reader is gone fails with `BrokenPipe` on every
/// send entry point, and `writable()` still resolves.
#[test]
fn send_reports_closed_peer() {
    run("test", async {
        let (read_end, write_end) = pipe();
        let dev = async_dev(write_end);
        close(read_end);
        let err = dev.send(b"x").await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        dev.writable().await.unwrap();
        assert_eq!(
            dev.try_send(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    });
}

/// A full pipe is not writable; closing the reader reports only an error
/// (no `EPOLLOUT`), which must still wake the pending writers.
#[test]
fn pending_send_wakes_when_peer_closes() {
    run("test", async {
        let (read_end, write_end) = pipe();
        fill_pipe(write_end);
        let dev = async_dev(write_end);
        let mut send = pin!(dev.send(b"x"));
        let mut writable = pin!(dev.writable());
        assert!(poll!(send.as_mut()).is_pending());
        assert!(poll!(writable.as_mut()).is_pending());
        close(read_end);
        let err = send.await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        writable.await.unwrap();
        let err = poll_fn(|cx| dev.poll_send(cx, b"x")).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    });
}

// ---------------------------------------------------------------------------
// Cancellation safety
// ---------------------------------------------------------------------------

/// Dropping pending read futures does not swallow a later error.
#[test]
fn dropped_futures_do_not_lose_the_error() {
    run("test", async {
        let (dev, fd) = error_only_device();
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
        provoke_socket_error(fd);
        assert_connection_refused(dev.recv(&mut buf).await.unwrap_err());
    });
}

/// Dropping pending read futures does not lose a packet that arrives later.
#[test]
fn dropped_futures_do_not_lose_data() {
    run("test", async {
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
        let n = dev.recv(&mut buf).await.unwrap();
        assert_eq!(&buf[..n], b"packet");
        close(peer);
    });
}

// ---------------------------------------------------------------------------
// Data and errors together, spurious readiness
// ---------------------------------------------------------------------------

/// Queued data and a pending socket error make the descriptor `EPOLLIN |
/// EPOLLERR`. The data must be delivered first and the error afterwards.
#[test]
fn data_pending_then_error() {
    run("test", async {
        let (a, b) = socketpair(libc::SOCK_STREAM);
        send_all(a, b"unread");
        let dev = async_dev(a);
        let mut buf = [0u8; 16];
        let n = {
            let mut first = pin!(dev.recv(&mut buf));
            assert!(poll!(first.as_mut()).is_pending());
            send_all(b, b"payload!");
            close(b);
            first.await.unwrap()
        };
        assert_eq!(&buf[..n], b"payload!");
        let err = dev.recv(&mut buf).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionReset);
    });
}

/// Readiness left over from data that was consumed elsewhere must resolve to
/// `WouldBlock` and then go back to waiting, not to an error or a spin.
#[test]
fn spurious_readiness_keeps_would_block_semantics() {
    run("test", async {
        let (dev, peer) = data_device();
        let raw = dev.get_ref().as_raw_fd();
        let mut buf = [0u8; 16];

        send_all(peer, b"stolen");
        dev.readable().await.unwrap();
        assert_eq!(raw_recv(raw, &mut buf), 6);
        assert_eq!(
            dev.try_recv(&mut buf).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(poll!(pin!(dev.readable())).is_pending());

        send_all(peer, b"stolen");
        dev.readable().await.unwrap();
        assert_eq!(raw_recv(raw, &mut buf), 6);
        let n = {
            let mut recv = pin!(dev.recv(&mut buf));
            assert!(poll!(recv.as_mut()).is_pending());
            send_all(peer, b"real");
            recv.await.unwrap()
        };
        assert_eq!(&buf[..n], b"real");
        close(peer);
    });
}

// ---------------------------------------------------------------------------
// Idle devices stay quiet; events cause a bounded number of wakeups.
// ---------------------------------------------------------------------------

/// Action that makes an idle device ready (a packet or an error).
type Trigger = Box<dyn FnOnce()>;

/// An idle datagram device whose trigger sends it a packet.
fn data_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, peer) = data_device();
    (dev, Box::new(move || send_all(peer, b"x")))
}

/// An idle error-only device whose trigger provokes the socket error.
fn error_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, fd) = error_only_device();
    (dev, Box::new(move || provoke_socket_error(fd)))
}

/// Runs the idle-then-wake contract for `poll_recv`, `poll_readable`, `recv`
/// and `readable` on a fresh device from `case`.
async fn idle_then_wakes(case: fn() -> (Arc<AsyncDevice>, Trigger), expect_error: bool) {
    {
        let (dev, trigger) = case();
        let mut buf = [0u8; 64];
        within(
            "poll_recv",
            assert_idle_then_wakes(
                "poll_recv",
                |cx| dev.poll_recv(cx, &mut buf).map_ok(|_| ()),
                idle(),
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
                idle(),
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
                idle(),
                trigger,
                expect_error && what == "recv",
            ),
        )
        .await;
    }
}

/// A healthy idle device stays quiet and wakes a bounded number of times per
/// packet.
#[test]
fn idle_data_device_wakes_once_per_event() {
    run("test", idle_then_wakes(data_case, false));
}

/// An idle error-only device stays quiet and wakes a bounded number of times
/// when the error appears.
#[test]
fn idle_error_device_wakes_once_per_event() {
    run("test", idle_then_wakes(error_case, true));
}

// ---------------------------------------------------------------------------
// readable() + try_recv() under traffic must not busy-loop.
// ---------------------------------------------------------------------------

/// Receives `packets` datagrams with the `readable()` + `try_recv()` idiom
/// while `traffic` writes them, and bounds the loop iterations.
fn assert_no_spin(packets: usize, size: usize, traffic: fn(RawFd, usize, usize)) {
    let size = size.max(8);
    let (dev, peer) = data_device();
    let cap = packets * 3 + 5;
    let iterations = Arc::new(AtomicUsize::new(0));
    let writer = std::thread::spawn(move || {
        traffic(peer, packets, size);
        peer
    });
    let outcome = run("reader", {
        let iterations = iterations.clone();
        async move {
            let mut buf = vec![0u8; size];
            let mut next = 0u64;
            while (next as usize) < packets {
                dev.readable().await.unwrap();
                if iterations.fetch_add(1, Ordering::Relaxed) >= cap {
                    return Err(format!("busy loop after {cap} iterations"));
                }
                match dev.try_recv(&mut buf) {
                    Ok(n) => {
                        assert_eq!(n, size);
                        let seq = u64::from_le_bytes(buf[..8].try_into().unwrap());
                        assert_eq!(seq, next, "packets reordered or lost");
                        next += 1;
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(e) => return Err(e.to_string()),
                }
            }
            Ok(())
        }
    });
    // Fail before joining: a writer blocked on a full queue never finishes.
    if let Err(e) = outcome {
        panic!("{e}");
    }
    close(writer.join().unwrap());
}

/// A `size`-byte packet whose first eight bytes carry `seq` little-endian.
fn packet(seq: usize, size: usize) -> Vec<u8> {
    let mut p = vec![0xAB; size];
    p[..8].copy_from_slice(&(seq as u64).to_le_bytes());
    p
}

/// Traffic pattern: all packets back to back.
fn burst(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
    }
}

/// Traffic pattern: one packet per millisecond.
fn trickle(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Traffic pattern: bursts of ten packets separated by a short pause.
fn bursts_with_gaps(peer: RawFd, packets: usize, size: usize) {
    for seq in 0..packets {
        send_all(peer, &packet(seq, size));
        if seq % 10 == 9 {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

/// Back-to-back small packets do not make the reader spin.
#[test]
fn burst_does_not_spin() {
    assert_no_spin(500, 16, burst);
}

/// Slowly arriving packets do not make the reader spin.
#[test]
fn trickle_does_not_spin() {
    assert_no_spin(100, 8, trickle);
}

/// Bursts separated by pauses do not make the reader spin.
#[test]
fn bursts_with_gaps_do_not_spin() {
    assert_no_spin(200, 64, bursts_with_gaps);
}

/// Near-maximum-size packets do not make the reader spin.
#[test]
fn large_packets_do_not_spin() {
    assert_no_spin(64, 60_000, burst);
}

// ---------------------------------------------------------------------------
// try_recv() must not steal or clobber wakeups registered by other waiters.
// ---------------------------------------------------------------------------

/// A parked waiter as a boxed future, paired with its kind.
type ParkedWaiter = (Waiter, Pin<Box<dyn Future<Output = io::Result<()>>>>);

/// Parks `kinds` on a device from `case`, calls `try_recv` over and over while
/// they are idle, then fires the trigger. Every waiter must still wake: a
/// `try_recv` that replaced their waker or consumed their readiness would leave
/// one of them hanging.
async fn try_recv_leaves_parked_waiters_alone(
    case: fn() -> (Arc<AsyncDevice>, Trigger),
    expect_error: bool,
    kinds: &[Waiter],
) {
    let (dev, trigger) = case();
    let mut waiters: Vec<ParkedWaiter> = Vec::new();
    for &kind in kinds {
        let mut waiter: Pin<Box<dyn Future<Output = io::Result<()>>>> =
            Box::pin(park(dev.clone(), kind));
        assert!(
            poll!(waiter.as_mut()).is_pending(),
            "{kind:?} ready while idle"
        );
        waiters.push((kind, waiter));
    }
    let mut buf = [0u8; 16];
    for _ in 0..50 {
        let err = dev.try_recv(&mut buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
        Timer::after(Duration::from_millis(1)).await;
    }
    trigger();
    let results = within(
        "parked waiters",
        futures::future::join_all(
            waiters
                .into_iter()
                .map(|(kind, waiter)| async move { (kind, waiter.await) }),
        ),
    )
    .await;
    for (kind, result) in results {
        assert_wake_outcome(kind, result, expect_error);
    }
}

/// A `poll_recv` waiter still receives the packet after `try_recv` probing.
#[test]
fn poll_recv_waiter_gets_packet_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(data_case, false, &[Waiter::PollRecv]),
    );
}

/// A `poll_recv` waiter still gets the error after `try_recv` probing.
#[test]
fn poll_recv_waiter_gets_error_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(error_case, true, &[Waiter::PollRecv]),
    );
}

/// A `readable()` waiter still wakes on data after `try_recv` probing.
#[test]
fn readable_waiter_wakes_on_data_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(data_case, false, &[Waiter::Readable]),
    );
}

/// A `readable()` waiter still wakes on the error after `try_recv` probing.
#[test]
fn readable_waiter_wakes_on_error_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(error_case, true, &[Waiter::Readable]),
    );
}

/// A `poll_readable` waiter still wakes on data after `try_recv` probing.
#[test]
fn poll_readable_waiter_wakes_on_data_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(data_case, false, &[Waiter::PollReadable]),
    );
}

/// A `poll_readable` waiter still wakes on the error after `try_recv` probing.
#[test]
fn poll_readable_waiter_wakes_on_error_despite_try_recv() {
    run(
        "test",
        try_recv_leaves_parked_waiters_alone(error_case, true, &[Waiter::PollReadable]),
    );
}

// ---------------------------------------------------------------------------
// Privileged: real TUN/TAP devices removed from under a pending reader.
//
// Run with CAP_NET_ADMIN, the `ip` tool and /dev/net/tun:
//   sudo -E env "PATH=$PATH" cargo test --lib --features async_io \
//     -- --ignored privileged_ --test-threads=1
// ---------------------------------------------------------------------------

/// Tests that need CAP_NET_ADMIN and real TUN/TAP devices.
mod privileged {
    use super::*;
    use crate::{DeviceBuilder, Layer};

    /// Creates a disabled TUN/TAP device with the given name and layer.
    fn build(name: &str, layer: Layer, multi_queue: bool) -> AsyncDevice {
        DeviceBuilder::new()
            .name(name)
            .layer(layer)
            .multi_queue(multi_queue)
            .enable(false)
            .build_async()
            .expect("create the device (needs CAP_NET_ADMIN and /dev/net/tun)")
    }

    /// Removes the network interface with `ip link del`.
    fn delete_link(name: &str) {
        let status = std::process::Command::new("ip")
            .args(["link", "del", name])
            .status()
            .expect("run ip");
        assert!(status.success(), "ip link del {name} failed");
    }

    /// Deleting the link under a pending `recv` wakes it with an error; later
    /// reads and sends keep failing.
    fn recv_fails_after_delete(name: &str, layer: Layer) {
        run("test", async {
            let dev = build(name, layer, false);
            let mut buf = vec![0u8; 2048];
            {
                let mut recv = pin!(dev.recv(&mut buf));
                assert!(poll!(recv.as_mut()).is_pending());
                delete_link(name);
                assert_read_error(recv.await.unwrap_err());
            }
            for _ in 0..3 {
                assert_read_error(dev.recv(&mut buf).await.unwrap_err());
            }
            assert_read_error(dev.try_recv(&mut buf).unwrap_err());
            dev.send(&[0u8; 20]).await.unwrap_err();
        });
    }

    /// Deleted TUN device.
    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_tun_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_aio_tun", Layer::L3);
    }

    /// Deleted TAP device.
    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_tap_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_aio_tap", Layer::L2);
    }

    /// `readable()` wakes when the link is deleted and `try_recv` reports the
    /// error.
    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_readable_try_recv_fails_after_link_delete() {
        run("test", async {
            let name = "trs_aio_rdbl";
            let dev = build(name, Layer::L3, false);
            let mut buf = vec![0u8; 2048];
            let mut readable = pin!(dev.readable());
            assert!(poll!(readable.as_mut()).is_pending());
            delete_link(name);
            readable.await.unwrap();
            assert_read_error(dev.try_recv(&mut buf).unwrap_err());
        });
    }

    /// A pending `poll_recv` wakes with an error when the link is deleted.
    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_poll_recv_fails_after_link_delete() {
        run("test", async {
            let name = "trs_aio_poll";
            let dev = build(name, Layer::L3, false);
            let mut buf = vec![0u8; 2048];
            let mut recv = pin!(poll_fn(|cx| dev.poll_recv(cx, &mut buf)));
            assert!(poll!(recv.as_mut()).is_pending());
            delete_link(name);
            assert_read_error(recv.await.unwrap_err());
        });
    }

    /// Every queue of a multi-queue device wakes with an error when the link
    /// is deleted.
    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_every_queue_fails_after_link_delete() {
        run("test", async {
            let name = "trs_aio_mq";
            let first = build(name, Layer::L3, true);
            let second = first.try_clone().expect("second queue");
            let mut buf_a = vec![0u8; 2048];
            let mut buf_b = vec![0u8; 2048];
            let mut recv_a = pin!(first.recv(&mut buf_a));
            let mut recv_b = pin!(second.recv(&mut buf_b));
            assert!(poll!(recv_a.as_mut()).is_pending());
            assert!(poll!(recv_b.as_mut()).is_pending());
            delete_link(name);
            let (a, b) = futures::join!(recv_a, recv_b);
            assert_read_error(a.unwrap_err());
            assert_read_error(b.unwrap_err());
        });
    }
}
