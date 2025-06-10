use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::{io, ptr};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_BUFFER_OVERFLOW, ERROR_HANDLE_EOF, ERROR_INVALID_DATA, ERROR_NO_MORE_ITEMS,
    WAIT_FAILED, WAIT_OBJECT_0,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};

use crate::platform::windows::ffi;
use crate::platform::windows::ffi::encode_utf16;

mod adapter;
mod adapter_win7;
mod wintun_log;
mod wintun_raw;

pub use adapter::check_adapter_if_orphaned_devices;

/// The maximum size of wintun's internal ring buffer (in bytes)
pub const MAX_RING_CAPACITY: u32 = 0x400_0000;

/// The minimum size of wintun's internal ring buffer (in bytes)
pub const MIN_RING_CAPACITY: u32 = 0x2_0000;

/// Maximum pool name length including zero terminator
pub const MAX_POOL: usize = 256;

pub struct TunDevice {
    index: u32,
    luid: NET_LUID_LH,
    win_tun_adapter: WinTunAdapter,
}
struct WinTunAdapter {
    win_tun: Arc<wintun_raw::wintun>,
    handle: wintun_raw::WINTUN_ADAPTER_HANDLE,
    event: OwnedHandle,
    ring_capacity: u32,
    state: State,
    session: RwLock<Option<WinTunSession>>,
    delete_driver: bool,
}
unsafe impl Send for WinTunAdapter {}
unsafe impl Sync for WinTunAdapter {}
struct WinTunSession {
    win_tun: Arc<wintun_raw::wintun>,
    handle: wintun_raw::WINTUN_SESSION_HANDLE,
    read_event: wintun_raw::HANDLE,
}
impl Drop for WinTunAdapter {
    fn drop(&mut self) {
        let session = self.session.write().unwrap().take();
        drop(session);
        unsafe {
            self.win_tun.WintunCloseAdapter(self.handle);
            if self.delete_driver {
                self.win_tun.WintunDeleteDriver();
            }
        }
    }
}
#[derive(Default)]
struct State {
    state: AtomicBool,
    lock: Mutex<()>,
}
impl State {
    fn check(&self) -> io::Result<()> {
        if self.is_enabled() {
            Ok(())
        } else {
            Err(io::Error::other("The interface has been disabled"))
        }
    }
    fn is_disabled(&self) -> bool {
        !self.state.load(Ordering::Relaxed)
    }
    fn is_enabled(&self) -> bool {
        self.state.load(Ordering::Relaxed)
    }
    fn disable(&self) {
        self.state.store(false, Ordering::Relaxed);
    }
    fn enable(&self) {
        self.state.store(true, Ordering::Relaxed);
    }
    fn lock(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().unwrap()
    }
}
impl WinTunAdapter {
    fn disable(&self) -> io::Result<()> {
        let _guard = self.state.lock();
        if self.state.is_disabled() {
            return Ok(());
        }
        self.state.disable();
        if let Err(e) = ffi::set_event(self.event.as_raw_handle()) {
            self.state.enable();
            return Err(e);
        }
        _ = self.session.write().unwrap().take();
        ffi::reset_event(self.event.as_raw_handle())
    }

