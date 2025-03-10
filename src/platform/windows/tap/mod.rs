use crate::platform::windows::{ffi, netsh};
use std::ops::DerefMut;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::Mutex;
use std::{io, time};
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::Ioctl::{FILE_ANY_ACCESS, FILE_DEVICE_UNKNOWN, METHOD_BUFFERED};
use windows_sys::Win32::System::IO::OVERLAPPED;

mod iface;

pub struct TapDevice {
    luid: NET_LUID_LH,
    handle: OwnedHandle,
    component_id: String,
    index: u32,
    need_delete: bool,
    read_io_overlapped: Mutex<(Option<Box<OVERLAPPED>>, Vec<u8>)>,
    write_io_overlapped: Mutex<Option<(Box<OVERLAPPED>, Vec<u8>)>>,
}
const READ_BUFFER_SIZE: usize = 14 + 65536;
unsafe impl Send for TapDevice {}

unsafe impl Sync for TapDevice {}

impl Drop for TapDevice {
    fn drop(&mut self) {
        if self.need_delete {
            let _ = iface::delete_interface(&self.component_id, &self.luid);
        }
    }
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
    pub fn create(component_id: &str) -> io::Result<Self> {
        let luid = iface::create_interface(component_id)?;
        // Even after retrieving the luid, we might need to wait
        let start = time::Instant::now();
        let handle = loop {
            // If we surpassed 2 seconds just return
            let now = time::Instant::now();
            if now - start > time::Duration::from_secs(3) {
                let _ = iface::delete_interface(component_id, &luid);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "Interface timed out",
                ));
            }

            match iface::open_interface(&luid) {
                Err(_) => {
                    std::thread::yield_now();
                    continue;
                }
                Ok(handle) => {
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
            Err(e) => {
                let _ = iface::delete_interface(component_id, &luid);
                Err(e)?
            }
        };
        Ok(Self {
            luid,
            handle,
            index,
            component_id: component_id.to_owned(),
            need_delete: true,
            read_io_overlapped: Mutex::new((None, vec![0; READ_BUFFER_SIZE])),
            write_io_overlapped: Mutex::new(None),
        })
    }

    /// Opens an existing tap-windows device by name
    pub fn open(component_id: &str, name: &str) -> io::Result<Self> {
        let luid = ffi::alias_to_luid(name)?;
        iface::check_interface(component_id, &luid)?;

        let handle = iface::open_interface(&luid)?;
        let index = ffi::luid_to_index(&luid)?;
        Ok(Self {
            index,
            luid,
            handle,
            component_id: component_id.to_owned(),
            need_delete: false,
            read_io_overlapped: Mutex::new((None, vec![0; READ_BUFFER_SIZE])),
            write_io_overlapped: Mutex::new(None),
        })
    }

    /// Sets the status of the interface to disconnected.
    /// Equivalent to `.set_status(false)`
    pub fn down(&self) -> io::Result<()> {
        self.set_status(false)
    }

    /// Retieve the mac of the interface
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

    // ///Retieve the mtu of the interface
    // pub fn get_mtu(&self) -> io::Result<u32> {
    //     let in_mtu: u32 = 0;
    //     let mut out_mtu = 0;
    //     ffi::device_io_control(self.handle, TAP_IOCTL_GET_MTU, &in_mtu, &mut out_mtu)
    //         .map(|_| out_mtu)
    // }

    /// Retrieve the name of the interface
    pub fn get_name(&self) -> io::Result<String> {
        ffi::luid_to_alias(&self.luid)
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
    pub fn try_read(&self, mut buf: &mut [u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.read_io_overlapped.try_lock() else {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        let (overlapped, read_buffer) = guard.deref_mut();
        let n = if let Some(overlapped) = overlapped {
            ffi::try_io_overlapped(self.handle.as_raw_handle(), overlapped)?
        } else {
            let overlapped = overlapped.insert(Box::new(ffi::io_overlapped()));
            ffi::try_read_file(self.handle.as_raw_handle(), overlapped, read_buffer)?
        };
        _ = overlapped.take();
        let n = n as usize;
        match io::copy(&mut &read_buffer[..n], &mut buf) {
            Ok(n) => Ok(n as usize),
            Err(e) => Err(e),
        }
    }
    pub fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        let Ok(mut guard) = self.write_io_overlapped.try_lock() else {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        };
        if let Some((overlapped, _write_buffer)) = guard.deref_mut() {
            ffi::try_io_overlapped(self.handle.as_raw_handle(), overlapped)?;
        }
        let (overlapped, write_buffer) =
            guard.insert((Box::new(ffi::io_overlapped()), buf.to_vec()));
        let rs = ffi::try_write_file(self.handle.as_raw_handle(), overlapped, write_buffer);
        match rs {
            Ok(len) => {
                guard.take();
                Ok(len as _)
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(buf.len()),
            Err(e) => Err(e),
        }
    }
    pub fn read(&self, mut buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self.read_io_overlapped.lock().unwrap();
        let (overlapped, read_buffer) = guard.deref_mut();
        let n = if let Some(overlapped) = overlapped.take() {
            ffi::wait_io_overlapped(self.handle.as_raw_handle(), &overlapped)? as usize
        } else {
            return ffi::read_file(self.handle.as_raw_handle(), buf).map(|res| res as _);
        };
        match io::copy(&mut &read_buffer[..n], &mut buf) {
            Ok(n) => Ok(n as usize),
            Err(e) => Err(e),
        }
    }
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut guard = self.write_io_overlapped.lock().unwrap();
        if let Some((overlapped, _write_buffer)) = guard.take() {
            ffi::wait_io_overlapped(self.handle.as_raw_handle(), &overlapped)?;
        }
        ffi::write_file(self.handle.as_raw_handle(), buf).map(|res| res as _)
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
