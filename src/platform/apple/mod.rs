#[cfg(feature = "utun_fd")]
#[allow(dead_code)]
/// Finds and returns the utun file descriptor for the current process.
///
/// This function searches through file descriptors 0-1024 to locate the utun
/// socket associated with the current process. It is specifically designed for
/// use with Apple's NEPacketTunnelProvider on macOS, iOS, tvOS, and related platforms.
///
/// # Platform-specific Behavior
///
/// This function is only available on Apple platforms (macOS, iOS, tvOS, etc.)
/// and should only be called within an NEPacketTunnelProvider context where
/// a utun interface has been established by the system.
///
/// # Returns
///
/// - `Some(fd)` - The file descriptor of the utun socket if found
/// - `None` - If no utun file descriptor is found in the searched range
///
/// # Feature
///
/// This function is only available when the `utun_fd` feature is enabled.
pub fn utun_fd() -> Option<i32> {
    unsafe {
        let mut ctl_info: libc::ctl_info = std::mem::zeroed();

        let name = b"com.apple.net.utun_control\0";
        std::ptr::copy_nonoverlapping(
            name.as_ptr() as *const libc::c_char,
            ctl_info.ctl_name.as_mut_ptr(),
            name.len(),
        );

        for fd in 0..1024i32 {
            let mut addr: libc::sockaddr_ctl = std::mem::zeroed();
            let mut len: libc::socklen_t =
                std::mem::size_of::<libc::sockaddr_ctl>() as libc::socklen_t;

            let mut ret: libc::c_int;

            ret = libc::getpeername(fd, &mut addr as *mut _ as *mut libc::sockaddr, &mut len);

            if ret != 0 || addr.sc_family != libc::AF_SYSTEM as u8 {
                continue;
            }

            if ctl_info.ctl_id == 0 {
                ret = libc::ioctl(fd, libc::CTLIOCGINFO, &mut ctl_info);
                if ret != 0 {
                    continue;
                }
            }

            if addr.sc_id == ctl_info.ctl_id {
                return Some(fd);
            }
        }

        None
    }
}
