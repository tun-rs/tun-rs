use crate::platform::unix::Tun;

/// A TUN device for iOS.
pub struct DeviceImpl {
    pub(crate) tun: Tun,
}
impl DeviceImpl {
    pub(crate) fn from_tun(tun: Tun) -> Self {
        Self { tun }
    }
}
