use crate::platform::windows::device::GUID_NETWORK_ADAPTER;
use crate::platform::windows::ffi;
use crate::platform::windows::ffi::decode_utf16;
use scopeguard::{guard, ScopeGuard};
use std::io;
use std::os::windows::io::{FromRawHandle, OwnedHandle};
use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE, KEY_WRITE,
};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::{
        DICD_GENERATE_ID, DICS_FLAG_GLOBAL, DIF_INSTALLDEVICE, DIF_INSTALLINTERFACES,
        DIF_REGISTERDEVICE, DIF_REGISTER_COINSTALLERS, DIF_REMOVE, DIGCF_PRESENT, DIREG_DRV,
        SPDIT_COMPATDRIVER, SPDRP_HARDWAREID,
    },
    Foundation::{GENERIC_READ, GENERIC_WRITE, TRUE},
    NetworkManagement::Ndis::NET_LUID_LH,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_SYSTEM, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    },
    System::Registry::{KEY_NOTIFY, KEY_QUERY_VALUE, REG_NOTIFY_CHANGE_NAME},
};
use winreg::RegKey;

/// Builds a `NET_LUID_LH` from its `NetLuidIndex` and `IfType` bitfields.
///
/// `Value` layout, matching the C bitfields `Reserved:24; NetLuidIndex:24;
/// IfType:16` (LSB-first on Windows' little-endian targets): `Reserved`
/// occupies bits 0..=23, `NetLuidIndex` bits 24..=47, `IfType` bits 48..=63.
fn net_luid(if_type: u64, net_luid_index: u64) -> NET_LUID_LH {
    NET_LUID_LH {
        Value: ((if_type & 0xFFFF) << 48) | ((net_luid_index & 0xFF_FFFF) << 24),
    }
}

/// Create a new interface and returns its NET_LUID
pub fn create_interface(component_id: &str) -> io::Result<NET_LUID_LH> {
    let devinfo = ffi::create_device_info_list(&GUID_NETWORK_ADAPTER)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let class_name = ffi::class_name_from_guid(&GUID_NETWORK_ADAPTER)?;

    let mut devinfo_data = ffi::create_device_info(
        devinfo,
        &class_name,
        &GUID_NETWORK_ADAPTER,
        "",
        DICD_GENERATE_ID,
    )?;

    ffi::set_selected_device(devinfo, &devinfo_data)?;
    ffi::set_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID, component_id)?;

    ffi::build_driver_info_list(devinfo, &mut devinfo_data, SPDIT_COMPATDRIVER)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_driver_info_list(devinfo, &devinfo_data, SPDIT_COMPATDRIVER);
    });

    let mut driver_version = 0;
    let mut member_index = 0;

    while let Some(drvinfo_data) =
        ffi::enum_driver_info(devinfo, &devinfo_data, SPDIT_COMPATDRIVER, member_index)
    {
        member_index += 1;

        if drvinfo_data.is_err() {
            continue;
        }
        let drvinfo_data = drvinfo_data?;
        if drvinfo_data.DriverVersion <= driver_version {
            continue;
        }

        let drvinfo_detail =
            match ffi::get_driver_info_detail(devinfo, &devinfo_data, &drvinfo_data) {
                Ok(drvinfo_detail) => drvinfo_detail,
                _ => continue,
            };

        let hardware_id = decode_utf16(&drvinfo_detail.HardwareID);
        if !hardware_id.eq_ignore_ascii_case(component_id) {
            continue;
        }

        if ffi::set_selected_driver(devinfo, &devinfo_data, &drvinfo_data).is_err() {
            continue;
        }

        driver_version = drvinfo_data.DriverVersion;
    }

    if driver_version == 0 {
        return Err(io::Error::new(io::ErrorKind::NotFound, "No driver found"));
    }

    let uninstaller = guard((), |_| {
        let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_REMOVE);
    });

    ffi::call_class_installer(devinfo, &devinfo_data, DIF_REGISTERDEVICE)?;

    let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_REGISTER_COINSTALLERS);
    let _ = ffi::call_class_installer(devinfo, &devinfo_data, DIF_INSTALLINTERFACES);

    ffi::call_class_installer(devinfo, &devinfo_data, DIF_INSTALLDEVICE)?;

    let key = ffi::open_dev_reg_key(
        devinfo,
        &devinfo_data,
        DICS_FLAG_GLOBAL,
        0,
        DIREG_DRV,
        KEY_QUERY_VALUE | KEY_NOTIFY,
    )?;

    let key = winreg::RegKey::predef(key as _);

    while key.get_value::<u32, &str>("*IfType").is_err() {
        ffi::notify_change_key_value(key.raw_handle() as _, TRUE, REG_NOTIFY_CHANGE_NAME, 2000)?;
    }

    while key.get_value::<u32, &str>("NetLuidIndex").is_err() {
        ffi::notify_change_key_value(key.raw_handle() as _, TRUE, REG_NOTIFY_CHANGE_NAME, 2000)?;
    }

    let if_type: u32 = key.get_value("*IfType")?;
    let luid_index: u32 = key.get_value("NetLuidIndex")?;

    // Defuse the uninstaller
    ScopeGuard::into_inner(uninstaller);

    Ok(net_luid(if_type as _, luid_index as _))
}

