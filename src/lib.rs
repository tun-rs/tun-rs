#![cfg_attr(docsrs, feature(doc_cfg))]

/*!
# Example:
```no_run
use tun_rs::DeviceBuilder;
let dev = DeviceBuilder::new()
            .name("utun7")
            .ipv4("10.0.0.12", 24, None)
            .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
            .mtu(1400)
            .build_sync()
            .unwrap();
let mut buf = [0;65535];
loop {
    let len = dev.recv(&mut buf).unwrap();
    println!("buf= {:?}",&buf[..len]);
}
```
# Example IOS/Android/... :
```no_run
#[cfg(unix)]
{
    use tun_rs::SyncDevice;
    // use PacketTunnelProvider/VpnService create tun fd
    let fd = 7799;
    let dev = unsafe{SyncDevice::from_fd(fd)};
    let mut buf = [0;65535];
    loop {
        let len = dev.recv(&mut buf).unwrap();
        println!("buf= {:?}",&buf[..len]);
    }
}
```
*/

extern crate alloc;

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
pub use crate::builder::*;
pub use crate::platform::*;

#[cfg_attr(docsrs, doc(cfg(any(feature = "async_io", feature = "async_tokio"))))]
#[cfg(any(feature = "async_io", feature = "async_tokio"))]
mod async_device;

#[cfg_attr(docsrs, doc(cfg(any(feature = "async_io", feature = "async_tokio"))))]
#[cfg(any(feature = "async_io", feature = "async_tokio"))]
pub use async_device::*;

#[cfg(any(
    target_os = "windows",
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
mod builder;
mod platform;
pub const PACKET_INFORMATION_LENGTH: usize = 4;
