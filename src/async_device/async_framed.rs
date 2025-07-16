use std::borrow::Borrow;
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
    wr: BytesMut,
    #[cfg(target_os = "linux")]
    packet_aggregator: Option<PacketAggregator>,
    #[cfg(target_os = "linux")]
    packet_splitter: Option<PacketSplitter>,
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
        let mut inner = DeviceFramedReadInner::from_framed(pin);
        inner.poll_next(cx)
    }
}
impl<I, C, T> Sink<I> for DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
    C: Encoder<I>,
{
    type Error = C::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed(pin).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed(pin).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed(pin).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed(pin).poll_close(cx)
    }
}
impl<C, T> DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
{
    pub fn new(dev: T, codec: C) -> DeviceFramed<C, T> {
        // The MTU of the network interface cannot be greater than this value.
        let recv_buffer_size = dev.borrow().mtu().map(|m| m as usize).unwrap_or(4096);
        #[cfg(target_os = "linux")]
        let (packet_splitter, packet_aggregator) = if dev.borrow().tcp_gso() {
            (
                Some(PacketSplitter::new(recv_buffer_size)),
                Some(PacketAggregator::new()),
            )
        } else {
            (None, None)
        };

        DeviceFramed {
            dev,
            codec,
            recv_buffer_size,
            send_buffer_size: recv_buffer_size,
            rd: BytesMut::with_capacity(recv_buffer_size),
            wr: BytesMut::new(),
            #[cfg(target_os = "linux")]
            packet_aggregator,
            #[cfg(target_os = "linux")]
            packet_splitter,
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
        #[cfg(target_os = "linux")]
        if let Some(packet_splitter) = &mut self.packet_splitter {
            packet_splitter.set_recv_buffer_size(read_buffer_size);
        }
    }
    /// Sets the size of the write buffer in bytes.
    ///
    /// On Linux, if GSO (Generic Segmentation Offload) is enabled, this setting is ignored,
    /// and the send buffer size is fixed to a larger value to accommodate large TCP segments.
    ///
    /// If the current buffer size is already greater than or equal to the requested size,
    /// this call has no effect.
    ///
    /// # Parameters
    /// - `write_buffer_size`: Desired size in bytes for the write buffer.
    pub fn set_write_buffer_size(&mut self, write_buffer_size: usize) {
        #[cfg(target_os = "linux")]
        if self.packet_aggregator.is_some() {
            // When GSO is enabled, send_buffer_size is no longer controlled by this parameter.
            return;
        }
        if self.send_buffer_size >= write_buffer_size {
            return;
        }
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
pub struct DeviceFramedRead<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    recv_buffer_size: usize,
    rd: BytesMut,
    #[cfg(target_os = "linux")]
    packet_splitter: Option<PacketSplitter>,
}
impl<C, T> DeviceFramedRead<C, T>
where
    T: Borrow<AsyncDevice>,
{
    pub fn new(dev: T, codec: C) -> DeviceFramedRead<C, T> {
        // The MTU of the network interface cannot be greater than this value.
        let recv_buffer_size = dev.borrow().mtu().map(|m| m as usize).unwrap_or(4096);
        #[cfg(target_os = "linux")]
        let packet_splitter = if dev.borrow().tcp_gso() {
            Some(PacketSplitter::new(recv_buffer_size))
        } else {
            None
        };

        DeviceFramedRead {
            dev,
            codec,
            recv_buffer_size,
            rd: BytesMut::with_capacity(recv_buffer_size),
            #[cfg(target_os = "linux")]
            packet_splitter,
        }
    }
    pub fn read_buffer_size(&self) -> usize {
        self.recv_buffer_size
    }
    /// Sets the size of the read buffer in bytes.
    ///
    /// It is recommended to set this value to the MTU (Maximum Transmission Unit)
    /// to ensure optimal packet reading performance.
    pub fn set_read_buffer_size(&mut self, read_buffer_size: usize) {
        self.recv_buffer_size = read_buffer_size;
        #[cfg(target_os = "linux")]
        if let Some(packet_splitter) = &mut self.packet_splitter {
            packet_splitter.set_recv_buffer_size(read_buffer_size);
        }
    }
    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}
impl<C, T> Unpin for DeviceFramedRead<C, T> {}
impl<C, T> Stream for DeviceFramedRead<C, T>
where
    T: Borrow<AsyncDevice>,
    C: Decoder,
{
    type Item = Result<C::Item, C::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        let mut inner = DeviceFramedReadInner::from_framed_read(pin);
        inner.poll_next(cx)
    }
}
pub struct DeviceFramedWrite<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    send_buffer_size: usize,
    wr: BytesMut,
    #[cfg(target_os = "linux")]
    packet_aggregator: Option<PacketAggregator>,
}
impl<C, T> DeviceFramedWrite<C, T>
where
    T: Borrow<AsyncDevice>,
{
    pub fn new(dev: T, codec: C) -> DeviceFramedWrite<C, T> {
        // The MTU of the network interface cannot be greater than this value.
        let recv_buffer_size = dev.borrow().mtu().map(|m| m as usize).unwrap_or(4096);
        #[cfg(target_os = "linux")]
        let packet_aggregator = if dev.borrow().tcp_gso() {
            Some(PacketAggregator::new())
        } else {
            None
        };

        DeviceFramedWrite {
            dev,
            codec,
            send_buffer_size: recv_buffer_size,
            wr: BytesMut::new(),
            #[cfg(target_os = "linux")]
            packet_aggregator,
        }
    }
    pub fn write_buffer_size(&self) -> usize {
        self.send_buffer_size
    }
    /// Sets the size of the write buffer in bytes.
    ///
    /// On Linux, if GSO (Generic Segmentation Offload) is enabled, this setting is ignored,
    /// and the send buffer size is fixed to a larger value to accommodate large TCP segments.
    ///
    /// If the current buffer size is already greater than or equal to the requested size,
    /// this call has no effect.
    ///
    /// # Parameters
    /// - `write_buffer_size`: Desired size in bytes for the write buffer.
    pub fn set_write_buffer_size(&mut self, write_buffer_size: usize) {
        #[cfg(target_os = "linux")]
        if self.packet_aggregator.is_some() {
            // When GSO is enabled, send_buffer_size is no longer controlled by this parameter.
            return;
        }
        if self.send_buffer_size >= write_buffer_size {
            return;
        }
        self.send_buffer_size = write_buffer_size;
    }
    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}
