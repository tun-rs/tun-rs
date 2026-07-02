/// Example demonstrating the use of both Tokio and async-std runtimes simultaneously
/// 
/// This example shows that when both async_tokio and async_io features are enabled,
/// you can use both runtimes in the same application.

#[cfg(not(all(feature = "async_tokio", feature = "async_io")))]
fn main() {
    eprintln!("This example requires both 'async_tokio' and 'async_io' features to be enabled.");
    eprintln!("Run with: cargo run --example dual_runtime --features async_tokio,async_io");
}

#[cfg(all(
    feature = "async_tokio",
    feature = "async_io",
    any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    )
))]
fn main() {
    use std::net::Ipv4Addr;
    use tun_rs::{DeviceBuilder, TokioAsyncDevice, AsyncIoDevice};
    
    println!("=== Dual Runtime Example ===");
    println!("This example demonstrates using both Tokio and async-std runtimes simultaneously.");
    println!();
    
    // Create a Tokio runtime device
    println!("Creating Tokio runtime device...");
    let tokio_device_result = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 100, 0, 1), 24, None)
        .build_tokio_async();
    
    match tokio_device_result {
        Ok(device) => {
            println!("✓ Tokio device created successfully");
            println!("  Name: {:?}", device.name());
            println!("  MTU: {:?}", device.mtu());
            
            // Run a Tokio task
            let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
            tokio_runtime.block_on(async {
                println!("  Running in Tokio runtime");
                // Wait for device to be readable (with timeout)
                let readable_future = device.readable();
                let timeout = tokio::time::sleep(tokio::time::Duration::from_millis(100));
                tokio::select! {
                    result = readable_future => {
                        println!("  Device is readable: {:?}", result);
                    }
                    _ = timeout => {
                        println!("  Timeout waiting for readable (expected in this example)");
                    }
                }
            });
        }
        Err(e) => {
            eprintln!("✗ Failed to create Tokio device: {}", e);
            eprintln!("  (This is expected if you don't have permission to create TUN devices)");
        }
    }
    
    println!();
    
    // Create an async-std runtime device
    println!("Creating async-std runtime device...");
    let async_io_device_result = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 100, 0, 2), 24, None)
        .build_async_io();
    
    match async_io_device_result {
        Ok(device) => {
            println!("✓ async-std device created successfully");
            println!("  Name: {:?}", device.name());
            println!("  MTU: {:?}", device.mtu());
            
            // Run an async-std task
            async_std::task::block_on(async {
                println!("  Running in async-std runtime");
                // Wait for device to be readable (with timeout)
                let readable_future = device.readable();
                let timeout = async_std::task::sleep(std::time::Duration::from_millis(100));
                match futures::future::select(
                    Box::pin(readable_future),
                    Box::pin(timeout)
                ).await {
                    futures::future::Either::Left((result, _)) => {
                        println!("  Device is readable: {:?}", result);
                    }
                    futures::future::Either::Right(_) => {
                        println!("  Timeout waiting for readable (expected in this example)");
                    }
                }
            });
        }
        Err(e) => {
            eprintln!("✗ Failed to create async-std device: {}", e);
            eprintln!("  (This is expected if you don't have permission to create TUN devices)");
        }
    }
    
    println!();
    
    // Demonstrate backward compatibility
    println!("Testing backward compatibility...");
    println!("When both features are enabled, AsyncDevice defaults to TokioAsyncDevice");
    let default_device_result = DeviceBuilder::new()
        .ipv4(Ipv4Addr::new(10, 100, 0, 3), 24, None)
        .build_async();
    
    match default_device_result {
        Ok(device) => {
            println!("✓ Default AsyncDevice created successfully (uses Tokio runtime)");
            println!("  Name: {:?}", device.name());
            println!("  MTU: {:?}", device.mtu());
        }
        Err(e) => {
            eprintln!("✗ Failed to create default device: {}", e);
            eprintln!("  (This is expected if you don't have permission to create TUN devices)");
        }
    }
    
    println!();
    println!("=== Example Complete ===");
    println!("Both runtimes can coexist in the same application!");
}

#[cfg(all(
    feature = "async_tokio",
    feature = "async_io",
    target_os = "windows"
))]
fn main() {
    println!("=== Dual Runtime Example (Windows) ===");
    println!("On Windows, AsyncDevice uses the blocking crate which is runtime-agnostic.");
    println!("Both Tokio and async-std can be used simultaneously.");
    println!();
    println!("Note: This example requires TAP-Windows or wintun driver to be installed.");
}
