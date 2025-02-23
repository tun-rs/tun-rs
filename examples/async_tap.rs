#![allow(unused_imports)]
use async_ctrlc::CtrlC;
use async_std::prelude::FutureExt;
use pnet_packet::arp::{ArpOperations, MutableArpPacket};
use pnet_packet::ethernet::{EtherTypes, EthernetPacket, MutableEthernetPacket};
use pnet_packet::icmp::IcmpTypes;
use pnet_packet::ip::IpNextHeaderProtocols;
use pnet_packet::{MutablePacket, Packet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::{fmt, io};
use tokio::sync::mpsc::Receiver;
use tun_rs::DeviceBuilder;
use tun_rs::Layer;

#[cfg(feature = "async_tokio")]
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd",))]
#[tokio::main]
async fn main() -> io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();
    let (tx, mut quit) = tokio::sync::mpsc::channel::<()>(1);

    ctrlc2::set_async_handler(async move {
        tx.send(()).await.expect("Signal error");
    })
    .await;
    let dev = Arc::new(
        DeviceBuilder::new()
            .name("tap0")
            .ipv4(Ipv4Addr::from([10, 0, 0, 9]), 24, None)
            .layer(Layer::L2)
            .mtu(1500)
            .build_async()?,
    );
    let mut buf = vec![0; 14 + 65536];
    loop {
        tokio::select! {
            _ = quit.recv() => {
                println!("Quit...");
                break;
            }
            len = dev.recv(&mut buf) => {
                if let Some(packet) =  EthernetPacket::new(&buf[..len?]){
                        match packet.get_ethertype(){
                            EtherTypes::Ipv4=>{
                                if let Some(rs) =ping(&packet){
                                    dev.send(&rs).await?;
                                }
                            }
                            EtherTypes::Arp=>{
                                  if let Some(rs) = arp(&packet) {
                                    dev.send(&rs).await?;
                                }
                            }
                            protocol=>{
                                 println!("ignore ether protocol: {}", protocol)
                            }
                        }
                }
            }
        }
    }
    Ok(())
}

#[cfg(feature = "async_std")]
#[cfg(any(target_os = "windows", target_os = "linux", target_os = "freebsd",))]
#[async_std::main]
async fn main() -> io::Result<()> {
    let dev = Arc::new(
        DeviceBuilder::new()
            .name("tap0")
            .ipv4(Ipv4Addr::from([10, 0, 0, 9]), 24, None)
            .layer(Layer::L2)
            .build_async()?,
    );
    let mut buf = vec![0; 14 + 65536];
    let ctrlc = CtrlC::new().expect("cannot create Ctrl+C handler?");
    ctrlc
        .race(async {
            _ = async {
                while let Ok(len) = dev.recv(&mut buf).await {
                    if let Some(packet) = EthernetPacket::new(&buf[..len]) {
                        match packet.get_ethertype() {
                            EtherTypes::Ipv4 => {
                                if let Some(rs) = ping(&packet) {
                                    dev.send(&rs).await?;
                                }
                            }
                            EtherTypes::Arp => {
                                if let Some(rs) = arp(&packet) {
                                    dev.send(&rs).await?;
                                }
                            }
                            protocol => {
                                println!("ignore ether protocol: {}", protocol)
                            }
                        }
                    }
                }
                Ok::<(), std::io::Error>(())
            }
            .await;
        })
        .await;
    println!("Quit...");
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "android", target_os = "macos"))]
fn main() -> io::Result<()> {
    unimplemented!()
}

#[allow(dead_code)]
pub fn ping(packet: &EthernetPacket) -> Option<Vec<u8>> {
    #[allow(clippy::single_match)]
    if let Some(ip_pkt) = pnet_packet::ipv4::Ipv4Packet::new(packet.payload()) {
        match ip_pkt.get_next_level_protocol() {
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
                        let mut ip_packet =
                            pnet_packet::ipv4::MutableIpv4Packet::new(&mut buf).unwrap();
                        ip_packet.set_total_length(ip_pkt.get_total_length());
                        ip_packet.set_header_length(ip_pkt.get_header_length());
                        ip_packet.set_destination(ip_pkt.get_source());
                        ip_packet.set_source(ip_pkt.get_destination());
                        ip_packet.set_identification(0x42);
                        ip_packet.set_next_level_protocol(IpNextHeaderProtocols::Icmp);
                        ip_packet.set_payload(&v);
                        ip_packet.set_ttl(64);
                        ip_packet.set_version(ip_pkt.get_version());
                        ip_packet
                            .set_checksum(pnet_packet::ipv4::checksum(&ip_packet.to_immutable()));
                        let mut buf = vec![0u8; 14 + len];
                        let mut ethernet_packet = MutableEthernetPacket::new(&mut buf).unwrap();
                        ethernet_packet.set_source(packet.get_destination());
                        ethernet_packet.set_destination(packet.get_source());
                        ethernet_packet.set_ethertype(packet.get_ethertype());
                        ethernet_packet.set_payload(ip_packet.packet());
                        println!("ping {}", ip_pkt.get_destination());
                        return Some(buf);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    None
}
#[allow(dead_code)]
pub fn arp(packet: &EthernetPacket) -> Option<Vec<u8>> {
    const MAC: [u8; 6] = [0xf, 0xf, 0xf, 0xf, 0xe, 0x9];
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
