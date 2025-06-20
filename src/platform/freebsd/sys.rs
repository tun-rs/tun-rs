use libc::{c_char, c_int, c_uint, ifreq, sockaddr, sockaddr_in6, time_t, IFNAMSIZ};
use nix::{ioctl_readwrite, ioctl_write_ptr};
use std::ffi::c_void;

pub const IN6_IFF_NODAD: i32 = 0x0020;

#[allow(dead_code)]
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct ctl_info {
    pub ctl_id: c_uint,
    pub ctl_name: [c_char; 96],
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct ifaliasreq {
    pub ifran: [c_char; IFNAMSIZ],
    pub addr: sockaddr,
    pub dstaddr: sockaddr,
    pub mask: sockaddr,
    pub ifra_vhid: c_int,
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct in6_ifaliasreq {
    pub ifra_name: [c_char; IFNAMSIZ],
    pub ifra_addr: sockaddr_in6,
    pub ifra_dstaddr: sockaddr_in6,
    pub ifra_prefixmask: sockaddr_in6,
    pub ifra_flags: libc::c_int,
    pub in6_addrlifetime: in6_addrlifetime,
    pub ifra_vhid: libc::c_int,
}

// #[allow(non_camel_case_types)]
// #[repr(C)]
// #[derive(Copy, Clone)]
// pub struct in_aliasreq  {
//     pub ifra_name: [c_char; IFNAMSIZ],
//     pub ifra_addr: sockaddr_in,
//     pub ifra_dstaddr: sockaddr_in,
//     pub ifra_mask: sockaddr_in,
// 	pub ifra_vhid:c_int
// }

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct in6_ifreq {
    pub ifra_name: [c_char; IFNAMSIZ],
    pub ifr_ifru: ifr_ifru_in6,
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub union ifr_ifru_in6 {
    pub ifru_addr: sockaddr_in6,
    pub ifru_dstaddr: sockaddr_in6,
    pub ifru_flags: c_int,
    pub ifru_flags6: c_int,
    pub ifru_metric: c_int,
    pub ifru_data: *const c_void,
    pub ifru_lifetime: in6_addrlifetime,
    pub ifru_stat: in6_ifstat,
    pub ifru_icmp6stat: icmp6_ifstat,
    pub ifru_scope_id: [u32; 16],
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct in6_addrlifetime {
    pub ia6t_expire: time_t,    /* valid lifetime expiration time */
    pub ia6t_preferred: time_t, /* preferred lifetime expiration time */
    pub ia6t_vltime: u32,       /* valid lifetime */
    pub ia6t_pltime: u32,       /* prefix lifetime */
}

#[allow(non_camel_case_types)]
type u_quad_t = u64;
#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct in6_ifstat {
    pub ifs6_in_receive: u_quad_t,      /* # of total input datagram */
    pub ifs6_in_hdrerr: u_quad_t,       /* # of datagrams with invalid hdr */
    pub ifs6_in_toobig: u_quad_t,       /* # of datagrams exceeded MTU */
    pub ifs6_in_noroute: u_quad_t,      /* # of datagrams with no route */
    pub ifs6_in_addrerr: u_quad_t,      /* # of datagrams with invalid dst */
    pub ifs6_in_protounknown: u_quad_t, /* # of datagrams with unknown proto */
    /* NOTE: increment on final dst if */
    pub ifs6_in_truncated: u_quad_t, /* # of truncated datagrams */
    pub ifs6_in_discard: u_quad_t,   /* # of discarded datagrams */
    /* NOTE: fragment timeout is not here */
    pub ifs6_in_deliver: u_quad_t, /* # of datagrams delivered to ULP */
    /* NOTE: increment on final dst if */
    pub ifs6_out_forward: u_quad_t, /* # of datagrams forwarded */
    /* NOTE: increment on outgoing if */
    pub ifs6_out_request: u_quad_t, /* # of outgoing datagrams from ULP */
    /* NOTE: does not include forwrads */
    pub ifs6_out_discard: u_quad_t,   /* # of discarded datagrams */
    pub ifs6_out_fragok: u_quad_t,    /* # of datagrams fragmented */
    pub ifs6_out_fragfail: u_quad_t,  /* # of datagrams failed on fragment */
    pub ifs6_out_fragcreat: u_quad_t, /* # of fragment datagrams */
    /* NOTE: this is # after fragment */
    pub ifs6_reass_reqd: u_quad_t, /* # of incoming fragmented packets */
    /* NOTE: increment on final dst if */
    pub ifs6_reass_ok: u_quad_t, /* # of reassembled packets */
    /* NOTE: this is # after reass */
    /* NOTE: increment on final dst if */
    pub ifs6_reass_fail: u_quad_t, /* # of reass failures */
    /* NOTE: may not be packet count */
    /* NOTE: increment on final dst if */
    pub ifs6_in_mcast: u_quad_t,  /* # of inbound multicast datagrams */
    pub ifs6_out_mcast: u_quad_t, /* # of outbound multicast datagrams */
}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Copy, Clone)]
pub struct icmp6_ifstat {
    /*
     * Input statistics
     */
    /* ipv6IfIcmpInMsgs, total # of input messages */
    pub ifs6_in_msg: u_quad_t,
    /* ipv6IfIcmpInErrors, # of input error messages */
    pub ifs6_in_error: u_quad_t,
    /* ipv6IfIcmpInDestUnreachs, # of input dest unreach errors */
    pub ifs6_in_dstunreach: u_quad_t,
    /* ipv6IfIcmpInAdminProhibs, # of input administratively prohibited errs */
    pub ifs6_in_adminprohib: u_quad_t,
    /* ipv6IfIcmpInTimeExcds, # of input time exceeded errors */
    pub ifs6_in_timeexceed: u_quad_t,
    /* ipv6IfIcmpInParmProblems, # of input parameter problem errors */
    pub ifs6_in_paramprob: u_quad_t,
    /* ipv6IfIcmpInPktTooBigs, # of input packet too big errors */
    pub ifs6_in_pkttoobig: u_quad_t,
    /* ipv6IfIcmpInEchos, # of input echo requests */
    pub ifs6_in_echo: u_quad_t,
    /* ipv6IfIcmpInEchoReplies, # of input echo replies */
    pub ifs6_in_echoreply: u_quad_t,
    /* ipv6IfIcmpInRouterSolicits, # of input router solicitations */
    pub ifs6_in_routersolicit: u_quad_t,
    /* ipv6IfIcmpInRouterAdvertisements, # of input router advertisements */
    pub ifs6_in_routeradvert: u_quad_t,
    /* ipv6IfIcmpInNeighborSolicits, # of input neighbor solicitations */
    pub ifs6_in_neighborsolicit: u_quad_t,
    /* ipv6IfIcmpInNeighborAdvertisements, # of input neighbor advertisements */
    pub ifs6_in_neighboradvert: u_quad_t,
    /* ipv6IfIcmpInRedirects, # of input redirects */
    pub ifs6_in_redirect: u_quad_t,
    /* ipv6IfIcmpInGroupMembQueries, # of input MLD queries */
    pub ifs6_in_mldquery: u_quad_t,
    /* ipv6IfIcmpInGroupMembResponses, # of input MLD reports */
    pub ifs6_in_mldreport: u_quad_t,
    /* ipv6IfIcmpInGroupMembReductions, # of input MLD done */
    pub ifs6_in_mlddone: u_quad_t,

    /*
     * Output statistics. We should solve unresolved routing problem...
     */
    /* ipv6IfIcmpOutMsgs, total # of output messages */
    pub ifs6_out_msg: u_quad_t,
    /* ipv6IfIcmpOutErrors, # of output error messages */
    pub ifs6_out_error: u_quad_t,
    /* ipv6IfIcmpOutDestUnreachs, # of output dest unreach errors */
    pub ifs6_out_dstunreach: u_quad_t,
    /* ipv6IfIcmpOutAdminProhibs, # of output administratively prohibited errs */
    pub ifs6_out_adminprohib: u_quad_t,
    /* ipv6IfIcmpOutTimeExcds, # of output time exceeded errors */
    pub ifs6_out_timeexceed: u_quad_t,
    /* ipv6IfIcmpOutParmProblems, # of output parameter problem errors */
    pub ifs6_out_paramprob: u_quad_t,
    /* ipv6IfIcmpOutPktTooBigs, # of output packet too big errors */
    pub ifs6_out_pkttoobig: u_quad_t,
    /* ipv6IfIcmpOutEchos, # of output echo requests */
    pub ifs6_out_echo: u_quad_t,
    /* ipv6IfIcmpOutEchoReplies, # of output echo replies */
    pub ifs6_out_echoreply: u_quad_t,
    /* ipv6IfIcmpOutRouterSolicits, # of output router solicitations */
    pub ifs6_out_routersolicit: u_quad_t,
    /* ipv6IfIcmpOutRouterAdvertisements, # of output router advertisements */
    pub ifs6_out_routeradvert: u_quad_t,
    /* ipv6IfIcmpOutNeighborSolicits, # of output neighbor solicitations */
    pub ifs6_out_neighborsolicit: u_quad_t,
    /* ipv6IfIcmpOutNeighborAdvertisements, # of output neighbor advertisements */
    pub ifs6_out_neighboradvert: u_quad_t,
    /* ipv6IfIcmpOutRedirects, # of output redirects */
    pub ifs6_out_redirect: u_quad_t,
    /* ipv6IfIcmpOutGroupMembQueries, # of output MLD queries */
    pub ifs6_out_mldquery: u_quad_t,
    /* ipv6IfIcmpOutGroupMembResponses, # of output MLD reports */
    pub ifs6_out_mldreport: u_quad_t,
    /* ipv6IfIcmpOutGroupMembReductions, # of output MLD done */
    pub ifs6_out_mlddone: u_quad_t,
}

ioctl_write_ptr!(siocsifflags, b'i', 16, ifreq);
ioctl_readwrite!(siocgifflags, b'i', 17, ifreq);

ioctl_write_ptr!(siocsifaddr, b'i', 12, ifreq);
ioctl_readwrite!(siocgifaddr, b'i', 33, ifreq);

ioctl_write_ptr!(siocsifdstaddr, b'i', 14, ifreq);
ioctl_readwrite!(siocgifdstaddr, b'i', 34, ifreq);

ioctl_write_ptr!(siocsifbrdaddr, b'i', 19, ifreq);
ioctl_readwrite!(siocgifbrdaddr, b'i', 35, ifreq);

ioctl_write_ptr!(siocsifnetmask, b'i', 22, ifreq);
ioctl_readwrite!(siocgifnetmask, b'i', 37, ifreq);

ioctl_write_ptr!(siocsifmtu, b'i', 52, ifreq);
ioctl_readwrite!(siocgifmtu, b'i', 51, ifreq);

ioctl_write_ptr!(siocaifaddr, b'i', 43, ifaliasreq);
ioctl_write_ptr!(siocdifaddr, b'i', 25, ifreq);

ioctl_write_ptr!(siocifcreate, b'i', 122, ifreq);

ioctl_write_ptr!(siocsifphyaddr, b'i', 70, ifaliasreq);

ioctl_write_ptr!(siocsifname, b'i', 40, ifreq);

ioctl_write_ptr!(siocsiflladdr, b'i', 60, ifreq);

ioctl_write_ptr!(siocifdestroy, b'i', 121, ifreq);

ioctl_write_ptr!(siocdifaddr_in6, b'i', 25, in6_ifreq);

ioctl_write_ptr!(siocaifaddr_in6, b'i', 27, in6_ifaliasreq);
