#![allow(unused)]

use pnet_packet::arp::{ArpOperations, MutableArpPacket};
use pnet_packet::ethernet::{EthernetPacket, MutableEthernetPacket};
use pnet_packet::icmp::IcmpTypes;
use pnet_packet::ip::IpNextHeaderProtocols;
use pnet_packet::{MutablePacket, Packet};
use std::net::Ipv4Addr;

pub fn ping(buf: &[u8]) -> Option<Vec<u8>> {
    #[allow(clippy::single_match)]
    match pnet_packet::ipv4::Ipv4Packet::new(buf) {
        Some(ip_pkt) => match ip_pkt.get_next_level_protocol() {
            IpNextHeaderProtocols::Icmp => {
                let icmp_pkt = pnet_packet::icmp::IcmpPacket::new(ip_pkt.payload()).unwrap();
                match icmp_pkt.get_icmp_type() {
                    IcmpTypes::EchoRequest => {
                        let mut v = ip_pkt.payload().to_owned();
                        let mut pkkt =
                            pnet_packet::icmp::MutableIcmpPacket::new(&mut v[..]).unwrap();
                        pkkt.set_icmp_type(IcmpTypes::EchoReply);
                        pkkt.set_checksum(pnet_packet::icmp::checksum(&pkkt.to_immutable()));
                        let len = ip_pkt.packet().len();
                        let mut buf = vec![0u8; len];
                        let mut res = pnet_packet::ipv4::MutableIpv4Packet::new(&mut buf).unwrap();
                        res.set_total_length(ip_pkt.get_total_length());
                        res.set_header_length(ip_pkt.get_header_length());
                        res.set_destination(ip_pkt.get_source());
                        res.set_source(ip_pkt.get_destination());
                        res.set_identification(0x42);
                        res.set_next_level_protocol(IpNextHeaderProtocols::Icmp);
                        res.set_payload(&v);
                        res.set_ttl(64);
                        res.set_version(ip_pkt.get_version());
                        res.set_checksum(pnet_packet::ipv4::checksum(&res.to_immutable()));
                        println!("ping: {}", ip_pkt.get_destination());
                        return Some(buf);
                    }
                    _ => {}
                }
            }
            _ => {}
        },
        None => {}
    }
    None
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
    println!("arp query {}", target_p);
    Some(buf)
}
