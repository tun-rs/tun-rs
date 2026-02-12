use std::borrow::Borrow;
use std::io;
use std::ops::Deref;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use bytes::{BufMut, Bytes, BytesMut};
use futures::Sink;
use futures_core::Stream;

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use crate::platform::offload::VirtioNetHdr;
use crate::platform::DeviceImpl;
use crate::AsyncDevice;
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
use crate::{GROTable, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};

/// Trait for async devices that can be used with framed I/O.
///
/// This trait abstracts the common interface needed by [`DeviceFramed`], [`DeviceFramedRead`],
/// and [`DeviceFramedWrite`]. Both [`TokioAsyncDevice`](crate::TokioAsyncDevice) and 
/// [`AsyncIoDevice`](crate::AsyncIoDevice) implement this trait.
pub trait AsyncFramedDevice: Deref<Target = DeviceImpl> {
    /// Polls the device for readability and attempts to receive data.
    fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>>;
    
    /// Polls the device for writability and attempts to send data.
    fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>>;
}

// Implement for TokioAsyncDevice (which might be aliased as AsyncDevice)
#[cfg(feature = "async_tokio")]
impl AsyncFramedDevice for crate::TokioAsyncDevice {
    fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.poll_recv(cx, buf)
    }
    
    fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.poll_send(cx, buf)
    }
}

// Implement for AsyncIoDevice
#[cfg(feature = "async_io")]
impl AsyncFramedDevice for crate::AsyncIoDevice {
    fn poll_recv(&self, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
        self.poll_recv(cx, buf)
    }
    
    fn poll_send(&self, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        self.poll_send(cx, buf)
    }
}

pub trait Decoder {
    /// The type of decoded frames.
    type Item;

    /// The type of unrecoverable frame decoding errors.
    type Error: From<io::Error>;

    /// Attempts to decode a frame from the provided buffer.
    ///
    /// Returns `Ok(Some(frame))` if a complete frame was decoded,
    /// `Ok(None)` if more data is needed, or `Err` on decoding errors.
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error>;

    /// Decodes a frame from the buffer when the stream has ended.
    ///
    /// This method is called when the underlying stream reaches EOF. The default
    /// implementation attempts a normal decode and returns an error if data remains
    /// in the buffer, indicating incomplete frames.
    ///
    /// Override this method if your decoder needs special handling for the end of stream,
    /// such as flushing partial frames or performing cleanup.
    ///
    /// # Arguments
    ///
    /// * `buf` - Buffer containing any remaining data
    ///
    /// # Returns
    ///
    /// - `Ok(Some(frame))` - Successfully decoded a final frame
    /// - `Ok(None)` - No more frames and buffer is empty (normal EOF)
    /// - `Err` - Incomplete data remains or decoding error
    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        match self.decode(buf)? {
            Some(frame) => Ok(Some(frame)),
            None => {
                if buf.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::other("bytes remaining on stream").into())
                }
            }
        }
    }
}

impl<T: Decoder> Decoder for &mut T {
    type Item = T::Item;
    type Error = T::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        T::decode(self, src)
    }
}

pub trait Encoder<Item> {
    /// The type of encoding errors.
    type Error: From<io::Error>;

    /// Encodes a frame into the buffer provided.
    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error>;
}

impl<T: Encoder<Item>, Item> Encoder<Item> for &mut T {
    type Error = T::Error;

    fn encode(&mut self, item: Item, dst: &mut BytesMut) -> Result<(), Self::Error> {
        T::encode(self, item, dst)
    }
}

