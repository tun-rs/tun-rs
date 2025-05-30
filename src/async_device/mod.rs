#[cfg(unix)]
pub(crate) mod unix;
#[cfg(all(unix, not(target_os = "macos")))]
pub use unix::AsyncDevice;
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

#[cfg(all(feature = "async_tokio", feature = "async_io", not(doc)))]
compile_error! {"More than one asynchronous runtime is simultaneously specified in features"}
