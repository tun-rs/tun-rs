use crate::platform::linux::offload::{
    gso_none_checksum, gso_split, handle_gro, VirtioNetHdr, VIRTIO_NET_HDR_F_NEEDS_CSUM,
    VIRTIO_NET_HDR_GSO_NONE, VIRTIO_NET_HDR_GSO_TCPV4, VIRTIO_NET_HDR_GSO_TCPV6,
    VIRTIO_NET_HDR_GSO_UDP_L4, VIRTIO_NET_HDR_LEN,
};
use crate::platform::unix::device::{ctl, ctl_v6};
use crate::platform::{ExpandBuffer, GROTable};
use crate::{
    builder::{DeviceConfig, Layer},
    platform::linux::sys::*,
    platform::{
        unix::{ipaddr_to_sockaddr, sockaddr_union, Fd, Tun},
        ETHER_ADDR_LEN,
    },
    ToIpv4Address, ToIpv4Netmask, ToIpv6Address, ToIpv6Netmask,
};
use libc::{
    self, c_char, c_short, ifreq, in6_ifreq, ARPHRD_ETHER, IFF_MULTI_QUEUE, IFF_NO_PI, IFF_RUNNING,
    IFF_TAP, IFF_TUN, IFF_UP, IFNAMSIZ, O_RDWR,
};
use mac_address::mac_address_by_name;
use std::net::Ipv6Addr;
use std::{
    ffi::CString,
    io, mem,
    net::{IpAddr, Ipv4Addr},
    os::unix::io::{AsRawFd, RawFd},
    ptr,
};

const OVERWRITE_SIZE: usize = mem::size_of::<libc::__c_anonymous_ifr_ifru>();

/// A TUN device using the TUN/TAP Linux driver.
pub struct DeviceImpl {
    pub(crate) tun: Tun,
    pub(crate) vnet_hdr: bool,
    pub(crate) udp_gso: bool,
    flags: c_short,
}

impl DeviceImpl {
    /// Create a new `Device` for the given `Configuration`.
    pub(crate) fn new(config: DeviceConfig) -> std::io::Result<Self> {
        let dev_name = match config.dev_name.as_ref() {
            Some(tun_name) => {
                let tun_name = CString::new(tun_name.clone())?;

                if tun_name.as_bytes_with_nul().len() > IFNAMSIZ {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "device name too long",
                    ));
                }

                Some(tun_name)
            }

