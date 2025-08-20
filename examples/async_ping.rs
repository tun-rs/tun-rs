#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::Arc;
#[allow(unused_imports)]
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
use tun_rs::DeviceBuilder;
#[allow(unused_imports)]
use tun_rs::{AsyncDevice, SyncDevice};

mod protocol_handle;

#[cfg(feature = "async_tokio")]
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

    let dev = Arc::new(
        DeviceBuilder::new()
            .ipv4(Ipv4Addr::new(10, 0, 0, 117), 24, None)
            .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
            .build_async()?,
    );

    println!("name:{:?}", dev.name()?);
    println!("addresses:{:?}", dev.addresses()?);
    let size = dev.mtu()? as usize;
    println!("mtu:{size:?}",);
    let mut buf = vec![0; size];
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Quit...");
                break;
            }
            len = dev.recv(&mut buf) => {
                let len = len?;
                println!("len = {len}");
                //println!("pkt: {:?}", &buf[..len?]);
                handle_pkt(&buf[..len], &dev).await?;
            }
        }
    }
    Ok(())
}

#[cfg(feature = "async_io")]
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
))]
#[async_std::main]
async fn main() -> std::io::Result<()> {
    use async_ctrlc::CtrlC;
    use async_std::prelude::FutureExt;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();
    let dev = Arc::new(
        DeviceBuilder::new()
            .ipv4(Ipv4Addr::from([10, 0, 0, 9]), 24, None)
            .build_async()?,
    );
    let size = dev.mtu()? as usize;
    let mut buf = vec![0; size];
    let ctrlc = CtrlC::new().expect("cannot create Ctrl+C handler?");
    ctrlc
        .race(async {
            while let Ok(len) = dev.recv(&mut buf).await {
                println!("len = {len}");
                //println!("pkt: {:?}", &buf[..len]);
                handle_pkt(&buf[..len], &dev).await.unwrap();
            }
        })
        .await;
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
async fn handle_pkt(pkt: &[u8], dev: &AsyncDevice) -> std::io::Result<()> {
    if let Some(buf) = protocol_handle::ping(pkt) {
        dev.send(&buf).await?;
    }
    Ok(())
}