    fn enable(&self) -> io::Result<()> {
        let _guard = self.state.lock();
        if self.state.is_disabled() {
            let mut session = self.session.write().unwrap();
            unsafe {
                let session_handle = self
                    .win_tun
                    .WintunStartSession(self.handle, self.ring_capacity);
                if session_handle.is_null() {
                    Err(io::Error::last_os_error())?
                }
                let read_event_handle = self.win_tun.WintunGetReadWaitEvent(session_handle);
                if read_event_handle.is_null() {
                    self.win_tun.WintunEndSession(session_handle);
                    Err(io::Error::last_os_error())?
                }

                let wintun_session = WinTunSession {
                    win_tun: self.win_tun.clone(),
                    handle: session_handle,
                    read_event: read_event_handle,
                };
                session.replace(wintun_session);
            }
            self.state.enable();
        }
        Ok(())
    }
    fn version(&self) -> io::Result<String> {
        let version = unsafe { self.win_tun.WintunGetRunningDriverVersion() };
        let v = version.to_be_bytes();
        Ok(format!(
            "{}.{}",
            u16::from_be_bytes([v[0], v[1]]),
            u16::from_be_bytes([v[2], v[3]])
        ))
    }
    fn send(&self, buf: &[u8], event: Option<&OwnedHandle>) -> io::Result<usize> {
        let guard = self.session.read().unwrap();
        if let Some(session) = guard.as_ref() {
            return session.send(buf, &self.state, event);
        }
        Err(io::Error::other("The interface has been disabled"))
    }
    fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let guard = self.session.read().unwrap();
        if let Some(session) = guard.as_ref() {
            return session.recv(&self.event, buf);
        }
        Err(io::Error::other("The interface has been disabled"))
    }
    fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        let guard = self.session.read().unwrap();
        if let Some(session) = guard.as_ref() {
            return session.try_send(buf);
        }
        Err(io::Error::other("The interface has been disabled"))
    }
    fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        let guard = self.session.read().unwrap();
        if let Some(session) = guard.as_ref() {
            return session.try_recv(buf);
        }
        Err(io::Error::other("The interface has been disabled"))
    }
    fn wait_readable_interruptible(&self, interrupt_event: &OwnedHandle) -> io::Result<()> {
        let guard = self.session.read().unwrap();
        if let Some(session) = guard.as_ref() {
            return session.wait_readable_interruptible(&self.event, interrupt_event);
        }
        Err(io::Error::other("The interface has been disabled"))
    }
}

impl Drop for WinTunSession {
    fn drop(&mut self) {
        unsafe {
            self.win_tun.WintunEndSession(self.handle);
        }
    }
}

impl WinTunSession {
    fn send(&self, buf: &[u8], state: &State, event: Option<&OwnedHandle>) -> io::Result<usize> {
        let mut count = 0;
        loop {
            return match self.try_send(buf) {
                Ok(len) => Ok(len),
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    state.check()?;
                    count += 1;
                    if count > 50 {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    if let Some(event) = event {
                        if ffi::wait_for_single_object(event.as_raw_handle(), 0).is_ok() {
                            return Err(io::Error::new(
                                io::ErrorKind::Interrupted,
                                "trigger interrupt",
                            ));
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(1));
                    continue;
                }
                Err(e) => Err(e),
            };
        }
    }
    fn recv(&self, inner_event: &OwnedHandle, buf: &mut [u8]) -> io::Result<usize> {
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
            self.wait_readable(inner_event)?;
        }
    }
    fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        assert!(buf.len() <= u32::MAX as _);
        let win_tun = &self.win_tun;
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
        let mut size = 0u32;

        let win_tun = &self.win_tun;
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
    fn wait_readable_interruptible(
        &self,
        inner_event: &OwnedHandle,
        interrupt_event: &OwnedHandle,
    ) -> io::Result<()> {
        //Wait on both the read handle and the shutdown handle so that we stop when requested
        let handles = [
            self.read_event,
            inner_event.as_raw_handle(),
            interrupt_event.as_raw_handle(),
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
                    Err(io::Error::other("The interface has been disabled"))
                } else if result == WAIT_OBJECT_0 + 2 {
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
    fn wait_readable(&self, inner_event: &OwnedHandle) -> io::Result<()> {
        //Wait on both the read handle and the shutdown handle so that we stop when requested
        let handles = [self.read_event, inner_event.as_raw_handle()];
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
                } else if result == WAIT_OBJECT_0 + 1 {
                    Err(io::Error::other("The interface has been disabled"))
                } else {
                    Err(io::Error::last_os_error())
                }
            }
        }
    }
}

