use std::borrow::Borrow;
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use bytes::{BufMut, Bytes, BytesMut};
use futures::Sink;
use futures_core::Stream;

#[cfg(target_os = "linux")]
use crate::platform::offload::VirtioNetHdr;
use crate::AsyncDevice;
#[cfg(target_os = "linux")]
use crate::{GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

pub trait Decoder {
    /// The type of decoded frames.
    type Item;

    /// The type of unrecoverable frame decoding errors.
    type Error: From<io::Error>;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error>;
    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(buf)? {
            Some(frame) => Ok(Some(frame)),
            None => {
                if buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(io::ErrorKind::Other, "bytes remaining on stream").into())
                }
            }
        }
    }
}
pub trait Encoder<Item> {
    /// The type of encoding errors.
    type Error: From<io::Error>;

    /// Encodes a frame into the buffer provided.
    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error>;
}

/// A unified `Stream` and `Sink` interface to an underlying `AsyncDevice`,
/// using the `Encoder` and `Decoder` traits to encode and decode frames.
///
/// Raw device interfaces work with packets, but higher-level code usually
/// wants to batch these into meaningful chunks, called "frames".
/// This struct layers framing on top of the device by using the `Encoder`
/// and `Decoder` traits to handle encoding and decoding of message frames.
/// Note that the incoming and outgoing frame types may be distinct.
///
/// This function returns a single object that is both `Stream` and `Sink`;
/// grouping this into a single object is often useful for layering things
/// which require both read and write access to the underlying device.
///
/// If you want to work more directly with the stream and sink, consider
/// calling `split` on the `DeviceFramed` returned by this method, which
/// will break them into separate objects, allowing them to interact more easily.
///
/// Additionally, you can create multiple framing tools using
/// `DeviceFramed::new(dev.clone(), BytesCodec::new())`(use `Arc<AsyncDevice>`), allowing multiple
/// independent framed streams to operate on the same device.
pub struct DeviceFramed<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    recv_buffer_size: usize,
    send_buffer_size: usize,
    rd: BytesMut,
    wr: VecDeque<BytesMut>,
    #[cfg(target_os = "linux")]
    gro_table: Option<GROTable>,
    #[cfg(target_os = "linux")]
    send_bufs: Vec<BytesMut>,
    #[cfg(target_os = "linux")]
    send_index: usize,
    #[cfg(target_os = "linux")]
    rds: Vec<BytesMut>,
    #[cfg(target_os = "linux")]
    recv_index: usize,
    #[cfg(target_os = "linux")]
    recv_num: usize,
    #[cfg(target_os = "linux")]
    sizes: Vec<usize>,
}
impl<C, T> Unpin for DeviceFramed<C, T> {}
impl<C, T> Stream for DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
    C: Decoder,
{
    type Item = Result<C::Item, C::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        #[cfg(target_os = "linux")]
        if pin.gro_table.is_some() && pin.recv_index < pin.recv_num {
            let buf = &mut pin.rds[pin.recv_index];
            pin.recv_index += 1;
            if let Some(frame) = pin.codec.decode_eof(buf)? {
                return Poll::Ready(Some(Ok(frame)));
            }
        }
        pin.rd.clear();
        #[cfg(target_os = "linux")]
        if pin.gro_table.is_some() {
            pin.rd.reserve(VIRTIO_NET_HDR_LEN + 65536);
        }
        pin.rd.reserve(pin.recv_buffer_size);
        let buf = unsafe { &mut *(pin.rd.chunk_mut() as *mut _ as *mut [u8]) };

        let len = ready!(pin.dev.borrow().poll_recv(cx, buf))?;
        unsafe { pin.rd.advance_mut(len) };

        #[cfg(target_os = "linux")]
        if pin.gro_table.is_some() {
            pin.recv_index = 0;
            pin.recv_num = 0;
            if len <= VIRTIO_NET_HDR_LEN {
                Err(io::Error::other(format!(
                    "length of packet ({len}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                )))?
            }
            for buf in &mut pin.rds {
                buf.resize(pin.recv_buffer_size, 0);
            }
            let hdr = VirtioNetHdr::decode(&pin.rd[..VIRTIO_NET_HDR_LEN])?;
            let num = pin.dev.borrow().handle_virtio_read(
                hdr,
                &mut pin.rd[VIRTIO_NET_HDR_LEN..len],
                &mut pin.rds,
                &mut pin.sizes,
                0,
            )?;
            if num == 0 {
                return Poll::Ready(None);
            }
            for i in 0..num {
                pin.rds[i].truncate(pin.sizes[i]);
            }
            pin.recv_num = num;
            pin.recv_index = 1;
            if let Some(frame) = pin.codec.decode_eof(&mut pin.rds[0])? {
                return Poll::Ready(Some(Ok(frame)));
            }
            return Poll::Ready(None);
        }
        if let Some(frame) = pin.codec.decode_eof(&mut pin.rd)? {
            return Poll::Ready(Some(Ok(frame)));
        }
        Poll::Ready(None)
    }
}
impl<I, C, T> Sink<I> for DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
    C: Encoder<I>,
{
    type Error = C::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        #[cfg(target_os = "linux")]
        if self.gro_table.is_some() && self.wr.len() < IDEAL_BATCH_SIZE {
            return Poll::Ready(Ok(()));
        }
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        #[allow(unused_mut)]
        let mut capacity = pin.send_buffer_size;

        #[cfg(target_os = "linux")]
        if pin.gro_table.is_some() {
            capacity = 65536;
        }
        let mut buf = BytesMut::with_capacity(capacity);
        #[cfg(target_os = "linux")]
        if pin.gro_table.is_some() {
            buf.resize(VIRTIO_NET_HDR_LEN, 0)
        }
        pin.codec.encode(item, &mut buf)?;
        pin.wr.push_back(buf);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        #[cfg(target_os = "linux")]
        if self.gro_table.is_some() {
            if !self.send_bufs.is_empty() {
                ready!(self.poll_send_bufs(cx))?;
            }
            while let Some(frame) = self.wr.pop_front() {
                self.send_bufs.push(frame);
                if self.send_bufs.len() == IDEAL_BATCH_SIZE {
                    break;
                }
            }
            self.handle_gro()?;
            ready!(self.poll_send_bufs(cx))?;
            return Poll::Ready(Ok(()));
        }
        while let Some(frame) = self.wr.front() {
            let rs = ready!(self.dev.borrow().poll_send(cx, frame));
            _ = self.wr.pop_front();
            rs?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}
#[cfg(target_os = "linux")]
impl<C, T> DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
{
    fn handle_gro(&mut self) -> io::Result<()> {
        if self.send_bufs.is_empty() {
            return Ok(());
        }
        let tun = self.dev.borrow();
        let gro_table = if let Some(gro_table) = &mut self.gro_table {
            gro_table
        } else {
            unreachable!()
        };
        gro_table.reset();
        crate::platform::offload::handle_gro(
            &mut self.send_bufs,
            VIRTIO_NET_HDR_LEN,
            &mut gro_table.tcp_gro_table,
            &mut gro_table.udp_gro_table,
            tun.udp_gso,
            &mut gro_table.to_write,
        )
    }

    fn poll_send_bufs(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.send_bufs.is_empty() {
            return Poll::Ready(Ok(()));
        }
        let gro_table = if let Some(gro_table) = &mut self.gro_table {
            gro_table
        } else {
            unreachable!()
        };
        for buf_idx in &gro_table.to_write[self.send_index..] {
            let rs = self.dev.borrow().poll_send(cx, &self.send_bufs[*buf_idx]);
            match rs {
                Poll::Ready(Ok(_)) => {
                    self.send_index += 1;
                }
                Poll::Ready(Err(e)) => {
                    self.send_index += 1;
                    if self.send_index == self.send_bufs.len() {
                        self.send_index = 0;
                        self.send_bufs.clear();
                    }
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
        self.send_index = 0;
        self.send_bufs.clear();
        Poll::Ready(Ok(()))
    }
}
impl<C, T> DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
{
    pub fn new(dev: T, codec: C) -> DeviceFramed<C, T> {
        // The MTU of the network interface cannot be greater than this value.
        let recv_buffer_size = dev.borrow().mtu().map(|m| m as usize).unwrap_or(4096);
        let send_buffer_size = recv_buffer_size;
        #[cfg(target_os = "linux")]
        let (gro_table, send_bufs, rds, sizes) = if dev.borrow().tcp_gso() {
            (
                Some(GROTable::new()),
                Vec::with_capacity(IDEAL_BATCH_SIZE),
                vec![BytesMut::zeroed(recv_buffer_size); IDEAL_BATCH_SIZE],
                vec![0; IDEAL_BATCH_SIZE],
            )
        } else {
            (None, Vec::new(), Vec::new(), Vec::new())
        };
        DeviceFramed {
            dev,
            codec,
            recv_buffer_size,
            send_buffer_size,
            rd: BytesMut::with_capacity(recv_buffer_size),
            wr: VecDeque::with_capacity(128),
            #[cfg(target_os = "linux")]
            gro_table,
            #[cfg(target_os = "linux")]
            send_bufs,
            #[cfg(target_os = "linux")]
            send_index: 0,
            #[cfg(target_os = "linux")]
            rds,
            #[cfg(target_os = "linux")]
            recv_index: 0,
            #[cfg(target_os = "linux")]
            recv_num: 0,
            #[cfg(target_os = "linux")]
            sizes,
        }
    }
    pub fn read_buffer_size(&self) -> usize {
        self.recv_buffer_size
    }
    pub fn write_buffer_size(&self) -> usize {
        self.send_buffer_size
    }

    /// Sets the size of the read buffer in bytes.
    ///
    /// It is recommended to set this value to the MTU (Maximum Transmission Unit)
    /// to ensure optimal packet reading performance.
    pub fn set_read_buffer_size(&mut self, read_buffer_size: usize) {
        self.recv_buffer_size = read_buffer_size;
    }
    pub fn set_write_buffer_size(&mut self, write_buffer_size: usize) {
        self.send_buffer_size = write_buffer_size;
    }
    /// Returns a reference to the read buffer.
    pub fn read_buffer(&self) -> &BytesMut {
        &self.rd
    }

    /// Returns a mutable reference to the read buffer.
    pub fn read_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.rd
    }
    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct BytesCodec(());
impl BytesCodec {
    /// Creates a new `BytesCodec` for shipping around raw bytes.
    pub fn new() -> BytesCodec {
        BytesCodec(())
    }
}
impl Decoder for BytesCodec {
    type Item = BytesMut;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
        if !buf.is_empty() {
            let rs = buf.clone();
            buf.clear();
            Ok(Some(rs))
        } else {
            Ok(None)
        }
    }
}

impl Encoder<Bytes> for BytesCodec {
    type Error = io::Error;

    fn encode(&mut self, data: Bytes, buf: &mut BytesMut) -> Result<(), io::Error> {
        buf.reserve(data.len());
        buf.put(data);
        Ok(())
    }
}

impl Encoder<BytesMut> for BytesCodec {
    type Error = io::Error;

    fn encode(&mut self, data: BytesMut, buf: &mut BytesMut) -> Result<(), io::Error> {
        buf.reserve(data.len());
        buf.put(data);
        Ok(())
    }
}
