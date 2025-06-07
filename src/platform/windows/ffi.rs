use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::{io, mem, ptr};

use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetIpInterfaceTable, MIB_IPINTERFACE_ROW, MIB_IPINTERFACE_TABLE,
};
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
use windows_sys::Win32::System::Threading::{
    ResetEvent, SetEvent, WaitForMultipleObjects, INFINITE,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::{
    core::GUID,
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            SetupDiBuildDriverInfoList, SetupDiCallClassInstaller, SetupDiClassNameFromGuidW,
            SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
            SetupDiDestroyDriverInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDriverInfoW,
            SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiGetDriverInfoDetailW,
            SetupDiOpenDevRegKey, SetupDiSetDeviceRegistryPropertyW, SetupDiSetSelectedDevice,
            SetupDiSetSelectedDriverW, HDEVINFO, MAX_CLASS_NAME_LEN, SP_DEVINFO_DATA,
            SP_DRVINFO_DATA_V2_W, SP_DRVINFO_DETAIL_DATA_W,
        },
        Foundation::{GetLastError, BOOL, ERROR_NO_MORE_ITEMS, FALSE, FILETIME, HANDLE, TRUE},
        NetworkManagement::{
            IpHelper::{
                ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToAlias,
                ConvertInterfaceLuidToGuid, ConvertInterfaceLuidToIndex,
            },
            Ndis::NET_LUID_LH,
        },
        Storage::FileSystem::{
            CreateFileW, ReadFile, WriteFile, FILE_CREATION_DISPOSITION, FILE_FLAGS_AND_ATTRIBUTES,
            FILE_SHARE_MODE,
        },
        System::{
            Com::StringFromGUID2,
            Registry::{RegNotifyChangeKeyValue, HKEY},
            Threading::{CreateEventW, WaitForSingleObject},
            IO::DeviceIoControl,
        },
    },
};

#[allow(non_camel_case_types)]
#[allow(non_snake_case)]
#[repr(C)]
#[derive(Clone, Copy)]
/// Custom type to handle variable size SP_DRVINFO_DETAIL_DATA_W
pub struct SP_DRVINFO_DETAIL_DATA_W2 {
    pub cbSize: u32,
    pub InfDate: FILETIME,
    pub CompatIDsOffset: u32,
    pub CompatIDsLength: u32,
    pub Reserved: usize,
    pub SectionName: [u16; 256],
    pub InfFileName: [u16; 260],
    pub DrvDescription: [u16; 256],
    pub HardwareID: [u16; 512],
}

/// Encode a string as a utf16 buffer
pub fn encode_utf16(string: &str) -> Vec<u16> {
    use std::iter::once;
    string.encode_utf16().chain(once(0)).collect()
}

pub fn decode_utf16(string: &[u16]) -> String {
    let end = string.iter().position(|b| *b == 0).unwrap_or(string.len());
    String::from_utf16_lossy(&string[..end])
}

pub fn string_from_guid(guid: &GUID) -> io::Result<String> {
    let mut string = vec![0; 39];

    match unsafe { StringFromGUID2(guid, string.as_mut_ptr(), string.len() as _) } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(decode_utf16(&string)),
    }
}

pub fn alias_to_luid(alias: &str) -> io::Result<NET_LUID_LH> {
    let alias = encode_utf16(alias);
    let mut luid = unsafe { mem::zeroed() };
    match unsafe { ConvertInterfaceAliasToLuid(alias.as_ptr(), &mut luid) } {
        0 => Ok(luid),
        _err => Err(io::Error::last_os_error()),
    }
}

pub fn luid_to_index(luid: &NET_LUID_LH) -> io::Result<u32> {
    let mut index = 0;
    match unsafe { ConvertInterfaceLuidToIndex(luid, &mut index) } {
        0 => Ok(index),
        _err => Err(io::Error::last_os_error()),
    }
}

pub fn luid_to_guid(luid: &NET_LUID_LH) -> io::Result<GUID> {
    let mut guid = unsafe { mem::zeroed() };
    match unsafe { ConvertInterfaceLuidToGuid(luid, &mut guid) } {
        0 => Ok(guid),
        _err => Err(io::Error::last_os_error()),
    }
}

