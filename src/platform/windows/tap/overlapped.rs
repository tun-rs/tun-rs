use crate::platform::windows::ffi;
use crate::platform::windows::tap::READ_BUFFER_SIZE;
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
    pub fn try_read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
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
                let result = io::copy(&mut &self.read_buffer[..len], &mut buf);
                match result {
                    Ok(n) => Ok(n as usize),
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
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
        let inner = &mut self.inner;
        loop {
            return if inner.no_pending_io {
                inner.reset()?;
                self.read_buffer.clear();
                self.read_buffer.extend_from_slice(buf);
                let result = ffi::try_write_file(
                    inner.file_handle.as_raw_handle(),
                    &mut inner.overlapped,
                    &self.read_buffer,
                )
                .map(|size| size as _);
                if let Err(e) = &result {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        inner.no_pending_io = false;
                        // WouldBlock here means the async write was successfully submitted and is still in progress
                        return Ok(buf.len());
                    }
                }
                result
            } else {
                let result =
                    ffi::try_io_overlapped(inner.file_handle.as_raw_handle(), &inner.overlapped)
                        .map(|size| size as _);
                if result.is_ok() {
                    inner.no_pending_io = true;
                    continue;
                }
                result
            };
        }
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
    pub fn wait_interruptible(&self, interrupt_event: &OwnedHandle) -> io::Result<()> {
        let handles = [self.event.as_raw_handle(), interrupt_event.as_raw_handle()];
        unsafe {
            let wait_ret = WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE);
            match wait_ret {
                windows_sys::Win32::Foundation::WAIT_OBJECT_0 => Ok(()),
                _ => {
                    if wait_ret == windows_sys::Win32::Foundation::WAIT_OBJECT_0 + 1 {
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
}
