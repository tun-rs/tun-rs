use crate::device::ETHER_ADDR_LEN;
use crate::getifaddrs::Interface;
use crate::platform::Device;
use crate::IntoAddress;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

/// An async TUN device wrapper around a TUN device.
pub struct AsyncDevice {
    inner: Arc<Device>,
    recv_task_lock: Arc<Mutex<Option<blocking::Task<io::Result<(Vec<u8>, usize)>>>>>,
    send_task_lock: Arc<Mutex<Option<blocking::Task<io::Result<usize>>>>>,
}

impl Drop for AsyncDevice {
    fn drop(&mut self) {
        let _ = self.inner.shutdown();
    }
}

impl AsyncDevice {
    /// Create a new `AsyncDevice` wrapping around a `Device`.
    pub fn new(device: Device) -> io::Result<AsyncDevice> {
        let inner = Arc::new(device);

        Ok(AsyncDevice {
            inner,
            recv_task_lock: Arc::new(Mutex::new(None)),
            send_task_lock: Arc::new(Mutex::new(None)),
        })
    }
    pub fn poll_recv(&self, cx: &mut Context<'_>, mut buf: &mut [u8]) -> Poll<io::Result<usize>> {
        match self.try_recv(buf) {
            Ok(len) => return Poll::Ready(Ok(len)),
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    return Poll::Ready(Err(e));
                }
            }
        }
        let mut guard = self.recv_task_lock.lock().unwrap();
        let mut task = if let Some(task) = guard.take() {
            task
        } else {
            let device = self.inner.clone();
            let size = buf.len();
            blocking::unblock(move || {
                let mut in_buf = vec![0; size];
                let n = device.recv(&mut in_buf)?;
                Ok((in_buf, n))
            })
        };
        match Pin::new(&mut task).poll(cx) {
            Poll::Ready(Ok((packet, n))) => {
                drop(guard);
                let mut packet: &[u8] = &packet[..n];
                match io::copy(&mut packet, &mut buf) {
                    Ok(n) => Poll::Ready(Ok(n as usize)),
                    Err(e) => Poll::Ready(Err(e)),
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => {
                guard.replace(task);
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
        loop {
            if let Some(task) = guard.as_mut() {
                match Pin::new(task).poll(cx) {
                    Poll::Ready(rs) => {
                        _ = guard.take();
                        // If the previous write was successful, continue.
                        // Otherwise, error.
                        rs?;
                        continue;
                    }
                    Poll::Pending => return Poll::Pending,
                }
            } else {
                let device = self.inner.clone();
                let buf = src.to_vec();
                let task = blocking::unblock(move || device.send(&buf));
                guard.replace(task);
                return Poll::Ready(Ok(src.len()));
            };
        }
    }

    /// Recv a packet from tun device - Not implemented for windows
    pub async fn recv(&self, mut buf: &mut [u8]) -> io::Result<usize> {
        match self.try_recv(buf) {
            Ok(len) => return Ok(len),
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    Err(e)?
                }
            }
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

    /// Send a packet to tun device - Not implemented for windows
    pub async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match self.inner.try_send(buf) {
            Ok(len) => return Ok(len),
            Err(e) => {
                if e.kind() != io::ErrorKind::WouldBlock {
                    Err(e)?
                }
            }
        }
        let buf = buf.to_vec();
        let device = self.inner.clone();
        blocking::unblock(move || device.send(&buf)).await
    }
    pub fn try_send(&self, buf: &[u8]) -> io::Result<usize> {
        self.inner.try_send(buf)
    }

    pub fn name(&self) -> crate::Result<String> {
        self.inner.name()
    }

    pub fn set_name(&self, name: &str) -> crate::Result<()> {
        self.inner.set_name(name)
    }

    pub fn if_index(&self) -> crate::Result<u32> {
        self.inner.if_index()
    }

    pub fn enabled(&self, value: bool) -> crate::Result<()> {
        self.inner.enabled(value)
    }

    pub fn addresses(&self) -> crate::Result<Vec<Interface>> {
        self.inner.addresses()
    }

    pub fn set_network_address<A: IntoAddress>(
        &self,
        address: A,
        netmask: A,
        destination: Option<A>,
    ) -> crate::Result<()> {
        self.inner
            .set_network_address(address, netmask, destination)
    }

    pub fn mtu(&self) -> crate::Result<u16> {
        self.inner.mtu()
    }

    pub fn set_mtu(&self, value: u16) -> crate::Result<()> {
        self.inner.set_mtu(value)
    }

    pub fn set_mac_address(&self, eth_addr: [u8; ETHER_ADDR_LEN as usize]) -> crate::Result<()> {
        self.inner.set_mac_address(eth_addr)
    }

    pub fn mac_address(&self) -> crate::Result<[u8; ETHER_ADDR_LEN as usize]> {
        self.inner.mac_address()
    }

    pub fn remove_network_address(&self, addrs: Vec<(std::net::IpAddr, u8)>) -> crate::Result<()> {
        self.inner.remove_network_address(addrs)
    }

    pub fn add_address_v6(&self, addr: std::net::IpAddr, prefix: u8) -> crate::Result<()> {
        self.inner.add_address_v6(addr, prefix)
    }

    pub fn set_metric(&self, metric: u16) -> crate::Result<()> {
        self.inner.set_metric(metric)
    }
}
