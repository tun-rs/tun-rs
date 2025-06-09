use crate::platform::windows::ffi;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::Mutex;

pub struct InterruptEvent {
    pub(crate) handle: OwnedHandle,
    state: Mutex<bool>,
}
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            handle: ffi::create_event()?,
            state: Default::default(),
        })
    }
    pub fn trigger(&self) -> io::Result<()> {
        let mut guard = self.state.lock().unwrap();
        *guard = true;
        ffi::set_event(self.handle.as_raw_handle())
    }
    #[cfg(feature = "interruptible")]
    pub fn is_trigger(&self) -> bool {
        *self.state.lock().unwrap()
    }
    #[cfg(feature = "interruptible")]
    pub fn reset(&self) -> io::Result<()> {
        let mut guard = self.state.lock().unwrap();
        *guard = false;
        ffi::reset_event(self.handle.as_raw_handle())
    }
}
