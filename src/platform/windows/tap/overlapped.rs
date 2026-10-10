use crate::platform::windows::ffi;
use crate::platform::windows::tap::READ_BUFFER_SIZE;
use bytes::buf::UninitSlice;
use bytes::BytesMut;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::Arc;
use windows_sys::Win32::System::Threading::{WaitForMultipleObjects, INFINITE};
use windows_sys::Win32::System::IO::OVERLAPPED;
pub(crate) struct ReadOverlapped {
    read_buffer: BytesMut,
    inner: OwnedOVERLAPPED,
}
impl ReadOverlapped {
    pub fn new(file_handle: Arc<OwnedHandle>) -> io::Result<ReadOverlapped> {
        let inner = OwnedOVERLAPPED::new(file_handle)?;
        Ok(Self {
            read_buffer: BytesMut::zeroed(READ_BUFFER_SIZE),
            inner,
        })
    }
    pub fn try_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.try_read_raw(buf.as_mut_ptr(), buf.len())
    }
    #[allow(dead_code)]
    pub fn try_read_uninit(&mut self, buf: &mut UninitSlice) -> io::Result<usize> {
        self.try_read_raw(buf.as_mut_ptr(), buf.len())
    }
    fn try_read_raw(&mut self, dst: *mut u8, dst_len: usize) -> io::Result<usize> {
        let inner = &mut self.inner;
        let result = if inner.no_pending_io {
            inner.reset()?;
            let result = ffi::try_read_file(
                inner.file_handle.as_raw_handle(),
                &mut inner.overlapped,
                &mut self.read_buffer,
            )
            .map(|size| size as _);
            if let Err(e) = &result {
                if e.kind() == io::ErrorKind::WouldBlock {
                    inner.no_pending_io = false;
                }
            }
            result
        } else {
            ffi::try_io_overlapped(inner.file_handle.as_raw_handle(), &inner.overlapped)
                .map(|size| size as _)
        };
        match result {
            Ok(len) => {
                inner.no_pending_io = true;
                if len > dst_len {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "receive buffer too small",
                    ));
                }
                unsafe {
                    std::ptr::copy_nonoverlapping(self.read_buffer.as_ptr(), dst, len);
                }
                Ok(len)
            }
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    inner.no_pending_io = true;
                }
                Err(e)
            }
        }
    }
    pub fn overlapped_event(&self) -> OverlappedEvent {
        OverlappedEvent {
            event: self.inner.event_handle.clone(),
        }
    }
}
pub(crate) struct WriteOverlapped {
    read_buffer: BytesMut,
    inner: OwnedOVERLAPPED,
}
impl WriteOverlapped {
    pub fn new(file_handle: Arc<OwnedHandle>) -> io::Result<WriteOverlapped> {
        let inner = OwnedOVERLAPPED::new(file_handle)?;
        Ok(Self {
            read_buffer: BytesMut::new(),
            inner,
        })
    }
    pub fn try_write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.finish_pending_nonblocking()? {
            return Err(io::Error::from(io::ErrorKind::WouldBlock));
        }
        self.submit(buf)
    }
    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.finish_pending_blocking()?;
        self.submit(buf)
    }
    pub fn write_interruptible(
        &mut self,
        buf: &[u8],
        interrupt_event: &OwnedHandle,
    ) -> io::Result<usize> {
        self.finish_pending_interruptible(interrupt_event)?;
        self.submit(buf)
    }
    fn submit(&mut self, buf: &[u8]) -> io::Result<usize> {
        let inner = &mut self.inner;
        inner.reset()?;
        self.read_buffer.clear();
        self.read_buffer.extend_from_slice(buf);
        match ffi::try_write_file(
            inner.file_handle.as_raw_handle(),
            &mut inner.overlapped,
            &self.read_buffer,
        ) {
            Ok(size) => {
                inner.no_pending_io = true;
                Ok(size as usize)
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                inner.no_pending_io = false;
                Ok(buf.len())
            }
            Err(e) => {
                inner.no_pending_io = true;
                Err(e)
            }
        }
    }
    fn finish_pending_nonblocking(&mut self) -> io::Result<bool> {
        let inner = &mut self.inner;
        if inner.no_pending_io {
            return Ok(true);
        }
        match ffi::try_io_overlapped(inner.file_handle.as_raw_handle(), &inner.overlapped) {
            Ok(_) => {
                inner.no_pending_io = true;
                Ok(true)
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(e) => {
                inner.no_pending_io = true;
                Err(e)
            }
        }
    }
    fn finish_pending_blocking(&mut self) -> io::Result<()> {
        let inner = &mut self.inner;
        if inner.no_pending_io {
            return Ok(());
        }
        let result = ffi::wait_io_overlapped(inner.file_handle.as_raw_handle(), &inner.overlapped);
        inner.no_pending_io = true;
        result.map(|_| ())
    }
    fn finish_pending_interruptible(&mut self, interrupt_event: &OwnedHandle) -> io::Result<()> {
        if self.inner.no_pending_io {
            return Ok(());
        }
        self.overlapped_event()
            .wait_interruptible(interrupt_event, None)?;
        self.finish_pending_blocking()
    }
    pub fn overlapped_event(&self) -> OverlappedEvent {
        OverlappedEvent {
            event: self.inner.event_handle.clone(),
        }
    }
}

