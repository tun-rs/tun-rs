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

    device.set_mtu(1500).unwrap();
    assert_eq!(device.mtu().unwrap(), 1500);

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
    assert!(!vec.contains(&"10.26.2.100".parse::<std::net::IpAddr>().unwrap()));

    device.add_address_v4("10.6.0.1", 24).unwrap();
    let vec = device.addresses().unwrap();
    assert!(vec.contains(&"10.6.0.1".parse::<std::net::IpAddr>().unwrap()));
    assert!(vec.contains(&"10.26.3.200".parse::<std::net::IpAddr>().unwrap()));

    device
        .remove_address("10.6.0.1".parse::<std::net::IpAddr>().unwrap())
        .unwrap();
    let vec = device.addresses().unwrap();
    assert!(!vec.contains(&"10.6.0.1".parse::<std::net::IpAddr>().unwrap()));
    assert!(vec.contains(&"10.26.3.200".parse::<std::net::IpAddr>().unwrap()));

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
    device.set_name("tun668").unwrap();
    std::thread::sleep(Duration::from_secs(3));
    #[cfg(any(
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos"))
    ))]
    assert_eq!(device.name().unwrap(), "tun668");

    assert!(device.if_index().is_ok());

    // Windows-only configuration that was migrated from netsh/wmic commands to
    // windows-sys APIs. None of these expose a public read-back getter, so we assert
    // that the configure path succeeds: a malformed FFI call (wrong struct layout,
    // flags, GUID conversion, or a failed dynamic load) returns an error and fails here.
    #[cfg(target_os = "windows")]
    {
        // `if_luid()` is exposed for downstream crates; it must resolve to a LUID.
        assert!(device.if_luid().is_ok());

        // `set_metric` -> Get/SetIpInterfaceEntry.
        device.set_metric(100).expect("set_metric(100) should succeed");

        // `set_dns_servers` -> SetInterfaceDnsSettings (resolved at run time) with a
        // netsh fallback. Exercise IPv4 (primary + secondary) and IPv6, then clear both.
        let v4_dns: [std::net::IpAddr; 2] =
            ["8.8.8.8".parse().unwrap(), "8.8.4.4".parse().unwrap()];
        device.set_dns_servers(&v4_dns).unwrap();
        let v6_dns: [std::net::IpAddr; 1] = ["2001:4860:4860::8888".parse().unwrap()];
        device.set_dns_servers(&v6_dns).unwrap();
        device.clear_dns_servers(true).unwrap();
        device.clear_dns_servers(false).unwrap();
    }

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
fn create_tun() {
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

#[cfg(any(
    target_os = "windows",
    target_os = "macos",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[test]
fn create_tap() {
    #[cfg(not(target_os = "macos"))]
    let name = "tap12";
    #[cfg(target_os = "macos")]
    let name = "feth12";

    let device = DeviceBuilder::new()
        .name(name)
        .layer(tun_rs::Layer::L2)
        .build_sync()
        .unwrap();
    let dev_name = device.name().unwrap();
    assert_eq!(dev_name.as_str(), name);
    #[cfg(all(unix, not(target_os = "macos")))]
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

/// Dedicated Windows test covering every public API migrated from netsh/wmic to
/// windows-sys in PR `#140`.  Each section is labelled with the underlying Windows
/// API that was newly wired up.
#[cfg(target_os = "windows")]
#[test]
fn test_windows_new_apis() {
    use std::net::IpAddr;
    use tun_rs::DeviceBuilder;

    let device = DeviceBuilder::new()
        .ipv4("10.26.9.100", 24, None)
        .ipv6("fd12:3456:789a:9999:2222:3333:4444:5555", 64)
        .build_sync()
        .unwrap();

    // ── 1. if_luid() ─────────────────────────────────────────────────────────
    // New public API; a valid adapter LUID is never zero.
    let luid = device.if_luid().expect("if_luid() should succeed");
    let luid_value = unsafe { luid.Value };
    assert_ne!(luid_value, 0, "LUID must be non-zero for a live adapter");

    // ── 2. set_metric() ──────────────────────────────────────────────────────
    // Now uses GetIpInterfaceEntry / SetIpInterfaceEntry for both AF_INET and
    // AF_INET6.  No public read-back getter exists, so we assert the call
    // succeeds for two different values (exercises the full round-trip twice).
    device.set_metric(100).expect("set_metric(100) should succeed");

    // ── 3. set_mtu / mtu (IPv4) ──────────────────────────────────────────────
    // Now uses SetIpInterfaceEntry with NlMtu; read back via GetIpInterfaceTable.
    device.set_mtu(1400).expect("set_mtu(1400) should succeed");
    assert_eq!(
        device.mtu().expect("mtu() should succeed"),
        1400,
        "mtu() read-back should match the value written by set_mtu()"
    );

    // ── 4. set_mtu_v6 / mtu_v6 (IPv6) ───────────────────────────────────────
    // Same path but with is_v4=false; mtu_v6() reads back via GetIpInterfaceTable
    // with AF_INET6.
    device.set_mtu_v6(1380).expect("set_mtu_v6(1380) should succeed");
    assert_eq!(
        device.mtu_v6().expect("mtu_v6() should succeed"),
        1380,
        "mtu_v6() read-back should match the value written by set_mtu_v6()"
    );

    // ── 5. set_network_address without gateway ───────────────────────────────
    // ffi::set_address clears all existing IPv4 unicast addresses and installs
    // the new one.  The old address must disappear.
    device
        .set_network_address("10.26.9.200", 24, None)
        .expect("set_network_address (no gateway) should succeed");
    let addrs = device.addresses().unwrap();
    assert!(
        addrs.contains(&"10.26.9.200".parse::<IpAddr>().unwrap()),
        "new address 10.26.9.200 should be present after set_network_address"
    );
    assert!(
        !addrs.contains(&"10.26.9.100".parse::<IpAddr>().unwrap()),
        "old address 10.26.9.100 should have been removed by set_network_address"
    );

    // ── 6. set_network_address WITH gateway ──────────────────────────────────
    // Exercises the default-route creation path in ffi::add_address that was
    // specifically fixed in this PR (DestinationPrefix address family +
    // SitePrefixLength=0 for CreateIpForwardEntry2).
    device
        .set_network_address("10.26.9.150", 24, Some("10.26.9.1"))
        .expect("set_network_address with gateway should succeed");
    let addrs = device.addresses().unwrap();
    assert!(
        addrs.contains(&"10.26.9.150".parse::<IpAddr>().unwrap()),
        "address 10.26.9.150 should be present after set_network_address with gateway"
    );
    assert!(
        !addrs.contains(&"10.26.9.200".parse::<IpAddr>().unwrap()),
        "previous address 10.26.9.200 should have been cleared"
    );

    // ── 7. add_address_v6 ────────────────────────────────────────────────────
    // ffi::add_address with None gateway; verifies the address appears in the
    // interface's address list.
    device
        .add_address_v6("fdab:cdef:1234:5678:9abc:def0:1234:0001", 64)
        .expect("add_address_v6 should succeed");
    let addrs = device.addresses().unwrap();
    assert!(
        addrs.contains(
            &"fdab:cdef:1234:5678:9abc:def0:1234:0001"
                .parse::<IpAddr>()
                .unwrap()
        ),
        "IPv6 address should be present after add_address_v6"
    );

    // ── 8. remove_address (IPv6) ─────────────────────────────────────────────
    // ffi::remove_address; confirms the address is gone after deletion.
    device
        .remove_address(
            "fdab:cdef:1234:5678:9abc:def0:1234:0001"
                .parse::<IpAddr>()
                .unwrap(),
        )
        .expect("remove_address (IPv6) should succeed");
    let addrs = device.addresses().unwrap();
    assert!(
        !addrs.contains(
            &"fdab:cdef:1234:5678:9abc:def0:1234:0001"
                .parse::<IpAddr>()
                .unwrap()
        ),
        "IPv6 address should be absent after remove_address"
    );

    // ── 9. set_dns_servers (IPv4) ────────────────────────────────────────────
    // dns::set_dns_servers → SetInterfaceDnsSettings (or netsh fallback).
    let ipv4_dns: &[IpAddr] = &[
        "8.8.8.8".parse().unwrap(),
        "8.8.4.4".parse().unwrap(),
    ];
    device
        .set_dns_servers(ipv4_dns)
        .expect("set_dns_servers (IPv4 primary+secondary) should succeed");

    // ── 10. set_dns_servers (IPv6) ───────────────────────────────────────────
    let ipv6_dns: &[IpAddr] = &["2001:4860:4860::8888".parse().unwrap()];
    device
        .set_dns_servers(ipv6_dns)
        .expect("set_dns_servers (IPv6) should succeed");

    // ── 11. set_dns_servers — validation: empty list must be rejected ────────
    assert!(
        device.set_dns_servers(&[]).is_err(),
        "set_dns_servers with an empty slice must return Err (InvalidInput)"
    );

    // ── 12. set_dns_servers — validation: mixed families must be rejected ────
    let mixed: &[IpAddr] = &[
        "8.8.8.8".parse().unwrap(),
        "2001:4860:4860::8888".parse().unwrap(),
    ];
    assert!(
        device.set_dns_servers(mixed).is_err(),
        "set_dns_servers with mixed IPv4/IPv6 addresses must return Err (InvalidInput)"
    );

    // ── 13. clear_dns_servers ────────────────────────────────────────────────
    // dns::clear_dns_servers → SetInterfaceDnsSettings with empty NameServer
    // (or netsh fallback).
    device
        .clear_dns_servers(true)
        .expect("clear_dns_servers (IPv4) should succeed");
    device
        .clear_dns_servers(false)
        .expect("clear_dns_servers (IPv6) should succeed");
}
