mod sockaddr;
#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "macos"
))]
pub(crate) use sockaddr::sockaddr_union;

#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
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
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
    ))
))]
/// A TUN device for Android/iOS/...
pub struct DeviceImpl {
    pub(crate) tun: Tun,
    pub(crate) op_lock: std::sync::Mutex<()>,
}
#[cfg(all(
    unix,
    not(any(
        target_os = "windows",
        target_os = "macos",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
    ))
))]
impl DeviceImpl {
    pub(crate) fn from_tun(tun: Tun) -> std::io::Result<Self> {
        Ok(Self { tun })
    }
}