            None => None,
        };
        unsafe {
            let mut req: ifreq = mem::zeroed();

            if let Some(dev_name) = dev_name.as_ref() {
                ptr::copy_nonoverlapping(
                    dev_name.as_ptr() as *const c_char,
                    req.ifr_name.as_mut_ptr(),
                    dev_name.as_bytes_with_nul().len(),
                );
            }
            let multi_queue = config.multi_queue.unwrap_or(false);
            let device_type: c_short = config.layer.unwrap_or(Layer::L3).into();
            let iff_no_pi = IFF_NO_PI as c_short;
            let iff_vnet_hdr = libc::IFF_VNET_HDR as c_short;
            let iff_multi_queue = IFF_MULTI_QUEUE as c_short;
            let packet_information = config.packet_information.unwrap_or(false);
            let offload = config.offload.unwrap_or(false);
            req.ifr_ifru.ifru_flags = device_type
                | if packet_information { 0 } else { iff_no_pi }
                | if multi_queue { iff_multi_queue } else { 0 }
                | if offload { iff_vnet_hdr } else { 0 };

            let fd = libc::open(c"/dev/net/tun".as_ptr() as *const _, O_RDWR, 0);
            let tun_fd = Fd::new(fd)?;
            if let Err(err) = tunsetiff(tun_fd.inner, &mut req as *mut _ as *mut _) {
                return Err(io::Error::from(err));
            }
            let (vnet_hdr, udp_gso) = if offload && libc::IFF_VNET_HDR != 0 {
                // tunTCPOffloads were added in Linux v2.6. We require their support if IFF_VNET_HDR is set.
                let tun_tcp_offloads = libc::TUN_F_CSUM | libc::TUN_F_TSO4 | libc::TUN_F_TSO6;
                let tun_udp_offloads = libc::TUN_F_USO4 | libc::TUN_F_USO6;
                if let Err(err) = tunsetoffload(tun_fd.inner, tun_tcp_offloads as _) {
                    log::warn!("unsupported offload: {err:?}");
                    (false, false)
                } else {
                    // tunUDPOffloads were added in Linux v6.2. We do not return an
                    // error if they are unsupported at runtime.
                    let rs =
                        tunsetoffload(tun_fd.inner, (tun_tcp_offloads | tun_udp_offloads) as _);
                    (true, rs.is_ok())
                }
            } else {
                (false, false)
            };

            let device = DeviceImpl {
                tun: Tun::new(tun_fd),
                vnet_hdr,
                udp_gso,
                flags: req.ifr_ifru.ifru_flags,
            };
            Ok(device)
        }
    }
    unsafe fn set_tcp_offloads(&self) -> io::Result<()> {
        let tun_tcp_offloads = libc::TUN_F_CSUM | libc::TUN_F_TSO4 | libc::TUN_F_TSO6;
        tunsetoffload(self.as_raw_fd(), tun_tcp_offloads as _)
            .map(|_| ())
            .map_err(|e| e.into())
    }
    unsafe fn set_tcp_udp_offloads(&self) -> io::Result<()> {
        let tun_tcp_offloads = libc::TUN_F_CSUM | libc::TUN_F_TSO4 | libc::TUN_F_TSO6;
        let tun_udp_offloads = libc::TUN_F_USO4 | libc::TUN_F_USO6;
        tunsetoffload(self.as_raw_fd(), (tun_tcp_offloads | tun_udp_offloads) as _)
            .map(|_| ())
            .map_err(|e| e.into())
    }
    pub(crate) fn from_tun(tun: Tun) -> Self {
        Self {
            tun,
            vnet_hdr: false,
            udp_gso: false,
            flags: 0,
        }
    }

    /// # Prerequisites
    /// - The `IFF_MULTI_QUEUE` flag must be enabled.
    /// - The system must support network interface multi-queue functionality.
    ///
    /// # Description
    /// When multi-queue is enabled, create a new queue by duplicating an existing one.
    pub fn try_clone(&self) -> io::Result<DeviceImpl> {
        let flags = self.flags;
        if flags & (IFF_MULTI_QUEUE as c_short) != IFF_MULTI_QUEUE as c_short {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "iff_multi_queue not enabled",
            ));
        }
        unsafe {
            let mut req = self.request()?;
            req.ifr_ifru.ifru_flags = flags;
            let fd = libc::open(c"/dev/net/tun".as_ptr() as *const _, O_RDWR);
            let tun_fd = Fd::new(fd)?;
            if let Err(err) = tunsetiff(tun_fd.inner, &mut req as *mut _ as *mut _) {
                return Err(io::Error::from(err));
            }
            let dev = DeviceImpl {
                tun: Tun::new(tun_fd),
                vnet_hdr: self.vnet_hdr,
                udp_gso: self.udp_gso,
                flags,
            };
            if dev.vnet_hdr {
                if dev.udp_gso {
                    dev.set_tcp_udp_offloads()?
                } else {
                    dev.set_tcp_offloads()?;
                }
            }

            Ok(dev)
        }
    }
    /// Returns whether UDP Generic Segmentation Offload (GSO) is enabled.
    ///
    /// This is determined by the `udp_gso` flag in the device.
    pub fn udp_gso(&self) -> bool {
        self.udp_gso
    }
    /// Returns whether TCP Generic Segmentation Offload (GSO) is enabled.
    ///
    /// In this implementation, this is represented by the `vnet_hdr` flag.
    pub fn tcp_gso(&self) -> bool {
        self.vnet_hdr
    }
    /// Sets the transmit queue length for the network interface.
    ///
    /// This method constructs an interface request (`ifreq`) structure,
    /// assigns the desired transmit queue length to the `ifru_metric` field,
    /// and calls the `change_tx_queue_len` function using the control file descriptor.
    /// If the underlying operation fails, an I/O error is returned.
    pub fn set_tx_queue_len(&self, tx_queue_len: u32) -> io::Result<()> {
        unsafe {
            let mut ifreq = self.request()?;
            ifreq.ifr_ifru.ifru_metric = tx_queue_len as _;
            if let Err(err) = change_tx_queue_len(ctl()?.as_raw_fd(), &ifreq) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    /// Retrieves the current transmit queue length for the network interface.
    ///
    /// This function constructs an interface request structure and calls `tx_queue_len`
    /// to populate it with the current transmit queue length. The value is then returned.
    pub fn tx_queue_len(&self) -> io::Result<u32> {
        unsafe {
            let mut ifreq = self.request()?;
            if let Err(err) = tx_queue_len(ctl()?.as_raw_fd(), &mut ifreq) {
                return Err(io::Error::from(err));
            }
            Ok(ifreq.ifr_ifru.ifru_metric as _)
        }
    }
    /// Make the device persistent.
    pub fn persist(&self) -> io::Result<()> {
        unsafe {
            if let Err(err) = tunsetpersist(self.as_raw_fd(), &1) {
                Err(io::Error::from(err))
            } else {
                Ok(())
            }
        }
    }

    /// Set the owner of the device.
    pub fn user(&self, value: i32) -> io::Result<()> {
        unsafe {
            if let Err(err) = tunsetowner(self.as_raw_fd(), &value) {
                Err(io::Error::from(err))
            } else {
                Ok(())
            }
        }
    }

    /// Set the group of the device.
    pub fn group(&self, value: i32) -> io::Result<()> {
        unsafe {
            if let Err(err) = tunsetgroup(self.as_raw_fd(), &value) {
                Err(io::Error::from(err))
            } else {
                Ok(())
            }
        }
    }
    /// send multiple fragmented data packets.
    /// GROTable can be reused, as it is used to assist in data merging.
    /// Offset is the starting position of the data. Need to meet offset>=10.
    pub fn send_multiple<B: ExpandBuffer>(
        &self,
        gro_table: &mut GROTable,
        bufs: &mut [B],
        mut offset: usize,
    ) -> io::Result<usize> {
        gro_table.reset();
        if self.vnet_hdr {
            handle_gro(
                bufs,
                offset,
                &mut gro_table.tcp_gro_table,
                &mut gro_table.udp_gro_table,
                self.udp_gso,
                &mut gro_table.to_write,
            )?;
            offset -= VIRTIO_NET_HDR_LEN;
        } else {
            for i in 0..bufs.len() {
                gro_table.to_write.push(i);
            }
        }

        let mut total = 0;
        let mut err = Ok(());
        for buf_idx in &gro_table.to_write {
            match self.send(&bufs[*buf_idx].as_ref()[offset..]) {
                Ok(n) => {
                    total += n;
                }
                Err(e) => {
                    if let Some(code) = e.raw_os_error() {
                        if libc::EBADFD == code {
                            return Err(e);
                        }
                    }
                    err = Err(e)
                }
            }
        }
        err?;
        Ok(total)
    }
    /// Recv a packet from tun device.
    /// If offload is enabled. This method can be used to obtain processed data.
    ///
    /// original_buffer is used to store raw data, including the VirtioNetHdr and the unsplit IP packet. The recommended size is 10 + 65535.
    /// bufs and sizes are used to store the segmented IP packets. bufs.len == sizes.len > 65535/MTU
    /// offset: Starting position
    pub fn recv_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &self,
        original_buffer: &mut [u8],
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        if bufs.is_empty() || bufs.len() != sizes.len() {
            return Err(io::Error::new(io::ErrorKind::Other, "bufs error"));
        }
        if self.vnet_hdr {
            let len = self.recv(original_buffer)?;
            if len <= VIRTIO_NET_HDR_LEN {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "length of packet ({len}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                    ),
                ))?
            }
            let hdr = VirtioNetHdr::decode(&original_buffer[..VIRTIO_NET_HDR_LEN])?;
            self.handle_virtio_read(
                hdr,
                &mut original_buffer[VIRTIO_NET_HDR_LEN..len],
                bufs,
                sizes,
                offset,
            )
        } else {
            let len = self.recv(bufs[0].as_mut())?;
            sizes[0] = len;
            Ok(1)
        }
    }
    /// https://github.com/WireGuard/wireguard-go/blob/12269c2761734b15625017d8565745096325392f/tun/tun_linux.go#L375
    /// handleVirtioRead splits in into bufs, leaving offset bytes at the front of
    /// each buffer. It mutates sizes to reflect the size of each element of bufs,
    /// and returns the number of packets read.
    pub(crate) fn handle_virtio_read<B: AsRef<[u8]> + AsMut<[u8]>>(
        &self,
        mut hdr: VirtioNetHdr,
        input: &mut [u8],
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        let len = input.len();
        if hdr.gso_type == VIRTIO_NET_HDR_GSO_NONE {
            if hdr.flags & VIRTIO_NET_HDR_F_NEEDS_CSUM != 0 {
                // This means CHECKSUM_PARTIAL in skb context. We are responsible
                // for computing the checksum starting at hdr.csumStart and placing
                // at hdr.csumOffset.
                gso_none_checksum(input, hdr.csum_start, hdr.csum_offset);
            }
            if bufs[0].as_ref()[offset..].len() < len {
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!(
                        "read len {len} overflows bufs element len {}",
                        bufs[0].as_ref().len()
                    ),
                ))?
            }
            sizes[0] = len;
            bufs[0].as_mut()[offset..offset + len].copy_from_slice(input);
            return Ok(1);
        }
        if hdr.gso_type != VIRTIO_NET_HDR_GSO_TCPV4
            && hdr.gso_type != VIRTIO_NET_HDR_GSO_TCPV6
            && hdr.gso_type != VIRTIO_NET_HDR_GSO_UDP_L4
        {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!("unsupported virtio GSO type: {}", hdr.gso_type),
            ))?
        }
        let ip_version = input[0] >> 4;
        match ip_version {
            4 => {
                if hdr.gso_type != VIRTIO_NET_HDR_GSO_TCPV4
                    && hdr.gso_type != VIRTIO_NET_HDR_GSO_UDP_L4
                {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("ip header version: 4, GSO type: {}", hdr.gso_type),
                    ))?
                }
            }
            6 => {
                if hdr.gso_type != VIRTIO_NET_HDR_GSO_TCPV6
                    && hdr.gso_type != VIRTIO_NET_HDR_GSO_UDP_L4
                {
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        format!("ip header version: 6, GSO type: {}", hdr.gso_type),
                    ))?
                }
            }
            ip_version => Err(io::Error::new(
                io::ErrorKind::Other,
                format!("invalid ip header version: {}", ip_version),
            ))?,
        }
        // Don't trust hdr.hdrLen from the kernel as it can be equal to the length
        // of the entire first packet when the kernel is handling it as part of a
        // FORWARD path. Instead, parse the transport header length and add it onto
        // csumStart, which is synonymous for IP header length.
        if hdr.gso_type == VIRTIO_NET_HDR_GSO_UDP_L4 {
            hdr.hdr_len = hdr.csum_start + 8
        } else {
            if len <= hdr.csum_start as usize + 12 {
                Err(io::Error::new(io::ErrorKind::Other, "packet is too short"))?
            }

            let tcp_h_len = ((input[hdr.csum_start as usize + 12] as u16) >> 4) * 4;
            if !(20..=60).contains(&tcp_h_len) {
                // A TCP header must be between 20 and 60 bytes in length.
                Err(io::Error::new(
                    io::ErrorKind::Other,
                    format!("tcp header len is invalid: {tcp_h_len}"),
                ))?
            }
            hdr.hdr_len = hdr.csum_start + tcp_h_len
        }
        if len < hdr.hdr_len as usize {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "length of packet ({len}) < virtioNetHdr.hdr_len ({})",
                    hdr.hdr_len
                ),
            ))?
        }
        if hdr.hdr_len < hdr.csum_start {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "virtioNetHdr.hdrLen ({}) < virtioNetHdr.csumStart ({})",
                    hdr.hdr_len, hdr.csum_start
                ),
            ))?
        }
        let c_sum_at = (hdr.csum_start + hdr.csum_offset) as usize;
        if c_sum_at + 1 >= len {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "end of checksum offset ({}) exceeds packet length ({len})",
                    c_sum_at + 1,
                ),
            ))?
        }
        gso_split(input, hdr, bufs, sizes, offset, ip_version == 6)
    }
}

