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

fn async_dev(fd: RawFd) -> AsyncDevice {
    AsyncDevice::new_dev(DeviceImpl::from_fd_for_test(fd)).unwrap()
}

async fn within<F: Future>(what: &str, fut: F) -> F::Output {
    match select(pin!(fut), pin!(Timer::after(HANG_GUARD))).await {
        Either::Left((value, _)) => value,
        Either::Right(_) => panic!("{what} hung"),
    }
}

fn run<F: Future>(what: &str, fut: F) -> F::Output {
    let _watchdog = Watchdog::arm();
    block_on(within(what, fut))
}

async fn idle() {
    Timer::after(IDLE_WINDOW).await;
}

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

fn assert_connection_refused(err: io::Error) {
    assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused, "{err}");
}

fn assert_read_error(err: io::Error) {
    assert_ne!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
}

// ---------------------------------------------------------------------------
// Error-only readiness: every read entry point wakes and keeps failing.
// ---------------------------------------------------------------------------

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

type Trigger = Box<dyn FnOnce()>;

fn data_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, peer) = data_device();
    (dev, Box::new(move || send_all(peer, b"x")))
}

fn error_case() -> (Arc<AsyncDevice>, Trigger) {
    let (dev, fd) = error_only_device();
    (dev, Box::new(move || provoke_socket_error(fd)))
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

#[test]
fn idle_data_device_wakes_once_per_event() {
    run("test", idle_then_wakes(data_case, false));
}

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

#[test]
fn burst_does_not_spin() {
    assert_no_spin(500, 16, burst);
}

#[test]
fn trickle_does_not_spin() {
    assert_no_spin(100, 8, trickle);
}

#[test]
fn bursts_with_gaps_do_not_spin() {
    assert_no_spin(200, 64, bursts_with_gaps);
}

#[test]
fn large_packets_do_not_spin() {
    assert_no_spin(64, 60_000, burst);
}

// ---------------------------------------------------------------------------
// Privileged: real TUN/TAP devices removed from under a pending reader.
//
// Run with CAP_NET_ADMIN, the `ip` tool and /dev/net/tun:
//   sudo -E env "PATH=$PATH" cargo test --lib --features async_io \
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

    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_tun_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_aio_tun", Layer::L3);
    }

    #[test]
    #[ignore = "needs CAP_NET_ADMIN, /dev/net/tun and the `ip` tool"]
    fn privileged_tap_recv_fails_after_link_delete() {
        recv_fails_after_delete("trs_aio_tap", Layer::L2);
    }

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
