#![allow(unused)]

use pnet_packet::arp::{ArpOperations, MutableArpPacket};
use pnet_packet::ethernet::{EthernetPacket, MutableEthernetPacket};
use pnet_packet::icmp::IcmpPacket;
use pnet_packet::icmp::IcmpTypes;
use pnet_packet::icmp::MutableIcmpPacket;
use pnet_packet::icmpv6::Icmpv6Packet;
use pnet_packet::icmpv6::Icmpv6Types;
use pnet_packet::icmpv6::MutableIcmpv6Packet;
use pnet_packet::ip::IpNextHeaderProtocols;
use pnet_packet::ipv4::{Ipv4Packet, MutableIpv4Packet};
use pnet_packet::ipv6::{Ipv6Packet, MutableIpv6Packet};
use pnet_packet::{MutablePacket, Packet};
use std::net::Ipv4Addr;

fn handle_ipv4_ping(ip_pkt: &Ipv4Packet) -> Option<Vec<u8>> {
    if ip_pkt.get_next_level_protocol() != IpNextHeaderProtocols::Icmp {
        return None;
    }

    let icmp_pkt = IcmpPacket::new(ip_pkt.payload())?;
    if icmp_pkt.get_icmp_type() != IcmpTypes::EchoRequest {
        return None;
    }

    println!(
        "IPv4 Ping Request: {} -> {}",
        ip_pkt.get_source(),
        ip_pkt.get_destination()
    );

    let mut icmp_payload = ip_pkt.payload().to_owned();
    let mut mutable_icmp_pkt = MutableIcmpPacket::new(&mut icmp_payload).unwrap();
    mutable_icmp_pkt.set_icmp_type(IcmpTypes::EchoReply);
    mutable_icmp_pkt.set_checksum(pnet_packet::icmp::checksum(
        &mutable_icmp_pkt.to_immutable(),
    ));

    let total_len = ip_pkt.get_total_length() as usize;
    let mut response_buf = vec![0u8; total_len];
    let mut res_ipv4_pkt = MutableIpv4Packet::new(&mut response_buf).unwrap();

    res_ipv4_pkt.set_version(4);
    res_ipv4_pkt.set_header_length(ip_pkt.get_header_length());
    res_ipv4_pkt.set_total_length(ip_pkt.get_total_length());
    res_ipv4_pkt.set_identification(0x42);
    res_ipv4_pkt.set_ttl(64);
    res_ipv4_pkt.set_next_level_protocol(IpNextHeaderProtocols::Icmp);
    res_ipv4_pkt.set_source(ip_pkt.get_destination());
    res_ipv4_pkt.set_destination(ip_pkt.get_source());
    res_ipv4_pkt.set_payload(&icmp_payload);
    res_ipv4_pkt.set_checksum(pnet_packet::ipv4::checksum(&res_ipv4_pkt.to_immutable()));

    Some(response_buf)
}

fn handle_ipv6_ping(ip_pkt: &Ipv6Packet) -> Option<Vec<u8>> {
    if ip_pkt.get_next_header() != IpNextHeaderProtocols::Icmpv6 {
        return None;
    }

    let icmpv6_pkt = Icmpv6Packet::new(ip_pkt.payload())?;
    if icmpv6_pkt.get_icmpv6_type() != Icmpv6Types::EchoRequest {
        return None;
    }

    println!(
        "IPv6 Ping Request: {} -> {}",
        ip_pkt.get_source(),
        ip_pkt.get_destination()
    );

    let mut icmp_payload = ip_pkt.payload().to_owned();
    let mut mutable_icmpv6_pkt = MutableIcmpv6Packet::new(&mut icmp_payload).unwrap();
    mutable_icmpv6_pkt.set_icmpv6_type(Icmpv6Types::EchoReply);

    let checksum = pnet_packet::icmpv6::checksum(
        &mutable_icmpv6_pkt.to_immutable(),
        &ip_pkt.get_destination(),
        &ip_pkt.get_source(),
    );
    mutable_icmpv6_pkt.set_checksum(checksum);

    let total_len = 40 + icmp_payload.len();
    let mut response_buf = vec![0u8; total_len];
    let mut res_ipv6_pkt = MutableIpv6Packet::new(&mut response_buf).unwrap();

    res_ipv6_pkt.set_version(6);
    res_ipv6_pkt.set_traffic_class(0);
    res_ipv6_pkt.set_flow_label(0);
    res_ipv6_pkt.set_payload_length(icmp_payload.len() as u16);
    res_ipv6_pkt.set_next_header(IpNextHeaderProtocols::Icmpv6);
    res_ipv6_pkt.set_hop_limit(64);
    res_ipv6_pkt.set_source(ip_pkt.get_destination());
    res_ipv6_pkt.set_destination(ip_pkt.get_source());
    res_ipv6_pkt.set_payload(&icmp_payload);

    Some(response_buf)
}

pub fn ping(buf: &[u8]) -> Option<Vec<u8>> {
    if buf.is_empty() {
        return None;
    }
    match buf[0] >> 4 {
        4 => {
            // IPv4
            let ipv4_packet = Ipv4Packet::new(buf)?;
            handle_ipv4_ping(&ipv4_packet)
        }
        6 => {
            // IPv6
            let ipv6_packet = Ipv6Packet::new(buf)?;
            handle_ipv6_ping(&ipv6_packet)
        }
        _ => {
            // unknown
            None
        }
    }
}
pub fn ping_ethernet(buf: &[u8]) -> Option<Vec<u8>> {
    if let Some(packet) = EthernetPacket::new(buf) {
        if let Some(ping_buf) = ping(packet.payload()) {
            let mut buf = vec![0u8; 14 + ping_buf.len()];

            let mut ethernet_packet = MutableEthernetPacket::new(&mut buf).unwrap();
            ethernet_packet.set_source(packet.get_destination());
            ethernet_packet.set_destination(packet.get_source());
            ethernet_packet.set_ethertype(packet.get_ethertype());
            ethernet_packet.set_payload(&ping_buf);
            return Some(buf);
        }
    }
    None
}
pub fn arp(buf: &[u8]) -> Option<Vec<u8>> {
    let packet = EthernetPacket::new(buf)?;
    // Use a valid MAC address
    const MAC: [u8; 6] = [0x2, 0xf, 0xf, 0xf, 0xe, 0x9];
    let mut buf = packet.packet().to_vec();
    let mut ethernet_packet = MutableEthernetPacket::new(&mut buf).unwrap();
    let sender_h = packet.get_source();
    let mut arp_packet = MutableArpPacket::new(ethernet_packet.payload_mut())?;
    if arp_packet.get_operation() != ArpOperations::Request {
        return None;
    }
    let sender_p = arp_packet.get_sender_proto_addr();
    let target_p = arp_packet.get_target_proto_addr();
    if target_p == Ipv4Addr::UNSPECIFIED
        || sender_p == Ipv4Addr::UNSPECIFIED
        || target_p == sender_p
    {
        return None;
    }
    arp_packet.set_operation(ArpOperations::Reply);
    arp_packet.set_target_hw_addr(sender_h);
    arp_packet.set_target_proto_addr(sender_p);
    arp_packet.set_sender_proto_addr(target_p);
    arp_packet.set_sender_hw_addr(MAC.into());
    ethernet_packet.set_destination(sender_h);
    ethernet_packet.set_source(MAC.into());
    println!("arp query {target_p}");
    Some(buf)
}
