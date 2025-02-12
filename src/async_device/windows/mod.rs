use crate::platform::DeviceImpl;
use crate::SyncDevice;
use std::future::Future;
use std::io;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

/// An async TUN device wrapper around a TUN device.
pub struct AsyncDevice {
    inner: Arc<DeviceImpl>,
    recv_task_lock: Arc<Mutex<Option<RecvTask>>>,
    send_task_lock: Arc<Mutex<Option<SendTask>>>,
}
type RecvTask = (blocking::Task<io::Result<(Vec<u8>, usize)>>, Vec<Waker>);
type SendTask = (blocking::Task<io::Result<usize>>, Vec<Waker>);
impl Deref for AsyncDevice {
    type Target = DeviceImpl;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl Drop for AsyncDevice {
    fn drop(&mut self) {
        _ = self.inner.shutdown();
    }
}
impl AsyncDevice {
    pub fn new(device: SyncDevice) -> io::Result<AsyncDevice> {
        AsyncDevice::new_dev(device.0)
    }
    /// Create a new `AsyncDevice` wrapping around a `Device`.
    pub(crate) fn new_dev(device: DeviceImpl) -> io::Result<AsyncDevice> {
        let inner = Arc::new(device);

        Ok(AsyncDevice {
            inner,
            recv_task_lock: Arc::new(Mutex::new(None)),
            send_task_lock: Arc::new(Mutex::new(None)),
        })
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, mut buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.try_recv(buf) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return Poll::Ready(rs),
        }
        let mut guard = self.recv_task_lock.lock().unwrap();
        let current = cx.waker();
        let (mut task, waiters) = if let Some((task, mut waiters)) = guard.take() {
            if !waiters.iter().any(|w| w.will_wake(current)) {
                waiters.push(current.clone());
            }
            (task, waiters)
        } else {
            let device = self.inner.clone();
            let size = buf.len();
            let task = blocking::unblock(move || {
                let mut in_buf = vec![0; size];
                let n = device.recv(&mut in_buf)?;
                Ok((in_buf, n))
            });
            (task, vec![current.clone()])
        };
        match Pin::new(&mut task).poll(cx) {
            Poll::Ready(rs) => {
                drop(guard);
                for waker in waiters {
                    if !waker.will_wake(current) {
                        waker.wake();
                    }
                }
                match rs {
                    Ok((packet, n)) => {
                        let mut packet: &[u8] = &packet[..n];
                        match io::copy(&mut packet, &mut buf) {
                            Ok(n) => Poll::Ready(Ok(n as usize)),
                            Err(e) => Poll::Ready(Err(e)),
                        }
                    }
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
            Poll::Pending => {
                guard.replace((task, waiters));
                Poll::Pending
            }
        }
    }
    pub fn poll_send(&self, cx: &mut Context<'_>, src: &[u8]) -> Poll<io::Result<usize>> {
        match self.try_send(src) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return Poll::Ready(rs),
        }
        let mut guard = self.send_task_lock.lock().unwrap();
        let current = cx.waker();
        let mut waiters = None;
        loop {
            if let Some((mut task, mut w)) = guard.take() {
                if !w.iter().any(|w| w.will_wake(current)) {
                    w.push(current.clone());
                }
                match Pin::new(&mut task).poll(cx) {
                    Poll::Ready(rs) => {
                        waiters = Some(w);
                        // If the previous write was successful, continue.
                        // Otherwise, error.
                        rs?;
                        continue;
                    }
                    Poll::Pending => {
                        guard.replace((task, w));
                        return Poll::Pending;
                    }
                }
            } else {
                let device = self.inner.clone();
                let buf = src.to_vec();
                let task = blocking::unblock(move || device.send(&buf));
                guard.replace((task, vec![]));
                drop(guard);
                if let Some(waiters) = waiters {
                    for waker in waiters {
                        if !waker.will_wake(current) {
                            waker.wake();
                        }
                    }
                }
                return Poll::Ready(Ok(src.len()));
            };
        }
    }

    /// Recv a packet from tun device
    pub async fn recv(&self, mut buf: &mut [u8]) -> io::Result<usize> {
        match self.try_recv(buf) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return rs,
        }
        let device = self.inner.clone();
        let size = buf.len();
        let (packet, n) = blocking::unblock(move || {
            let mut in_buf = vec![0; size];
            let n = device.recv(&mut in_buf)?;
            Ok::<(Vec<u8>, usize), io::Error>((in_buf, n))
        })
        .await?;
        let mut packet: &[u8] = &packet[..n];

        match io::copy(&mut packet, &mut buf) {
            Ok(n) => Ok(n as usize),
            Err(e) => Err(e),
        }
    }
    pub fn try_recv(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.inner.try_recv(buf)
    }

    /// Send a packet to tun device
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.try_send(buf) {
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
            rs => return rs,
        }
        let buf = buf.to_vec();
        let device = self.inner.clone();
        blocking::unblock(move || device.send(&buf)).await
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_send(buf)
    }
}
