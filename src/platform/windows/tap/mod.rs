use crate::platform::windows::tap::overlapped::{ReadOverlapped, WriteOverlapped};
use crate::platform::windows::{ffi, netsh};
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::{Arc, Mutex};
use std::{io, time};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::Ioctl::{FILE_ANY_ACCESS, FILE_DEVICE_UNKNOWN, METHOD_BUFFERED};

mod iface;
mod overlapped;

pub struct TapDevice {
    tap_interface: TapInterface,
    handle: Arc<OwnedHandle>,
    index: u32,
    read_io_overlapped: Mutex<ReadOverlapped>,
    write_io_overlapped: Mutex<WriteOverlapped>,
}
pub(crate) const READ_BUFFER_SIZE: usize = 14 + 65536;
unsafe impl Send for TapDevice {}

unsafe impl Sync for TapDevice {}

impl Drop for TapInterface {
    fn drop(&mut self) {
        if self.need_delete {
            let _ = iface::delete_interface(&self.component_id, &self.luid);
        }
    }
}
struct TapInterface {
    luid: NET_LUID_LH,
    component_id: String,
    need_delete: bool,
}

fn get_version(handle: HANDLE) -> io::Result<[u64; 3]> {
    let in_version: [u64; 3] = [0; 3];
    let mut out_version: [u64; 3] = [0; 3];
    ffi::device_io_control(handle, TAP_IOCTL_GET_VERSION, &in_version, &mut out_version)
        .map(|_| out_version)
}

impl TapDevice {
    pub fn index(&self) -> u32 {
        self.index
    }
    /// Creates a new tap-windows device
    pub fn create(component_id: &str, persist: bool, mut mac: Option<&String>) -> io::Result<Self> {
        let luid = iface::create_interface(component_id)?;
        let mut tap_interface = TapInterface {
            luid,
            component_id: component_id.to_string(),
            need_delete: true, // Initialization must be true
        };
        std::thread::sleep(time::Duration::from_millis(20));
        // Even after retrieving the luid, we might need to wait
        let start = time::Instant::now();
        let handle = loop {
            // If we surpassed 2 seconds just return
            let now = time::Instant::now();
            if now - start > time::Duration::from_secs(3) {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Interface timed out",
                ));
            }

