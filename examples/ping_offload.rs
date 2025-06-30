#[allow(unused_imports)]
use bytes::BytesMut;
#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use tun_rs::{AsyncDevice, DeviceBuilder, SyncDevice};
#[allow(unused_imports)]
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use tun_rs::{GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};
mod protocol_handle;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();

    let dev = Arc::new({
        let builder = DeviceBuilder::new().ipv4(Ipv4Addr::from([10, 0, 0, 9]), 24, None);
        #[cfg(target_os = "linux")]
        let builder = builder.offload(true);
        builder.build_async()?
    });
    println!("TCP-GSO:{},UDP-GSO:{}", dev.tcp_gso(), dev.udp_gso());
    let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
    let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
    let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    let mut gro_table = GROTable::default();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Quit...");
                break;
            }
            num = dev.recv_multiple(&mut original_buffer,&mut bufs,&mut sizes,0) => {
                let num = num?;
                for i in 0..num  {
                    if let Some(reply) = protocol_handle::ping(&bufs[i][..sizes[i]]){
                        let mut buf = BytesMut::with_capacity(VIRTIO_NET_HDR_LEN+reply.len());
                        buf.resize(VIRTIO_NET_HDR_LEN,0);
                        buf.extend_from_slice(&reply);
                        let mut bufs = [&mut buf];
                        dev.send_multiple(&mut gro_table,&mut bufs,VIRTIO_NET_HDR_LEN).await?;
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(all(target_os = "linux", not(target_env = "ohos")),))]
fn main() -> std::io::Result<()> {
    unimplemented!()
}