impl DeviceImpl {
    /// Prepare a new request.
    unsafe fn request(&self) -> io::Result<ifreq> {
        request(&self.name()?)
    }
    fn set_address_v4(&self, addr: Ipv4Addr) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            ipaddr_to_sockaddr(addr, 0, &mut req.ifr_ifru.ifru_addr, OVERWRITE_SIZE);
            if let Err(err) = siocsifaddr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    fn set_netmask(&self, value: Ipv4Addr) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_netmask, OVERWRITE_SIZE);
            if let Err(err) = siocsifnetmask(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }

    fn set_destination(&self, value: Ipv4Addr) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_dstaddr, OVERWRITE_SIZE);
            if let Err(err) = siocsifdstaddr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }

    pub fn remove_address_v6(&self, addr: Ipv6Addr, prefix: u8) -> io::Result<()> {
        unsafe {
            let if_index = self.if_index()?;
            let ctl = ctl_v6()?;
            let mut ifrv6: in6_ifreq = mem::zeroed();
            ifrv6.ifr6_ifindex = if_index as i32;
            ifrv6.ifr6_prefixlen = prefix as _;
            ifrv6.ifr6_addr = sockaddr_union::from(std::net::SocketAddr::new(addr.into(), 0))
                .addr6
                .sin6_addr;
            if let Err(err) = siocdifaddr_in6(ctl.as_raw_fd(), &ifrv6) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    /// Retrieves the name of the network interface.
    pub fn name(&self) -> io::Result<String> {
        unsafe { name(self.as_raw_fd()) }
    }
    /// Sets a new name for the network interface.
    ///
    /// This function converts the provided name into a C-compatible string,
    /// checks that its length does not exceed the maximum allowed (IFNAMSIZ),
    /// and then copies it into an interface request structure. It then uses a system call
    /// (via `siocsifname`) to apply the new name.
    pub fn set_name(&self, value: &str) -> io::Result<()> {
        unsafe {
            let tun_name = CString::new(value)?;

            if tun_name.as_bytes_with_nul().len() > IFNAMSIZ {
                return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
            }

            let mut req = self.request()?;
            ptr::copy_nonoverlapping(
                tun_name.as_ptr() as *const c_char,
                req.ifr_ifru.ifru_newname.as_mut_ptr(),
                value.len(),
            );

            if let Err(err) = siocsifname(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }

            Ok(())
        }
    }

    fn ifru_flags(&self) -> io::Result<i16> {
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;

            if let Err(err) = siocgifflags(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }
            Ok(req.ifr_ifru.ifru_flags)
        }
    }
    /// Checks whether the network interface is currently running.
    ///
    /// The interface is considered running if both the IFF_UP and IFF_RUNNING flags are set.
    pub fn is_running(&self) -> io::Result<bool> {
        let flags = self.ifru_flags()?;
        Ok(flags & (IFF_UP | IFF_RUNNING) as c_short == (IFF_UP | IFF_RUNNING) as c_short)
    }
    /// Enables or disables the network interface.
    ///
    /// If `value` is true, the interface is enabled by setting the IFF_UP and IFF_RUNNING flags.
    /// If false, the IFF_UP flag is cleared. The change is applied using a system call.
    pub fn enabled(&self, value: bool) -> io::Result<()> {
        unsafe {
            let ctl = ctl()?;
            let mut req = self.request()?;

            if let Err(err) = siocgifflags(ctl.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            if value {
                req.ifr_ifru.ifru_flags |= (IFF_UP | IFF_RUNNING) as c_short;
            } else {
                req.ifr_ifru.ifru_flags &= !(IFF_UP as c_short);
            }

            if let Err(err) = siocsifflags(ctl.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }

            Ok(())
        }
    }
    /// Retrieves the broadcast address of the network interface.
    ///
    /// This function populates an interface request with the broadcast address via a system call,
    /// converts it into a sockaddr structure, and then extracts the IP address.
    pub fn broadcast(&self) -> io::Result<IpAddr> {
        unsafe {
            let mut req = self.request()?;
            if let Err(err) = siocgifbrdaddr(ctl()?.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }
            let sa = sockaddr_union::from(req.ifr_ifru.ifru_broadaddr);
            Ok(std::net::SocketAddr::try_from(sa)?.ip())
        }
    }
    /// Sets the broadcast address of the network interface.
    ///
    /// This function converts the given IP address into a sockaddr structure (with a specified overwrite size)
    /// and then applies it to the interface via a system call.
    pub fn set_broadcast(&self, value: IpAddr) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            ipaddr_to_sockaddr(value, 0, &mut req.ifr_ifru.ifru_broadaddr, OVERWRITE_SIZE);
            if let Err(err) = siocsifbrdaddr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }
    /// Sets the IPv4 network address, netmask, and an optional destination address.
    ///
    /// This function sets the interface's address, netmask, and if provided, the destination address.
    /// It calls the helper methods `set_address_v4`, `set_netmask`, and `set_destination` respectively.
    pub fn set_network_address<IPv4: ToIpv4Address, Netmask: ToIpv4Netmask>(
        &self,
        address: IPv4,
        netmask: Netmask,
        destination: Option<IPv4>,
    ) -> io::Result<()> {
        self.set_address_v4(address.ipv4()?)?;
        self.set_netmask(netmask.netmask()?)?;
        if let Some(destination) = destination {
            self.set_destination(destination.ipv4()?)?;
        }
        Ok(())
    }
    /// Removes an IP address from the interface.
    ///
    /// For IPv4 addresses, it iterates over the current addresses and if a match is found,
    /// resets the address to `0.0.0.0` (unspecified).
    /// For IPv6 addresses, it retrieves the interface addresses by name and removes the matching address,
    /// taking into account its prefix length.
    pub fn remove_address(&self, addr: IpAddr) -> io::Result<()> {
        match addr {
            IpAddr::V4(_) => {
                for x in self.addresses()? {
                    if x == addr {
                        return self.set_address_v4(Ipv4Addr::UNSPECIFIED);
                    }
                }
            }
            IpAddr::V6(addr_v6) => {
                let addrs = crate::platform::get_if_addrs_by_name(self.name()?)?;
                for x in addrs {
                    if x.address == addr {
                        if let Some(netmask) = x.netmask {
                            let prefix = ipnet::ip_mask_to_prefix(netmask).unwrap_or(0);
                            self.remove_address_v6(addr_v6, prefix)?
                        }
                    }
                }
            }
        }
        Ok(())
    }
    /// Adds an IPv6 address to the interface.
    ///
    /// This function creates an `in6_ifreq` structure, fills in the interface index,
    /// prefix length, and IPv6 address (converted into a sockaddr structure),
    /// and then applies it using a system call.
    pub fn add_address_v6<IPv6: ToIpv6Address, Netmask: ToIpv6Netmask>(
        &self,
        addr: IPv6,
        netmask: Netmask,
    ) -> io::Result<()> {
        unsafe {
            let if_index = self.if_index()?;
            let ctl = ctl_v6()?;
            let mut ifrv6: in6_ifreq = mem::zeroed();
            ifrv6.ifr6_ifindex = if_index as i32;
            ifrv6.ifr6_prefixlen = netmask.prefix()? as u32;
            ifrv6.ifr6_addr =
                sockaddr_union::from(std::net::SocketAddr::new(addr.ipv6()?.into(), 0))
                    .addr6
                    .sin6_addr;
            if let Err(err) = siocsifaddr_in6(ctl.as_raw_fd(), &ifrv6) {
                return Err(io::Error::from(err));
            }
        }
        Ok(())
    }
    /// Retrieves the current MTU (Maximum Transmission Unit) for the interface.
    ///
    /// This function constructs an interface request and uses a system call (via `siocgifmtu`)
    /// to obtain the MTU. The result is then converted to a u16.
    pub fn mtu(&self) -> io::Result<u16> {
        unsafe {
            let mut req = self.request()?;

            if let Err(err) = siocgifmtu(ctl()?.as_raw_fd(), &mut req) {
                return Err(io::Error::from(err));
            }

            req.ifr_ifru
                .ifru_mtu
                .try_into()
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{e:?}")))
        }
    }
    /// Sets the MTU (Maximum Transmission Unit) for the interface.
    ///
    /// This function creates an interface request, sets the `ifru_mtu` field to the new value,
    /// and then applies it via a system call.
    pub fn set_mtu(&self, value: u16) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            req.ifr_ifru.ifru_mtu = value as i32;

            if let Err(err) = siocsifmtu(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }
    /// Sets the MAC (hardware) address for the interface.
    ///
    /// This function constructs an interface request and copies the provided MAC address
    /// into the hardware address field. It then applies the change via a system call.
    /// This operation is typically supported only for TAP devices.
    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> io::Result<()> {
        unsafe {
            let mut req = self.request()?;
            req.ifr_ifru.ifru_hwaddr.sa_family = ARPHRD_ETHER;
            req.ifr_ifru.ifru_hwaddr.sa_data[0..ETHER_ADDR_LEN as usize]
                .copy_from_slice(eth_addr.map(|c| c as _).as_slice());
            if let Err(err) = siocsifhwaddr(ctl()?.as_raw_fd(), &req) {
                return Err(io::Error::from(err));
            }
            Ok(())
        }
    }
    /// Retrieves the MAC (hardware) address of the interface.
    ///
    /// This function queries the MAC address by the interface name using a helper function.
    /// An error is returned if the MAC address cannot be found.
    pub fn mac_address(&self) -> io::Result<[u8; ETHER_ADDR_LEN as usize]> {
        let mac = mac_address_by_name(&self.name()?)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?
            .ok_or(io::Error::from(io::ErrorKind::NotFound))?;
        Ok(mac.bytes())
    }
}

unsafe fn name(fd: RawFd) -> io::Result<String> {
    let mut req: ifreq = mem::zeroed();
    if let Err(err) = tungetiff(fd, &mut req as *mut _ as *mut _) {
        return Err(io::Error::from(err));
    }
    let c_str = std::ffi::CStr::from_ptr(req.ifr_name.as_ptr() as *const c_char);
    let tun_name = c_str.to_string_lossy().into_owned();
    Ok(tun_name)
}

unsafe fn request(name: &str) -> io::Result<ifreq> {
    let mut req: ifreq = mem::zeroed();
    ptr::copy_nonoverlapping(
        name.as_ptr() as *const c_char,
        req.ifr_name.as_mut_ptr(),
        name.len(),
    );
    Ok(req)
}

impl From<Layer> for c_short {
    fn from(layer: Layer) -> Self {
        match layer {
            Layer::L2 => IFF_TAP as c_short,
            Layer::L3 => IFF_TUN as c_short,
        }
    }
}
