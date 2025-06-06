use crate::platform::windows::ffi;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};

pub struct InterruptEvent(pub(crate) OwnedHandle);
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
        Ok(Self(ffi::create_event()?))
    }
    pub fn wake(&self) -> io::Result<()> {
        ffi::set_event(self.0.as_raw_handle())
    }
}
