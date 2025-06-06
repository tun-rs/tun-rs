use std::{mem, ptr};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiGetClassDevsExW, SetupDiGetDevicePropertyW,
};
use windows_sys::Win32::Devices::Properties::DEVPROP_TYPE_BINARY;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_INVALID_DATA, FILETIME, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::windows::{
    device::GUID_NETWORK_ADAPTER,
    ffi::{destroy_device_info_list, encode_utf16, enum_device_info},
    tun::adapter::{get_device_name, DEVPKEY_Wintun_OwningProcess},
};

#[repr(C)]
pub struct OwningProcess {
    process_id: u32,
    creation_time: FILETIME,
}

pub fn check_adapter_if_orphaned_devices_win7(adapter_name: &str) -> bool {
    let device_name = encode_utf16("ROOT\\Wintun");
    let dev_info = unsafe {
        SetupDiGetClassDevsExW(
            &GUID_NETWORK_ADAPTER,
            device_name.as_ptr(),
            ptr::null_mut(),
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if dev_info == INVALID_HANDLE_VALUE as isize {
        if unsafe { GetLastError() } != ERROR_INVALID_DATA {
            log::error!("Failed to get adapters");
        }
        return false;
    }

    let mut index = 0;
    let is_orphaned_adapter = loop {
        match enum_device_info(dev_info, index) {
            Some(ret) => {
                let Ok(devinfo_data) = ret else {
                    continue;
                };

                unsafe {
                    let mut ptype = mem::zeroed();
                    let mut buf: [u8; mem::size_of::<OwningProcess>()] = mem::zeroed();

                    let ok = SetupDiGetDevicePropertyW(
                        dev_info,
                        &devinfo_data,
                        &DEVPKEY_Wintun_OwningProcess,
                        &mut ptype,
                        &mut buf as _,
                        buf.len() as _,
                        ptr::null_mut(),
                        0,
                    );

                    if ok != 0 && ptype == DEVPROP_TYPE_BINARY && {
                        let owning_process = buf.as_ptr() as *const OwningProcess;
                        !process_is_stale(&*owning_process)
                    } {
                        continue;
                    }
                }

                let Ok(name) = get_device_name(dev_info, &devinfo_data) else {
                    index += 1;
                    continue;
                };
                if adapter_name == &name {
                    break true;
                }
            }
            None => break false,
        }

        index += 1;
    };
    _ = destroy_device_info_list(dev_info);
    is_orphaned_adapter
}

fn process_is_stale(owning_process: &OwningProcess) -> bool {
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            owning_process.process_id,
        )
    };
    if process.is_null() {
        return true;
    }
    let mut creation_time: FILETIME = unsafe { std::mem::zeroed() };
    let mut unused: FILETIME = unsafe { std::mem::zeroed() };
    let ret = unsafe {
        GetProcessTimes(
            process,
            &mut creation_time,
            &mut unused,
            &mut unused,
            &mut unused,
        )
    };
    _ = unsafe { CloseHandle(process) };
    if ret == 0 {
        return false;
    }
    return creation_time.dwHighDateTime == owning_process.creation_time.dwHighDateTime
        && creation_time.dwLowDateTime == owning_process.creation_time.dwLowDateTime;
}
