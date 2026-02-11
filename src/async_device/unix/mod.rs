#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use crate::platform::offload::{handle_gro, VirtioNetHdr, VIRTIO_NET_HDR_LEN};
use crate::platform::DeviceImpl;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use crate::platform::GROTable;
use crate::SyncDevice;
use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::ops::Deref;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, RawFd};

#[cfg(feature = "async_tokio")]
mod tokio;
#[cfg(feature = "async_tokio")]
pub use self::tokio::TokioAsyncDevice;

#[cfg(feature = "async_io")]
mod async_io;
#[cfg(feature = "async_io")]
pub use self::async_io::AsyncIoDevice;

// For backward compatibility, AsyncDevice is an alias for the Tokio version when available
// If only async_io is enabled, it aliases to AsyncIoDevice
#[cfg(all(feature = "async_tokio", not(feature = "async_io")))]
pub type AsyncDevice = TokioAsyncDevice;
#[cfg(all(feature = "async_io", not(feature = "async_tokio")))]
pub type AsyncDevice = AsyncIoDevice;
#[cfg(all(feature = "async_tokio", feature = "async_io"))]
pub type AsyncDevice = TokioAsyncDevice;

// Trait implementations for AsyncDevice type alias are inherited from the concrete types
