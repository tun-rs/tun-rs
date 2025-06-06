mod device;
pub(crate) mod ffi;
#[cfg(feature = "interruptible")]
mod interrupt;
mod netsh;
mod tap;
mod tun;
#[cfg(feature = "interruptible")]
pub use interrupt::InterruptEvent;

pub use device::DeviceImpl;
