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

#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
))]
pub(crate) fn read_operation_lock(
    lock: &std::sync::RwLock<()>,
) -> std::sync::RwLockReadGuard<'_, ()> {
    lock.read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(any(
    all(target_os = "linux", not(target_env = "ohos")),
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "illumos",
))]
pub(crate) fn write_operation_lock(
    lock: &std::sync::RwLock<()>,
) -> std::sync::RwLockWriteGuard<'_, ()> {
    lock.write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(all(
    test,
    any(
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "macos",
        target_os = "ios",
        target_os = "tvos",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
    )
))]
mod operation_lock_tests {
    use super::{read_operation_lock, write_operation_lock};
    use std::sync::{Arc, RwLock};

    #[test]
    fn poisoned_operation_lock_remains_usable() {
        let lock = Arc::new(RwLock::new(()));
        let panic_lock = Arc::clone(&lock);
        assert!(std::thread::spawn(move || {
            let _guard = panic_lock
                .write()
                .expect("fresh operation lock must be available");
            panic!("poison operation lock");
        })
        .join()
        .is_err());
        assert!(lock.is_poisoned());

        drop(read_operation_lock(&lock));
        drop(write_operation_lock(&lock));
    }
}

pub(crate) mod device;

#[cfg(target_os = "illumos")]
mod illumos;

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
/// A TUN device for Android/iOS/illumos/...
pub struct DeviceImpl {
    #[cfg(target_os = "illumos")]
    pub(crate) illumos: std::sync::OnceLock<illumos::State>,
    pub(crate) tun: Tun,
    #[allow(dead_code)]
    pub(crate) op_lock: std::sync::RwLock<()>,
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
        Ok(Self {
            tun,
            op_lock: std::sync::RwLock::new(()),
            #[cfg(target_os = "illumos")]
            illumos: std::sync::OnceLock::new(),
        })
    }
}