/// A unified `Stream` and `Sink` interface over an async device,
/// using `Encoder` and `Decoder` traits to frame packets as higher-level messages.
///
/// This struct works with any device type that implements [`AsyncFramedDevice`], including:
/// - [`AsyncDevice`] (type alias, default)
/// - [`TokioAsyncDevice`](crate::TokioAsyncDevice) (when `async_tokio` feature is enabled)
/// - [`AsyncIoDevice`](crate::AsyncIoDevice) (when `async_io` feature is enabled)
///
/// Raw device interfaces (such as TUN/TAP) operate on individual packets,
/// but higher-level protocols often work with logical frames. This struct
/// provides an abstraction layer that decodes incoming packets into frames,
/// and encodes outgoing frames into packet buffers.
///
/// On Linux, this struct also supports Generic Segmentation Offload (GSO) for sending
/// and Generic Receive Offload (GRO) for receiving, allowing multiple small packets
/// to be aggregated or split transparently for performance optimization.
///
/// This struct combines both reading and writing into a single object. If separate
/// control over read/write is needed, consider calling `.split()` to obtain
/// [`DeviceFramedRead`] and [`DeviceFramedWrite`] separately.
///
/// # Type Parameter
///
/// - `C`: The codec that implements [`Encoder`] and/or [`Decoder`]
/// - `T`: The device type (defaults to [`AsyncDevice`]), must implement [`AsyncFramedDevice`]
///
/// # Examples
///
/// Using with the default [`AsyncDevice`]:
///
/// ```no_run
/// use tun_rs::{AsyncDevice, DeviceBuilder};
/// # #[cfg(feature = "async_framed")]
/// use tun_rs::async_framed::DeviceFramed;
/// use bytes::BytesMut;
///
/// # #[cfg(all(feature = "async_tokio", feature = "async_framed"))]
/// # #[tokio::main]
/// # async fn main() -> std::io::Result<()> {
/// // Create a device
/// let dev = DeviceBuilder::new()
///     .ipv4("10.0.0.1", 24, None)
///     .build_async()?;
///
/// // Define a simple codec (implement Encoder and Decoder traits)
/// struct SimpleCodec;
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "async_tokio", feature = "async_framed")))]
/// # fn main() {}
/// ```
///
/// Using with explicit runtime types:
///
/// ```no_run
/// # #[cfg(all(feature = "async_tokio", feature = "async_framed"))]
/// use tun_rs::{TokioAsyncDevice, DeviceBuilder};
/// # #[cfg(all(feature = "async_tokio", feature = "async_framed"))]
/// use tun_rs::async_framed::DeviceFramed;
///
/// # #[cfg(all(feature = "async_tokio", feature = "async_framed"))]
/// # #[tokio::main]
/// # async fn main() -> std::io::Result<()> {
/// // Explicitly use Tokio device
/// let dev: TokioAsyncDevice = DeviceBuilder::new()
///     .ipv4("10.0.0.1", 24, None)
///     .build_tokio_async()?;
///
/// // Use with framed I/O
/// // let framed = DeviceFramed::new(dev, codec);
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "async_tokio", feature = "async_framed")))]
/// # fn main() {}
/// ```
///
/// You can also create multiple independent framing streams using:
/// `DeviceFramed::new(dev.clone(), BytesCodec::new())`, with the device wrapped
/// in `Arc<AsyncDevice>`.
///
/// A unified async read/write interface for TUN/TAP devices using framed I/O
///
/// Combines an async device with a codec to provide `Stream` and `Sink` implementations
/// for reading and writing framed packets.
///
/// # Examples
///
/// ## Basic usage with BytesCodec
///
/// ```no_run
/// use bytes::BytesMut;
/// use futures::{SinkExt, StreamExt};
/// use tun_rs::async_framed::{BytesCodec, DeviceFramed};
/// use tun_rs::DeviceBuilder;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     // Create a TUN device with IPv4 configuration
///     let dev = DeviceBuilder::new()
///         .name("tun0")
///         .mtu(1500)
///         .ipv4("10.0.0.1", "255.255.255.0", None)
///         .build_async()?;
///
///     // Create a framed device with BytesCodec
///     let mut framed = DeviceFramed::new(dev, BytesCodec::new());
///
///     // Send a frame (Replace with real IP message)
///     let packet = b"[IP Packet: 10.0.0.1 -> 10.0.0.2] Hello, TUN!";
///     framed.send(BytesMut::from(packet)).await?;
///
///     // Receive frames
///     while let Some(frame) = framed.next().await {
///         match frame {
///             Ok(bytes) => println!("Received: {:?}", bytes),
///             Err(e) => eprintln!("Error receiving frame: {}", e),
///         }
///     }
///     Ok(())
/// }
/// ```
pub struct DeviceFramed<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    r_state: ReadState,
    w_state: WriteState,
}
impl<C, T> Unpin for DeviceFramed<C, T> {}
impl<C, T> Stream for DeviceFramed<C, T>
where
    T: AsyncFramedDevice,
    C: Decoder,
{
    type Item = Result<C::Item, C::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        DeviceFramedReadInner::new(&pin.dev, &mut pin.codec, &mut pin.r_state).poll_next(cx)
    }
}
impl<I, C, T> Sink<I> for DeviceFramed<C, T>
where
    T: AsyncFramedDevice,
    C: Encoder<I>,
{
    type Error = C::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.w_state).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.w_state).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.w_state).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.w_state).poll_close(cx)
    }
}
impl<C, T> DeviceFramed<C, T>
where
    T: AsyncFramedDevice,
{
    /// Construct from a [`AsyncDevice`] with a specific codec
    pub fn new(dev: T, codec: C) -> DeviceFramed<C, T> {
        let buffer_size = compute_buffer_size(&dev);
        DeviceFramed {
            r_state: ReadState::new(buffer_size, dev.borrow()),
            w_state: WriteState::new(buffer_size, dev.borrow()),
            dev,
            codec,
        }
    }

    /// Returns the size of the read buffer in bytes.
    ///
    /// This indicates how much space is available for receiving packet data.
    pub fn read_buffer_size(&self) -> usize {
        self.r_state.read_buffer_size()
    }

    /// Returns the size of the write buffer in bytes.
    ///
    /// This indicates how much space is available for buffering outbound packets.
    pub fn write_buffer_size(&self) -> usize {
        self.w_state.write_buffer_size()
    }

    /// Sets the size of the read buffer in bytes.
    ///
    /// Must be at least as large as the MTU to ensure complete packet reception.
    pub fn set_read_buffer_size(&mut self, read_buffer_size: usize) {
        self.r_state.set_read_buffer_size(read_buffer_size);
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
        self.w_state.set_write_buffer_size(write_buffer_size);
    }
    /// Returns a reference to the read buffer.
    pub fn read_buffer(&self) -> &BytesMut {
        &self.r_state.rd
    }

    /// Returns a mutable reference to the read buffer.
    ///
    /// This allows direct manipulation of the buffer contents, which can be useful
    /// for advanced use cases or optimization.
    pub fn read_buffer_mut(&mut self) -> &mut BytesMut {
        &mut self.r_state.rd
    }
    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}

