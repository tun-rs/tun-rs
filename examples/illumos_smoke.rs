use std::net::Ipv4Addr;
use tun_rs::DeviceBuilder;

fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .name("tun7")
        .ipv4(Ipv4Addr::new(10, 99, 0, 1), 24, None)
        .mtu(1400)
        .build_sync()?;

    println!("created {:?}; in another shell run: ping 10.99.0.2", dev.name());

    let mut buf = [0u8; 2048];
    loop {
        let n = dev.recv(&mut buf)?;
        let p = &buf[..n];
        // IPv4 + ICMP echo request to 10.99.0.2?
        if n >= 28 && p[0] >> 4 == 4 && p[9] == 1 && p[20] == 8 && p[16..20] == [10, 99, 0, 2] {
            println!("echo request, sending reply");
            let mut r = p.to_vec();
            let (src, dst) = (p[12..16].to_vec(), p[16..20].to_vec());
            r[12..16].copy_from_slice(&dst);
            r[16..20].copy_from_slice(&src);
            r[20] = 0; // echo reply
            // fix ICMP checksum: request checksum + 0x0800
            let ck = u16::from_be_bytes([r[22], r[23]]) as u32 + 0x0800;
            let ck = ((ck & 0xffff) + (ck >> 16)) as u16;
            r[22..24].copy_from_slice(&ck.to_be_bytes());
            // IP header checksum
            r[10] = 0;
            r[11] = 0;
            let ihl = ((r[0] & 0x0f) as usize) * 4;
            let mut sum: u32 = 0;
            for c in r[..ihl].chunks(2) {
                sum += u16::from_be_bytes([c[0], c[1]]) as u32;
            }
            while sum >> 16 != 0 {
                sum = (sum & 0xffff) + (sum >> 16);
            }
            r[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
            dev.send(&r)?;
        }
    }
}
