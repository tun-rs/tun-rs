#![allow(unused_imports)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use pnet_packet::ip::IpNextHeaderProtocols;
use pnet_packet::Packet;
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
use tun_rs::DeviceBuilder;
use tun_rs::SyncDevice;

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[cfg(not(any(feature = "async_tokio", feature = "async_io")))]
#[test]
fn test_udp_v4() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv4("10.26.1.100", 24, None)
        .build_sync()
        .unwrap();
    let device = Arc::new(device);
    let _device = device.clone();
    let test_udp_v4 = Arc::new(AtomicBool::new(false));
    let test_udp_v4_c = test_udp_v4.clone();
    let recv_flag = Arc::new(AtomicBool::new(false));
    let recv_flag_c = recv_flag.clone();
    std::thread::spawn(move || {
        let mut buf = [0; 65535];
        loop {
            let len = device.recv(&mut buf).unwrap();
            if let Some(ipv4_packet) = pnet_packet::ipv4::Ipv4Packet::new(&buf[..len]) {
                if ipv4_packet.get_next_level_protocol() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv4_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v4.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            if test_udp_v4.load(Ordering::Relaxed) {
                recv_flag.store(true, Ordering::Release);
                break;
            }
        }
    });
    std::thread::sleep(Duration::from_secs(6));
    let udp_socket = std::net::UdpSocket::bind("10.26.1.100:0").unwrap();
    udp_socket
        .send_to(test_msg.as_bytes(), "10.26.1.101:8080")
        .unwrap();
    let time_now = std::time::Instant::now();
    // check whether the thread completes
    while !recv_flag_c.load(Ordering::Acquire) {
        if time_now.elapsed().as_secs() > 2 {
            // no promise due to the timeout
            let v4 = test_udp_v4_c.load(Ordering::Relaxed);
            assert!(v4, "timeout: test_udp_v4 = {v4}");
            return;
        }
    }
    // recv_flag_c == true
    // all modifications to test_udp_v4_c must be visible
    let v4 = test_udp_v4_c.load(Ordering::Relaxed);
    assert!(v4);
}

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[cfg(not(any(feature = "async_tokio", feature = "async_io")))]
#[test]
fn test_udp_v6() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv6("fd12:3456:789a:1111:2222:3333:4444:5555", 64)
        .build_sync()
        .unwrap();
    let device = Arc::new(device);
    let _device = device.clone();
    let test_udp_v6 = Arc::new(AtomicBool::new(false));
    let test_udp_v6_c = test_udp_v6.clone();
    let recv_flag = Arc::new(AtomicBool::new(false));
    let recv_flag_c = recv_flag.clone();
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
                            test_udp_v6.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            if test_udp_v6.load(Ordering::Relaxed) {
                recv_flag.store(true, Ordering::Release);
                break;
            }
        }
    });
    std::thread::sleep(Duration::from_secs(6));
    let udp_socket =
        std::net::UdpSocket::bind("[fd12:3456:789a:1111:2222:3333:4444:5555]:0").unwrap();
    udp_socket
        .send_to(
            test_msg.as_bytes(),
            "[fd12:3456:789a:1111:2222:3333:4444:5556]:8080",
        )
        .unwrap();
    let time_now = std::time::Instant::now();
    // check whether the thread completes
    while !recv_flag_c.load(Ordering::Acquire) {
        if time_now.elapsed().as_secs() > 2 {
            // no promise due to the timeout
            let v6 = test_udp_v6_c.load(Ordering::Relaxed);
            assert!(v6, "timeout: test_udp_v6 = {v6}");
            return;
        }
    }
    // recv_flag_c == true
    // all modifications to test_udp_v6_c must be visible
    let v6 = test_udp_v6_c.load(Ordering::Relaxed);
    assert!(v6);
}
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[cfg(feature = "async_tokio")]
#[tokio::test]
async fn test_udp_v4() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv4("10.26.1.100", 24, None)
        .build_async()
        .unwrap();

    let device = Arc::new(device);
    let _device = device.clone();
    let test_udp_v4 = Arc::new(AtomicBool::new(false));
    let test_udp_v4_c = test_udp_v4.clone();
    let recv_flag = Arc::new(AtomicBool::new(false));
    let recv_flag_c = recv_flag.clone();
    let handler = tokio::spawn(async move {
        let mut buf = [0; 65535];
        loop {
            let len = device.recv(&mut buf).await.unwrap();
            if let Some(ipv4_packet) = pnet_packet::ipv4::Ipv4Packet::new(&buf[..len]) {
                if ipv4_packet.get_next_level_protocol() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv4_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v4.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }
            if test_udp_v4.load(Ordering::Relaxed) {
                recv_flag.store(true, Ordering::Release);
                break;
            }
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;

    let udp_socket = tokio::net::UdpSocket::bind("10.26.1.200:0").await.unwrap();
    udp_socket
        .send_to(test_msg.as_bytes(), "10.26.1.101:8080")
        .await
        .unwrap();
    tokio::select! {
        _=tokio::time::sleep(Duration::from_secs(2))=>{
            // no promise due to the timeout
            let v4 = test_udp_v4_c.load(Ordering::Relaxed);
            assert!(v4, "timeout: test_udp_v4 = {v4}");
        }
        _=handler=>{
            // all modifications to test_udp_v4_c and test_udp_v6_c must be visible
            let flag = recv_flag_c.load(Ordering::Acquire); //synchronize
            assert!(flag, "recv_flag = {flag}");
            let v4 = test_udp_v4_c.load(Ordering::Relaxed);
            assert!(v4);
        }
    }
}
#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[cfg(feature = "async_tokio")]
#[tokio::test]
async fn test_udp_v6() {
    let test_msg = "test udp";
    let device = DeviceBuilder::new()
        .ipv6("fd12:3456:789a:1111:2222:3333:4444:5555", 64)
        .build_async()
        .unwrap();

    let device = Arc::new(device);
    let _device = device.clone();
    let test_udp_v6 = Arc::new(AtomicBool::new(false));
    let test_udp_v6_c = test_udp_v6.clone();
    let recv_flag = Arc::new(AtomicBool::new(false));
    let recv_flag_c = recv_flag.clone();
    let handler = tokio::spawn(async move {
        let mut buf = [0; 65535];
        loop {
            let len = device.recv(&mut buf).await.unwrap();
            if let Some(ipv6_packet) = pnet_packet::ipv6::Ipv6Packet::new(&buf[..len]) {
                if ipv6_packet.get_next_header() == IpNextHeaderProtocols::Udp {
                    if let Some(udp_packet) =
                        pnet_packet::udp::UdpPacket::new(ipv6_packet.payload())
                    {
                        if udp_packet.payload() == test_msg.as_bytes() {
                            test_udp_v6.store(true, Ordering::Relaxed);
                        }
                    }
                }
            }

            if test_udp_v6.load(Ordering::Relaxed) {
                recv_flag.store(true, Ordering::Release);
                break;
            }
        }
    });
    tokio::time::sleep(Duration::from_secs(6)).await;
    let udp_socket = tokio::net::UdpSocket::bind("[fd12:3456:789a:1111:2222:3333:4444:5555]:0")
        .await
        .unwrap();
    udp_socket
        .send_to(
            test_msg.as_bytes(),
            "[fd12:3456:789a:1111:2222:3333:4444:5556]:8080",
        )
        .await
        .unwrap();

    tokio::select! {
        _=tokio::time::sleep(Duration::from_secs(2))=>{
            // no promise due to the timeout
            let v6 = test_udp_v6_c.load(Ordering::Relaxed);
            assert!(v6, "timeout: test_udp_v6 = {v6}");
        }
        _=handler=>{
            // all modifications to test_udp_v4_c and test_udp_v6_c must be visible
            let flag = recv_flag_c.load(Ordering::Acquire); //synchronize
            assert!(flag, "recv_flag = {flag}");
            let v6 = test_udp_v6_c.load(Ordering::Relaxed);
            assert!(v6 );
        }
    }
}

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[test]
fn test_op() {
    let device = DeviceBuilder::new()
        .ipv4("10.26.2.100", 24, None)
        .ipv6("fd12:3456:789a:5555:2222:3333:4444:5555", 120)
        .build_sync()
        .unwrap();

    #[cfg(any(target_os = "macos", target_os = "openbsd"))]
    device.set_ignore_packet_info(true);
    #[cfg(any(target_os = "macos", target_os = "openbsd"))]
    assert!(device.ignore_packet_info());

    device.set_mtu(1800).unwrap();
    assert_eq!(device.mtu().unwrap(), 1800);

    #[cfg(target_os = "macos")]
    device.set_associate_route(true);
    #[cfg(target_os = "macos")]
    assert!(device.associate_route());

    let vec = device.addresses().unwrap();
    assert!(vec
        .iter()
        .any(|ip| *ip == "10.26.2.100".parse::<std::net::Ipv4Addr>().unwrap()));
    assert!(vec.iter().any(|ip| *ip
        == "fd12:3456:789a:5555:2222:3333:4444:5555"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()));

    device.set_network_address("10.26.3.200", 24, None).unwrap();
    let vec = device.addresses().unwrap();
    assert!(vec
        .iter()
        .any(|ip| *ip == "10.26.3.200".parse::<std::net::Ipv4Addr>().unwrap()));
    assert!(vec.iter().any(|ip| *ip
        == "fd12:3456:789a:5555:2222:3333:4444:5555"
            .parse::<std::net::Ipv6Addr>()
            .unwrap()));

    device.add_address_v4("10.6.0.1", 24).unwrap();
    let vec = device.addresses().unwrap();
    assert!(vec.contains(&"10.6.0.1".parse::<std::net::IpAddr>().unwrap()));

    device
        .remove_address("10.6.0.1".parse::<std::net::IpAddr>().unwrap())
        .unwrap();
    let vec = device.addresses().unwrap();
    assert!(!vec.contains(&"10.6.0.1".parse::<std::net::IpAddr>().unwrap()));

    device
        .add_address_v6("fdab:cdef:1234:5678:9abc:def0:1234:5678", 64)
        .unwrap();
    let vec = device.addresses().unwrap();
    assert!(vec.contains(
        &"fdab:cdef:1234:5678:9abc:def0:1234:5678"
            .parse::<std::net::IpAddr>()
            .unwrap()
    ));

    device.enabled(true).unwrap();

    #[cfg(any(
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos"))
    ))]
    device.set_name("tun666").unwrap();
    #[cfg(any(
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos"))
    ))]
    assert_eq!(device.name().unwrap(), "tun666");

    assert!(device.if_index().is_ok());

    #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
    assert!(device.is_running().unwrap());
}

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[test]
fn create_dev() {
    #[cfg(not(target_os = "macos"))]
    let name = "tun12";
    #[cfg(target_os = "macos")]
    let name = "utun12";

    let device = DeviceBuilder::new().name(name).build_sync().unwrap();
    let dev_name = device.name().unwrap();
    assert_eq!(dev_name.as_str(), name);
    #[cfg(unix)]
    {
        use std::os::fd::IntoRawFd;
        let fd = device.into_raw_fd();
        unsafe {
            let sync_device = SyncDevice::from_fd(fd).unwrap();
            let dev_name = sync_device.name().unwrap();
            assert_eq!(dev_name, name);
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn test_unix_fd() {
    use std::os::fd::IntoRawFd;
    let device = unsafe { SyncDevice::from_fd(1).unwrap() };
    let fd = device.into_raw_fd();
    assert_eq!(fd, 1)
}
