#[allow(unused_imports)]
use std::net::Ipv4Addr;
#[allow(unused_imports)]
use std::sync::{mpsc::Receiver, Arc};
#[allow(unused_imports)]
use tun_rs::{DeviceBuilder, SyncDevice};

mod protocol_handle;
fn main() -> std::io::Result<()> {
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
#[cfg(any(target_os = "ios", target_os = "android",))]
fn main_entry(_quit: Receiver<()>) -> std::io::Result<()> {
    unimplemented!()
}
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "freebsd",
))]
fn main_entry(quit: Receiver<()>) -> std::io::Result<()> {
    let dev = Arc::new(
        DeviceBuilder::new()
            .ipv4(Ipv4Addr::new(10, 0, 0, 9), 24, None)
            .build_sync()?,
    );

    #[cfg(target_os = "macos")]
    dev.set_ignore_packet_info(true);

    let mut buf = [0; 4096];

    #[cfg(feature = "experimental")]
    let dev2 = dev.clone();
    #[cfg(feature = "experimental")]
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        dev2.shutdown().unwrap();
    });

    std::thread::spawn(move || {
        loop {
            let amount = dev.recv(&mut buf);
            println!("amount == {amount:?}");
            let amount = amount?;
            let pkt = &buf[0..amount];
            handle_pkt(pkt, &dev).unwrap();
        }
        #[allow(unreachable_code)]
        Ok::<(), std::io::Error>(())
    });
    quit.recv().expect("Quit error.");
    Ok(())
}

#[allow(dead_code)]
fn handle_pkt(pkt: &[u8], dev: &SyncDevice) -> std::io::Result<()> {
    if let Some(buf) = protocol_handle::ping(pkt) {
        dev.send(&buf)?;
    }
    Ok(())
}