impl<C, T> DeviceFramed<C, T>
where
    T: AsyncFramedDevice + Clone,
    C: Clone,
{
    /// Split the framed device to read-half and write-half
    ///
    /// # Example
    /// ```
    /// use std::net::Ipv4Addr;
    /// use std::sync::Arc;
    /// use tun_rs::{
    ///     async_framed::{BytesCodec, DeviceFramed},
    ///     DeviceBuilder,
    /// };
    /// let dev = Arc::new(
    ///     DeviceBuilder::new()
    ///         .ipv4(Ipv4Addr::new(10, 0, 0, 21), 24, None)
    ///         .build_async()?,
    /// );
    /// let (r, w) = DeviceFramed::new(dev, BytesCodec::new()).split();
    /// ```
    pub fn split(self) -> (DeviceFramedRead<C, T>, DeviceFramedWrite<C, T>) {
        let dev = self.dev;
        let codec = self.codec;
        (
            DeviceFramedRead::new(dev.clone(), codec.clone()),
            DeviceFramedWrite::new(dev, codec),
        )
    }
}

/// A `Stream`-only abstraction over an `AsyncDevice` that extracts frames from raw packet input.
///
/// `DeviceFramedRead` provides a read-only framing interface for the underlying device,
/// using a `Decoder` to parse incoming packets into structured frames. This is useful
/// when reading and writing logic need to be handled independently, such as in split
/// or concurrent tasks.
///
/// Internally, it maintains a receive buffer and optional packet processing
/// for GRO (Generic Receive Offload) support on Linux with offload enabled.
///
/// # Examples
///
/// ```no_run
/// use futures::StreamExt;
/// use tun_rs::async_framed::{BytesCodec, DeviceFramedRead};
/// use tun_rs::DeviceBuilder;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     // Create a TUN device with IPv4 configuration
///     let dev = DeviceBuilder::new()
///         .name("tun0")
///         .mtu(1500)
///         .ipv4("10.0.0.1", "255.255.255.0", None)
///         .build_async()?;
///
///     // Create a read-only framed device
///     let mut framed_read = DeviceFramedRead::new(dev, BytesCodec::new());
///
///     // Receive frames
///     while let Some(frame) = framed_read.next().await {
///         match frame {
///             Ok(bytes) => println!("Received: {:?}", bytes),
///             Err(e) => eprintln!("Error receiving frame: {}", e),
///         }
///     }
///     Ok(())
/// }
/// ```
///
/// See [`DeviceFramed`] for a unified read/write interface.
pub struct DeviceFramedRead<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    state: ReadState,
}
impl<C, T> DeviceFramedRead<C, T>
where
    T: AsyncFramedDevice,
{
    /// Construct from a [`AsyncDevice`] with a specific codec.
    ///
    /// The read side of the framed device.
    /// # Example
    /// ```
    /// use std::net::Ipv4Addr;
    /// use std::sync::Arc;
    /// use tun_rs::{
    ///     async_framed::{BytesCodec, DeviceFramedRead, DeviceFramedWrite},
    ///     DeviceBuilder,
    /// };
    /// let dev = Arc::new(
    ///     DeviceBuilder::new()
    ///         .ipv4(Ipv4Addr::new(10, 0, 0, 21), 24, None)
    ///         .build_async()?,
    /// );
    /// let mut w = DeviceFramedWrite::new(dev.clone(), BytesCodec::new());
    /// let mut r = DeviceFramedRead::new(dev, BytesCodec::new());
    /// ```
    /// # Note
    /// An efficient way is to directly use [`DeviceFramed::split`] if the device is cloneable
    pub fn new(dev: T, codec: C) -> DeviceFramedRead<C, T> {
        let buffer_size = compute_buffer_size(&dev);
        DeviceFramedRead {
            state: ReadState::new(buffer_size, dev.borrow()),
            dev,
            codec,
        }
    }

    /// Returns the size of the read buffer in bytes.
    ///
    /// This indicates how much space is available for receiving packet data.
    pub fn read_buffer_size(&self) -> usize {
        self.state.read_buffer_size()
    }
    /// Sets the size of the read buffer in bytes.
    ///
    /// Must be at least as large as the MTU to ensure complete packet reception.
    pub fn set_read_buffer_size(&mut self, read_buffer_size: usize) {
        self.state.set_read_buffer_size(read_buffer_size);
    }
    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}