/// Check if the given interface exists and is a valid network device
pub fn check_interface(component_id: &str, luid: &NET_LUID_LH) -> io::Result<()> {
    let devinfo = ffi::get_class_devs(&GUID_NETWORK_ADAPTER, DIGCF_PRESENT)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let mut member_index = 0;

    while let Some(devinfo_data) = ffi::enum_device_info(devinfo, member_index) {
        member_index += 1;

        if devinfo_data.is_err() {
            continue;
        }
        let devinfo_data = devinfo_data?;

        let hardware_id =
            ffi::get_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID);
        if hardware_id.is_err() {
            continue;
        }
        if !hardware_id?.eq_ignore_ascii_case(component_id) {
            continue;
        }

        let key = match ffi::open_dev_reg_key(
            devinfo,
            &devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DRV,
            KEY_QUERY_VALUE | KEY_NOTIFY,
        ) {
            Ok(key) => winreg::RegKey::predef(key as _),
            Err(_) => continue,
        };

        let if_type: u32 = match key.get_value("*IfType") {
            Ok(if_type) => if_type,
            Err(_) => continue,
        };

        let luid_index: u32 = match key.get_value("NetLuidIndex") {
            Ok(luid_index) => luid_index,
            Err(_) => continue,
        };

        let luid2 = net_luid(if_type as _, luid_index as _);

        if unsafe { luid.Value != luid2.Value } {
            continue;
        }

        // Found it!
        return Ok(());
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Device not found"))
}

/// Deletes an existing interface
pub fn delete_interface(component_id: &str, luid: &NET_LUID_LH) -> io::Result<()> {
    let devinfo = ffi::get_class_devs(&GUID_NETWORK_ADAPTER, DIGCF_PRESENT)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let mut member_index = 0;

    while let Some(devinfo_data) = ffi::enum_device_info(devinfo, member_index) {
        member_index += 1;

        if devinfo_data.is_err() {
            continue;
        }
        let devinfo_data = devinfo_data?;

        let hardware_id =
            ffi::get_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID);
        if hardware_id.is_err() {
            continue;
        }
        if !hardware_id?.eq_ignore_ascii_case(component_id) {
            continue;
        }

        let key = ffi::open_dev_reg_key(
            devinfo,
            &devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DRV,
            KEY_QUERY_VALUE | KEY_NOTIFY,
        );
        if key.is_err() {
            continue;
        }
        let key = winreg::RegKey::predef(key? as _);

        let if_type: u32 = match key.get_value("*IfType") {
            Ok(if_type) => if_type,
            Err(_) => continue,
        };

        let luid_index: u32 = match key.get_value("NetLuidIndex") {
            Ok(luid_index) => luid_index,
            Err(_) => continue,
        };

        let luid2 = net_luid(if_type as _, luid_index as _);

        if unsafe { luid.Value != luid2.Value } {
            continue;
        }

        // Found it!
        return ffi::call_class_installer(devinfo, &devinfo_data, DIF_REMOVE);
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Device not found"))
}

