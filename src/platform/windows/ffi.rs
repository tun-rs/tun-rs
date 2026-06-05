use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
use std::{io, mem, ptr};

use windows_sys::Win32::Foundation::{ERROR_IO_PENDING, ERROR_OBJECT_ALREADY_EXISTS, NO_ERROR};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    CreateIpForwardEntry2, CreateUnicastIpAddressEntry, DeleteIpForwardEntry2,
    DeleteUnicastIpAddressEntry, FreeMibTable, GetIpForwardTable2, GetIpInterfaceEntry,
    GetIpInterfaceTable, GetUnicastIpAddressTable, InitializeIpForwardEntry,
    InitializeUnicastIpAddressEntry, SetIpInterfaceEntry, MIB_IPFORWARD_ROW2, MIB_IPFORWARD_TABLE2,
    MIB_IPINTERFACE_ROW, MIB_IPINTERFACE_TABLE, MIB_UNICASTIPADDRESS_ROW,
    MIB_UNICASTIPADDRESS_TABLE,
};
use windows_sys::Win32::Networking::WinSock::{
    NlroManual, AF_INET, AF_INET6, MIB_IPPROTO_NETMGMT, SOCKADDR_INET,
};
use windows_sys::Win32::System::Threading::{ResetEvent, SetEvent};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::{
    core::{BOOL, GUID},
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            SetupDiBuildDriverInfoList, SetupDiCallClassInstaller, SetupDiClassNameFromGuidW,
            SetupDiCreateDeviceInfoList, SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList,
            SetupDiDestroyDriverInfoList, SetupDiEnumDeviceInfo, SetupDiEnumDriverInfoW,
            SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiGetDriverInfoDetailW,
            SetupDiOpenDevRegKey, SetupDiSetClassInstallParamsW, SetupDiSetDeviceRegistryPropertyW,
            SetupDiSetSelectedDevice, SetupDiSetSelectedDriverW, DICS_DISABLE, DICS_ENABLE,
            DICS_FLAG_GLOBAL, DIF_PROPERTYCHANGE, HDEVINFO, MAX_CLASS_NAME_LEN,
            SP_CLASSINSTALL_HEADER, SP_DEVINFO_DATA, SP_DRVINFO_DATA_V2_W,
            SP_DRVINFO_DETAIL_DATA_W, SP_PROPCHANGE_PARAMS,
        },
        Foundation::{GetLastError, ERROR_NO_MORE_ITEMS, FALSE, FILETIME, HANDLE, TRUE},
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
        CancelIoEx(handle, io_overlapped);
        wait_io_overlapped(handle, io_overlapped)
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

/// Converts a Rust `IpAddr` into a Windows `SOCKADDR_INET` (port/scope left zero).
fn sockaddr_inet_from_ip(ip: IpAddr) -> SOCKADDR_INET {
    let mut sa = SOCKADDR_INET::default();
    match ip {
        IpAddr::V4(v4) => {
            sa.Ipv4.sin_family = AF_INET;
            sa.Ipv4.sin_addr.S_un.S_addr = u32::from_ne_bytes(v4.octets());
        }
        IpAddr::V6(v6) => {
            sa.Ipv6.sin6_family = AF_INET6;
            sa.Ipv6.sin6_addr.u.Byte = v6.octets();
        }
    }
    sa
}