impl<C, T> Unpin for DeviceFramedRead<C, T> {}
impl<C, T> Stream for DeviceFramedRead<C, T>
where
    T: AsyncFramedDevice,
    C: Decoder,
{
    type Item = Result<C::Item, C::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let pin = self.get_mut();
        DeviceFramedReadInner::new(&pin.dev, &mut pin.codec, &mut pin.state).poll_next(cx)
    }
}

/// A `Sink`-only abstraction over an `AsyncDevice` that serializes outbound frames into raw packets.
///
/// `DeviceFramedWrite` provides a write-only framing interface for the underlying device,
/// using an `Encoder` to convert structured frames into raw packet bytes. This allows
/// decoupled and concurrent handling of outbound data, which is especially useful in
/// async contexts where reads and writes occur in different tasks.
///
/// Internally, it manages a send buffer and optional packet processing
/// for GSO (Generic Segmentation Offload) support on Linux with offload enabled.
///
/// See [`DeviceFramed`] for a unified read/write interface.
///
/// # Examples
///
/// ```no_run
/// use bytes::BytesMut;
/// use futures::SinkExt;
/// use tun_rs::async_framed::{BytesCodec, DeviceFramedWrite};
/// use tun_rs::DeviceBuilder;
///
/// #[tokio::main]
/// async fn main() -> std::io::Result<()> {
///     // Create a TUN device with IPv4 configuration
///     let dev = DeviceBuilder::new()
///         .name("tun0")
///         .mtu(1500)
///         .ipv4("10.0.0.1", "255.255.255.0", None)
///         .build_async()?;
///
///     // Create a write-only framed device
///     let mut framed_write = DeviceFramedWrite::new(dev, BytesCodec::new());
///
///     // Send a frame (Replace with real IP message)
///     let packet = b"[IP Packet: 10.0.0.1 -> 10.0.0.2] Hello, TUN!";
///     framed_write.send(BytesMut::from(packet)).await?;
///
///     Ok(())
/// }
/// ```
pub struct DeviceFramedWrite<C, T = AsyncDevice> {
    dev: T,
    codec: C,
    state: WriteState,
}
impl<C, T> DeviceFramedWrite<C, T>
where
    T: AsyncFramedDevice,
{
    /// Construct from a [`AsyncDevice`] with a specific codec.
    ///
    /// The write side of the framed device.
    /// # Example
    /// ```
    /// use std::net::Ipv4Addr;
    /// use std::sync::Arc;
    /// use tun_rs::{
    ///     async_framed::{BytesCodec, DeviceFramedRead, DeviceFramedWrite},
    ///     DeviceBuilder,
    /// };
    /// let dev = Arc::new(
    ///     DeviceBuilder::new()
    ///         .ipv4(Ipv4Addr::new(10, 0, 0, 21), 24, None)
    ///         .build_async()?,
    /// );
    /// let mut w = DeviceFramedWrite::new(dev.clone(), BytesCodec::new());
    /// let mut r = DeviceFramedRead::new(dev, BytesCodec::new());
    /// ```
    /// # Note
    /// An efficient way is to directly use [`DeviceFramed::split`] if the device is cloneable
    pub fn new(dev: T, codec: C) -> DeviceFramedWrite<C, T> {
        let buffer_size = compute_buffer_size(&dev);
        DeviceFramedWrite {
            state: WriteState::new(buffer_size, dev.borrow()),
            dev,
            codec,
        }
    }

    /// Returns the size of the write buffer in bytes.
    ///
    /// This indicates how much space is available for buffering outbound packets.
    pub fn write_buffer_size(&self) -> usize {
        self.state.send_buffer_size
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
        self.state.set_write_buffer_size(write_buffer_size);
    }

    /// Consumes the `Framed`, returning its underlying I/O stream.
    pub fn into_inner(self) -> T {
        self.dev
    }
}