impl TunDevice {
    pub fn open(
        wintun_path: &str,
        name: &str,
        ring_capacity: u32,
        delete_driver: bool,
    ) -> std::io::Result<Self> {
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
            let event = ffi::create_event()?;

            let win_tun = wintun_raw::wintun::new(wintun_path).map_err(io::Error::other)?;
            wintun_log::set_default_logger_if_unset(&win_tun);
            let adapter = win_tun.WintunOpenAdapter(name_utf16.as_ptr());
            if adapter.is_null() {
                Err(io::Error::last_os_error())?
            }
            let mut luid: wintun_raw::NET_LUID = std::mem::zeroed();
            win_tun.WintunGetAdapterLUID(adapter, &mut luid as *mut wintun_raw::NET_LUID);

            let win_tun_adapter = WinTunAdapter {
                win_tun: Arc::new(win_tun),
                handle: adapter,
                state: State::default(),
                event,
                ring_capacity,
                session: Default::default(),
                delete_driver,
            };
            let luid = std::mem::transmute::<wintun_raw::_NET_LUID_LH, NET_LUID_LH>(luid);
            let index = ffi::luid_to_index(&luid)?;

            let tun = Self {
                luid,
                index,
                win_tun_adapter,
            };
            Ok(tun)
        }
    }
    pub fn create(
        wintun_path: &str,
        name: &str,
        description: &str,
        guid: Option<u128>,
        ring_capacity: u32,
        delete_driver: bool,
    ) -> std::io::Result<Self> {
        let range = MIN_RING_CAPACITY..=MAX_RING_CAPACITY;
        if !range.contains(&ring_capacity) {
            Err(io::Error::other(format!(
                "ring capacity {ring_capacity} not in [{MIN_RING_CAPACITY},{MAX_RING_CAPACITY}]"
            )))?;
        }
        let name_utf16 = encode_utf16(name);
        let description_utf16 = encode_utf16(description);
        if name_utf16.len() > MAX_POOL {
            Err(io::Error::other("name too long"))?;
        }
        if description_utf16.len() > MAX_POOL {
            Err(io::Error::other("tunnel type too long"))?;
        }
        unsafe {
            let event = ffi::create_event()?;

            let win_tun = wintun_raw::wintun::new(wintun_path).map_err(io::Error::other)?;
            wintun_log::set_default_logger_if_unset(&win_tun);
            //SAFETY: guid is a unique integer so transmuting either all zeroes or the user's preferred
            //guid to the wintun_raw guid type is safe and will allow the windows kernel to see our GUID

            let guid_ptr = guid
                .map(|guid| {
                    let guid_struct: wintun_raw::GUID = std::mem::transmute(guid);
                    &guid_struct as *const wintun_raw::GUID
                })
                .unwrap_or(ptr::null());

            //SAFETY: the function is loaded from the wintun dll properly, we are providing valid
            //pointers, and all the strings are correct null terminated UTF-16. This safety rationale
            //applies for all Wintun* functions below
            let adapter = win_tun.WintunCreateAdapter(
                name_utf16.as_ptr(),
                description_utf16.as_ptr(),
                guid_ptr,
            );
            if adapter.is_null() {
                Err(io::Error::last_os_error())?
            }
            let mut luid: wintun_raw::NET_LUID = std::mem::zeroed();
            win_tun.WintunGetAdapterLUID(adapter, &mut luid as *mut wintun_raw::NET_LUID);

            let win_tun_adapter = WinTunAdapter {
                win_tun: Arc::new(win_tun),
                handle: adapter,
                state: State::default(),
                event,
                ring_capacity,
                session: Default::default(),
                delete_driver,
            };
            let luid = std::mem::transmute::<wintun_raw::_NET_LUID_LH, NET_LUID_LH>(luid);
            let index = ffi::luid_to_index(&luid)?;

            let tun = Self {
                luid,
                index,
                win_tun_adapter,
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
        self.win_tun_adapter.send(buf, None)
    }

    #[allow(dead_code)]
    pub(crate) fn send_interruptible(&self, buf: &[u8], event: &OwnedHandle) -> io::Result<usize> {
        self.win_tun_adapter.send(buf, Some(event))
    }
    #[allow(dead_code)]
    pub(crate) fn wait_readable_interruptible(
        &self,
        interrupt_event: &OwnedHandle,
    ) -> io::Result<()> {
        self.win_tun_adapter
            .wait_readable_interruptible(interrupt_event)
    }
    pub fn recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.win_tun_adapter.recv(buf)
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.win_tun_adapter.try_send(buf)
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.win_tun_adapter.try_recv(buf)
    }
    pub fn shutdown(&self) -> io::Result<()> {
        self.win_tun_adapter.disable()
    }
    pub fn version(&self) -> io::Result<String> {
        self.win_tun_adapter.version()
    }
    pub fn enabled(&self, value: bool) -> io::Result<()> {
        if value {
            self.win_tun_adapter.enable()
        } else {
            self.win_tun_adapter.disable()
        }
    }
}
