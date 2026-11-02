/// # Safety
unsafe fn sockaddr_to_rs_addr(sa: &sockaddr_union) -> Option<std::net::SocketAddr> {
    // SAFETY: `sa` contains initialized address data, and its family selects
    // the matching initialized union member.
    unsafe {
        match sa.addr_stor.ss_family as libc::c_int {
            libc::AF_INET => {
                let sa_in = sa.addr4;
                let ip = std::net::Ipv4Addr::from(sa_in.sin_addr.s_addr.to_ne_bytes());
                let port = u16::from_be(sa_in.sin_port);
                Some(std::net::SocketAddr::new(ip.into(), port))
            }
            libc::AF_INET6 => {
                let sa_in6 = sa.addr6;
                let ip = std::net::Ipv6Addr::from(sa_in6.sin6_addr.s6_addr);
                let port = u16::from_be(sa_in6.sin6_port);
                Some(std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                    ip,
                    port,
                    sa_in6.sin6_flowinfo,
                    sa_in6.sin6_scope_id,
                )))
            }
            _ => None,
        }
    }
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
pub(crate) fn sockaddr_v4_to_rs_addr(sa: &libc::sockaddr) -> Option<std::net::SocketAddr> {
    if libc::c_int::from(sa.sa_family) != libc::AF_INET {
        return None;
    }

    let data = sa.sa_data.map(|byte| byte.to_ne_bytes()[0]);
    let port = u16::from_be_bytes([data[0], data[1]]);
    let ip = std::net::Ipv4Addr::new(data[2], data[3], data[4], data[5]);
    Some(std::net::SocketAddr::new(ip.into(), port))
}

fn rs_addr_to_sockaddr(addr: std::net::SocketAddr) -> sockaddr_union {
    match addr {
        std::net::SocketAddr::V4(ipv4) => {
            let mut addr: sockaddr_union = unsafe { std::mem::zeroed() };
            #[cfg(any(
                target_os = "freebsd",
                target_os = "macos",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            {
                addr.addr4.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
            }
            addr.addr4.sin_family = libc::AF_INET as libc::sa_family_t;
            addr.addr4.sin_addr.s_addr = u32::from_ne_bytes(ipv4.ip().octets());
            addr.addr4.sin_port = ipv4.port().to_be();
            addr
        }
        std::net::SocketAddr::V6(ipv6) => {
            let mut addr: sockaddr_union = unsafe { std::mem::zeroed() };
            #[cfg(any(
                target_os = "freebsd",
                target_os = "macos",
                target_os = "openbsd",
                target_os = "netbsd"
            ))]
            {
                addr.addr6.sin6_len = std::mem::size_of::<libc::sockaddr_in6>() as u8;
            }
            addr.addr6.sin6_family = libc::AF_INET6 as libc::sa_family_t;
            addr.addr6.sin6_addr.s6_addr = ipv6.ip().octets();
            addr.addr6.sin6_port = ipv6.port().to_be();
            addr.addr6.sin6_flowinfo = ipv6.flowinfo();
            addr.addr6.sin6_scope_id = ipv6.scope_id();
            addr
        }
    }
}

