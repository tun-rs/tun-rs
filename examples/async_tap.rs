#![allow(unused_imports)]
use pnet_packet::ethernet::{EtherTypes, EthernetPacket};
use pnet_packet::Packet;
use std::io;
use std::net::Ipv4Addr;
use std::sync::Arc;
use tun_rs::DeviceBuilder;
use tun_rs::Layer;

mod protocol_handle;
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
#[tokio::main]
async fn main() -> io::Result<()> {
    main0().await?;

    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Ok(())
}
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos"
))]
async fn main0() -> io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();
    let (tx, mut quit) = tokio::sync::mpsc::channel::<()>(1);

    ctrlc2::set_async_handler(async move {
        tx.send(()).await.expect("Signal error");
    })
    .await;
    let dev = Arc::new(
        DeviceBuilder::new()
            // .name("feth0")
            // .peer_feth("feth1")
            .ipv4(Ipv4Addr::from([10, 0, 0, 9]), 24, None)
            .layer(Layer::L2)
            .mtu(1400)
            .build_async()?,
    );
    println!("Waiting for all interfaces...");
    let mut buf = vec![0; 14 + 65536];
    loop {
        tokio::select! {
            _ = quit.recv() => {
                println!("Quit...");
                break;
            }
            len = dev.recv(&mut buf) => {
                if let Some(packet) = EthernetPacket::new(&buf[..len?]){
                        match packet.get_ethertype(){
                            EtherTypes::Ipv4=>{
                                if let Some(buf) = protocol_handle::ping_ethernet(packet.packet()){
                                    dev.send(&buf).await?;
                                }
                            }
                            EtherTypes::Arp=>{
                                  if let Some(rs) = protocol_handle::arp(packet.packet()) {
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

#[cfg(any(target_os = "ios", target_os = "android"))]
fn main() -> io::Result<()> {
    unimplemented!()
}
