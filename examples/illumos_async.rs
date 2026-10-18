use std::net::Ipv4Addr;
use tun_rs::DeviceBuilder;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .name("tun7")
        .ipv4(Ipv4Addr::new(10, 99, 0, 1), 24, None)
        .mtu(1400)
        .build_async()?;

    println!("created {:?}", dev.name());
    println!("in another shell run: ping 10.99.0.2");

    let mut buf = [0u8; 2048];
    loop {
        let n = dev.recv(&mut buf).await?;
        let p = &buf[..n];
        println!("async read {n} bytes, proto {}, dst {:?}", p[9], &p[16..20]);
    }
}
