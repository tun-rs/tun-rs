#[cfg(unix)]
pub(crate) mod unix;

use std::io;
use std::task::{Context, Poll};
#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(feature = "async_io")]
pub use unix::AsyncIoDevice;

#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(feature = "async_tokio")]
pub use unix::TokioDevice;

#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(all(feature = "async_tokio", not(feature = "async_io")))]
pub type AsyncDevice = TokioDevice;

#[cfg(all(unix, not(target_os = "macos")))]
#[cfg(all(feature = "async_io", not(feature = "async_tokio")))]
pub type AsyncDevice = AsyncIoDevice;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::AsyncDevice;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::AsyncDevice;

#[cfg(any(feature = "async_io", feature = "async_tokio"))]
#[cfg(feature = "async_framed")]
pub mod async_framed;

pub trait Pollable {
    fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>>;
    fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>>;
}
