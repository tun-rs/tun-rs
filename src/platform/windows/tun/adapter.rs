use std::{io, mem, ptr};

use crate::windows::{
    device::GUID_NETWORK_ADAPTER,
    ffi::{decode_utf16, destroy_device_info_list, encode_utf16, enum_device_info},
};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::{Devices::Properties::DEVPROPKEY, Foundation::ERROR_INSUFFICIENT_BUFFER};
use windows_sys::{
    core::GUID,
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                CM_Get_DevNode_Status, SetupDiGetClassDevsExW, SetupDiGetDevicePropertyW,
                CM_DEVNODE_STATUS_FLAGS, CR_SUCCESS, DN_HAS_PROBLEM, HDEVINFO, SP_DEVINFO_DATA,
            },
            Properties::DEVPROPID_FIRST_USABLE,
        },
        System::SystemInformation::{GetVersionExA, OSVERSIONINFOA},
    },
};

#[allow(non_upper_case_globals)]
pub const DEVPKEY_Wintun_Name: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID {
        data1: 0x3361c968,
        data2: 0x2f2e,
        data3: 0x4660,
        data4: [0xb4, 0x7e, 0x69, 0x9c, 0xdc, 0x4c, 0x32, 0xb9],
    },
    pid: DEVPROPID_FIRST_USABLE + 1,
};

#[allow(non_upper_case_globals)]
pub const DEVPKEY_Wintun_OwningProcess: DEVPROPKEY = DEVPROPKEY {
    fmtid: GUID {
        data1: 0x3361c968,
        data2: 0x2f2e,
        data3: 0x4660,
        data4: [0xb4, 0x7e, 0x69, 0x9c, 0xdc, 0x4c, 0x32, 0xb9],
    },
    pid: DEVPROPID_FIRST_USABLE + 3,
};

pub fn check_adapter_if_orphaned_devices(adapter_name: &str) -> bool {
    if is_windows_seven() {
        return super::adapter_win7::check_adapter_if_orphaned_devices_win7(adapter_name);
    }

    let device_name = encode_utf16("SWD\\Wintun");
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
        log::error!("Failed to get adapters");
        return false;
    }

    let mut index = 0;
    let is_orphaned_adapter = loop {
        match enum_device_info(dev_info, index) {
            Some(ret) => {
                let Ok(devinfo_data) = ret else {
                    continue;
                };

                let Ok(status) = dev_node_status(&devinfo_data) else {
                    index += 1;
                    continue;
                };
                if status & DN_HAS_PROBLEM == 0 {
                    index += 1;
                    continue;
                }

                let Ok(name) = get_device_name(dev_info, &devinfo_data) else {
                    index += 1;
                    continue;
                };

                if adapter_name == name {
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

pub fn get_device_name(devinfo: HDEVINFO, devinfo_data: &SP_DEVINFO_DATA) -> io::Result<String> {
    let mut prop_type: u32 = 0;
    let mut required_size: u32 = 0;
    let ok = unsafe {
        SetupDiGetDevicePropertyW(
            devinfo,
            devinfo_data,
            &DEVPKEY_Wintun_Name,
            &mut prop_type,
            ptr::null_mut(),
            0,
            &mut required_size,
            0,
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
            return Err(err);
        }
    }

    let mut buf: Vec<u16> = vec![0; (required_size / 2) as usize];

    let ok = unsafe {
        SetupDiGetDevicePropertyW(
            devinfo,
            devinfo_data,
            &DEVPKEY_Wintun_Name,
            &mut prop_type,
            buf.as_mut_ptr() as *mut u8,
            required_size,
            &mut required_size,
            0,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(decode_utf16(&buf))
}

fn is_windows_seven() -> bool {
    let mut info = OSVERSIONINFOA {
        dwOSVersionInfoSize: mem::size_of::<OSVERSIONINFOA>() as u32,
        dwMajorVersion: 0,
        dwMinorVersion: 0,
        dwBuildNumber: 0,
        dwPlatformId: 0,
        szCSDVersion: [0; 128],
    };

    unsafe {
        if GetVersionExA(&mut info as *mut _) == 0 {
            return false;
        }
    }

    info.dwMajorVersion == 6 && info.dwMinorVersion == 1
}

fn dev_node_status(devinfo_data: &SP_DEVINFO_DATA) -> io::Result<CM_DEVNODE_STATUS_FLAGS> {
    let mut pulstatus = 0;
    let mut pulproblemnumber = 0;

    let cr = unsafe {
        CM_Get_DevNode_Status(
            &mut pulstatus,
            &mut pulproblemnumber,
            devinfo_data.DevInst,
            0,
        )
    };

    if cr != CR_SUCCESS {
        return Err(io::Error::last_os_error());
    }

    Ok(pulstatus)
}