            match ffi::luid_to_guid(&luid) {
                Err(_) => {
                    std::thread::sleep(time::Duration::from_millis(20));
                    continue;
                }
                Ok(guid) => {
                    if let Some(mac) = mac.take() {
                        let guid = ffi::string_from_guid(&guid)?;
                        let name = ffi::luid_to_alias(&luid)?;
                        iface::set_adapter_mac_by_guid(&guid, mac)?;
                        std::thread::sleep(time::Duration::from_millis(20));
                        iface::enable_adapter(&name, false)?;
                        std::thread::sleep(time::Duration::from_millis(20));
                        iface::enable_adapter(&name, true)?;
                    }
                    let handle = iface::open_interface(&luid)?;
                    if get_version(handle.as_raw_handle()).is_err() {
                        std::thread::sleep(time::Duration::from_millis(200));
                        continue;
                    }
                    break handle;
                }
            };
        };

        let index = match ffi::luid_to_index(&luid) {
            Ok(index) => index,
            Err(e) => Err(e)?,
        };
        let handle = Arc::new(handle);
        let read_io_overlapped = ReadOverlapped::new(handle.clone())?;
        let write_io_overlapped = WriteOverlapped::new(handle.clone())?;
        // Set to desired value after successful creation
        tap_interface.need_delete = !persist;
        Ok(Self {
            tap_interface,
            handle,
            index,
            read_io_overlapped: Mutex::new(read_io_overlapped),
            write_io_overlapped: Mutex::new(write_io_overlapped),
        })
    }

    /// Opens an existing tap-windows device by name
    pub fn open(
        component_id: &str,
        name: &str,
        persist: bool,
        mac: Option<&String>,
    ) -> io::Result<Self> {
        let luid = ffi::alias_to_luid(name)?;
        iface::check_interface(component_id, &luid)?;
        if let Some(mac) = mac {
            let guid = ffi::luid_to_guid(&luid)?;
            let guid = ffi::string_from_guid(&guid)?;
            iface::set_adapter_mac_by_guid(&guid, mac)?;
            std::thread::sleep(time::Duration::from_millis(1));
            iface::enable_adapter(name, false)?;
            std::thread::sleep(time::Duration::from_millis(1));
            iface::enable_adapter(name, true)?;
        }
        let handle = iface::open_interface(&luid)?;
        let index = ffi::luid_to_index(&luid)?;
        let tap_interface = TapInterface {
            luid,
            component_id: component_id.to_string(),
            need_delete: !persist,
        };
        let handle = Arc::new(handle);
        let read_io_overlapped = ReadOverlapped::new(handle.clone())?;
        let write_io_overlapped = WriteOverlapped::new(handle.clone())?;

        Ok(Self {
            index,
            tap_interface,
            handle,
            read_io_overlapped: Mutex::new(read_io_overlapped),
            write_io_overlapped: Mutex::new(write_io_overlapped),
        })
    }

    /// Sets the status of the interface to disconnected.
    /// Equivalent to `.set_status(false)`
    pub fn down(&self) -> io::Result<()> {
        self.set_status(false)
    }

    /// Retrieve the MAC address of the interface
    pub fn get_mac(&self) -> io::Result<[u8; 6]> {
        let mut mac = [0; 6];
        ffi::device_io_control(
            self.handle.as_raw_handle(),
            TAP_IOCTL_GET_MAC,
            &(),
            &mut mac,
        )
        .map(|_| mac)
    }
    pub fn set_mac(&self, _mac: &[u8; 6]) -> io::Result<()> {
        Err(io::Error::from(io::ErrorKind::Unsupported))?
    }

    /// Retrieve the version of the driver
    pub fn get_version(&self) -> io::Result<[u64; 3]> {
        get_version(self.handle.as_raw_handle())
    }

    // ///Retrieve the MTU of the interface
    // pub fn get_mtu(&self) -> io::Result<u32> {
    //     let in_mtu: u32 = 0;
    //     let mut out_mtu = 0;
    //     ffi::device_io_control(self.handle, TAP_IOCTL_GET_MTU, &in_mtu, &mut out_mtu)
    //         .map(|_| out_mtu)
    // }

    /// Retrieve the name of the interface
    pub fn get_name(&self) -> io::Result<String> {
        ffi::luid_to_alias(&self.tap_interface.luid)
    }

    /// Set the name of the interface
    pub fn set_name(&self, newname: &str) -> io::Result<()> {
        let name = self.get_name()?;
        netsh::set_interface_name(&name, newname)
    }

    // /// Set the ip of the interface
    // pub fn set_ip<A, B>(&self, address: A, mask: B) -> io::Result<()>
    // where
    //     A: Into<net::Ipv4Addr>,
    //     B: Into<net::Ipv4Addr>,
    // {
    //     let address = address.into().to_string();
    //     let mask = mask.into().to_string();
    //
    //     netsh::set_interface_ip(self.index, address.into(), mask.into(), None)
    // }

    /// Set the status of the interface, true for connected,
    /// false for disconnected.
    pub fn set_status(&self, status: bool) -> io::Result<()> {
        let status: u32 = if status { 1 } else { 0 };
        let mut out_status: u32 = 0;
        ffi::device_io_control(
            self.handle.as_raw_handle(),
            TAP_IOCTL_SET_MEDIA_STATUS,
            &status,
            &mut out_status,
        )
    }
    #[cfg(any(
        feature = "interruptible",
        feature = "async_tokio",
        feature = "async_io"
    ))]
    pub fn wait_readable_interruptible(&self, interrupt_event: &OwnedHandle) -> io::Result<()> {
        let guard = self.read_io_overlapped.lock().unwrap();
        let event = guard.overlapped_event();
        drop(guard);
        event.wait_interruptible(interrupt_event)
    }
    pub fn wait_readable(&self) -> io::Result<()> {
        let guard = self.read_io_overlapped.lock().unwrap();
        let event_handle = guard.overlapped_event();
        drop(guard);
        event_handle.wait()
    }

    pub fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.read_io_overlapped.try_lock() else {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        guard.try_read(buf)
    }
    pub fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.write_io_overlapped.try_lock() else {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        guard.try_write(buf)
    }
    pub fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            match self.try_read(buf) {
                Ok(len) => return Ok(len),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            }
            self.wait_readable()?
        }
    }
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        loop {
            match self.try_write(buf) {
                Ok(len) => return Ok(len),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let guard = self.write_io_overlapped.lock().unwrap();
                    let event = guard.overlapped_event();
                    drop(guard);
                    event.wait()?
                }
                Err(e) => return Err(e),
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn write_interruptible(
        &self,
        buf: &[u8],
        interrupt_event: &OwnedHandle,
    ) -> io::Result<usize> {
        loop {
            match self.try_write(buf) {
                Ok(len) => return Ok(len),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let guard = self.write_io_overlapped.lock().unwrap();
                    let event = guard.overlapped_event();
                    drop(guard);
                    event.wait_interruptible(interrupt_event)?
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[allow(non_snake_case)]
#[inline]
const fn CTL_CODE(DeviceType: u32, Function: u32, Method: u32, Access: u32) -> u32 {
    (DeviceType << 16) | (Access << 14) | (Function << 2) | Method
}

const TAP_IOCTL_GET_MAC: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 1, METHOD_BUFFERED, FILE_ANY_ACCESS);
const TAP_IOCTL_GET_VERSION: u32 =
    CTL_CODE(FILE_DEVICE_UNKNOWN, 2, METHOD_BUFFERED, FILE_ANY_ACCESS);
// const TAP_IOCTL_GET_MTU: u32 = CTL_CODE(FILE_DEVICE_UNKNOWN, 3, METHOD_BUFFERED, FILE_ANY_ACCESS);
const TAP_IOCTL_SET_MEDIA_STATUS: u32 =
    CTL_CODE(FILE_DEVICE_UNKNOWN, 6, METHOD_BUFFERED, FILE_ANY_ACCESS);
