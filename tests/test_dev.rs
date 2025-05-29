#![allow(unused_imports)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pnet_packet::ip::IpNextHeaderProtocols;
use pnet_packet::Packet;
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd"
))]
use tun_rs::DeviceBuilder;
use tun_rs::SyncDevice;

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd"
))]
#[cfg(not(any(feature = "async_tokio", feature = "async_io")))]
#[test]
fn test_udp() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv4("10.26.1.100", 24, None)
        .ipv6("CDCD:910A:2222:5498:8475:1112:1900:2025", 64)
        .build_sync()
        .unwrap();
    let device = Arc::new(device);
    let test_udp_v4 = Arc::new(AtomicBool::new(false));
    let test_udp_v6 = Arc::new(AtomicBool::new(false));
    let test_udp_v4_c = test_udp_v4.clone();
    let test_udp_v6_c = test_udp_v6.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 65535];
        loop {
            let len = device.recv(&mut buf).unwrap();
            if let Some(ipv6_packet) = pnet_packet::ipv6::Ipv6Packet::new(&buf[..len]) {
                if ipv6_packet.get_next_header() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv6_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v6.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
            if let Some(ipv4_packet) = pnet_packet::ipv4::Ipv4Packet::new(&buf[..len]) {
                if ipv4_packet.get_next_level_protocol() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv4_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v4.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
    });
    std::thread::sleep(Duration::from_secs(6));
    let udp_socket =
        std::net::UdpSocket::bind("[CDCD:910A:2222:5498:8475:1112:1900:2025]:0").unwrap();
    udp_socket
        .send_to(
            test_msg.as_bytes(),
            "[CDCD:910A:2222:5498:8475:1112:1900:2024]:8080",
        )
        .unwrap();

    let udp_socket = std::net::UdpSocket::bind("10.26.1.100:0").unwrap();
    udp_socket
        .send_to(test_msg.as_bytes(), "10.26.1.101:8080")
        .unwrap();
    std::thread::sleep(Duration::from_secs(1));
    assert!(test_udp_v4_c.load(Ordering::SeqCst));
    assert!(test_udp_v6_c.load(Ordering::SeqCst));
}

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd"
))]
#[cfg(feature = "async_tokio")]
#[tokio::test]
async fn test_udp() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv4("10.26.1.100", 24, None)
        .ipv6("CDCD:910A:2222:5498:8475:1112:1900:2025", 64)
        .build_async()
        .unwrap();
    let vec = device.addresses().unwrap();
    assert!(vec
        .iter()
        .any(|ip| *ip == "10.26.1.100".parse::<std::net::Ipv4Addr>().unwrap()));
    assert!(vec.iter().any(|ip| *ip
        == "CDCD:910A:2222:5498:8475:1112:1900:2025"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()));
    let device = Arc::new(device);
    let test_udp_v4 = Arc::new(AtomicBool::new(false));
    let test_udp_v6 = Arc::new(AtomicBool::new(false));
    let test_udp_v4_c = test_udp_v4.clone();
    let test_udp_v6_c = test_udp_v6.clone();
    tokio::spawn(async move {
        let mut buf = [0; 65535];
        loop {
            let len = device.recv(&mut buf).await.unwrap();
            if let Some(ipv6_packet) = pnet_packet::ipv6::Ipv6Packet::new(&buf[..len]) {
                if ipv6_packet.get_next_header() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv6_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v6.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
            if let Some(ipv4_packet) = pnet_packet::ipv4::Ipv4Packet::new(&buf[..len]) {
                if ipv4_packet.get_next_level_protocol() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv4_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v4.store(true, Ordering::SeqCst);
                        }
                    }
                }
            }
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    let udp_socket = tokio::net::UdpSocket::bind("[CDCD:910A:2222:5498:8475:1112:1900:2025]:0")
        .await
        .unwrap();
    udp_socket
        .send_to(
            test_msg.as_bytes(),
            "[CDCD:910A:2222:5498:8475:1112:1900:2024]:8080",
        )
        .await
        .unwrap();

    let udp_socket = tokio::net::UdpSocket::bind("10.26.1.100:0").await.unwrap();
    udp_socket
        .send_to(test_msg.as_bytes(), "10.26.1.101:8080")
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(test_udp_v4_c.load(Ordering::SeqCst));
    assert!(test_udp_v6_c.load(Ordering::SeqCst));
}

#[cfg(unix)]
#[tokio::test]
async fn test_unix_fd() {
    use std::os::fd::IntoRawFd;
    let device = unsafe { SyncDevice::from_fd(1) };
    let fd = device.into_raw_fd();
    assert_eq!(fd, 1)
}
