# iOS/tvOS Integration Guide

This guide provides detailed instructions for integrating tun-rs with iOS and tvOS applications using `NEPacketTunnelProvider`.

## Table of Contents

- [Overview](#overview)
- [Getting the File Descriptor](#getting-the-file-descriptor)
  - [iOS 16+ Recommended Method](#ios-16-recommended-method)
  - [Legacy KVO Method](#legacy-kvo-method)
- [Complete Integration Example](#complete-integration-example)
- [WireGuardKit Method (Alternative)](#wireguardkit-method-alternative)
- [Troubleshooting](#troubleshooting)

## Overview

On iOS and tvOS, you cannot create TUN interfaces directly. Instead, you must:
1. Use `NEPacketTunnelProvider` to establish a VPN tunnel
2. Get the file descriptor from the packet flow
3. Pass the file descriptor to your Rust code via FFI
4. Use `tun_rs::SyncDevice::from_fd()` or `tun_rs::AsyncDevice::from_fd()` to manage the tunnel

## Getting the File Descriptor

### iOS 16+ Recommended Method

Starting with iOS 16, the Key-Value Observing (KVO) approach for accessing `socket.fileDescriptor` may return `nil`. The recommended approach is to search for the file descriptor by iterating through available file descriptors and matching against the utun control socket.

Here's the robust method adapted from [WireGuard](https://github.com/WireGuard/wireguard-apple):

```swift
import Foundation
import NetworkExtension

class PacketTunnelProvider: NEPacketTunnelProvider {
    
    /// Finds the tunnel file descriptor by searching through available file descriptors
    private func getTunnelFileDescriptor() -> Int32? {
        var ctlInfo = ctl_info()
        withUnsafeMutablePointer(to: &ctlInfo.ctl_name) {
            $0.withMemoryRebound(to: CChar.self, capacity: MemoryLayout.size(ofValue: $0.pointee)) {
                _ = strcpy($0, "com.apple.net.utun_control")
            }
        }
        
        // Search through file descriptors to find the utun socket
        // Note: Range 0...1024 is used to ensure we find the fd. In practice, the utun
        // socket is typically in the low range (< 100) and found quickly. This method
        // is from WireGuard's production implementation and only runs once at tunnel startup.
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
        // Configure tunnel settings
        let tunnelNetworkSettings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "10.0.0.1")
        tunnelNetworkSettings.mtu = 1400
        
        let ipv4Settings = NEIPv4Settings(addresses: ["10.0.0.2"], subnetMasks: ["255.255.255.0"])
        ipv4Settings.includedRoutes = [NEIPv4Route.default()]
        tunnelNetworkSettings.ipv4Settings = ipv4Settings
        
        // Apply settings first
        setTunnelNetworkSettings(tunnelNetworkSettings) { [weak self] error in
            guard let self = self else {
                completionHandler(NSError(domain: "TunnelError", code: 1, userInfo: [NSLocalizedDescriptionKey: "Self deallocated"]))
                return
            }
            
            if let error = error {
                completionHandler(error)
                return
            }
            
            // Get the file descriptor after settings are applied
            guard let tunFd = self.getTunnelFileDescriptor() else {
                completionHandler(NSError(domain: "TunnelError", code: 2, userInfo: [NSLocalizedDescriptionKey: "Cannot locate tunnel file descriptor"]))
                return
            }
            
            os_log(.default, "Starting tunnel with fd %{public}d", tunFd)
            
            // Start your Rust tunnel implementation on a background thread
            DispatchQueue.global(qos: .userInitiated).async {
                start_tun(tunFd)
            }
            
            completionHandler(nil)
        }
    }
    
    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        // Implement cleanup if needed
        os_log(.default, "Stopping tunnel, reason: %{public}@", String(describing: reason))
        completionHandler()
    }
}
```

### Legacy KVO Method

**⚠️ Warning:** This method is **deprecated** and may not work on iOS 16 and later.

```swift
// This may return nil on iOS 16+
let tunFd = self.packetFlow.value(forKeyPath: "socket.fileDescriptor") as? Int32
guard let unwrappedFd = tunFd else {
    os_log(.error, "Cannot start tunnel: file descriptor is nil")
    return
}
```

If you encounter issues with the file descriptor being `nil`, switch to the recommended method above.

## Complete Integration Example

Here's a complete example showing both Swift and Rust sides:

### Swift Side (PacketTunnelProvider)

```swift
import NetworkExtension
import os.log

class PacketTunnelProvider: NEPacketTunnelProvider {
    
    private var tunnelQueue: DispatchQueue?
    
    private func getTunnelFileDescriptor() -> Int32? {
        var ctlInfo = ctl_info()
        withUnsafeMutablePointer(to: &ctlInfo.ctl_name) {
            $0.withMemoryRebound(to: CChar.self, capacity: MemoryLayout.size(ofValue: $0.pointee)) {
                _ = strcpy($0, "com.apple.net.utun_control")
            }
        }
        
        // Search through file descriptors to find the utun socket
        // Note: Range 0...1024 ensures we find the fd. In practice, found quickly in low range.
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
        os_log(.info, "Starting tunnel...")
        
        // 1. Create tunnel network settings
        let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: "10.0.0.1")
        settings.mtu = 1400
        
        // 2. Configure IPv4
        let ipv4Settings = NEIPv4Settings(addresses: ["10.0.0.2"], subnetMasks: ["255.255.255.0"])
        ipv4Settings.includedRoutes = [NEIPv4Route.default()]
        settings.ipv4Settings = ipv4Settings
        
        // 3. Configure DNS (optional)
        let dnsSettings = NEDNSSettings(servers: ["8.8.8.8", "8.8.4.4"])
        settings.dnsSettings = dnsSettings
        
        // 4. Apply settings
        setTunnelNetworkSettings(settings) { [weak self] error in
            guard let self = self else {
                completionHandler(NSError(domain: "TunnelError", code: 1))
                return
            }
            
            if let error = error {
                os_log(.error, "Failed to set tunnel settings: %{public}@", error.localizedDescription)
                completionHandler(error)
                return
            }
            
            // 5. Get file descriptor
            guard let tunFd = self.getTunnelFileDescriptor() else {
                let error = NSError(
                    domain: "TunnelError",
                    code: 2,
                    userInfo: [NSLocalizedDescriptionKey: "Cannot locate tunnel file descriptor"]
                )
                os_log(.error, "Failed to get file descriptor")
                completionHandler(error)
                return
            }
            
            os_log(.info, "Tunnel file descriptor obtained: %{public}d", tunFd)
            
            // 6. Start Rust tunnel processing
            let queue = DispatchQueue(label: "com.yourapp.tunnel", qos: .userInitiated)
            self.tunnelQueue = queue
            
            queue.async {
                os_log(.info, "Starting Rust tunnel processing...")
                // This will block until the tunnel stops
                start_tun(tunFd)
                os_log(.info, "Rust tunnel processing ended")
            }
            
            completionHandler(nil)
        }
    }
    
    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping () -> Void) {
        os_log(.info, "Stopping tunnel, reason: %{public}@", String(describing: reason))
        
        // Signal your Rust code to stop (you may need to implement a stop mechanism)
        stop_tun()
        
        completionHandler()
    }
    
    override func handleAppMessage(_ messageData: Data, completionHandler: ((Data?) -> Void)?) {
        // Handle messages from the main app if needed
        if let handler = completionHandler {
            handler(nil)
        }
    }
}
```

### Rust Side (FFI + tun-rs)

```rust
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tun_rs::SyncDevice;

// Global flag to signal tunnel shutdown
static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn start_tun(fd: std::os::raw::c_int) {
    SHOULD_STOP.store(false, Ordering::SeqCst);
    
    // Create device from file descriptor
    let tun = unsafe {
        match SyncDevice::from_fd(fd as RawFd) {
            Ok(device) => device,
            Err(e) => {
                eprintln!("Failed to create device from fd {}: {:?}", fd, e);
                return;
            }
        }
    };
    
    println!("Tunnel started with fd {}", fd);
    
    let mut buf = vec![0u8; 4096];
    
    // Main packet processing loop
    loop {
        // Check if we should stop
        if SHOULD_STOP.load(Ordering::SeqCst) {
            println!("Stop signal received, exiting tunnel loop");
            break;
        }
        
        // Receive packet
        match tun.recv(&mut buf) {
            Ok(len) => {
                println!("Received packet: {} bytes", len);
                
                // Process packet here
                // For example, parse IP header, handle routing, etc.
                
                // Echo back (for testing)
                if let Err(e) = tun.send(&buf[..len]) {
                    eprintln!("Failed to send packet: {:?}", e);
                }
            }
            Err(e) => {
                eprintln!("Failed to receive packet: {:?}", e);
                // Consider whether to break or continue based on error type
                if e.kind() == std::io::ErrorKind::Interrupted {
                    continue;
                }
                break;
            }
        }
    }
    
    println!("Tunnel loop ended");
}

#[no_mangle]
pub extern "C" fn stop_tun() {
    println!("Stopping tunnel...");
    SHOULD_STOP.store(true, Ordering::SeqCst);
}
```

### Async Version (Tokio)

If you prefer async operations:

```rust
use std::os::unix::io::RawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::runtime::Runtime;
use tun_rs::AsyncDevice;

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "C" fn start_tun(fd: std::os::raw::c_int) {
    SHOULD_STOP.store(false, Ordering::SeqCst);
    
    // Create a Tokio runtime
    let rt = match Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create runtime: {:?}", e);
            return;
        }
    };
    
    rt.block_on(async move {
        // Create async device from file descriptor
        let tun = unsafe {
            match AsyncDevice::from_fd(fd as RawFd) {
                Ok(device) => device,
                Err(e) => {
                    eprintln!("Failed to create device from fd {}: {:?}", fd, e);
                    return;
                }
            }
        };
        
        println!("Async tunnel started with fd {}", fd);
        
        let mut buf = vec![0u8; 4096];
        
        loop {
            if SHOULD_STOP.load(Ordering::SeqCst) {
                println!("Stop signal received");
                break;
            }
            
            // Async receive
            match tun.recv(&mut buf).await {
                Ok(len) => {
                    println!("Received packet: {} bytes", len);
                    
                    // Process and echo back
                    if let Err(e) = tun.send(&buf[..len]).await {
                        eprintln!("Failed to send: {:?}", e);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to receive: {:?}", e);
                    break;
                }
            }
        }
        
        println!("Async tunnel loop ended");
    });
}

#[no_mangle]
pub extern "C" fn stop_tun() {
    println!("Stopping tunnel...");
    SHOULD_STOP.store(true, Ordering::SeqCst);
}
```

### Cargo Configuration

Add tun-rs to your `Cargo.toml`:

```toml
[lib]
name = "your_tunnel_lib"
crate-type = ["staticlib", "cdylib"]

[dependencies]
tun-rs = "2"

# For async support
# tun-rs = { version = "2", features = ["async"] }
# tokio = { version = "1", features = ["rt", "rt-multi-thread"] }
```

## WireGuardKit Method (Alternative)

If you prefer to use the WireGuardKit approach directly, you can integrate it into your project:

### Setup

1. Add the `WireGuardKit` folder from [wireguard-apple](https://github.com/WireGuard/wireguard-apple) to your Xcode project
2. Create a bridging header (e.g., `YourApp-Bridging-Header.h`) and include:

```objc
#import "WireGuardKitC/WireGuardKitC.h"
```

3. Configure the bridging header in your Xcode project build settings

### Using WireGuardKit's Adapter

```swift
import WireGuardKit

class PacketTunnelProvider: NEPacketTunnelProvider {
    private var adapter: WireGuardAdapter?
    
    override func startTunnel(options: [String : NSObject]?, completionHandler: @escaping (Error?) -> Void) {
        let adapter = WireGuardAdapter(with: self) { logLevel, message in
            print("[\(logLevel)]: \(message)")
        }
        self.adapter = adapter
        
        // Now you can use adapter.tunnelFileDescriptor
        if let fd = adapter.tunnelFileDescriptor {
            // Pass to your Rust code
            start_tun(fd)
        }
        
        // Continue with tunnel setup...
    }
}
```

## Troubleshooting

### File Descriptor is Nil

**Symptom:** `getTunnelFileDescriptor()` returns `nil`

**Solutions:**
1. Ensure `setTunnelNetworkSettings()` completes successfully before trying to get the FD
2. Check that your app has the proper VPN entitlements
3. Verify the Network Extension capability is enabled in your project
4. Make sure to call `getTunnelFileDescriptor()` **after** tunnel settings are applied

### Build Fails with Missing Symbols

**Symptom:** Linking errors when building the Rust library

**Solutions:**
1. Ensure your `Cargo.toml` specifies the correct crate-type:
   ```toml
   [lib]
   crate-type = ["staticlib"]
   ```
2. Build the Rust library for iOS targets:
   ```bash
   cargo build --target aarch64-apple-ios --release
   cargo build --target x86_64-apple-ios --release  # For simulator
   ```

### Tunnel Stops Unexpectedly

**Symptom:** Tunnel disconnects without clear error

**Solutions:**
1. Check system logs in Console.app
2. Implement proper error handling in your Rust code
3. Ensure the packet processing thread doesn't crash
4. Use `os_log` extensively for debugging

### Entitlements Issues

**Required Entitlements:**

In your Network Extension's entitlements file:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>com.apple.developer.networking.networkextension</key>
    <array>
        <string>packet-tunnel-provider</string>
    </array>
</dict>
</plist>
```

### Permissions and Capabilities

1. Enable **Network Extensions** capability in your Xcode project
2. Ensure your provisioning profile includes the Network Extensions entitlement
3. Test on a physical device (some features may not work in simulator)

## Additional Resources

- [Apple's Network Extension Documentation](https://developer.apple.com/documentation/networkextension)
- [NEPacketTunnelProvider Reference](https://developer.apple.com/documentation/networkextension/nepackettunnelprovider)
- [WireGuard iOS Implementation](https://github.com/WireGuard/wireguard-apple)
- [tun-rs Examples](https://github.com/tun-rs/tun-rs/tree/main/examples)

## Notes

- The file descriptor search method iterates through FDs 0-1024. This range is from WireGuard's production implementation and covers all realistic scenarios. In practice, the utun socket is typically found in the low range (< 100) very quickly, and this only runs once at tunnel startup, so performance impact is negligible.
- On iOS, you cannot create TUN devices directly using `DeviceBuilder`. You must use the file descriptor from `NEPacketTunnelProvider`.
- The tunnel processing should run on a background thread/queue to avoid blocking the main thread.
- Proper error handling and logging are crucial for debugging iOS Network Extensions, as they run in a sandboxed environment with limited debugging capabilities.