impl<C, T> Unpin for DeviceFramedWrite<C, T> {}
impl<I, C, T> Sink<I> for DeviceFramedWrite<C, T>
where
    T: Borrow<AsyncDevice>,
    C: Encoder<I>,
{
    type Error = C::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed_write(pin).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed_write(pin).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed_write(pin).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::from_framed_write(pin).poll_close(cx)
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

#[cfg(target_os = "linux")]
struct PacketSplitter {
    bufs: Vec<BytesMut>,
    sizes: Vec<usize>,
    recv_index: usize,
    recv_num: usize,
    recv_buffer_size: usize,
}
#[cfg(target_os = "linux")]
impl PacketSplitter {
    fn new(recv_buffer_size: usize) -> PacketSplitter {
        let bufs = vec![BytesMut::zeroed(recv_buffer_size); IDEAL_BATCH_SIZE];
        let sizes = vec![0usize; IDEAL_BATCH_SIZE];
        Self {
            bufs,
            sizes,
            recv_index: 0,
            recv_num: 0,
            recv_buffer_size,
        }
    }
    fn handle(&mut self, dev: &AsyncDevice, input: &mut [u8]) -> io::Result<()> {
        if input.len() <= VIRTIO_NET_HDR_LEN {
            Err(io::Error::other(format!(
                "length of packet ({}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                input.len(),
            )))?
        }
        for buf in &mut self.bufs {
            buf.resize(self.recv_buffer_size, 0);
        }
        let hdr = VirtioNetHdr::decode(&input[..VIRTIO_NET_HDR_LEN])?;
        let num = dev.handle_virtio_read(
            hdr,
            &mut input[VIRTIO_NET_HDR_LEN..],
            &mut self.bufs,
            &mut self.sizes,
            0,
        )?;

        for i in 0..num {
            self.bufs[i].truncate(self.sizes[i]);
        }
        self.recv_num = num;
        self.recv_index = 0;
        Ok(())
    }
    fn next(&mut self) -> Option<&mut BytesMut> {
        if self.recv_index >= self.recv_num {
            None
        } else {
            let buf = &mut self.bufs[self.recv_index];
            self.recv_index += 1;
            Some(buf)
        }
    }
    fn set_recv_buffer_size(&mut self, recv_buffer_size: usize) {
        self.recv_buffer_size = recv_buffer_size;
    }
}
#[cfg(target_os = "linux")]
struct PacketAggregator {
    gro_table: GROTable,
    offset: usize,
    bufs: Vec<BytesMut>,
    send_index: usize,
}
#[cfg(target_os = "linux")]
impl PacketAggregator {
    fn new() -> PacketAggregator {
        Self {
            gro_table: Default::default(),
            offset: 0,
            bufs: Vec::with_capacity(IDEAL_BATCH_SIZE),
            send_index: 0,
        }
    }
    fn get(&mut self) -> &mut BytesMut {
        if self.offset < self.bufs.len() {
            let buf = &mut self.bufs[self.offset];
            self.offset += 1;
            buf.clear();
            buf.reserve(VIRTIO_NET_HDR_LEN + 65536);
            return buf;
        }
        assert_eq!(self.offset, self.bufs.len());
        self.bufs
            .push(BytesMut::with_capacity(VIRTIO_NET_HDR_LEN + 65536));
        let idx = self.offset;
        self.offset += 1;
        &mut self.bufs[idx]
    }
    fn handle(&mut self, dev: &AsyncDevice) -> io::Result<()> {
        if self.offset == 0 {
            return Ok(());
        }
        if !self.gro_table.to_write.is_empty() {
            return Ok(());
        }
        crate::platform::offload::handle_gro(
            &mut self.bufs[..self.offset],
            VIRTIO_NET_HDR_LEN,
            &mut self.gro_table.tcp_gro_table,
            &mut self.gro_table.udp_gro_table,
            dev.udp_gso,
            &mut self.gro_table.to_write,
        )
    }
    fn poll_send_bufs(&mut self, cx: &mut Context<'_>, dev: &AsyncDevice) -> Poll<io::Result<()>> {
        if self.offset == 0 {
            return Poll::Ready(Ok(()));
        }
        let gro_table = &mut self.gro_table;
        let bufs = &self.bufs[..self.offset];
        for buf_idx in &gro_table.to_write[self.send_index..] {
            let rs = dev.poll_send(cx, &bufs[*buf_idx]);
            match rs {
                Poll::Ready(Ok(_)) => {
                    self.send_index += 1;
                }
                Poll::Ready(Err(e)) => {
                    self.send_index += 1;
                    if self.send_index >= gro_table.to_write.len() {
                        self.reset();
                    }
                    return Poll::Ready(Err(e));
                }
                Poll::Pending => {
                    return Poll::Pending;
                }
            }
        }
        self.reset();
        Poll::Ready(Ok(()))
    }
    fn reset(&mut self) {
        self.gro_table.reset();
        for buf in self.bufs[..self.offset].iter_mut() {
            buf.clear();
        }
        self.offset = 0;
        self.send_index = 0;
    }
    fn has_capacity(&self) -> bool {
        IDEAL_BATCH_SIZE > self.offset && self.gro_table.to_write.is_empty()
    }
}
struct DeviceFramedReadInner<'a, C, T = AsyncDevice> {
    dev: &'a T,
    codec: &'a mut C,
    recv_buffer_size: usize,
    rd: &'a mut BytesMut,
    #[cfg(target_os = "linux")]
    packet_splitter: &'a mut Option<PacketSplitter>,
}
impl<'a, C, T> DeviceFramedReadInner<'a, C, T>
where
    T: Borrow<AsyncDevice>,
    C: Decoder,
{
    fn from_framed(framed: &'a mut DeviceFramed<C, T>) -> DeviceFramedReadInner<'a, C, T> {
        DeviceFramedReadInner {
            dev: &framed.dev,
            codec: &mut framed.codec,
            recv_buffer_size: framed.recv_buffer_size,
            rd: &mut framed.rd,
            #[cfg(target_os = "linux")]
            packet_splitter: &mut framed.packet_splitter,
        }
    }
    fn from_framed_read(framed: &'a mut DeviceFramedRead<C, T>) -> DeviceFramedReadInner<'a, C, T> {
        DeviceFramedReadInner {
            dev: &framed.dev,
            codec: &mut framed.codec,
            recv_buffer_size: framed.recv_buffer_size,
            rd: &mut framed.rd,
            #[cfg(target_os = "linux")]
            packet_splitter: &mut framed.packet_splitter,
        }
    }
    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<C::Item, C::Error>>> {
        #[cfg(target_os = "linux")]
        if let Some(packet_splitter) = &mut self.packet_splitter {
            if let Some(buf) = packet_splitter.next() {
                if let Some(frame) = self.codec.decode_eof(buf)? {
                    return Poll::Ready(Some(Ok(frame)));
                }
            }
        }

        self.rd.clear();
        #[cfg(target_os = "linux")]
        if self.packet_splitter.is_some() {
            self.rd.reserve(VIRTIO_NET_HDR_LEN + 65536);
        }
        self.rd.reserve(self.recv_buffer_size);
        let buf = unsafe { &mut *(self.rd.chunk_mut() as *mut _ as *mut [u8]) };

        let len = ready!(self.dev.borrow().poll_recv(cx, buf))?;
        unsafe { self.rd.advance_mut(len) };

        #[cfg(target_os = "linux")]
        if let Some(packet_splitter) = &mut self.packet_splitter {
            packet_splitter.handle(self.dev.borrow(), self.rd)?;
            if let Some(buf) = packet_splitter.next() {
                if let Some(frame) = self.codec.decode_eof(buf)? {
                    return Poll::Ready(Some(Ok(frame)));
                }
            }
            return Poll::Ready(None);
        }
        if let Some(frame) = self.codec.decode_eof(self.rd)? {
            return Poll::Ready(Some(Ok(frame)));
        }
        Poll::Ready(None)
    }
}
struct DeviceFramedWriteInner<'a, C, T = AsyncDevice> {
    dev: &'a T,
    codec: &'a mut C,
    send_buffer_size: usize,
    wr: &'a mut BytesMut,
    #[cfg(target_os = "linux")]
    packet_aggregator: &'a mut Option<PacketAggregator>,
}
impl<'a, C, T> DeviceFramedWriteInner<'a, C, T>
where
    T: Borrow<AsyncDevice>,
{
    fn from_framed(framed: &'a mut DeviceFramed<C, T>) -> DeviceFramedWriteInner<'a, C, T> {
        DeviceFramedWriteInner {
            dev: &framed.dev,
            codec: &mut framed.codec,
            send_buffer_size: framed.send_buffer_size,
            wr: &mut framed.wr,
            #[cfg(target_os = "linux")]
            packet_aggregator: &mut framed.packet_aggregator,
        }
    }
    fn from_framed_write(
        framed: &'a mut DeviceFramedWrite<C, T>,
    ) -> DeviceFramedWriteInner<'a, C, T> {
        DeviceFramedWriteInner {
            dev: &framed.dev,
            codec: &mut framed.codec,
            send_buffer_size: framed.send_buffer_size,
            wr: &mut framed.wr,
            #[cfg(target_os = "linux")]
            packet_aggregator: &mut framed.packet_aggregator,
        }
    }

    fn poll_ready<I>(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>>
    where
        C: Encoder<I>,
    {
        #[cfg(target_os = "linux")]
        if let Some(packet_aggregator) = &self.packet_aggregator {
            if packet_aggregator.has_capacity() {
                return Poll::Ready(Ok(()));
            }
        }
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    fn start_send<I>(&mut self, item: I) -> Result<(), C::Error>
    where
        C: Encoder<I>,
    {
        #[cfg(target_os = "linux")]
        if let Some(packet_aggregator) = &mut self.packet_aggregator {
            let buf = packet_aggregator.get();
            buf.resize(VIRTIO_NET_HDR_LEN, 0);
            self.codec.encode(item, buf)?;
            return Ok(());
        }
        let buf = &mut self.wr;
        buf.clear();
        buf.reserve(self.send_buffer_size);
        self.codec.encode(item, buf)?;
        Ok(())
    }

    fn poll_flush<I>(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>>
    where
        C: Encoder<I>,
    {
        let dev = self.dev.borrow();

        #[cfg(target_os = "linux")]
        if let Some(packet_aggregator) = &mut self.packet_aggregator {
            packet_aggregator.handle(dev)?;
            ready!(packet_aggregator.poll_send_bufs(cx, dev))?;
            return Poll::Ready(Ok(()));
        }

        // On non-Linux systems or when GSO is disabled on Linux, `wr` will contain at most one element
        if !self.wr.is_empty() {
            let rs = ready!(dev.poll_send(cx, self.wr));
            self.wr.clear();
            rs?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close<I>(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>>
    where
        C: Encoder<I>,
    {
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}
