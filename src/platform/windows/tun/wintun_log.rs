#![allow(dead_code)]
use log::*;

use crate::platform::windows::tun::wintun_raw;
use widestring::U16CStr;

/// Sets the logger wintun will use when logging. Maps to the WintunSetLogger C function
pub fn set_logger(win_tun: &wintun_raw::wintun, f: wintun_raw::WINTUN_LOGGER_CALLBACK) {
    unsafe { win_tun.WintunSetLogger(f) };
}

pub fn reset_logger(win_tun: &wintun_raw::wintun) {
    set_logger(win_tun, None);
}

/// The logger that is active by default. Logs messages to the log crate
///
/// # Safety
/// `message` must be a valid pointer that points to an aligned null terminated UTF-16 string
#[cfg(target_pointer_width = "32")]
pub unsafe extern "stdcall" fn default_logger(
    level: wintun_raw::WINTUN_LOGGER_LEVEL,
    _timestamp: wintun_raw::DWORD64,
    message: *const wintun_raw::WCHAR,
) {
    default_logger_(level, message)
}
/// The logger that is active by default. Logs messages to the log crate
///
/// # Safety
/// `message` must be a valid pointer that points to an aligned null terminated UTF-16 string
#[cfg(target_pointer_width = "64")]
pub unsafe extern "C" fn default_logger(
    level: wintun_raw::WINTUN_LOGGER_LEVEL,
    _timestamp: wintun_raw::DWORD64,
    message: *const wintun_raw::WCHAR,
) {
    default_logger_(level, message)
}
fn default_logger_(level: wintun_raw::WINTUN_LOGGER_LEVEL, message: *const wintun_raw::WCHAR) {
    //Cant wait for RFC 2585
    #[allow(unused_unsafe)]
    //Wintun will always give us a valid UTF16 null termineted string
    let msg = unsafe { U16CStr::from_ptr_str(message) };
    let utf8_msg = msg.to_string_lossy();
    match level {
        wintun_raw::WINTUN_LOGGER_LEVEL_WINTUN_LOG_INFO => info!("WinTun: {utf8_msg}"),
        wintun_raw::WINTUN_LOGGER_LEVEL_WINTUN_LOG_WARN => warn!("WinTun: {utf8_msg}"),
        wintun_raw::WINTUN_LOGGER_LEVEL_WINTUN_LOG_ERR => error!("WinTun: {utf8_msg}"),
        _ => error!("WinTun: {utf8_msg} (with invalid log level {level})"),
    }
}

pub(crate) fn set_default_logger_if_unset(win_tun: &wintun_raw::wintun) {
    set_logger(win_tun, Some(default_logger));
}
