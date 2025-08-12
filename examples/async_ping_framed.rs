#[allow(unused_imports)]
use bytes::BytesMut;
#[allow(unused_imports)]
use futures::{SinkExt, StreamExt};
#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::Arc;
use tun_rs::async_framed::{BytesCodec, DeviceFramed};
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[allow(unused_imports)]
use tun_rs::DeviceBuilder;
#[allow(unused_imports)]
use tun_rs::{AsyncDevice, SyncDevice};

mod protocol_handle;
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();

    let dev = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 0, 0, 21), 24, None)
        .build_async()?;
    let mut framed = DeviceFramed::new(dev, BytesCodec::new());
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                log::info!("Quit...");
                break;
            }
            next = framed.next() => {
                if let Some(rs) = next{
                    let buf = rs?;
                    handle_pkt(&buf, &mut framed).await?;
                }else{
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(any(
    target_os = "ios",
    target_os = "tvos",
    target_os = "android",
    all(target_os = "linux", target_env = "ohos")
))]

fn main() -> std::io::Result<()> {
    unimplemented!()
}

#[allow(dead_code)]
async fn handle_pkt(pkt: &[u8], framed: &mut DeviceFramed<BytesCodec>) -> std::io::Result<()> {
    if let Some(buf) = protocol_handle::ping(pkt) {
        framed.send(BytesMut::from(buf.as_slice())).await?;
    }
    Ok(())
}