pub fn luid_to_alias(luid: &NET_LUID_LH) -> io::Result<String> {
    // IF_MAX_STRING_SIZE + 1
    let mut alias = vec![0; 257];
    match unsafe { ConvertInterfaceLuidToAlias(luid, alias.as_mut_ptr(), alias.len()) } {
        0 => Ok(decode_utf16(&alias)),
        _err => Err(io::Error::last_os_error()),
    }
}
pub fn reset_event(handle: RawHandle) -> io::Result<()> {
    unsafe {
        if FALSE == ResetEvent(handle) {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn wait_for_single_object(handle: RawHandle, timeout: u32) -> io::Result<()> {
    unsafe {
        if 0 == WaitForSingleObject(handle, timeout) {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
pub fn set_event(handle: RawHandle) -> io::Result<()> {
    unsafe {
        if FALSE == SetEvent(handle) {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}
pub fn create_event() -> io::Result<OwnedHandle> {
    unsafe {
        let read_event_handle = CreateEventW(ptr::null_mut(), 1, 0, ptr::null_mut());
        if read_event_handle.is_null() {
            Err(io::Error::last_os_error())?
        }
        Ok(OwnedHandle::from_raw_handle(read_event_handle))
    }
}

pub fn create_file(
    file_name: &str,
    desired_access: u32,
    share_mode: FILE_SHARE_MODE,
    creation_disposition: FILE_CREATION_DISPOSITION,
    flags_and_attributes: FILE_FLAGS_AND_ATTRIBUTES,
) -> io::Result<HANDLE> {
    let file_name = encode_utf16(file_name);
    let handle = unsafe {
        CreateFileW(
            file_name.as_ptr(),
            desired_access,
            share_mode,
            ptr::null_mut(),
            creation_disposition,
            flags_and_attributes,
            ptr::null_mut(),
        )
    };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

pub fn io_overlapped() -> OVERLAPPED {
    OVERLAPPED {
        Internal: 0,
        InternalHigh: 0,
        Anonymous: windows_sys::Win32::System::IO::OVERLAPPED_0 {
            Anonymous: windows_sys::Win32::System::IO::OVERLAPPED_0_0 {
                Offset: 0,
                OffsetHigh: 0,
            },
        },
        hEvent: ptr::null_mut(),
    }
}

pub fn try_read_file(
    handle: HANDLE,
    io_overlapped: &mut OVERLAPPED,
    buffer: &mut [u8],
) -> io::Result<u32> {
    let mut ret = 0;
    //https://www.cnblogs.com/linyilong3/archive/2012/05/03/2480451.html
    unsafe {
        if 0 == ReadFile(
            handle,
            buffer.as_mut_ptr() as _,
            buffer.len() as _,
            &mut ret,
            io_overlapped,
        ) {
            Err(error_map())
        } else {
            Ok(ret)
        }
    }
}

pub fn try_write_file(
    handle: HANDLE,
    io_overlapped: &mut OVERLAPPED,
    buffer: &[u8],
) -> io::Result<u32> {
    let mut ret = 0;
    unsafe {
        if 0 == WriteFile(
            handle,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut ret,
            io_overlapped,
        ) {
            Err(error_map())
        } else {
            Ok(ret)
        }
    }
}
fn error_map() -> io::Error {
    let e = io::Error::last_os_error();
    if e.raw_os_error().unwrap_or(0) == ERROR_IO_PENDING as i32 {
        io::Error::from(io::ErrorKind::WouldBlock)
    } else {
        e
    }
}

pub fn try_io_overlapped(handle: HANDLE, io_overlapped: &OVERLAPPED) -> io::Result<u32> {
    let mut ret = 0;
    unsafe {
        if 0 == GetOverlappedResult(handle, io_overlapped, &mut ret, 0) {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        } else {
            Ok(ret)
        }
    }
}
#[allow(dead_code)]
pub fn cancel_io_overlapped(handle: HANDLE, io_overlapped: &OVERLAPPED) -> io::Result<u32> {
    unsafe {
        windows_sys::Win32::System::IO::CancelIoEx(handle, io_overlapped);
        wait_io_overlapped(handle, io_overlapped)
    }
}

pub fn read_file(
    handle: HANDLE,
    buffer: &mut [u8],
    interrupt_event: Option<RawHandle>,
) -> io::Result<u32> {
    let mut ret = 0;
    //https://www.cnblogs.com/linyilong3/archive/2012/05/03/2480451.html
    unsafe {
        let mut io_overlapped = io_overlapped();
        let io_event = create_event()?;
        io_overlapped.hEvent = io_event.as_raw_handle();

        if 0 == ReadFile(
            handle,
            buffer.as_mut_ptr() as _,
            buffer.len() as _,
            &mut ret,
            &mut io_overlapped,
        ) {
            let e = io::Error::last_os_error();
            if e.raw_os_error().unwrap_or(0) == ERROR_IO_PENDING as i32 {
                if let Some(interrupt_event) = interrupt_event {
                    wait_io_overlapped_interruptible(handle, &io_overlapped, interrupt_event)
                } else {
                    wait_io_overlapped(handle, &io_overlapped)
                }
            } else {
                Err(e)
            }
        } else {
            Ok(ret)
        }
    }
}

pub fn write_file(
    handle: HANDLE,
    buffer: &[u8],
    interrupt_event: Option<RawHandle>,
) -> io::Result<u32> {
    let mut ret = 0;
    let mut io_overlapped = io_overlapped();
    let io_event = create_event()?;
    io_overlapped.hEvent = io_event.as_raw_handle();
    unsafe {
        if 0 == WriteFile(
            handle,
            buffer.as_ptr() as _,
            buffer.len() as _,
            &mut ret,
            &mut io_overlapped,
        ) {
            let e = io::Error::last_os_error();
            if e.raw_os_error().unwrap_or(0) == ERROR_IO_PENDING as i32 {
                if let Some(interrupt_event) = interrupt_event {
                    wait_io_overlapped_interruptible(handle, &io_overlapped, interrupt_event)
                } else {
                    wait_io_overlapped(handle, &io_overlapped)
                }
            } else {
                Err(e)
            }
        } else {
            Ok(ret)
        }
    }
}

pub fn wait_io_overlapped(handle: HANDLE, io_overlapped: &OVERLAPPED) -> io::Result<u32> {
    let mut ret = 0;
    unsafe {
        if 0 == GetOverlappedResult(handle, io_overlapped, &mut ret, 1) {
            Err(io::Error::last_os_error())
        } else {
            Ok(ret)
        }
    }
}
pub fn wait_io_overlapped_interruptible(
    handle: HANDLE,
    io_overlapped: &OVERLAPPED,
    interrupt_event: RawHandle,
) -> io::Result<u32> {
    let handles = [io_overlapped.hEvent, interrupt_event];
    unsafe {
        let wait_ret = WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
        match wait_ret {
            windows_sys::Win32::Foundation::WAIT_OBJECT_0 => {
                let mut transferred = 0u32;
                let ok = GetOverlappedResult(handle, io_overlapped, &mut transferred, 0);
                if ok != 0 {
                    Ok(transferred)
                } else {
                    Err(io::Error::last_os_error())
                }
            }
            _ => {
                if wait_ret == windows_sys::Win32::Foundation::WAIT_OBJECT_0 + 1 {
                    _ = CancelIoEx(handle, io_overlapped);
                    _ = WaitForSingleObject(io_overlapped.hEvent, INFINITE);
                    Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "trigger interrupt",
                    ))
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }
    }
}

pub fn create_device_info_list(guid: &GUID) -> io::Result<HDEVINFO> {
    match unsafe { SetupDiCreateDeviceInfoList(guid, ptr::null_mut()) } {
        -1 => Err(io::Error::last_os_error()),
        devinfo => Ok(devinfo),
    }
}

pub fn get_class_devs(guid: &GUID, flags: u32) -> io::Result<HDEVINFO> {
    match unsafe { SetupDiGetClassDevsW(guid, ptr::null(), ptr::null_mut(), flags) } {
        -1 => Err(io::Error::last_os_error()),
        devinfo => Ok(devinfo),
    }
}

pub fn destroy_device_info_list(devinfo: HDEVINFO) -> io::Result<()> {
    match unsafe { SetupDiDestroyDeviceInfoList(devinfo) } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn class_name_from_guid(guid: &GUID) -> io::Result<String> {
    let mut class_name = vec![0; MAX_CLASS_NAME_LEN as usize];
    match unsafe {
        SetupDiClassNameFromGuidW(
            guid,
            class_name.as_mut_ptr(),
            class_name.len() as _,
            ptr::null_mut(),
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(decode_utf16(&class_name)),
    }
}

pub fn create_device_info(
    devinfo: HDEVINFO,
    device_name: &str,
    guid: &GUID,
    device_description: &str,
    creation_flags: u32,
) -> io::Result<SP_DEVINFO_DATA> {
    let mut devinfo_data: SP_DEVINFO_DATA = unsafe { mem::zeroed() };
    devinfo_data.cbSize = mem::size_of_val(&devinfo_data) as _;
    let device_name = encode_utf16(device_name);
    let device_description = encode_utf16(device_description);
    match unsafe {
        SetupDiCreateDeviceInfoW(
            devinfo,
            device_name.as_ptr(),
            guid,
            device_description.as_ptr(),
            ptr::null_mut(),
            creation_flags,
            &mut devinfo_data,
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(devinfo_data),
    }
}

pub fn set_selected_device(devinfo: HDEVINFO, devinfo_data: &SP_DEVINFO_DATA) -> io::Result<()> {
    match unsafe { SetupDiSetSelectedDevice(devinfo, devinfo_data as *const _ as _) } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn set_device_registry_property(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    property: u32,
    value: &str,
) -> io::Result<()> {
    let value = encode_utf16(value);
    match unsafe {
        SetupDiSetDeviceRegistryPropertyW(
            devinfo,
            devinfo_data as *const _ as _,
            property,
            value.as_ptr() as _,
            (value.len() * 2) as _,
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn get_device_registry_property(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    property: u32,
) -> io::Result<String> {
    let mut value = vec![0; 32];

    match unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            devinfo,
            devinfo_data as *const _ as _,
            property,
            ptr::null_mut(),
            value.as_mut_ptr() as _,
            (value.len() * 2) as _,
            ptr::null_mut(),
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(decode_utf16(&value)),
    }
}

pub fn build_driver_info_list(
    devinfo: HDEVINFO,
    devinfo_data: &mut SP_DEVINFO_DATA,
    driver_type: u32,
) -> io::Result<()> {
    match unsafe { SetupDiBuildDriverInfoList(devinfo, devinfo_data as *const _ as _, driver_type) }
    {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn destroy_driver_info_list(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    driver_type: u32,
) -> io::Result<()> {
    match unsafe {
        SetupDiDestroyDriverInfoList(devinfo, devinfo_data as *const _ as _, driver_type)
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn get_driver_info_detail(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    drvinfo_data: &SP_DRVINFO_DATA_V2_W,
) -> io::Result<SP_DRVINFO_DETAIL_DATA_W2> {
    let mut drvinfo_detail: SP_DRVINFO_DETAIL_DATA_W2 = unsafe { mem::zeroed() };
    drvinfo_detail.cbSize = mem::size_of::<SP_DRVINFO_DETAIL_DATA_W>() as _;

    match unsafe {
        SetupDiGetDriverInfoDetailW(
            devinfo,
            devinfo_data as *const _ as _,
            drvinfo_data as *const _ as _,
            &mut drvinfo_detail as *mut _ as _,
            mem::size_of_val(&drvinfo_detail) as _,
            ptr::null_mut(),
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(drvinfo_detail),
    }
}

pub fn set_selected_driver(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    drvinfo_data: &SP_DRVINFO_DATA_V2_W,
) -> io::Result<()> {
    match unsafe {
        SetupDiSetSelectedDriverW(
            devinfo,
            devinfo_data as *const _ as _,
            drvinfo_data as *const _ as _,
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn call_class_installer(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    install_function: u32,
) -> io::Result<()> {
    match unsafe {
        SetupDiCallClassInstaller(install_function, devinfo, devinfo_data as *const _ as _)
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn open_dev_reg_key(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    scope: u32,
    hw_profile: u32,
    key_type: u32,
    sam_desired: u32,
) -> io::Result<HKEY> {
    const INVALID_KEY_VALUE: HKEY = windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE as _;

    match unsafe {
        SetupDiOpenDevRegKey(
            devinfo,
            devinfo_data as *const _ as _,
            scope,
            hw_profile,
            key_type,
            sam_desired,
        )
    } {
        INVALID_KEY_VALUE => Err(io::Error::last_os_error()),
        key => Ok(key),
    }
}

pub fn notify_change_key_value(
    key: HKEY,
    watch_subtree: BOOL,
    notify_filter: u32,
    milliseconds: u32,
) -> io::Result<()> {
    const INVALID_HANDLE_VALUE: HKEY = windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE as _;

    let event = match unsafe { CreateEventW(ptr::null_mut(), FALSE, FALSE, ptr::null()) } {
        INVALID_HANDLE_VALUE => Err(io::Error::last_os_error()),
        event => Ok(event),
    }?;

    match unsafe { RegNotifyChangeKeyValue(key, watch_subtree, notify_filter, event, TRUE) } {
        0 => Ok(()),
        _err => Err(io::Error::last_os_error()),
    }?;

    match unsafe { WaitForSingleObject(event, milliseconds) } {
        0 => Ok(()),
        0x102 => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Registry timed out",
        )),
        _ => Err(io::Error::last_os_error()),
    }
}

pub fn enum_driver_info(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    driver_type: u32,
    member_index: u32,
) -> Option<io::Result<SP_DRVINFO_DATA_V2_W>> {
    let mut drvinfo_data: SP_DRVINFO_DATA_V2_W = unsafe { mem::zeroed() };
    drvinfo_data.cbSize = mem::size_of_val(&drvinfo_data) as _;
    match unsafe {
        SetupDiEnumDriverInfoW(
            devinfo,
            devinfo_data as *const _ as _,
            driver_type,
            member_index,
            &mut drvinfo_data,
        )
    } {
        0 if unsafe { GetLastError() == ERROR_NO_MORE_ITEMS } => None,
        0 => Some(Err(io::Error::last_os_error())),
        _ => Some(Ok(drvinfo_data)),
    }
}

pub fn enum_device_info(
    devinfo: HDEVINFO,
    member_index: u32,
) -> Option<io::Result<SP_DEVINFO_DATA>> {
    let mut devinfo_data: SP_DEVINFO_DATA = unsafe { mem::zeroed() };
    devinfo_data.cbSize = mem::size_of_val(&devinfo_data) as _;

    match unsafe { SetupDiEnumDeviceInfo(devinfo, member_index, &mut devinfo_data) } {
        0 if unsafe { GetLastError() == ERROR_NO_MORE_ITEMS } => None,
        0 => Some(Err(io::Error::last_os_error())),
        _ => Some(Ok(devinfo_data)),
    }
}

pub fn device_io_control(
    handle: HANDLE,
    io_control_code: u32,
    in_buffer: &impl Copy,
    out_buffer: &mut impl Copy,
) -> io::Result<()> {
    let mut junk = 0;
    match unsafe {
        DeviceIoControl(
            handle,
            io_control_code,
            in_buffer as *const _ as _,
            mem::size_of_val(in_buffer) as _,
            out_buffer as *mut _ as _,
            mem::size_of_val(out_buffer) as _,
            &mut junk,
            ptr::null_mut(),
        )
    } {
        0 => Err(io::Error::last_os_error()),
        _ => Ok(()),
    }
}

pub fn get_mtu_by_index(index: u32, is_v4: bool) -> io::Result<u32> {
    // https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-getipinterfacetable#examples
    let mut if_table: *mut MIB_IPINTERFACE_TABLE = ptr::null_mut();
    let mut mtu = None;
    unsafe {
        if GetIpInterfaceTable(if is_v4 { AF_INET } else { AF_INET6 }, &mut if_table) != NO_ERROR {
            return Err(io::Error::last_os_error());
        }
        let ifaces = std::slice::from_raw_parts::<MIB_IPINTERFACE_ROW>(
            &(*if_table).Table[0],
            (*if_table).NumEntries as usize,
        );
        for x in ifaces {
            if x.InterfaceIndex == index {
                mtu = Some(x.NlMtu);
                break;
            }
        }
        windows_sys::Win32::NetworkManagement::IpHelper::FreeMibTable(if_table as _);
    }
    if let Some(mtu) = mtu {
        Ok(mtu)
    } else {
        Err(io::Error::from(io::ErrorKind::NotFound))
    }
}
