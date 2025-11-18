#[allow(unused_imports)]
use std::net::Ipv4Addr;
use std::sync::mpsc::Receiver;
#[allow(unused_imports)]
use std::sync::Arc;
use std::time::Duration;
#[allow(unused_imports)]
#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "macos"
))]
use tun_rs::DeviceBuilder;
#[allow(unused_imports)]
use tun_rs::InterruptEvent;
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
    let dev = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 0, 0, 12), 24, None)
        .mtu(1400)
        .build_sync()?;

    println!("if_index = {:?}", dev.if_index());
    #[cfg(unix)]
    dev.set_nonblocking(true)?;

    let event = Arc::new(InterruptEvent::new()?);
    let event_clone = event.clone();
    let join = std::thread::spawn(move || {
        let mut buf = [0; 4096];
        loop {
            match dev.recv_intr_timeout(&mut buf, &event_clone, Some(Duration::from_millis(1000))) {
                Ok(len) => {
                    println!("recv_intr_timeout Ok({len})");
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => {
                    println!("read_interruptible Err({e:?})");
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {
                    // If the interrupt event is to be reused, it must be reset before the next wait.
                    if event_clone.is_trigger() {
                        event_clone.reset().unwrap();
                        println!("read_interruptible Err({e:?})");
                    }
                    return;
                }
                Err(e) => {
                    println!("Error: {e:?}");
                    return;
                }
            }
        }
    });
    _ = quit.recv();
    std::thread::sleep(Duration::from_millis(100));
    event.trigger()?;
    join.join().unwrap();
    Ok(())
}
