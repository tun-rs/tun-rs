#[cfg(target_os = "illumos")]
fn main() -> std::io::Result<()> {
    use std::net::Ipv4Addr;
    use tun_rs::DeviceBuilder;

    let dev = DeviceBuilder::new()
        .name("tun7")
        .ipv4(Ipv4Addr::new(10, 99, 0, 1), 24, None)
        .mtu(1400)
        .build_sync()?;

    println!(
        "created {:?}; in another shell run: ping 10.99.0.2",
        dev.name()
    );

    let mut buf = [0u8; 2048];
    loop {
        let n = dev.recv(&mut buf)?;
        let p = &buf[..n];
        // Need a full minimal IPv4 header before reading the IHL.
        if n < 20 || p[0] >> 4 != 4 {
            continue;
        }
        let ihl = ((p[0] & 0x0f) as usize) * 4;
        // Need the whole 8-byte ICMP echo header after the IP header.
        if ihl < 20 || n < ihl + 8 {
            continue;
        }
        // ICMP echo request to 10.99.0.2?
        if p[9] == 1 && p[ihl] == 8 && p[16..20] == [10, 99, 0, 2] {
            println!("echo request, sending reply");
            let mut r = p.to_vec();
            let (src, dst) = (p[12..16].to_vec(), p[16..20].to_vec());
            r[12..16].copy_from_slice(&dst);
            r[16..20].copy_from_slice(&src);
            r[ihl] = 0; // echo reply
                        // ICMP checksum: request checksum + 0x0800
            let ck = u16::from_be_bytes([r[ihl + 2], r[ihl + 3]]) as u32 + 0x0800;
            let ck = ((ck & 0xffff) + (ck >> 16)) as u16;
            r[ihl + 2..ihl + 4].copy_from_slice(&ck.to_be_bytes());
            // IP header checksum
            r[10] = 0;
            r[11] = 0;
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

#[cfg(not(target_os = "illumos"))]
fn main() {
    eprintln!("this example only runs on illumos");
}
