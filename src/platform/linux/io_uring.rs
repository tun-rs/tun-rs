use crate::platform::offload::{handle_gro, VirtioNetHdr};
use crate::{DeviceImpl, ExpandBuffer, GROTable, SyncDevice, IDEAL_BATCH_SIZE, VIRTIO_NET_HDR_LEN};
use bytes::BytesMut;
use io_uring::cqueue::buffer_select;
use io_uring::squeue::Flags;
use io_uring::{opcode, types, IoUring};
use io_uring_buf_ring::IoUringBufRing;
use std::io;
use std::ops::Deref;
use std::os::fd::AsRawFd;
use std::sync::Arc;

pub struct IoUringBuilder {
    read_io_uring: IoUring,
    write_io_uring: IoUring,
}
impl IoUringBuilder {
    pub fn new() -> io::Result<IoUringBuilder> {
        let read_io_uring = IoUring::new(IDEAL_BATCH_SIZE as _)?;
        let write_io_uring = IoUring::new(IDEAL_BATCH_SIZE as _)?;
        Ok(Self {
            read_io_uring,
            write_io_uring,
        })
    }
    pub fn build(self, device: SyncDevice) -> io::Result<(SyncIoUringReader, SyncIoUringWriter)> {
        let device = Arc::new(device.0);
        let reader = SyncIoUringReader::new(self.read_io_uring, device.clone());
        let writer = SyncIoUringWriter::new(self.write_io_uring, device.clone());
        Ok((reader, writer))
    }
}
const BUF_GROUP: u16 = 1;
const RING_ENTRIES_SIZE: usize = 1;
pub struct SyncIoUringReader {
    ring: IoUring,
    device: Arc<DeviceImpl>,
    pending_ops: usize,
    buf_ring: IoUringBufRing<BytesMut>,
}
impl SyncIoUringReader {
    fn new(ring: IoUring, device: Arc<DeviceImpl>) -> Self {
        let buf_ring = IoUringBufRing::new_with_buffers(
            &ring,
            vec![BytesMut::with_capacity(VIRTIO_NET_HDR_LEN + 65536); RING_ENTRIES_SIZE],
            BUF_GROUP,
        )
        .unwrap();

        Self {
            ring,
            device,
            pending_ops: 0,
            buf_ring,
        }
    }
    pub fn read_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &mut self,
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        if bufs.is_empty() || bufs.len() != sizes.len() {
            return Err(io::Error::other("bufs error"));
        }
        loop {
            match self.try_read_multiple(bufs, sizes, offset) {
                Ok(n) => {
                    return Ok(n);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => return Err(e),
            };
            self.submit_reads()?;
        }
    }
    fn try_read_multiple<B: AsRef<[u8]> + AsMut<[u8]>>(
        &mut self,
        bufs: &mut [B],
        sizes: &mut [usize],
        offset: usize,
    ) -> io::Result<usize> {
        if self.device.vnet_hdr {
            let (bid, len) = self.completion_next()?;
            let mut original_buffer = unsafe {
                self.buf_ring
                    .get_buf(bid, len)
                    .ok_or(io::Error::new(io::ErrorKind::NotFound, "not found bid"))?
            };
            if len <= VIRTIO_NET_HDR_LEN {
                Err(io::Error::other(format!(
                    "length of packet ({len}) <= VIRTIO_NET_HDR_LEN ({VIRTIO_NET_HDR_LEN})",
                )))?
            }
            let hdr = VirtioNetHdr::decode(&original_buffer[..VIRTIO_NET_HDR_LEN])?;
            self.device.handle_virtio_read(
                hdr,
                &mut original_buffer[VIRTIO_NET_HDR_LEN..len],
                bufs,
                sizes,
                offset,
            )
        } else {
            let len = self.try_read(&mut bufs[0].as_mut()[offset..])?;
            sizes[0] = len;
            Ok(1)
        }
    }
    fn try_read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let (bid, len) = self.completion_next()?;
        unsafe {
            if let Some(buffer) = self.buf_ring.get_buf(bid, len) {
                if buf.len() < len {
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "too short"));
                }
                buf[..len].copy_from_slice(&buffer);
                Ok(len)
            } else {
                Err(io::Error::new(io::ErrorKind::NotFound, "not found bid"))
            }
        }
    }
    fn completion_next(&mut self) -> io::Result<(u16, usize)> {
        if let Some(entity) = self.ring.completion().next() {
            self.pending_ops -= 1;
            if entity.result() >= 0 {
                let bid = buffer_select(entity.flags()).unwrap();
                return Ok((bid, entity.result() as usize));
            }
            return Err(io::Error::from_raw_os_error(-entity.result()));
        }
        Err(io::Error::from(io::ErrorKind::WouldBlock))
    }
    fn submit_reads(&mut self) -> io::Result<()> {
        if self.pending_ops > 0 {
            return Ok(());
        }
        let mut submission_queue = self.ring.submission();
        for i in 0..RING_ENTRIES_SIZE {
            let read_e =
                opcode::Read::new(types::Fd(self.device.as_raw_fd()), std::ptr::null_mut(), 0)
                    .buf_group(BUF_GROUP)
                    .offset(i as _)
                    .build()
                    .user_data(i as _)
                    .flags(Flags::BUFFER_SELECT);
            unsafe {
                // SAFETY: `self.original_buffers.len()` is calculated to ensure we never exceed the submission queue's
                // available capacity, so `push()` is guaranteed to succeed. Therefore, `unwrap()` is safe here.
                submission_queue.push(&read_e).unwrap();
            }
            self.pending_ops += 1;
        }
        drop(submission_queue);
        self.ring.submit_and_wait(RING_ENTRIES_SIZE)?;
        Ok(())
    }
    fn submit_and_wait(&mut self) -> io::Result<()> {
        self.ring.submit_and_wait(1)?;
        Ok(())
    }
}

