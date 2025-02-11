Tun/Tap interfaces
==============
[![Crates.io](https://img.shields.io/crates/v/tun-rs.svg)](https://crates.io/crates/tun-rs)
[![tun-rs](https://docs.rs/tun-rs/badge.svg)](https://docs.rs/tun-rs/latest/tun_rs)
[![Apache-2.0](https://img.shields.io/github/license/tun-rs/tun-rs?style=flat)](https://github.com/tun-rs/tun-rs/blob/master/LICENSE)

This crate allows the creation and usage of Tun/Tap interfaces(**supporting both Ipv4 and ipv6**), aiming to make this
cross-platform.

## Features:

1. Supporting TUN and TAP
2. Supporting both IPv4 and IPv6
3. Supporting Synchronous and Asynchronous API
4. Supporting Tokio and async-std asynchronous runtimes
5. All platforms have consistent IP packets(macOS's 4-byte head information can be eliminated)
6. Experimentally supporting shutdown for Synchronous version
7. Supporting Offload (`TSO`/`GSO`) on Linux
8. Supporting `multi-queue` on Linux
9. Having a consistent behavior of setting up routes when creating a device

## Supported Platforms

| Platform | TUN | TAP |
|----------|-----|-----|
| Windows  | ✅   | ✅   |
| Linux    | ✅   | ✅   |
| macOS    | ✅   | ⬜   |
| FreeBSD  | ✅   | ✅   |
| Android  | ✅   | ⬜   |
| iOS      | ✅   | ⬜   |

Usage
-----
First, add the following to your `Cargo.toml`:

```toml
[dependencies]
tun-rs = "2"
```

If you want to use the TUN interface with asynchronous runtimes, you need to enable the `async`(aliased
as `async_tokio`), or `async_std` feature:

```toml
[dependencies]
# tokio
tun-rs = { version = "2", features = ["async"] }

# async-std
#tun-rs = { version = "2", features = ["async_std"] }
```

Example
-------
The following example creates and configures a TUN interface and reads packets from it synchronously.

```rust
use tun_rs::DeviceBuilder;

fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
        .mtu(1400)
        .build_sync()?;

    let mut buf = [0; 4096];
    loop {
        let amount = dev.recv(&mut buf)?;
        println!("{:?}", &buf[0..amount]);
    }
}
```

An example of asynchronously reading packets from an interface

````rust
use tun_rs::DeviceBuilder;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .build_async()?;

    let mut buf = vec![0; 1500];
    loop {
        let len = dev.recv(&mut buf).await?;
        println!("pkt: {:?}", &buf[..len]);
        //dev.send(buf).await?;
    }
    Ok(())
}
````

On Unix, a device can also be directly created using a file descriptor (fd).
```rust
use tun_rs::SyncDevice;

fn main() -> std::io::Result<()> {
    // Pass a valid fd value
    let fd = 0;
    let dev = unsafe { SyncDevice::from_fd(fd) };
    // let async_dev = unsafe { tun_rs::AsyncDevice::from_fd(fd)?};

    let mut buf = [0; 4096];
    loop {
        let amount = dev.recv(&mut buf)?;
        println!("{:?}", &buf[0..amount]);
    }
}
```
More examples are [here](https://github.com/tun-rs/tun-rs/tree/main/examples)

Linux
-----
You will need the `tun-rs` module to be loaded and root is required to create
interfaces.

`TSO`/`GSO` and `multi-queue` is supported on the Linux platform, enable it via the config

````rust
use tun_rs::DeviceBuilder;
#[cfg(target_os = "linux")]
use tun_rs::{GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

#[cfg(target_os = "linux")]
fn main() -> std::io::Result<()> {
    let builder = DeviceBuilder::new()
        // enable `multi-queue`
        // .multi_queue(true)
        // enable Offload (`TSO`/`GSO`)
        .offload(true)
        .ipv4("10.0.0.1", 24, None)
        .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
        .mtu(1400);

    let dev = builder.build_sync()?;
    // use `multi-queue`
    // let dev_clone = dev.try_clone()?; 
    let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
    let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
    let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    let mut gro_table = GROTable::default();
    loop {
        let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;
        for i in 0..num {
            println!("num={num},bytes={:?}", &bufs[i][..sizes[i]]);
        }
    }
}
````

macOS & FreeBSD
-----
`tun-rs` will automatically set up a route according to the provided configuration, which does a similar thing like
this:
> sudo route -n add -net 10.0.0.0/24 10.0.0.1


iOS
----
You can pass the file descriptor of the TUN device to `tun-rs` to create the interface.

Here is an example to create the TUN device on iOS and pass the `fd` to `tun-rs`:

```swift
// Swift
class PacketTunnelProvider: NEPacketTunnelProvider {
    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        let tunnelNetworkSettings = createTunnelSettings() // Configure TUN address, DNS, mtu, routing...
        setTunnelNetworkSettings(tunnelNetworkSettings) { [weak self] error in
            // The tunnel of this tunFd is contains `Packet Information` prifix.
            let tunFd = self?.packetFlow.value(forKeyPath: "socket.fileDescriptor") as! Int32
            DispatchQueue.global(qos: .default).async {
                start_tun(tunFd)
            }
            completionHandler(nil)
        }
    }
}
```

```rust
#[no_mangle]
pub extern "C" fn start_tun(fd: std::os::raw::c_int) {
    // This is safe if the provided fd is valid
    let tun = unsafe { tun_rs::SyncDevice::from_raw_fd(fd) };
    let mut buf = [0u8; 1500];
    while let Ok(packet) = tun.recv(&mut buf) {
        ...
    }
}
```

Android
-----

```java
// use android.net.VpnService
private void startVpn(DeviceConfig config) {
    Builder builder = new Builder();
    builder
       .allowFamily(OsConstants.AF_INET)
       .addAddress("10.0.0.2", 24);
    ParcelFileDescriptor vpnInterface = builder.setSession("tun-rs")
                 .establish();
    int fd = vpnInterface.getFd();
    // Pass the fd to tun-rs using JNI
    // example: let tun = unsafe { tun_rs::SyncDevice::from_raw_fd(fd) };
}
```

Windows
-----

#### Tun:

You need to copy the [wintun.dll](https://wintun.net/) file which matches your architecture to
the same directory as your executable and run your program as administrator.

#### Tap:

When using the tap network interface, you need to manually
install [tap-windows](https://build.openvpn.net/downloads/releases/) that matches your architecture.
