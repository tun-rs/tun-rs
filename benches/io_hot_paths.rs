use bytes::{Bytes, BytesMut};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use tun_rs::async_framed::{BytesCodec, Decoder, Encoder};

fn bench_framed_codec(c: &mut Criterion) {
    let payload = Bytes::from(vec![0x42; 1500]);

    c.bench_function("framed_bytes_codec_encode_decode_1500", |b| {
        b.iter(|| {
            let mut codec = BytesCodec::new();
            let mut buf = BytesMut::with_capacity(black_box(payload.len()));
            codec.encode(black_box(payload.clone()), &mut buf).unwrap();
            let frame = codec.decode_eof(&mut buf).unwrap().unwrap();
            black_box(frame);
        })
    });
}

#[cfg(all(target_os = "linux", not(target_env = "ohos")))]
mod linux_offload {
    use super::*;
    use criterion::BatchSize;
    use tun_rs::{
        checksum, checksum_no_fold, gso_split, GROTable, VirtioNetHdr, VIRTIO_NET_HDR_GSO_TCPV4,
        VIRTIO_NET_HDR_LEN,
    };

    const IPH_LEN: usize = 20;
    const TCPH_LEN: usize = 20;

    fn pseudo_header_sum(src: &[u8], dst: &[u8], protocol: u8, total_len: u16) -> u64 {
        let sum = checksum_no_fold(src, 0);
        let sum = checksum_no_fold(dst, sum);
        let len = total_len.to_be_bytes();
        checksum_no_fold(&[0, protocol, len[0], len[1]], sum)
    }

    fn make_ipv4_tcp_packet(seq: u32, payload_len: u16) -> Vec<u8> {
        make_ipv4_tcp_packet_for_flow(seq, payload_len, 0)
    }

