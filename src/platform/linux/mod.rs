mod sys;

mod checksum;
mod device;
#[cfg(feature = "io_uring")]
pub mod io_uring;
pub(crate) mod offload;

#[cfg(feature = "io_uring")]
pub use io_uring::*;

pub use device::DeviceImpl;
pub use offload::ExpandBuffer;
pub use offload::GROTable;
pub use offload::IDEAL_BATCH_SIZE;
pub use offload::VIRTIO_NET_HDR_LEN;
