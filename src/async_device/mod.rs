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

#[cfg(all(
    any(feature = "async_io", feature = "async_tokio"),
    feature = "async_framed"
))]
#[cfg_attr(
    docsrs,
    doc(cfg(all(
        any(feature = "async_io", feature = "async_tokio"),
        feature = "async_framed"
    )))
)]
pub mod async_framed;

#[cfg(all(feature = "async_tokio", feature = "async_io", not(doc)))]
compile_error! {"More than one asynchronous runtime is simultaneously specified in features"}

#[cfg(unix)]
pub struct BorrowedAsyncDevice<'dev> {
    dev: AsyncDevice,
    _phantom: std::marker::PhantomData<&'dev AsyncDevice>,
}
#[cfg(unix)]
impl std::ops::Deref for BorrowedAsyncDevice<'_> {
    type Target = AsyncDevice;
    fn deref(&self) -> &Self::Target {
        &self.dev
    }
}
#[cfg(unix)]
impl BorrowedAsyncDevice<'_> {
    /// # Safety
    /// The fd passed in must be a valid, open file descriptor.
    /// Unlike [`from_fd`], this function does **not** take ownership of `fd`,
    /// and therefore will not close it when dropped.  
    /// The caller is responsible for ensuring the lifetime and eventual closure of `fd`.
    pub unsafe fn borrow_raw(fd: std::os::fd::RawFd) -> std::io::Result<Self> {
        #[allow(unused_unsafe)]
        unsafe {
            Ok(Self {
                dev: AsyncDevice::borrow_raw(fd)?,
                _phantom: std::marker::PhantomData,
            })
        }
    }
}