    fn make_ipv4_tcp_packet_for_flow(seq: u32, payload_len: u16, flow_id: u8) -> Vec<u8> {
        const IPH_LEN_U16: u16 = 20;
        const TCPH_LEN_U16: u16 = 20;

        let total_len_u16 = IPH_LEN_U16 + TCPH_LEN_U16 + payload_len;
        let total_len = usize::from(total_len_u16);
        let mut pkt = vec![0u8; total_len];

        pkt[0] = 0x45;
        pkt[2..4].copy_from_slice(&total_len_u16.to_be_bytes());
        pkt[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        pkt[6] = 0x40;
        pkt[8] = 64;
        pkt[9] = 6;
        pkt[12..16].copy_from_slice(&[10, 0, 0, 1]);
        pkt[16..20].copy_from_slice(&[10, 0, 0, 2]);

        let source_port = 10_000u16 + u16::from(flow_id);
        pkt[IPH_LEN..IPH_LEN + 2].copy_from_slice(&source_port.to_be_bytes());
        pkt[IPH_LEN + 2..IPH_LEN + 4].copy_from_slice(&10001u16.to_be_bytes());
        pkt[IPH_LEN + 4..IPH_LEN + 8].copy_from_slice(&seq.to_be_bytes());
        pkt[IPH_LEN + 8..IPH_LEN + 12].copy_from_slice(&1u32.to_be_bytes());
        pkt[IPH_LEN + 12] = 5 << 4;
        pkt[IPH_LEN + 13] = 0x10;
        pkt[IPH_LEN + 14..IPH_LEN + 16].copy_from_slice(&4096u16.to_be_bytes());

        for (byte, value) in pkt[IPH_LEN + TCPH_LEN..]
            .iter_mut()
            .zip((0u8..=u8::MAX).cycle())
        {
            *byte = value;
        }

        let ip_checksum = !checksum(&pkt[..IPH_LEN], 0);
        pkt[10..12].copy_from_slice(&ip_checksum.to_be_bytes());

        let pseudo = pseudo_header_sum(&pkt[12..16], &pkt[16..20], 6, TCPH_LEN_U16 + payload_len);
        let tcp_checksum = !checksum(&pkt[IPH_LEN..], pseudo);
        pkt[IPH_LEN + 16..IPH_LEN + 18].copy_from_slice(&tcp_checksum.to_be_bytes());

        pkt
    }

    fn make_gro_buffer(seq: u32, payload_len: u16) -> Vec<u8> {
        make_gro_buffer_for_flow(seq, payload_len, 0)
    }

    fn make_gro_buffer_for_flow(seq: u32, payload_len: u16, flow_id: u8) -> Vec<u8> {
        let packet = make_ipv4_tcp_packet_for_flow(seq, payload_len, flow_id);
        let mut buf = Vec::with_capacity(VIRTIO_NET_HDR_LEN + 65536);
        buf.resize(VIRTIO_NET_HDR_LEN, 0);
        buf.extend_from_slice(&packet);
        buf
    }

    fn bench_gro_case(c: &mut Criterion, name: &str, templates: &[Vec<u8>]) {
        c.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let bufs = templates
                        .iter()
                        .map(|template| {
                            let mut buf = Vec::with_capacity(VIRTIO_NET_HDR_LEN + 65536);
                            buf.extend_from_slice(template);
                            buf
                        })
                        .collect::<Vec<_>>();
                    (GROTable::new(), bufs)
                },
                |(mut table, mut bufs)| {
                    table
                        .apply_gro(black_box(&mut bufs), black_box(VIRTIO_NET_HDR_LEN), false)
                        .unwrap();
                    black_box(bufs);
                },
                BatchSize::SmallInput,
            )
        });
    }

    pub fn bench(c: &mut Criterion) {
        let checksum_payload = vec![0x5a; 64 * 1024];
        c.bench_function("linux_checksum_64k", |b| {
            b.iter(|| checksum(black_box(checksum_payload.as_slice()), black_box(0)))
        });

        let gso_input = make_ipv4_tcp_packet(1, 8192);
        let gso_hdr = VirtioNetHdr {
            gso_type: VIRTIO_NET_HDR_GSO_TCPV4,
            hdr_len: (IPH_LEN + TCPH_LEN) as u16,
            gso_size: 1440,
            csum_start: IPH_LEN as u16,
            csum_offset: 16,
            ..Default::default()
        };
        c.bench_function("linux_gso_split_tcpv4_8k", |b| {
            b.iter_batched(
                || {
                    (
                        gso_input.clone(),
                        vec![vec![0u8; VIRTIO_NET_HDR_LEN + 1600]; 16],
                        vec![0usize; 16],
                    )
                },
                |(mut input, mut out, mut sizes)| {
                    let segments = gso_split(
                        black_box(&mut input),
                        black_box(gso_hdr),
                        black_box(&mut out),
                        black_box(&mut sizes),
                        black_box(VIRTIO_NET_HDR_LEN),
                        black_box(false),
                    )
                    .unwrap();
                    black_box(segments);
                },
                BatchSize::SmallInput,
            )
        });

        let gro_one_flow = (0u32..32)
            .map(|idx| make_gro_buffer(1 + idx * 512, 512))
            .collect::<Vec<_>>();
        bench_gro_case(c, "linux_handle_gro_tcpv4_32x512", &gro_one_flow);

        let gro_eight_flows = (0u8..32)
            .map(|idx| {
                let flow_id = idx % 8;
                let seq = 1 + u32::from(idx / 8) * 512;
                make_gro_buffer_for_flow(seq, 512, flow_id)
            })
            .collect::<Vec<_>>();
        bench_gro_case(c, "linux_handle_gro_tcpv4_32x512_8flows", &gro_eight_flows);

        let gro_32_flows = (0u8..32)
            .map(|flow_id| make_gro_buffer_for_flow(1, 512, flow_id))
            .collect::<Vec<_>>();
        bench_gro_case(c, "linux_handle_gro_tcpv4_32x512_32flows", &gro_32_flows);

        let gro_128_eight_flows = (0u8..128)
            .map(|idx| {
                let flow_id = idx % 8;
                let seq = 1 + u32::from(idx / 8) * 128;
                make_gro_buffer_for_flow(seq, 128, flow_id)
            })
            .collect::<Vec<_>>();
        bench_gro_case(
            c,
            "linux_handle_gro_tcpv4_128x128_8flows",
            &gro_128_eight_flows,
        );

        let gro_128_flows = (0u8..128)
            .map(|flow_id| make_gro_buffer_for_flow(1, 128, flow_id))
            .collect::<Vec<_>>();
        bench_gro_case(c, "linux_handle_gro_tcpv4_128x128_128flows", &gro_128_flows);
    }
}

fn benches(c: &mut Criterion) {
    bench_framed_codec(c);

    #[cfg(all(target_os = "linux", not(target_env = "ohos")))]
    linux_offload::bench(c);
}

criterion_group!(io_hot_paths, benches);
criterion_main!(io_hot_paths);