impl<C, T> Unpin for DeviceFramedWrite<C, T> {}
impl<I, C, T> Sink<I> for DeviceFramedWrite<C, T>
where
    T: AsyncFramedDevice,
    C: Encoder<I>,
{
    type Error = C::Error;

    fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.state).poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.state).start_send(item)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.state).poll_flush(cx)
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let pin = self.get_mut();
        DeviceFramedWriteInner::new(&pin.dev, &mut pin.codec, &mut pin.state).poll_close(cx)
    }
}
fn compute_buffer_size<T: AsyncFramedDevice>(_dev: &T) -> usize {
    #[cfg(any(
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
    ))]
    let mtu = _dev.borrow().mtu().map(|m| m as usize).unwrap_or(4096);

    #[cfg(not(any(
        target_os = "windows",
        all(target_os = "linux", not(target_env = "ohos")),
        target_os = "macos",
        target_os = "freebsd",
        target_os = "openbsd",
    )))]
    let mtu = 4096usize;

    #[cfg(windows)]
    {
        let mtu_v6 = _dev.borrow().mtu_v6().map(|m| m as usize).unwrap_or(4096);
        mtu.max(mtu_v6)
    }
    #[cfg(not(windows))]
    mtu
}
struct ReadState {
    recv_buffer_size: usize,
    rd: BytesMut,
    #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
    packet_splitter: Option<PacketSplitter>,
}
impl ReadState {
    pub(crate) fn new<D: AsyncFramedDevice>(recv_buffer_size: usize, _device: &D) -> ReadState {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        let packet_splitter = if _device.tcp_gso() {
            Some(PacketSplitter::new(recv_buffer_size))
        } else {
            None
        };

        ReadState {
            recv_buffer_size,
            rd: BytesMut::with_capacity(recv_buffer_size),
            #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
            packet_splitter,
        }
    }

    pub(crate) fn read_buffer_size(&self) -> usize {
        self.recv_buffer_size
    }

    pub(crate) fn set_read_buffer_size(&mut self, read_buffer_size: usize) {
        self.recv_buffer_size = read_buffer_size;
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_splitter) = &mut self.packet_splitter {
            packet_splitter.set_recv_buffer_size(read_buffer_size);
        }
    }
}
struct WriteState {
    send_buffer_size: usize,
    wr: BytesMut,
    #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
    packet_arena: Option<PacketArena>,
}
impl WriteState {
    pub(crate) fn new<D: AsyncFramedDevice>(send_buffer_size: usize, _device: &D) -> WriteState {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        let packet_arena = if _device.tcp_gso() {
            Some(PacketArena::new())
        } else {
            None
        };

        WriteState {
            send_buffer_size,
            wr: BytesMut::new(),
            #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
            packet_arena,
        }
    }
    pub(crate) fn write_buffer_size(&self) -> usize {
        self.send_buffer_size
    }