/// Maps a Win32 status code (`NETIOAPI_API` / `WIN32_ERROR`) to an `io::Result`.
pub(crate) fn win_result(code: u32) -> io::Result<()> {
    if code == NO_ERROR {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

/// Sets the interface metric (routing cost) for both IPv4 and IPv6 by interface index.
pub fn set_interface_metric(index: u32, metric: u32) -> io::Result<()> {
    for family in [AF_INET, AF_INET6] {
        let mut row = MIB_IPINTERFACE_ROW {
            Family: family,
            InterfaceIndex: index,
            ..Default::default()
        };
        win_result(unsafe { GetIpInterfaceEntry(&mut row) })?;

        row.Metric = metric;
        row.UseAutomaticMetric = false;
        win_result(unsafe { SetIpInterfaceEntry(&mut row) })?;
    }
    Ok(())
}

/// Sets the MTU (`NlMtu`) of the interface for the given family by interface index.
pub fn set_interface_mtu(index: u32, mtu: u32, is_v4: bool) -> io::Result<()> {
    let mut row = MIB_IPINTERFACE_ROW {
        Family: if is_v4 { AF_INET } else { AF_INET6 },
        InterfaceIndex: index,
        ..Default::default()
    };
    win_result(unsafe { GetIpInterfaceEntry(&mut row) })?;

    row.NlMtu = mtu;
    // `GetIpInterfaceEntry` returns a `SitePrefixLength` that `SetIpInterfaceEntry`
    // rejects (notably for IPv4); reset it to 0 before writing back. This is the
    // conventional workaround and is harmless for IPv6, where site prefixes are unused.
    row.SitePrefixLength = 0;
    win_result(unsafe { SetIpInterfaceEntry(&mut row) })
}

/// Adds a single unicast address to the interface, optionally installing a
/// default route via `gateway`. An already-existing identical entry is ignored.
pub fn add_address(
    index: u32,
    address: IpAddr,
    prefix: u8,
    gateway: Option<IpAddr>,
) -> io::Result<()> {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.InterfaceIndex = index;
    row.Address = sockaddr_inet_from_ip(address);
    row.OnLinkPrefixLength = prefix;

    let code = unsafe { CreateUnicastIpAddressEntry(&row) };
    if code != ERROR_OBJECT_ALREADY_EXISTS {
        win_result(code)?;
    }

    if let Some(gateway) = gateway {
        let mut route = MIB_IPFORWARD_ROW2::default();
        unsafe { InitializeIpForwardEntry(&mut route) };
        route.InterfaceIndex = index;
        // Install a default route (0.0.0.0/0 or ::/0) via `gateway`. `DestinationPrefix`
        // must carry a valid address family matching `NextHop`; `InitializeIpForwardEntry`
        // leaves it zeroed (AF_UNSPEC), which `CreateIpForwardEntry2` rejects as "not
        // specified". Use the unspecified address of the gateway's family.
        let unspecified = if gateway.is_ipv4() {
            IpAddr::V4(Ipv4Addr::UNSPECIFIED)
        } else {
            IpAddr::V6(Ipv6Addr::UNSPECIFIED)
        };
        route.DestinationPrefix.Prefix = sockaddr_inet_from_ip(unspecified);
        route.DestinationPrefix.PrefixLength = 0;
        // `InitializeIpForwardEntry` sets SitePrefixLength to an illegal value (255); for a
        // default route it must not exceed the destination prefix length (0), or
        // `CreateIpForwardEntry2` also fails with ERROR_INVALID_PARAMETER.
        route.SitePrefixLength = 0;
        route.NextHop = sockaddr_inet_from_ip(gateway);
        route.Metric = 0;
        route.Protocol = MIB_IPPROTO_NETMGMT;
        route.Origin = NlroManual;

        let code = unsafe { CreateIpForwardEntry2(&route) };
        if code != ERROR_OBJECT_ALREADY_EXISTS {
            win_result(code)?;
        }
    }
    Ok(())
}

/// Removes a single unicast address from the interface.
pub fn remove_address(index: u32, address: IpAddr) -> io::Result<()> {
    let mut row = MIB_UNICASTIPADDRESS_ROW::default();
    unsafe { InitializeUnicastIpAddressEntry(&mut row) };
    row.InterfaceIndex = index;
    row.Address = sockaddr_inet_from_ip(address);
    win_result(unsafe { DeleteUnicastIpAddressEntry(&row) })
}

/// Removes every unicast address of the given family from the interface.
fn clear_addresses(index: u32, is_v4: bool) -> io::Result<()> {
    let family = if is_v4 { AF_INET } else { AF_INET6 };
    let mut table: *mut MIB_UNICASTIPADDRESS_TABLE = ptr::null_mut();
    win_result(unsafe { GetUnicastIpAddressTable(family, &mut table) })?;

    // Copy out the rows we want to delete before freeing the table.
    let rows: Vec<MIB_UNICASTIPADDRESS_ROW> = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    }
    .iter()
    .filter(|row| row.InterfaceIndex == index)
    .copied()
    .collect();
    unsafe { FreeMibTable(table as _) };

    for row in &rows {
        win_result(unsafe { DeleteUnicastIpAddressEntry(row) })?;
    }
    // Also drop the gateway/default route(s) installed by `add_address` for this family,
    // keeping the route lifecycle symmetric with `set_address` (replace).
    clear_default_routes(index, is_v4)
}

/// Removes the interface's default routes (`0.0.0.0/0` / `::/0`) for the given family.
///
/// These are the gateway routes installed by [`add_address`]. On-link/subnet routes are
/// removed automatically by Windows when the owning address is deleted, so only the
/// explicit default route needs to be cleaned up here. This runs from [`clear_addresses`]
/// so that [`set_address`] replaces the old gateway route instead of leaking it; routes are
/// only ever created through `add_address` (i.e. via `set_address`), so this is the matching
/// teardown.
fn clear_default_routes(index: u32, is_v4: bool) -> io::Result<()> {
    let family = if is_v4 { AF_INET } else { AF_INET6 };
    let mut table: *mut MIB_IPFORWARD_TABLE2 = ptr::null_mut();
    win_result(unsafe { GetIpForwardTable2(family, &mut table) })?;

    // Copy out this interface's default routes before freeing the table.
    let rows: Vec<MIB_IPFORWARD_ROW2> = unsafe {
        std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize)
    }
    .iter()
    .filter(|row| row.InterfaceIndex == index && row.DestinationPrefix.PrefixLength == 0)
    .copied()
    .collect();
    unsafe { FreeMibTable(table as _) };

    for row in &rows {
        win_result(unsafe { DeleteIpForwardEntry2(row) })?;
    }
    Ok(())
}