/// Open an handle to an interface
pub fn open_interface(luid: &NET_LUID_LH) -> io::Result<OwnedHandle> {
    let guid = ffi::luid_to_guid(luid).and_then(|guid| ffi::string_from_guid(&guid))?;

    let path = format!(r"\\.\Global\{guid}.tap");

    let handle = ffi::create_file(
        &path,
        GENERIC_READ | GENERIC_WRITE,
        FILE_SHARE_READ | FILE_SHARE_WRITE,
        OPEN_EXISTING,
        FILE_ATTRIBUTE_SYSTEM | FILE_FLAG_OVERLAPPED,
    )?;
    unsafe { Ok(OwnedHandle::from_raw_handle(handle)) }
}

pub fn set_adapter_mac_by_guid(adapter_guid: &str, new_mac: &str) -> io::Result<()> {
    let class_path =
        r"SYSTEM\CurrentControlSet\Control\Class\{4D36E972-E325-11CE-BFC1-08002BE10318}";
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let class = hklm.open_subkey_with_flags(class_path, KEY_READ | KEY_WRITE)?;

    let mut found = false;
    for i in 0..256 {
        let subkey_name = format!("{i:04}");
        if let Ok(subkey) = class.open_subkey_with_flags(&subkey_name, KEY_READ | KEY_WRITE) {
            let guid: String = subkey.get_value("NetCfgInstanceId").unwrap_or_default();
            if guid.eq_ignore_ascii_case(adapter_guid) {
                let subkey =
                    class.open_subkey_with_flags(&subkey_name, KEY_SET_VALUE | KEY_WRITE)?;
                subkey.set_value("NetworkAddress", &new_mac)?;
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Registry entry not found for given adapter GUID",
        ));
    }

    Ok(())
}
/// Enables or disables the adapter via SetupAPI (`DIF_PROPERTYCHANGE`).
pub fn enable_adapter(component_id: &str, luid: &NET_LUID_LH, val: bool) -> io::Result<()> {
    let devinfo = ffi::get_class_devs(&GUID_NETWORK_ADAPTER, DIGCF_PRESENT)?;

    let _guard = guard((), |_| {
        let _ = ffi::destroy_device_info_list(devinfo);
    });

    let mut member_index = 0;

    while let Some(devinfo_data) = ffi::enum_device_info(devinfo, member_index) {
        member_index += 1;

        if devinfo_data.is_err() {
            continue;
        }
        let devinfo_data = devinfo_data?;

        let hardware_id =
            ffi::get_device_registry_property(devinfo, &devinfo_data, SPDRP_HARDWAREID);
        if hardware_id.is_err() {
            continue;
        }
        if !hardware_id?.eq_ignore_ascii_case(component_id) {
            continue;
        }

        let key = ffi::open_dev_reg_key(
            devinfo,
            &devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DRV,
            KEY_QUERY_VALUE | KEY_NOTIFY,
        );
        if key.is_err() {
            continue;
        }
        let key = winreg::RegKey::predef(key? as _);

        let if_type: u32 = match key.get_value("*IfType") {
            Ok(if_type) => if_type,
            Err(_) => continue,
        };

        let luid_index: u32 = match key.get_value("NetLuidIndex") {
            Ok(luid_index) => luid_index,
            Err(_) => continue,
        };

        let luid2 = net_luid(if_type as _, luid_index as _);

        if unsafe { luid.Value != luid2.Value } {
            continue;
        }

        // Found it!
        return ffi::set_device_state(devinfo, &devinfo_data, val);
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "Device not found"))
}

#[cfg(test)]
mod tests {
    use super::net_luid;

    #[test]
    fn net_luid_packs_if_type_and_index() {
        // e.g. IF_TYPE_PROP_VIRTUAL (53), NetLuidIndex 5
        let luid = net_luid(53, 5);
        assert_eq!(unsafe { luid.Value }, (53u64 << 48) | (5u64 << 24));
    }

    #[test]
    fn net_luid_truncates_to_field_widths() {
        // Bits above IfType:16 / NetLuidIndex:24 must not leak into
        // neighbouring fields.
        let a = net_luid(0x1_2345, 0x1AB_CDEF);
        let b = net_luid(0x2345, 0xAB_CDEF);
        assert_eq!(unsafe { a.Value }, unsafe { b.Value });
    }

    #[test]
    fn net_luid_zero_keeps_reserved_clear() {
        let luid = net_luid(0, 0);
        assert_eq!(unsafe { luid.Value }, 0);
    }
}
