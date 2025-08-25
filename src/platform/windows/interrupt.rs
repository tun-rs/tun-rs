use crate::platform::windows::ffi;
use std::io;
use std::os::windows::io::{AsRawHandle, OwnedHandle};
use std::sync::Mutex;

pub struct InterruptEvent {
    pub(crate) handle: OwnedHandle,
    state: Mutex<i32>,
}
impl InterruptEvent {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            handle: ffi::create_event()?,
            state: Mutex::new(0),
        })
    }
    pub fn trigger(&self) -> io::Result<()> {
        self.trigger_value(1)
    }
    pub fn trigger_value(&self, val: i32) -> io::Result<()> {
        let mut guard = self.state.lock().unwrap();
        *guard = val;
        ffi::set_event(self.handle.as_raw_handle())
    }
    #[cfg(feature = "interruptible")]
    pub fn is_trigger(&self) -> bool {
        *self.state.lock().unwrap() != 0
    }
    #[cfg(feature = "interruptible")]
    pub fn value(&self) -> i32 {
        *self.state.lock().unwrap()
    }
    #[cfg(feature = "interruptible")]
    pub fn reset(&self) -> io::Result<()> {
        let mut guard = self.state.lock().unwrap();
        *guard = 0;
        ffi::reset_event(self.handle.as_raw_handle())
    }
}
