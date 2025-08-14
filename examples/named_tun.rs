#[allow(unused_imports)]
use std::net::Ipv4Addr;
use std::sync::mpsc::Receiver;
#[allow(unused_imports)]
use std::sync::Arc;

#[allow(unused_imports)]
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "macos",
    target_os = "openbsd",
    target_os = "netbsd",
))]
use tun_rs::DeviceBuilder;
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
#[allow(unused_imports)]
use tun_rs::Layer;
fn main() -> Result<(), std::io::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("trace")).init();
    let (tx, rx) = std::sync::mpsc::channel();

    let handle = ctrlc2::set_handler(move || {
        tx.send(()).expect("Signal error.");
        true
    })
    .expect("Error setting Ctrl-C handler");

    main_entry(rx)?;
    handle.join().unwrap();
    Ok(())
}
#[cfg(any(
    target_os = "ios",
    target_os = "tvos",
    target_os = "android",
    all(target_os = "linux", target_env = "ohos")
))]
fn main_entry(_quit: Receiver<()>) -> Result<(), std::io::Error> {
    unimplemented!()
}
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
fn main_entry(quit: Receiver<()>) -> Result<(), std::io::Error> {
    #[allow(unused_imports)]
    use std::net::IpAddr;
    let dev = Arc::new(
        DeviceBuilder::new()
            .name("utun66")
            .ipv4(Ipv4Addr::new(10, 0, 0, 12), 24, None)
            .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
            .mtu(1400)
            .build_sync()?,
    );
    println!("addr {:?}", dev.addresses());

    println!("if_index = {:?}", dev.if_index());
    println!("mtu = {:?}", dev.mtu());
    #[cfg(windows)]
    {
        dev.set_mtu_v6(2000)?;
        println!("mtu ipv6 = {:?}", dev.mtu_v6());
        println!("version = {:?}", dev.version());
    }
    let _join = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        loop {
            let amount = dev.recv(&mut buf)?;
            println!("{:?}", &buf[0..amount]);
        }
        #[allow(unreachable_code)]
        std::io::Result::Ok(())
    });
    _ = quit.recv();
    Ok(())
}
