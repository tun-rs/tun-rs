//! Interface DNS configuration via `SetInterfaceDnsSettings`.
//!
//! [`SetInterfaceDnsSettings`] requires Windows 10, build 19041 (version 2004) or newer, so
//! the function is resolved at run time from `iphlpapi.dll` instead of being linked
//! statically (a static import would prevent the binary from loading on older systems such
//! as Windows 7). When the function is unavailable, we transparently fall back to the legacy
//! [`netsh`](super::netsh) commands.
//!
//! [`SetInterfaceDnsSettings`]: https://learn.microsoft.com/en-us/windows/win32/api/netioapi/nf-netioapi-setinterfacednssettings

use std::io;
use std::net::IpAddr;
use std::sync::OnceLock;

use libloading::os::windows::{Library, Symbol, LOAD_LIBRARY_SEARCH_SYSTEM32};
use windows_sys::core::GUID;
use windows_sys::Win32::NetworkManagement::IpHelper::{
    DNS_INTERFACE_SETTINGS, DNS_INTERFACE_SETTINGS_VERSION1, DNS_SETTING_IPV6,
    DNS_SETTING_NAMESERVER,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;

use super::ffi;
use super::netsh;

/// Signature of `iphlpapi!SetInterfaceDnsSettings`.
type SetInterfaceDnsSettingsFn =
    unsafe extern "system" fn(interface: GUID, settings: *const DNS_INTERFACE_SETTINGS) -> u32;

/// Lazily resolved `SetInterfaceDnsSettings`. It holds only a function pointer (which is
/// `Send + Sync`), so the wrapper is automatically thread-safe.
struct DnsApi {
    set_interface_dns_settings: SetInterfaceDnsSettingsFn,
}

/// Cached result of resolving the API: `Some` if available, `None` if this Windows version
/// does not export `SetInterfaceDnsSettings`. Resolution is attempted only once.
static DNS_API: OnceLock<Option<DnsApi>> = OnceLock::new();

impl DnsApi {
    fn get() -> Option<&'static DnsApi> {
        DNS_API.get_or_init(Self::load).as_ref()
    }

    fn load() -> Option<DnsApi> {
        // Load `iphlpapi.dll` from `System32` only, to avoid DLL search-order hijacking.
        let library =
            unsafe { Library::load_with_flags("iphlpapi.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
                .ok()?;
        let func = unsafe {
            // SAFETY: the signature matches the documented `SetInterfaceDnsSettings`.
            let symbol: Symbol<SetInterfaceDnsSettingsFn> =
                library.get(b"SetInterfaceDnsSettings\0").ok()?;
            *symbol
        };
        // Intentionally leak the handle: `iphlpapi.dll` is a system library that stays mapped
        // for the lifetime of the process, and `func` must remain valid for just as long.
        std::mem::forget(library);
        Some(DnsApi {
            set_interface_dns_settings: func,
        })
    }

    /// Applies `servers` for one address family. An empty slice clears the configured
    /// servers for that family.
    fn apply(&self, guid: &GUID, servers: &[IpAddr], is_ipv4: bool) -> io::Result<()> {
        // The API takes a comma-separated, NUL-terminated wide string of addresses.
        let nameserver = servers
            .iter()
            .map(|addr| addr.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut nameserver = ffi::encode_utf16(&nameserver);

        let flags = if is_ipv4 {
            DNS_SETTING_NAMESERVER
        } else {
            DNS_SETTING_NAMESERVER | DNS_SETTING_IPV6
        };

        let settings = DNS_INTERFACE_SETTINGS {
            Version: DNS_INTERFACE_SETTINGS_VERSION1,
            Flags: flags as u64,
            NameServer: nameserver.as_mut_ptr(),
            ..Default::default()
        };

        // SAFETY: `settings` and the `nameserver` buffer it points at outlive the call, and
        // `guid` identifies the target interface.
        let code = unsafe { (self.set_interface_dns_settings)(*guid, &settings) };
        ffi::win_result(code)
    }
}

/// Sets the interface DNS servers, preferring the Windows API and falling back to `netsh`
/// on systems where `SetInterfaceDnsSettings` is unavailable.
///
/// `dns_servers` must be non-empty and all of the same address family.
pub fn set_dns_servers(index: u32, luid: &NET_LUID_LH, dns_servers: &[IpAddr]) -> io::Result<()> {
    if dns_servers.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "DNS servers list cannot be empty",
        ));
    }
    let is_ipv4 = dns_servers[0].is_ipv4();
    if !dns_servers.iter().all(|addr| addr.is_ipv4() == is_ipv4) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "All DNS servers must be either IPv4 or IPv6",
        ));
    }

    match DnsApi::get() {
        Some(api) => {
            let guid = ffi::luid_to_guid(luid)?;
            api.apply(&guid, dns_servers, is_ipv4)?;
        }
        None => netsh::set_dns_servers(index, dns_servers)?,
    }
    flush_resolver_cache();
    Ok(())
}

/// Clears the interface DNS servers for one address family, restoring automatic resolution.
pub fn clear_dns_servers(index: u32, luid: &NET_LUID_LH, is_ipv4: bool) -> io::Result<()> {
    match DnsApi::get() {
        Some(api) => {
            let guid = ffi::luid_to_guid(luid)?;
            api.apply(&guid, &[], is_ipv4)?;
        }
        None => netsh::clear_dns_servers(index, is_ipv4)?,
    }
    flush_resolver_cache();
    Ok(())
}

// `DnsFlushResolverCache` has been exported by `dnsapi.dll` on every supported Windows
// version, so unlike `SetInterfaceDnsSettings` it is safe to link statically.
#[link(name = "dnsapi")]
extern "system" {
    fn DnsFlushResolverCache() -> i32;
}

/// Best-effort flush of the system DNS resolver cache. A failure here does not invalidate the
/// DNS configuration that was just applied, so it is only logged.
fn flush_resolver_cache() {
    // SAFETY: the function takes no arguments and is always present in `dnsapi.dll`.
    if unsafe { DnsFlushResolverCache() } == 0 {
        log::warn!(
            "DnsFlushResolverCache failed: {}",
            io::Error::last_os_error()
        );
    }
}
