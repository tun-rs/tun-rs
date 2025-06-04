use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{io, ptr};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_BUFFER_OVERFLOW, ERROR_HANDLE_EOF, ERROR_INVALID_DATA, ERROR_NO_MORE_ITEMS,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};

use crate::platform::windows::ffi;
use crate::platform::windows::ffi::encode_utf16;

mod wintun_log;
mod wintun_raw;

/// The maximum size of wintun's internal ring buffer (in bytes)
pub const MAX_RING_CAPACITY: u32 = 0x400_0000;

/// The minimum size of wintun's internal ring buffer (in bytes)
pub const MIN_RING_CAPACITY: u32 = 0x2_0000;

/// Maximum pool name length including zero terminator
pub const MAX_POOL: usize = 256;

pub struct TunDevice {
    index: u32,
    luid: NET_LUID_LH,
    session: SessionHandle,
}
struct AdapterHandle {
    win_tun: wintun_raw::wintun,
    handle: wintun_raw::WINTUN_ADAPTER_HANDLE,
    shutdown_state: AtomicBool,
    shutdown_event: OwnedHandle,
    ring_capacity: u32,
}
impl Drop for AdapterHandle {
    fn drop(&mut self) {
        unsafe {
            self.win_tun.WintunCloseAdapter(self.handle);
            self.win_tun.WintunDeleteDriver();
        }
    }
}
impl AdapterHandle {
    fn version(&self) -> io::Result<String> {
        let version = unsafe { self.win_tun.WintunGetRunningDriverVersion() };
        let v = version.to_be_bytes();
        Ok(format!(
            "{}.{}",
            u16::from_be_bytes([v[0], v[1]]),
            u16::from_be_bytes([v[2], v[3]])
        ))
    }
    fn start_session(self) -> io::Result<SessionHandle> {
        unsafe {
            let session = self
                .win_tun
                .WintunStartSession(self.handle, self.ring_capacity);
            if session.is_null() {
                Err(io::Error::last_os_error())?
            }
            let read_event_handle = self.win_tun.WintunGetReadWaitEvent(session);
            if read_event_handle.is_null() {
                Err(io::Error::last_os_error())?
            }

            let session = SessionHandle {
                adapter: self,
                handle: session,
                read_event: read_event_handle,
            };
            Ok(session)
        }
    }
    fn shutdown(&self) -> io::Result<()> {
        self.shutdown_state.store(true, Ordering::SeqCst);
        ffi::set_event(self.shutdown_event.as_raw_handle())
    }
    fn is_shutdown(&self) -> bool {
        self.shutdown_state.load(Ordering::SeqCst)
    }
}
unsafe impl Send for AdapterHandle {}
unsafe impl Sync for AdapterHandle {}
struct SessionHandle {
    adapter: AdapterHandle,
    handle: wintun_raw::WINTUN_SESSION_HANDLE,
    read_event: wintun_raw::HANDLE,
}
impl Drop for SessionHandle {
    fn drop(&mut self) {
        unsafe {
            self.adapter.win_tun.WintunEndSession(self.handle);
        }
    }
}
unsafe impl Send for SessionHandle {}
unsafe impl Sync for SessionHandle {}
impl TunDevice {
    pub fn open(wintun_path: &str, name: &str, ring_capacity: u32) -> std::io::Result<Self> {
        let range = MIN_RING_CAPACITY..=MAX_RING_CAPACITY;
        if !range.contains(&ring_capacity) {
            Err(io::Error::other(format!(
                "ring capacity {ring_capacity} not in [{MIN_RING_CAPACITY},{MAX_RING_CAPACITY}]"
            )))?;
        }
        let name_utf16 = encode_utf16(name);
        if name_utf16.len() > MAX_POOL {
            Err(io::Error::other("name too long"))?;
        }

        unsafe {
            let shutdown_event = ffi::create_event()?;

            let win_tun = wintun_raw::wintun::new(wintun_path).map_err(io::Error::other)?;
            wintun_log::set_default_logger_if_unset(&win_tun);
            let adapter = win_tun.WintunOpenAdapter(name_utf16.as_ptr());
            if adapter.is_null() {
                Err(io::Error::last_os_error())?
            }
            let mut luid: wintun_raw::NET_LUID = std::mem::zeroed();
            win_tun.WintunGetAdapterLUID(adapter, &mut luid as *mut wintun_raw::NET_LUID);

            let adapter = AdapterHandle {
                win_tun,
                handle: adapter,
                ring_capacity,
                shutdown_event,
                shutdown_state: AtomicBool::new(false),
            };
            let luid = std::mem::transmute::<wintun_raw::_NET_LUID_LH, NET_LUID_LH>(luid);
            let index = ffi::luid_to_index(&luid)?;
            let session = adapter.start_session()?;

            let tun = Self {
                luid,
                index,
                session,
            };
            Ok(tun)
        }
    }
    pub fn create(
        wintun_path: &str,
        name: &str,
        tunnel_type: &str,
        guid: u128,
        ring_capacity: u32,
    ) -> std::io::Result<Self> {
        let range = MIN_RING_CAPACITY..=MAX_RING_CAPACITY;
        if !range.contains(&ring_capacity) {
            Err(io::Error::other(format!(
                "ring capacity {ring_capacity} not in [{MIN_RING_CAPACITY},{MAX_RING_CAPACITY}]"
            )))?;
        }
        let name_utf16 = encode_utf16(name);
        let tunnel_type_utf16 = encode_utf16(tunnel_type);
        if name_utf16.len() > MAX_POOL {
            Err(io::Error::other("name too long"))?;
        }
        if tunnel_type_utf16.len() > MAX_POOL {
            Err(io::Error::other("tunnel type too long"))?;
        }
        unsafe {
            let shutdown_event = ffi::create_event()?;

            let win_tun = wintun_raw::wintun::new(wintun_path).map_err(io::Error::other)?;
            wintun_log::set_default_logger_if_unset(&win_tun);
            //SAFETY: guid is a unique integer so transmuting either all zeroes or the user's preferred
            //guid to the wintun_raw guid type is safe and will allow the windows kernel to see our GUID

            let guid_struct: wintun_raw::GUID = std::mem::transmute(guid);
            let guid_ptr = &guid_struct as *const wintun_raw::GUID;

            //SAFETY: the function is loaded from the wintun dll properly, we are providing valid
            //pointers, and all the strings are correct null terminated UTF-16. This safety rationale
            //applies for all Wintun* functions below
            let adapter = win_tun.WintunCreateAdapter(
                name_utf16.as_ptr(),
                tunnel_type_utf16.as_ptr(),
                guid_ptr,
            );
            if adapter.is_null() {
                Err(io::Error::last_os_error())?
            }
            let mut luid: wintun_raw::NET_LUID = std::mem::zeroed();
            win_tun.WintunGetAdapterLUID(adapter, &mut luid as *mut wintun_raw::NET_LUID);

            let adapter = AdapterHandle {
                win_tun,
                handle: adapter,
                ring_capacity,
                shutdown_event,
                shutdown_state: AtomicBool::new(false),
            };
            let luid = std::mem::transmute::<wintun_raw::_NET_LUID_LH, NET_LUID_LH>(luid);
            let index = ffi::luid_to_index(&luid)?;
            let session = adapter.start_session()?;

            let tun = Self {
                luid,
                index,
                session,
            };
            Ok(tun)
        }
    }
    pub fn index(&self) -> u32 {
        self.index
    }
    pub fn get_name(&self) -> io::Result<String> {
        ffi::luid_to_alias(&self.luid)
    }
    pub fn send(&self, buf: &[u8]) -> io::Result<usize> {
        self.session.send(buf, None)
    }