    pub(crate) fn set_write_buffer_size(&mut self, write_buffer_size: usize) {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if self.packet_arena.is_some() {
            // When GSO is enabled, send_buffer_size is no longer controlled by this parameter.
            return;
        }
        if self.send_buffer_size >= write_buffer_size {
            return;
        }
        self.send_buffer_size = write_buffer_size;
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
            // Use split_to to efficiently transfer ownership without copying
            Ok(Some(buf.split_to(buf.len())))
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

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
struct PacketSplitter {
    bufs: Vec<BytesMut>,
    sizes: Vec<usize>,
    recv_index: usize,
    recv_num: usize,
    recv_buffer_size: usize,
}
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
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
    fn handle<D: AsyncFramedDevice>(&mut self, dev: &D, input: &mut [u8]) -> io::Result<()> {
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
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
struct PacketArena {
    gro_table: GROTable,
    offset: usize,
    bufs: Vec<BytesMut>,
    send_index: usize,
}
#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
impl PacketArena {
    fn new() -> PacketArena {
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
    fn handle<D: AsyncFramedDevice>(&mut self, dev: &D) -> io::Result<()> {
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
    fn poll_send_bufs<D: AsyncFramedDevice>(&mut self, cx: &mut Context<'_>, dev: &D) -> Poll<io::Result<()>> {
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
    fn is_idle(&self) -> bool {
        IDEAL_BATCH_SIZE > self.offset && self.gro_table.to_write.is_empty()
    }
}
struct DeviceFramedReadInner<'a, C, T = AsyncDevice> {
    dev: &'a T,
    codec: &'a mut C,
    state: &'a mut ReadState,
}
impl<'a, C, T> DeviceFramedReadInner<'a, C, T>
where
    T: AsyncFramedDevice,
    C: Decoder,
{
    fn new(
        dev: &'a T,
        codec: &'a mut C,
        state: &'a mut ReadState,
    ) -> DeviceFramedReadInner<'a, C, T> {
        DeviceFramedReadInner { dev, codec, state }
    }

    fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<C::Item, C::Error>>> {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_splitter) = &mut self.state.packet_splitter {
            if let Some(buf) = packet_splitter.next() {
                if let Some(frame) = self.codec.decode_eof(buf)? {
                    return Poll::Ready(Some(Ok(frame)));
                }
            }
        }

        self.state.rd.clear();
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if self.state.packet_splitter.is_some() {
            self.state.rd.reserve(VIRTIO_NET_HDR_LEN + 65536);
        }
        self.state.rd.reserve(self.state.recv_buffer_size);
        let buf = unsafe { &mut *(self.state.rd.chunk_mut() as *mut _ as *mut [u8]) };

        let len = ready!(self.dev.poll_recv(cx, buf))?;
        unsafe { self.state.rd.advance_mut(len) };

        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_splitter) = &mut self.state.packet_splitter {
            packet_splitter.handle(self.dev, &mut self.state.rd)?;
            if let Some(buf) = packet_splitter.next() {
                if let Some(frame) = self.codec.decode_eof(buf)? {
                    return Poll::Ready(Some(Ok(frame)));
                }
            }
            return Poll::Ready(None);
        }
        if let Some(frame) = self.codec.decode_eof(&mut self.state.rd)? {
            return Poll::Ready(Some(Ok(frame)));
        }
        Poll::Ready(None)
    }
}
struct DeviceFramedWriteInner<'a, C, T = AsyncDevice> {
    dev: &'a T,
    codec: &'a mut C,
    state: &'a mut WriteState,
}
impl<'a, C, T> DeviceFramedWriteInner<'a, C, T>
where
    T: AsyncFramedDevice,
{
    fn new(
        dev: &'a T,
        codec: &'a mut C,
        state: &'a mut WriteState,
    ) -> DeviceFramedWriteInner<'a, C, T> {
        DeviceFramedWriteInner { dev, codec, state }
    }

    fn poll_ready<I>(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>>
    where
        C: Encoder<I>,
    {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_arena) = &self.state.packet_arena {
            if packet_arena.is_idle() {
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
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_arena) = &mut self.state.packet_arena {
            let buf = packet_arena.get();
            buf.resize(VIRTIO_NET_HDR_LEN, 0);
            self.codec.encode(item, buf)?;
            return Ok(());
        }
        let buf = &mut self.state.wr;
        buf.clear();
        buf.reserve(self.state.send_buffer_size);
        self.codec.encode(item, buf)?;
        Ok(())
    }

    fn poll_flush<I>(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), C::Error>>
    where
        C: Encoder<I>,
    {
        #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
        if let Some(packet_arena) = &mut self.state.packet_arena {
            packet_arena.handle(self.dev)?;
            ready!(packet_arena.poll_send_bufs(cx, self.dev))?;
            return Poll::Ready(Ok(()));
        }

        // On non-Linux systems or when GSO is disabled on Linux, `wr` will contain at most one element
        if !self.state.wr.is_empty() {
            let rs = ready!(self.dev.poll_send(cx, &self.state.wr));
            self.state.wr.clear();
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
