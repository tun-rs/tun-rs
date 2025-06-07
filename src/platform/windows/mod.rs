mod device;
pub(crate) mod ffi;
#[cfg(any(
    feature = "interruptible",
    feature = "async_tokio",
    feature = "async_io"
))]
mod interrupt;
mod netsh;
mod tap;
mod tun;
#[cfg(any(
    feature = "interruptible",
    feature = "async_tokio",
    feature = "async_io"
))]
pub use interrupt::InterruptEvent;

pub use device::DeviceImpl;
