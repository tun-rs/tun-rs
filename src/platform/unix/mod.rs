mod sockaddr;
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "macos"
))]
pub(crate) use sockaddr::sockaddr_union;

#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd"
))]
#[allow(unused_imports)]
pub(crate) use sockaddr::ipaddr_to_sockaddr;

mod fd;
pub(crate) use self::fd::Fd;
#[cfg(feature = "interruptible")]
mod interrupt;
#[cfg(feature = "interruptible")]
pub use interrupt::InterruptEvent;
mod tun;
pub(crate) use self::tun::Tun;

pub(crate) mod device;

#[cfg(all(
    unix,
    not(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd"
    ))
))]
/// A TUN device for Android/iOS/...
pub struct DeviceImpl {
    pub(crate) tun: Tun,
}
#[cfg(all(
    unix,
    not(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd"
    ))
))]
impl DeviceImpl {
    pub(crate) fn from_tun(tun: Tun) -> Self {
        Self { tun }
    }
}
