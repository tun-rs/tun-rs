<div align="center">

# 🚀 tun-rs

### High-Performance Cross-Platform TUN/TAP Library for Rust

[![Crates.io](https://img.shields.io/crates/v/tun-rs.svg)](https://crates.io/crates/tun-rs)
[![Documentation](https://docs.rs/tun-rs/badge.svg)](https://docs.rs/tun-rs/latest/tun_rs)
[![Apache-2.0](https://img.shields.io/github/license/tun-rs/tun-rs?style=flat)](https://github.com/tun-rs/tun-rs/blob/master/LICENSE)
[![Downloads](https://img.shields.io/crates/d/tun-rs.svg)](https://crates.io/crates/tun-rs)

[Features](#-key-features) • [Performance](#-performance-benchmarks) • [Installation](#-installation) • [Examples](#-examples) • [Platforms](#-supported-platforms)

</div>

---

## 📖 Overview

**tun-rs** is a powerful, production-ready Rust library for creating and managing TUN and TAP virtual network interfaces. Built with performance and cross-platform compatibility in mind, it provides both synchronous and asynchronous APIs to suit your application's needs.

### 🎯 Why Choose tun-rs?

- **🏆 Exceptional Performance**: Achieves up to **70.6 Gbps** throughput with concurrent operations and offload features
- **🌍 True Cross-Platform**: Consistent API across Windows, Linux, macOS, BSD, iOS, Android, and more
- **⚡ Async-Ready**: Native support for `tokio` and `async-io` runtimes
- **🔧 Feature-Rich**: Multiple IP addresses, DNS support, hardware offload, multi-queue, and more
- **🛡️ Production-Tested**: Used in real-world VPN and networking applications
- **📦 Zero Hassle**: Minimal dependencies and straightforward API design

## 🌟 Key Features

### Core Capabilities
- ✅ **Dual Mode Support**: Both TUN (Layer 3) and TAP (Layer 2) interfaces
- ✅ **Multiple IP Addresses**: Assign multiple IPv4 and IPv6 addresses to a single interface
- ✅ **Sync & Async APIs**: Choose between synchronous blocking or async non-blocking operations
- ✅ **Runtime Flexibility**: Optional `tokio` or `async-io` integration for async operations

### Platform-Specific Optimizations
- 🚀 **Hardware Offload**: TSO/GSO support on Linux for maximum throughput
- 🔀 **Multi-Queue**: Leverage multiple CPU cores with multi-queue support on Linux
- 🍎 **macOS TAP**: Native TAP implementation using `feth` pairs
- 🪟 **Windows DNS**: Full DNS configuration support on Windows

### Developer Experience
- 🎯 **Consistent Behavior**: Unified packet format across all platforms (no platform-specific headers)
- 🔄 **Automatic Routing**: Consistent route setup behavior when creating devices
- 🛑 **Graceful Shutdown**: Proper cleanup and shutdown support for sync operations
- 📝 **Rich Examples**: Comprehensive examples for common use cases

## 💻 Supported Platforms

| Platform     | TUN | TAP | Notes |
|--------------|:---:|:---:|-------|
| **Linux**    | ✅  | ✅  | Full offload & multi-queue support |
| **Windows**  | ✅  | ✅  | Requires wintun.dll / tap-windows |
| **macOS**    | ✅  | ✅* | TAP via feth pairs |
| **FreeBSD**  | ✅  | ✅  | Full support |
| **OpenBSD**  | ✅  | ✅  | Full support |
| **NetBSD**   | ✅  | ✅  | Full support |
| **Android**  | ✅  | -   | Via VpnService API |
| **iOS**      | ✅  | -   | Via NEPacketTunnelProvider |
| **tvOS**     | ✅  | -   | Via NEPacketTunnelProvider |
| **OpenHarmony** | ✅ | -  | TUN support |
| **Other Unix*** | ✅ | -  | Via raw file descriptor |

> **Note**: For unlisted Unix-like platforms, you can use the raw file descriptor API to integrate with platform-specific TUN implementations.

## 🚀 Performance Benchmarks

tun-rs delivers **exceptional performance** compared to other TUN implementations. Benchmarks conducted on Linux (Ubuntu 20.04, i7-13700K, DDR5 32GB) using [tun-benchmark2](https://github.com/tun-rs/tun-benchmark2) show impressive results:

### 🏆 Highlights

- **Peak Performance**: Up to **70.6 Gbps** with concurrent sync operations + offload
- **Best Async Performance**: **35.7 Gbps** async without channel buffering + offload
- **Optimized Async**: **31.4 Gbps** with BytesPool optimization
- **Rust vs Go**: Peak-to-peak, tun-rs achieves **2.3x higher throughput** (70.6 vs 30.1 Gbps)

### 📊 Performance Comparison

| Configuration | Throughput | CPU Usage | Memory | Retransmissions |
|--------------|------------|-----------|--------|-----------------|
| **Sync + Offload + Concurrent** | 🥇 **70.6 Gbps** | 124% | 10.6 MB | 2748 |
| **Async + Offload** | 🥈 **35.7 Gbps** | 64.9% | 7.4 MB | 0 |
| **Async + Offload + BytesPool** | 🥉 **31.4 Gbps** | 93.0% | 16.0 MB | 0 |
| Sync + Offload + Channel | 29.5 Gbps | 90.4% | 14.9 MB | 0 |
| Go + Offload + BytesPool | 30.1 Gbps | 101.6% | 39.5 MB | 0 |
| Go + Offload | 28.8 Gbps | 64.1% | 4.2 MB | 0 |
| Async (no offload) | 8.84 Gbps | 87.6% | 3.7 MB | 326 |

### 💡 Key Takeaways

1. **Hardware Offload is Critical**: TSO/GSO increases throughput by 3-4x
2. **Concurrent I/O Scales**: Dual-threaded sync operations achieve the highest throughput
3. **Memory Efficiency**: tun-rs uses significantly less memory than Go alternatives
4. **Zero Retransmissions**: Offload configurations achieve reliable, lossless transmission
5. **Flexible Performance Profile**: Choose async for low CPU usage or sync+concurrent for maximum throughput

<details>
<summary>📈 View Full Benchmark Chart</summary>

![Performance Benchmark](https://raw.githubusercontent.com/tun-rs/tun-benchmark2/main/flamegraph/canvas2.png)

</details>

> For detailed benchmark methodology and results, visit [tun-benchmark2](https://github.com/tun-rs/tun-benchmark2)

---

## 📦 Installation
Add tun-rs to your `Cargo.toml`:

```toml
[dependencies]
# Base synchronous API (no async runtime required)
tun-rs = "2"

# For Tokio async runtime
tun-rs = { version = "2", features = ["async"] }

# For async-std, smol, or other async-io based runtimes
tun-rs = { version = "2", features = ["async_io"] }

# For framed codec support (with tokio)
tun-rs = { version = "2", features = ["async", "async_framed"] }
```

---

## 🎓 Quick Start

### Basic Synchronous TUN Interface

Create and use a TUN interface in just a few lines:

```rust
use tun_rs::DeviceBuilder;

fn main() -> std::io::Result<()> {
    // Create a TUN device with IPv4 and IPv6 addresses
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .ipv6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)
        .mtu(1400)
        .build_sync()?;

    println!("TUN device created: {:?}", dev.name());
    
    let mut buf = [0; 1400];
    loop {
        let amount = dev.recv(&mut buf)?;
        println!("Received {} bytes: {:?}", amount, &buf[0..amount]);
    }
}
```

### Asynchronous TUN with Tokio

Use async/await for non-blocking I/O:

````rust
use tun_rs::DeviceBuilder;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .build_async()?;

    println!("Async TUN device ready!");
    
    let mut buf = vec![0; 65536];
    loop {
        let len = dev.recv(&mut buf).await?;
        println!("Received packet: {} bytes", len);
        
        // Echo the packet back
        dev.send(&buf[..len]).await?;
    }
}
````

---

## 📚 Examples

### Multiple IP Addresses
Assign multiple IP addresses to a single interface:

````rust
use tun_rs::DeviceBuilder;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .build_async()?;

    // Add multiple IPv4 addresses
    dev.add_address_v4("10.1.0.1", 24)?;
    dev.add_address_v4("10.2.0.1", 24)?;
    
    // Add multiple IPv6 addresses
    dev.add_address_v6("CDCD:910A:2222:5498:8475:1111:3900:2021", 64)?;
    dev.add_address_v6("BDCD:910A:2222:5498:8475:1111:3900:2021", 64)?;

    // Remove addresses when needed
    // dev.remove_address("10.2.0.1".parse().unwrap())?;
    
    println!("All addresses: {:?}", dev.addresses()?);

    let mut buf = vec![0; 65536];
    loop {
        let len = dev.recv(&mut buf).await?;
        println!("Received on multi-IP device: {} bytes", len);
    }
}
````

### Using Raw File Descriptor (Unix)

Create a device from an existing file descriptor (useful for iOS, Android, or custom TUN implementations):

```rust
use tun_rs::SyncDevice;
use std::os::unix::io::RawFd;

fn main() -> std::io::Result<()> {
    // The fd must be a valid file descriptor obtained from:
    // - iOS: Use getTunnelFileDescriptor() - see iOS section (KVO method deprecated on iOS 16+)
    // - Android: VpnService.Builder().establish().getFd()
    // - Linux: open("/dev/net/tun", O_RDWR)
    // - Or any other platform-specific TUN device API
    let fd: RawFd = get_tun_fd_from_platform(); // Your platform-specific function
    
    // Take ownership of the file descriptor
    let dev = unsafe { SyncDevice::from_fd(fd)? };
    
    // Or borrow without taking ownership
    // let dev = unsafe { tun_rs::BorrowedSyncDevice::borrow_raw(fd)? };

    // Also works with async devices
    // let async_dev = unsafe { tun_rs::AsyncDevice::from_fd(fd)? };

    let mut buf = [0; 4096];
    loop {
        let amount = dev.recv(&mut buf)?;
        println!("Received: {:?}", &buf[0..amount]);
    }
}

// NOTE: This is example-only code. In real applications, obtain the fd from:
// - iOS: Use getTunnelFileDescriptor() method (see iOS section) - KVO method deprecated on iOS 16+
// - Android: fd = vpnInterface.getFd()  (from VpnService.Builder().establish())
// - Linux: fd = open("/dev/net/tun", O_RDWR) or use DeviceBuilder instead
fn get_tun_fd_from_platform() -> RawFd {
    // See the iOS and Android sections below for complete working examples
    unimplemented!("Replace with your platform-specific fd acquisition code")
}
```

### Linux Hardware Offload (TSO/GSO)

Maximize performance with hardware offload features:

````rust
use tun_rs::DeviceBuilder;
#[cfg(target_os = "linux")]
use tun_rs::{GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

#[cfg(target_os = "linux")]
fn main() -> std::io::Result<()> {
    let builder = DeviceBuilder::new()
        .offload(true)  // Enable TSO/GSO for 3-4x throughput boost
        // .multi_queue(true)  // Enable multi-queue for concurrent I/O
        .ipv4("10.0.0.1", 24, None)
        .mtu(1400);

    let dev = builder.build_sync()?;
    
    // For multi-queue, clone the device for use in other threads
    // let dev_clone = dev.try_clone()?; 
    
    let mut original_buffer = vec![0; VIRTIO_NET_HDR_LEN + 65535];
    let mut bufs = vec![vec![0u8; 1500]; IDEAL_BATCH_SIZE];
    let mut sizes = vec![0; IDEAL_BATCH_SIZE];
    
    // Optional: Use GROTable for Generic Receive Offload optimization
    // let mut gro_table = GROTable::default();
    
    loop {
        let num = dev.recv_multiple(&mut original_buffer, &mut bufs, &mut sizes, 0)?;
        println!("Received {} packets in batch", num);
        for i in 0..num {
            println!("Packet {}: {} bytes", i, sizes[i]);
        }
    }
}
````

### TAP Interface (Layer 2)

```rust
use tun_rs::{DeviceBuilder, Layer};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .layer(Layer::L2)  // TAP mode for Ethernet frames
        .build_async()?;

    let mut buf = vec![0; 65536];
    loop {
        let len = dev.recv(&mut buf).await?;
        // Process Ethernet frame
        println!("Ethernet frame: {} bytes", len);
    }
}
```

> **💡 More Examples**: Check out [examples/](https://github.com/tun-rs/tun-rs/tree/main/examples) for additional use cases including:
> - ICMP ping responder
> - iOS/Android integration
> - Framed codec usage
> - Interruptible operations
> - And more!

---

## 🔧 Platform-Specific Setup

### Linux
**Requirements:**
- TUN kernel module must be loaded: `sudo modprobe tun`
- Root privileges or `CAP_NET_ADMIN` capability required

**Performance Tips:**
- Enable hardware offload for 3-4x throughput improvement
- Use multi-queue for concurrent operations across multiple cores
- Consider using `recv_multiple()` for batch packet processing

```bash
# Load TUN module
sudo modprobe tun

# Run your application
sudo ./your_app
```

### macOS & BSD
**Automatic Routing:**
tun-rs automatically configures routes based on your IP settings:

```bash
# Example: This route is added automatically for 10.0.0.0/24
sudo route -n add -net 10.0.0.0/24 10.0.0.1
```

**TAP Mode on macOS:**
- Implemented using `feth` interface pairs
- Interfaces persist until explicitly destroyed
- Uses BPF for I/O operations
- Multiple file descriptors involved (be careful with `AsRawFd`)

```rust
use tun_rs::{DeviceBuilder, Layer};

fn main() -> std::io::Result<()> {
    let dev = DeviceBuilder::new()
        .layer(Layer::L2)  // TAP mode
        .ipv4("10.0.0.1", 24, None)
        .build_sync()?;
    
    // TAP interface is ready
    println!("macOS TAP device: {:?}", dev.name());
    Ok(())
}
```

### Windows

**TUN Mode:**
1. Download [wintun.dll](https://wintun.net/) matching your architecture (x64, x86, ARM, or ARM64)
2. Place `wintun.dll` in the same directory as your executable
3. Run your application as Administrator

**TAP Mode:**
1. Install [tap-windows](https://build.openvpn.net/downloads/releases/) matching your architecture
2. Run your application as Administrator

```rust
use tun_rs::DeviceBuilder;

fn main() -> std::io::Result<()> {
    // Windows supports DNS configuration
    let dev = DeviceBuilder::new()
        .ipv4("10.0.0.1", 24, None)
        .build_sync()?;
    
    // Set DNS servers (Windows-specific)
    #[cfg(windows)]
    {
        // Configure DNS if needed
        println!("TUN version: {:?}", dev.version());
    }
    
    Ok(())
}
```

### iOS / tvOS

Integrate with `NEPacketTunnelProvider`. **Note:** On iOS 16+, the KVO method for getting the file descriptor may not work. Use the robust method below:

```swift
// Swift
class PacketTunnelProvider: NEPacketTunnelProvider {
    
    // Robust method to get file descriptor (iOS 16+ compatible)
    private func getTunnelFileDescriptor() -> Int32? {
        var ctlInfo = ctl_info()
        withUnsafeMutablePointer(to: &ctlInfo.ctl_name) {
            $0.withMemoryRebound(to: CChar.self, capacity: MemoryLayout.size(ofValue: $0.pointee)) {
                _ = strcpy($0, "com.apple.net.utun_control")
            }
        }
        
        // Search for the utun socket (range 0...1024 from WireGuard; typically found quickly)
        for fd: Int32 in 0...1024 {
            var addr = sockaddr_ctl()
            var ret: Int32 = -1
            var len = socklen_t(MemoryLayout.size(ofValue: addr))
            
            withUnsafeMutablePointer(to: &addr) {
                $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    ret = getpeername(fd, $0, &len)
                }
            }
            
            if ret != 0 || addr.sc_family != AF_SYSTEM {
                continue
            }
            
            if ctlInfo.ctl_id == 0 {
                ret = ioctl(fd, CTLIOCGINFO, &ctlInfo)
                if ret != 0 {
                    continue
                }
            }
            
            if addr.sc_id == ctlInfo.ctl_id {
                return fd
            }
        }
        return nil
    }
    
    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        let tunnelNetworkSettings = createTunnelSettings() // Configure TUN address, DNS, mtu, routing...
        setTunnelNetworkSettings(tunnelNetworkSettings) { [weak self] error in
            guard let self = self else { return }
            
            // Get the file descriptor using the robust method
            guard let tunFd = self.getTunnelFileDescriptor() else {
                completionHandler(NSError(domain: "TunnelError", code: 2, 
                    userInfo: [NSLocalizedDescriptionKey: "Cannot locate tunnel file descriptor"]))
                return
            }
            
            DispatchQueue.global(qos: .userInitiated).async {
                start_tun(tunFd)
            }
            completionHandler(nil)
        }
    }
}
```

```rust
// Rust FFI
#[no_mangle]
pub extern "C" fn start_tun(fd: std::os::raw::c_int) {
    // Create device from file descriptor
    let tun = unsafe { tun_rs::SyncDevice::from_fd(fd).unwrap() };
    let mut buf = [0u8; 1500];
    while let Ok(len) = tun.recv(&mut buf) {
        // Process packet
        println!("iOS: received {} bytes", len);
    }
}
```

> **📖 For complete iOS/tvOS integration guide**, including async examples, WireGuardKit method, and troubleshooting, see [docs/iOS-Integration.md](docs/iOS-Integration.md)

### Android

Integrate with Android `VpnService`:

```java
// Java
private void startVpn() {
    VpnService.Builder builder = new VpnService.Builder();
    builder
        .allowFamily(OsConstants.AF_INET)
        .addAddress("10.0.0.2", 24);
    ParcelFileDescriptor vpnInterface = builder
        .setSession("tun-rs")
        .establish();
    int fd = vpnInterface.getFd();
    
    // Pass fd to Rust via JNI
    startTunNative(fd);
}
```

```rust
// Rust JNI binding
#[no_mangle]
pub extern "C" fn startTunNative(fd: std::os::raw::c_int) {
    let tun = unsafe { tun_rs::SyncDevice::from_fd(fd).unwrap() };
    let mut buf = [0u8; 1500];
    while let Ok(len) = tun.recv(&mut buf) {
        // Process packet
    }
}
```

---

## 🤝 Comparison with Other Libraries

| Feature | tun-rs | go-tun | tun-tap/tokio-tun |
|---------|--------|--------|-------------------|
| **Peak Throughput** | 70.6 Gbps | 30.1 Gbps | Not benchmarked |
| **Memory Usage** | ✅ Low (3-16 MB) | ❌ High (39-43 MB) | Unknown |
| **Cross-Platform** | ✅ 11+ platforms | ⚠️ Limited | ⚠️ Linux/macOS only |
| **Async Support** | ✅ Tokio + async-io | ✅ Go runtime | ⚠️ Varies by lib |
| **Hardware Offload** | ✅ TSO/GSO/Multi-queue | ✅ TSO/GSO | ❌ No |
| **Multiple IPs** | ✅ Yes | ❌ No | ❌ No |
| **TAP on macOS** | ✅ Yes (feth) | ❌ No | ❌ No |
| **Mobile Support** | ✅ iOS/Android | ❌ No | ❌ No |

> **Note**: Benchmarks for go-tun were conducted in the same environment. Other Rust TUN libraries lack comprehensive cross-platform support and haven't been benchmarked in our test suite.

---

## 🛠️ API Overview

### Device Creation

```rust
use tun_rs::{DeviceBuilder, Layer};

// Build with configuration
let dev = DeviceBuilder::new()
    .name("tun0")                    // Optional: specify interface name
    .layer(Layer::L3)                // L3 (TUN) or L2 (TAP)
    .ipv4("10.0.0.1", 24, None)     // IPv4 address, prefix, destination
    .ipv6("fd00::1", 64)            // IPv6 address, prefix
    .mtu(1400)                       // Set MTU
    .offload(true)                   // Enable offload (Linux only)
    .multi_queue(true)               // Enable multi-queue (Linux only)
    .build_sync()?;                  // Or build_async()
```

### Core Operations

```rust
// Receive packet
let mut buf = vec![0; 65536];
let len = dev.recv(&mut buf)?;

// Send packet
dev.send(&packet)?;

// Device information
let name = dev.name()?;
let mtu = dev.mtu()?;
let addresses = dev.addresses()?;
let if_index = dev.if_index()?;

// Address management
dev.add_address_v4("10.1.0.1", 24)?;
dev.add_address_v6("fd00::2", 64)?;
dev.remove_address(addr)?;
```

### Async Operations

```rust
// Async recv/send
let len = dev.recv(&mut buf).await?;
dev.send(&packet).await?;

// Use with tokio::select!
tokio::select! {
    result = dev.recv(&mut buf) => { /* handle */ }
    _ = shutdown_signal => { /* cleanup */ }
}
```

---

## 📖 Documentation

- **API Documentation**: [docs.rs/tun-rs](https://docs.rs/tun-rs)
- **iOS/tvOS Integration**: [docs/iOS-Integration.md](docs/iOS-Integration.md)
- **Examples**: [github.com/tun-rs/tun-rs/tree/main/examples](https://github.com/tun-rs/tun-rs/tree/main/examples)
- **Benchmark Details**: [github.com/tun-rs/tun-benchmark2](https://github.com/tun-rs/tun-benchmark2)

---

## 🐛 Troubleshooting

<details>
<summary><b>Permission denied when creating TUN interface</b></summary>

**Linux/BSD/macOS**: Run with `sudo` or grant `CAP_NET_ADMIN` capability:
```bash
sudo ./your_app
# Or grant capability
sudo setcap cap_net_admin+ep ./your_app
```

**Windows**: Run as Administrator

**Android/iOS**: Use platform VPN APIs to obtain file descriptor
</details>

<details>
<summary><b>Module not loaded on Linux</b></summary>

```bash
sudo modprobe tun
# Make it persistent
echo "tun" | sudo tee -a /etc/modules
```
</details>

<details>
<summary><b>wintun.dll not found on Windows</b></summary>

1. Download from [wintun.net](https://wintun.net/)
2. Extract the DLL for your architecture (x64, x86, ARM, ARM64)
3. Place in the same directory as your executable
</details>

<details>
<summary><b>Low performance without offload</b></summary>

On Linux, enable hardware offload for 3-4x performance boost:
```rust
let dev = DeviceBuilder::new()
    .offload(true)
    .build_sync()?;
```
</details>

<details>
<summary><b>iOS file descriptor returns nil (iOS 16+)</b></summary>

**Symptom:** `packetFlow.value(forKeyPath: "socket.fileDescriptor")` returns `nil` on iOS 16+

**Solution:** The KVO method is deprecated. Use the robust file descriptor search method instead:

```swift
private func getTunnelFileDescriptor() -> Int32? {
    var ctlInfo = ctl_info()
    withUnsafeMutablePointer(to: &ctlInfo.ctl_name) {
        $0.withMemoryRebound(to: CChar.self, capacity: MemoryLayout.size(ofValue: $0.pointee)) {
            _ = strcpy($0, "com.apple.net.utun_control")
        }
    }
    
    // Range 0...1024 from WireGuard implementation; typically found quickly in low range
    for fd: Int32 in 0...1024 {
        var addr = sockaddr_ctl()
        var ret: Int32 = -1
        var len = socklen_t(MemoryLayout.size(ofValue: addr))
        
        withUnsafeMutablePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                ret = getpeername(fd, $0, &len)
            }
        }
        
        if ret != 0 || addr.sc_family != AF_SYSTEM {
            continue
        }
        
        if ctlInfo.ctl_id == 0 {
            ret = ioctl(fd, CTLIOCGINFO, &ctlInfo)
            if ret != 0 {
                continue
            }
        }
        
        if addr.sc_id == ctlInfo.ctl_id {
            return fd
        }
    }
    return nil
}
```

See [docs/iOS-Integration.md](docs/iOS-Integration.md) for complete examples and troubleshooting.
</details>

---

## 🙏 Contributing

Contributions are welcome! Please feel free to submit a Pull Request. For major changes, please open an issue first to discuss what you would like to change.

### Development

```bash
# Clone repository
git clone https://github.com/tun-rs/tun-rs.git
cd tun-rs

# Run tests (requires root/admin)
cargo test

# Build examples
cargo build --examples --features async

# Run example
sudo ./target/debug/examples/read
```

---

## 📄 License

Licensed under the [Apache License 2.0](LICENSE)

---

## 🌟 Acknowledgments

- Thanks to all [contributors](https://github.com/tun-rs/tun-rs/graphs/contributors)
- Inspired by the networking community's need for high-performance TUN/TAP implementations
- Special thanks to the Rust async ecosystem (tokio, async-io) for making async networking seamless

---

<div align="center">

**[⬆ Back to Top](#-tun-rs)**

Made with ❤️ by the tun-rs team

</div>
