// Test that both tokio and async_io features can be enabled simultaneously

#[cfg(all(
    feature = "async_tokio",
    feature = "async_io",
    any(target_os = "linux", target_os = "macos")
))]
#[tokio::test]
async fn test_tokio_device() {
    use tun_rs::{TokioAsyncDevice, DeviceBuilder};
    
    // This test verifies TokioAsyncDevice can be created and used
    let device = DeviceBuilder::new()
        .ipv4("10.99.0.1", 24, None)
        .build_tokio_async();
    
    // If device creation fails (e.g., due to permissions), skip the test
    if device.is_err() {
        eprintln!("Skipping test due to device creation error (likely permissions)");
        return;
    }
    
    let device = device.unwrap();
    
    // Just verify we can call methods on the device
    assert!(device.mtu().is_ok());
}

#[cfg(all(
    feature = "async_tokio",
    feature = "async_io",
    any(target_os = "linux", target_os = "macos")
))]
#[async_std::test]
async fn test_async_io_device() {
    use tun_rs::{AsyncIoDevice, DeviceBuilder};
    
    // This test verifies AsyncIoDevice can be created and used
    let device = DeviceBuilder::new()
        .ipv4("10.99.0.2", 24, None)
        .build_async_io();
    
    // If device creation fails (e.g., due to permissions), skip the test
    if device.is_err() {
        eprintln!("Skipping test due to device creation error (likely permissions)");
        return;
    }
    
    let device = device.unwrap();
    
    // Just verify we can call methods on the device
    assert!(device.mtu().is_ok());
}

#[cfg(all(
    feature = "async_tokio",
    feature = "async_io",
    any(target_os = "linux", target_os = "macos")
))]
#[tokio::test]
async fn test_async_device_defaults_to_tokio() {
    use tun_rs::{AsyncDevice, TokioAsyncDevice};
    
    // Verify that AsyncDevice is the same type as TokioAsyncDevice for backward compatibility
    // This is a compile-time check
    fn assert_same_type<T>(_: &T, _: &T) {}
    
    let device = std::mem::MaybeUninit::<AsyncDevice>::uninit();
    let tokio_device = std::mem::MaybeUninit::<TokioAsyncDevice>::uninit();
    
    // This will only compile if they're the same type
    unsafe {
        assert_same_type(&*device.as_ptr(), &*tokio_device.as_ptr());
    }
}

#[cfg(not(all(feature = "async_tokio", feature = "async_io")))]
#[test]
fn test_dual_runtime_not_enabled() {
    // This test runs when both features are not enabled
    // It's here to ensure the test file compiles even without both features
    println!("Dual runtime test requires both async_tokio and async_io features");
}