/// # Safety
/// `dst` must point to writable storage for at least
/// `min(size, size_of::<sockaddr_union>())` bytes. The pointer must come from
/// the complete backing object being overwritten, not a narrower field
/// reference. The destination must not overlap the local source value.
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[allow(dead_code)]
pub(crate) unsafe fn ipaddr_to_sockaddr<T>(
    src_addr: T,
    src_port: u16,
    dst: *mut libc::c_void,
    size: usize,
) where
    T: Into<std::net::IpAddr>,
{
    let sa = rs_addr_to_sockaddr((src_addr.into(), src_port).into());
    let copy_len = size.min(std::mem::size_of::<sockaddr_union>());
    // SAFETY: `dst` is writable for `copy_len` bytes and does not overlap `sa`.
    unsafe {
        std::ptr::copy_nonoverlapping((&raw const sa).cast::<u8>(), dst.cast::<u8>(), copy_len);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union sockaddr_union {
    pub addr_stor: libc::sockaddr_storage,
    pub addr6: libc::sockaddr_in6,
    pub addr4: libc::sockaddr_in,
    pub addr: libc::sockaddr,
}

impl From<libc::sockaddr_storage> for sockaddr_union {
    fn from(addr: libc::sockaddr_storage) -> Self {
        sockaddr_union { addr_stor: addr }
    }
}

impl From<libc::sockaddr_in6> for sockaddr_union {
    fn from(addr: libc::sockaddr_in6) -> Self {
        sockaddr_union { addr6: addr }
    }
}

impl From<libc::sockaddr_in> for sockaddr_union {
    fn from(addr: libc::sockaddr_in) -> Self {
        sockaddr_union { addr4: addr }
    }
}

impl From<std::net::SocketAddr> for sockaddr_union {
    fn from(addr: std::net::SocketAddr) -> Self {
        rs_addr_to_sockaddr(addr)
    }
}

impl TryFrom<sockaddr_union> for std::net::SocketAddr {
    type Error = std::io::Error;

    fn try_from(addr: sockaddr_union) -> Result<Self, Self::Error> {
        unsafe { sockaddr_to_rs_addr(&addr).ok_or(std::io::ErrorKind::InvalidInput.into()) }
    }
}

impl<T: Into<std::net::IpAddr>> From<(T, u16)> for sockaddr_union {
    fn from((ip, port): (T, u16)) -> Self {
        let ip: std::net::IpAddr = ip.into();
        rs_addr_to_sockaddr(std::net::SocketAddr::new(ip, port))
    }
}

#[test]
fn test_conversion() {
    let old = std::net::SocketAddr::new([127, 0, 0, 1].into(), 0x0208);
    let addr = rs_addr_to_sockaddr(old);
    unsafe {
        if cfg!(target_endian = "big") {
            assert_eq!(0x7f000001, addr.addr4.sin_addr.s_addr);
            assert_eq!(0x0208, addr.addr4.sin_port);
        } else if cfg!(target_endian = "little") {
            assert_eq!(0x0100007f, addr.addr4.sin_addr.s_addr);
            assert_eq!(0x0802, addr.addr4.sin_port);
        } else {
            unreachable!();
        }
    };
    let ip = unsafe { sockaddr_to_rs_addr(&addr).unwrap() };
    assert_eq!(ip, old);

    let old = std::net::SocketAddr::new(std::net::Ipv6Addr::LOCALHOST.into(), 0x0208);
    let addr = rs_addr_to_sockaddr(old);
    let ip = unsafe { sockaddr_to_rs_addr(&addr).unwrap() };
    assert_eq!(ip, old);
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        let old = std::net::IpAddr::V4([10, 0, 0, 33].into());
        let mut addr: sockaddr_union = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::sockaddr_in>();

        unsafe { ipaddr_to_sockaddr(old, 0x0208, (&raw mut addr).cast(), size) };
        let ip = unsafe { sockaddr_to_rs_addr(&addr).unwrap() };
        assert_eq!(ip, std::net::SocketAddr::new(old, 0x0208));
    }
}

#[test]
fn ipv6_conversion_preserves_flowinfo_and_scope_id() {
    let old = std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
        std::net::Ipv6Addr::LOCALHOST,
        0x0208,
        0x0123_4567,
        17,
    ));
    let addr = rs_addr_to_sockaddr(old);
    let actual = unsafe { sockaddr_to_rs_addr(&addr).unwrap() };
    assert_eq!(actual, old);
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[test]
fn ipaddr_to_sockaddr_writes_through_complete_ifreq_union_storage() {
    let old = std::net::IpAddr::V4([10, 26, 1, 100].into());
    let mut ifru: libc::__c_anonymous_ifr_ifru = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::__c_anonymous_ifr_ifru>();

    unsafe { ipaddr_to_sockaddr(old, 0x0208, (&raw mut ifru).cast(), size) };

    let raw = unsafe { ifru.ifru_addr };
    let actual = sockaddr_v4_to_rs_addr(&raw).expect("IPv4 sockaddr must decode");
    assert_eq!(actual, std::net::SocketAddr::new(old, 0x0208));
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[test]
fn generic_sockaddr_decoder_rejects_non_ipv4_families() {
    let raw = libc::sockaddr {
        sa_family: libc::sa_family_t::try_from(libc::AF_INET6).expect("AF_INET6 fits sa_family_t"),
        sa_data: [0; 14],
    };

    assert_eq!(sockaddr_v4_to_rs_addr(&raw), None);
}
