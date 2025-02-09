use std::borrow::Borrow;
use std::collections::VecDeque;
use std::io;
use std::pin::Pin;
use std::task::{ready, Context, Poll};

use bytes::{BufMut, Bytes, BytesMut};
use futures::Sink;
use futures_core::Stream;

use crate::AsyncDevice;

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

const INITIAL_RD_CAPACITY: usize = 64 * 1024;
const INITIAL_WR_CAPACITY: usize = 8 * 1024;

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
    rd: BytesMut,
    wr: VecDeque<BytesMut>,
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

        pin.rd.reserve(INITIAL_RD_CAPACITY);
        let buf = unsafe { &mut *(pin.rd.chunk_mut() as *mut _ as *mut [u8]) };

        let len = ready!(pin.dev.borrow().poll_recv(cx, buf))?;
        unsafe { pin.rd.advance_mut(len) };
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
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: I) -> Result<(), Self::Error> {
        let pin = self.get_mut();
        let mut buf = BytesMut::with_capacity(INITIAL_WR_CAPACITY);
        pin.codec.encode(item, &mut buf)?;
        pin.wr.push_back(buf);
        Ok(())
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        while let Some(frame) = self.wr.pop_front() {
            ready!(self.dev.borrow().poll_send(cx, &frame))?;
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        ready!(self.poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}

impl<C, T> DeviceFramed<C, T>
where
    T: Borrow<AsyncDevice>,
{
    pub fn new(dev: T, codec: C) -> DeviceFramed<C, T> {
        DeviceFramed {
            dev,
            codec,
            rd: BytesMut::with_capacity(INITIAL_RD_CAPACITY),
            wr: Default::default(),
        }
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
            let len = buf.len();
            Ok(Some(buf.split_to(len)))
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