pub(crate) struct OwnedOVERLAPPED {
    file_handle: Arc<OwnedHandle>,
    event_handle: Arc<OwnedHandle>,
    overlapped: Box<OVERLAPPED>,
    no_pending_io: bool,
}
impl Drop for OwnedOVERLAPPED {
    fn drop(&mut self) {
        if !self.no_pending_io {
            _ = ffi::cancel_io_overlapped(self.file_handle.as_raw_handle(), self.as_overlapped());
        }
    }
}
impl OwnedOVERLAPPED {
    pub fn new(file_handle: Arc<OwnedHandle>) -> io::Result<OwnedOVERLAPPED> {
        let event_handle = Arc::new(ffi::create_event()?);
        // Set the event to signaled when initializing OVERLAPPED,
        // so that the first wait does not block unexpectedly
        ffi::set_event(event_handle.as_raw_handle())?;
        let mut overlapped = Box::new(ffi::io_overlapped());
        overlapped.hEvent = event_handle.as_raw_handle();
        Ok(Self {
            file_handle,
            event_handle,
            overlapped,
            no_pending_io: true,
        })
    }

    pub fn as_overlapped(&self) -> &OVERLAPPED {
        &self.overlapped
    }
    pub fn reset(&self) -> io::Result<()> {
        ffi::reset_event(self.event_handle.as_raw_handle())
    }
}

pub struct OverlappedEvent {
    event: Arc<OwnedHandle>,
}
impl OverlappedEvent {
    pub fn wait(&self) -> io::Result<()> {
        ffi::wait_for_single_object(self.event.as_raw_handle(), INFINITE)
    }
    pub fn wait_interruptible(
        &self,
        interrupt_event: &OwnedHandle,
        timeout: Option<std::time::Duration>,
    ) -> io::Result<()> {
        let handles = [self.event.as_raw_handle(), interrupt_event.as_raw_handle()];
        let mut remaining = timeout;
        loop {
            let timeout_ms = remaining.map_or(INFINITE, ffi::finite_wait_timeout_millis);
            // SAFETY: handles is a live two-element array of valid wait handles;
            // WaitForMultipleObjects borrows it synchronously and count matches.
            let wait_ret = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout_ms) };
            match wait_ret {
                windows_sys::Win32::Foundation::WAIT_OBJECT_0 => return Ok(()),
                windows_sys::Win32::Foundation::WAIT_TIMEOUT => {
                    let Some(limit) = remaining else {
                        return Err(io::Error::other(
                            "infinite WaitForMultipleObjects unexpectedly timed out",
                        ));
                    };
                    let waited = std::time::Duration::from_millis(u64::from(timeout_ms));
                    if limit <= waited {
                        return Err(io::Error::from(io::ErrorKind::TimedOut));
                    }
                    remaining = Some(limit - waited);
                }
                value if value == windows_sys::Win32::Foundation::WAIT_OBJECT_0 + 1 => {
                    return Err(io::Error::new(
                        io::ErrorKind::Interrupted,
                        "trigger interrupt",
                    ));
                }
                windows_sys::Win32::Foundation::WAIT_FAILED => {
                    return Err(io::Error::last_os_error());
                }
                value => {
                    return Err(io::Error::other(format!(
                        "WaitForMultipleObjects returned unexpected status {value:#x}"
                    )));
                }
            }
        }
    }
}