/// Replaces all addresses of the same family on the interface with `address`,
/// optionally installing a default route via `gateway`.
pub fn set_address(
    index: u32,
    address: IpAddr,
    prefix: u8,
    gateway: Option<IpAddr>,
) -> io::Result<()> {
    clear_addresses(index, address.is_ipv4())?;
    add_address(index, address, prefix, gateway)
}

/// Enables or disables a device via SetupAPI (`DIF_PROPERTYCHANGE`), equivalent
/// to enabling/disabling it in Device Manager.
pub fn set_device_state(
    devinfo: HDEVINFO,
    devinfo_data: &SP_DEVINFO_DATA,
    enable: bool,
) -> io::Result<()> {
    let params = SP_PROPCHANGE_PARAMS {
        ClassInstallHeader: SP_CLASSINSTALL_HEADER {
            cbSize: mem::size_of::<SP_CLASSINSTALL_HEADER>() as u32,
            InstallFunction: DIF_PROPERTYCHANGE,
        },
        StateChange: if enable { DICS_ENABLE } else { DICS_DISABLE },
        Scope: DICS_FLAG_GLOBAL,
        HwProfile: 0,
    };

    // `ClassInstallHeader` is the first field, so the struct pointer doubles as
    // the header pointer. Cast from the whole struct to avoid taking a reference
    // to a field of a `packed` struct (illegal on x86).
    let ok = unsafe {
        SetupDiSetClassInstallParamsW(
            devinfo,
            devinfo_data as *const _,
            &params as *const SP_PROPCHANGE_PARAMS as *const SP_CLASSINSTALL_HEADER,
            mem::size_of::<SP_PROPCHANGE_PARAMS>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }

    call_class_installer(devinfo, devinfo_data, DIF_PROPERTYCHANGE)
}