    #[cfg(any(feature = "async_tokio", feature = "async_io"))]
    pub(crate) fn send_cancelable(
        &self,
        buf: &[u8],
        cancel_event: &OwnedHandle,
    ) -> io::Result<usize> {
        self.session.send(buf, Some(cancel_event))
    }
    pub(crate) fn wait_readable_cancelable(&self, cancel_event: &OwnedHandle) -> io::Result<()> {
        self.session.wait_readable_cancelable(cancel_event)
    }
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.session.recv(buf)
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.session.try_send(buf)
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.session.try_recv(buf)
    }
    pub fn shutdown(&self) -> io::Result<()> {
        self.session.shutdown()
    }
    pub fn version(&self) -> io::Result<String> {
        self.session.adapter.version()
    }
}

impl SessionHandle {
    fn send(&self, buf: &[u8], cancel_event: Option<&OwnedHandle>) -> io::Result<usize> {
        let mut count = 0;
        loop {
            return match self.try_send(buf) {
                Ok(len) => Ok(len),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    count += 1;
                    if count > 50 {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    if let Some(cancel_event) = cancel_event {
                        if ffi::wait_for_single_object(cancel_event.as_raw_handle(), 0).is_ok() {
                            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancel"));
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(e) => Err(e),
            };
        }
    }
    fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            for i in 0..64 {
                return match self.try_recv(buf) {
                    Ok(n) => Ok(n),
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if i > 32 {
                            std::thread::yield_now()
                        } else {
                            std::hint::spin_loop();
                        }
                        continue;
                    }
                    Err(e) => Err(e),
                };
            }
            self.wait_readable()?;
        }
    }
    fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        assert!(buf.len() <= u32::MAX as _);
        self.check_shutdown()?;
        let win_tun = &self.adapter.win_tun;
        let handle = self.handle;
        let bytes_ptr = unsafe { win_tun.WintunAllocateSendPacket(handle, buf.len() as u32) };
        if bytes_ptr.is_null() {
            match unsafe { GetLastError() } {
                ERROR_HANDLE_EOF => Err(std::io::Error::from(io::ErrorKind::WriteZero)),
                ERROR_BUFFER_OVERFLOW => Err(std::io::Error::from(io::ErrorKind::WouldBlock)),
                ERROR_INVALID_DATA => Err(std::io::Error::from(io::ErrorKind::InvalidData)),
                e => Err(io::Error::from_raw_os_error(e as i32)),
            }
        } else {
            unsafe { ptr::copy_nonoverlapping(buf.as_ptr(), bytes_ptr, buf.len()) };
            unsafe { win_tun.WintunSendPacket(handle, bytes_ptr) };
            Ok(buf.len())
        }
    }
    fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.check_shutdown()?;
        let mut size = 0u32;

        let win_tun = &self.adapter.win_tun;
        let handle = self.handle;
        let ptr = unsafe { win_tun.WintunReceivePacket(handle, &mut size as *mut u32) };

        if ptr.is_null() {
            // Wintun returns ERROR_NO_MORE_ITEMS instead of blocking if packets are not available
            return match unsafe { GetLastError() } {
                ERROR_HANDLE_EOF => Err(std::io::Error::from(io::ErrorKind::UnexpectedEof)),
                ERROR_NO_MORE_ITEMS => Err(std::io::Error::from(io::ErrorKind::WouldBlock)),
                e => Err(io::Error::from_raw_os_error(e as i32)),
            };
        }
        let size = size as usize;
        if size > buf.len() {
            unsafe { win_tun.WintunReleaseReceivePacket(handle, ptr) };
            use std::io::{Error, ErrorKind::InvalidInput};
            return Err(Error::new(InvalidInput, "destination buffer too small"));
        }
        unsafe { ptr::copy_nonoverlapping(ptr, buf.as_mut_ptr(), size) };
        unsafe { win_tun.WintunReleaseReceivePacket(handle, ptr) };
        Ok(size)
    }
    pub(crate) fn wait_readable_cancelable(&self, cancel_event: &OwnedHandle) -> io::Result<()> {
        self.check_shutdown()?;
        //Wait on both the read handle and the shutdown handle so that we stop when requested
        let handles = [
            self.read_event,
            cancel_event.as_raw_handle(),
            self.adapter.shutdown_event.as_raw_handle(),
        ];
        let result = unsafe {
            //SAFETY: We abide by the requirements of WaitForMultipleObjects, handles is a
            //pointer to valid, aligned, stack memory
            WaitForMultipleObjects(3, &handles as _, 0, INFINITE)
        };
        match result {
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => {
                if result == WAIT_OBJECT_0 {
                    //We have data!
                    Ok(())
                } else if result == WAIT_OBJECT_0 + 1 {
                    Err(io::Error::new(io::ErrorKind::Interrupted, "cancel"))
                } else {
                    //Shutdown event triggered
                    Err(io::Error::other(format!(
                        "Shutdown event triggered {}",
                        io::Error::last_os_error()
                    )))
                }
            }
        }
    }
    fn wait_readable(&self) -> io::Result<()> {
        self.check_shutdown()?;
        //Wait on both the read handle and the shutdown handle so that we stop when requested
        let handles = [self.read_event, self.adapter.shutdown_event.as_raw_handle()];
        let result = unsafe {
            //SAFETY: We abide by the requirements of WaitForMultipleObjects, handles is a
            //pointer to valid, aligned, stack memory
            WaitForMultipleObjects(2, &handles as _, 0, INFINITE)
        };
        match result {
            WAIT_FAILED => Err(io::Error::last_os_error()),
            _ => {
                if result == WAIT_OBJECT_0 {
                    //We have data!
                    Ok(())
                } else {
                    //Shutdown event triggered
                    Err(io::Error::other(format!(
                        "Shutdown event triggered {}",
                        io::Error::last_os_error()
                    )))
                }
            }
        }
    }
    fn shutdown(&self) -> io::Result<()> {
        self.adapter.shutdown()
    }
    fn check_shutdown(&self) -> io::Result<()> {
        if self.adapter.is_shutdown() {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        Ok(())
    }
}
