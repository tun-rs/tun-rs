#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(any(feature = "async_std", feature = "async_tokio"))]
#[cfg(feature = "async_framed")]
pub mod async_framed;

#[cfg(all(feature = "async_tokio", feature = "async_std", not(doc)))]
compile_error! {"More than one asynchronous runtime is simultaneously specified in features"}
