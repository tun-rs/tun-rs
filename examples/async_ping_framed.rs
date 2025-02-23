use bytes::BytesMut;
use futures::{SinkExt, StreamExt};
#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::Arc;
use tun_rs::async_framed::{BytesCodec, DeviceFramed};
#[allow(unused_imports)]
use tun_rs::DeviceBuilder;
#[allow(unused_imports)]
use tun_rs::{AsyncDevice, SyncDevice};

mod protocol_handle;
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
))]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<()>(1);

    ctrlc2::set_async_handler(async move {
        tx.send(()).await.expect("Signal error");
    })
    .await;

    let dev = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 0, 0, 21), 24, None)
        .build_async()?;
    let mut framed = DeviceFramed::new(dev, BytesCodec::new());
    loop {
        tokio::select! {
            _ = rx.recv() => {
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
        };
    }
    Ok(())
}

#[cfg(any(target_os = "ios", target_os = "android",))]
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
