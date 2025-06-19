use bytes::BytesMut;
#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use tun_rs::DeviceBuilder;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use tun_rs::{IoUringBuilder, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

mod protocol_handle;

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();

    ctrlc2::set_handler(move || {
        println!("Ctrl+C signaled");
        std::process::exit(0);
    })
    .expect("Error setting Ctrl-C handler");

    let dev = DeviceBuilder::new()
        .offload(true)
        .ipv4(Ipv4Addr::new(10, 0, 0, 117), 24, None)
        .build_sync()?;
    let builder = IoUringBuilder::new()?;
    let (mut reader, mut writer) = builder.build(dev)?;
    let size = writer.mtu()? as usize;

    let mut bufs = vec![BytesMut::zeroed(size); IDEAL_BATCH_SIZE];
    let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    loop {
        let num = reader.read_multiple(&mut bufs, &mut sizes, 0)?;
        for i in 0..num {
            let buf = &bufs[i];
            let len = sizes[i];
            if let Some(reply) = protocol_handle::ping(&buf[..len]) {
                let mut buf = BytesMut::with_capacity(VIRTIO_NET_HDR_LEN + 65536);
                buf.resize(VIRTIO_NET_HDR_LEN, 0);
                buf.extend_from_slice(&reply);
                let mut bufs = [&mut buf];
                writer.write_multiple(&mut bufs, VIRTIO_NET_HDR_LEN)?;
            }
        }
    }
}
#[cfg(not(all(target_os = "linux", not(target_env = "ohos")),))]
fn main() -> std::io::Result<()> {
    unimplemented!()
}