pub struct SyncIoUringWriter {
    ring: IoUring,
    device: Arc<DeviceImpl>,
    gro_table: GROTable,
    pending_ops: usize,
    send_buffers: Vec<BytesMut>,
}
impl Deref for SyncIoUringWriter {
    type Target = DeviceImpl;

    fn deref(&self) -> &Self::Target {
        &self.device
    }
}
impl SyncIoUringWriter {
    fn new(ring: IoUring, device: Arc<DeviceImpl>) -> Self {
        let send_buffers =
            vec![BytesMut::with_capacity(VIRTIO_NET_HDR_LEN + 65536); IDEAL_BATCH_SIZE];
        Self {
            ring,
            device,
            gro_table: Default::default(),
            pending_ops: 0,
            send_buffers,
        }
    }
    pub fn write_multiple<B: ExpandBuffer>(
        &mut self,
        bufs: &mut [B],
        mut offset: usize,
    ) -> io::Result<usize> {
        self.wait_completed()?;
        let gro_table = &mut self.gro_table;
        gro_table.reset();
        if self.device.vnet_hdr {
            handle_gro(
                bufs,
                offset,
                &mut gro_table.tcp_gro_table,
                &mut gro_table.udp_gro_table,
                self.device.udp_gso,
                &mut gro_table.to_write,
            )?;
            offset -= VIRTIO_NET_HDR_LEN;
        } else {
            for i in 0..bufs.len() {
                gro_table.to_write.push(i);
            }
        }

        for buf_idx in &gro_table.to_write {
            if self.ring.submission().is_full() {
                Self::wait_completed0(&mut self.pending_ops, &mut self.ring)?;
            }
            let mut submission_queue = self.ring.submission();
            let bytes_mut = &mut self.send_buffers[*buf_idx];
            bytes_mut.clear();
            bytes_mut.extend_from_slice(&bufs[*buf_idx].as_ref()[offset..]);

            let write_e = opcode::Write::new(
                types::Fd(self.device.as_raw_fd()),
                bytes_mut.as_ptr(),
                bytes_mut.len() as _,
            )
            .build()
            .user_data(*buf_idx as _);
            unsafe {
                // queue is not full, so `push()` is guaranteed to succeed. Therefore, `unwrap()` is safe here.
                submission_queue.push(&write_e).unwrap();
                self.pending_ops += 1;
            }
        }
        self.wait_completed()
    }

    fn wait_completed(&mut self) -> io::Result<usize> {
        Self::wait_completed0(&mut self.pending_ops, &mut self.ring)
    }
    fn wait_completed0(pending_ops: &mut usize, ring: &mut IoUring) -> io::Result<usize> {
        if *pending_ops == 0 {
            return Ok(0);
        }
        ring.submit_and_wait(*pending_ops as _)?;
        let mut err = 0;
        let mut total = 0;
        while let Some(entry) = ring.completion().next() {
            *pending_ops -= 1;
            if entry.result() >= 0 {
                total += 1;
            } else {
                err = entry.result();
            }
        }
        if err < 0 {
            return Err(io::Error::from_raw_os_error(err));
        }
        Ok(total)
    }
}
