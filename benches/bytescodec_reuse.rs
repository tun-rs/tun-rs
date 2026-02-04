use bytes::BytesMut;
use std::io;
use std::time::Instant;

// Original implementation: clone + clear
fn decode_clone_clear(buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
    if !buf.is_empty() {
        let rs = buf.clone();
        buf.clear();
        Ok(Some(rs))
    } else {
        Ok(None)
    }
}

// Optimized implementation: split_to
fn decode_split_to(buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
    if !buf.is_empty() {
        Ok(Some(buf.split_to(buf.len())))
    } else {
        Ok(None)
    }
}

fn benchmark_realistic_reuse(
    name: &str,
    packet_size: usize,
    iterations: usize,
    use_split: bool,
) {
    // Simulate real usage: buffer is reused across iterations
    let mut buf = BytesMut::with_capacity(packet_size);
    let data = vec![0u8; packet_size];
    
    let mut total_capacity_changes = 0u64;
    let mut last_capacity = buf.capacity();
    
    let start = Instant::now();
    for _ in 0..iterations {
        // Simulate receiving data into the buffer
        buf.extend_from_slice(&data);
        
        // Track capacity changes (indicates reallocation)
        if buf.capacity() != last_capacity {
            total_capacity_changes += 1;
            last_capacity = buf.capacity();
        }
        
        // Decode (extract the packet)
        if use_split {
            let _ = decode_split_to(&mut buf);
        } else {
            let _ = decode_clone_clear(&mut buf);
        }
    }
    let elapsed = start.elapsed();
    
    let throughput = (iterations as f64) / elapsed.as_secs_f64();
    
    println!("{} ({} bytes):", name, packet_size);
    println!("  Total time: {:?}", elapsed);
    println!("  Throughput: {:.0} ops/sec", throughput);
    println!("  Capacity changes: {}", total_capacity_changes);
    println!("  Final capacity: {}", buf.capacity());
}

fn main() {
    let iterations = 100_000;
    let packet_sizes = vec![64, 512, 1500, 9000];
    
    println!("=== BytesCodec Buffer Reuse Benchmark ===");
    println!("Iterations: {} (simulating realistic buffer reuse)\n", iterations);
    
    for &size in &packet_sizes {
        println!("--- Packet size: {} bytes ---", size);
        benchmark_realistic_reuse("clone+clear", size, iterations, false);
        benchmark_realistic_reuse("split_to   ", size, iterations, true);
        println!();
    }
    
    println!("\n=== Detailed Comparison ===");
    for &size in &packet_sizes {
        let mut buf1 = BytesMut::with_capacity(size);
        let mut buf2 = BytesMut::with_capacity(size);
        let data = vec![0u8; size];
        
        let mut allocs1 = 0;
        let mut allocs2 = 0;
        
        let start1 = Instant::now();
        for _ in 0..iterations {
            let cap_before = buf1.capacity();
            buf1.extend_from_slice(&data);
            if buf1.capacity() != cap_before {
                allocs1 += 1;
            }
            let _ = decode_clone_clear(&mut buf1);
        }
        let time1 = start1.elapsed();
        
        let start2 = Instant::now();
        for _ in 0..iterations {
            let cap_before = buf2.capacity();
            buf2.extend_from_slice(&data);
            if buf2.capacity() != cap_before {
                allocs2 += 1;
            }
            let _ = decode_split_to(&mut buf2);
        }
        let time2 = start2.elapsed();
        
        let diff_pct = if time1 < time2 {
            -((time2.as_nanos() as f64 - time1.as_nanos() as f64) / time1.as_nanos() as f64) * 100.0
        } else {
            ((time1.as_nanos() as f64 - time2.as_nanos() as f64) / time1.as_nanos() as f64) * 100.0
        };
        
        println!("{} bytes:", size);
        println!("  clone+clear: {:?}, {} allocs", time1, allocs1);
        println!("  split_to:    {:?}, {} allocs", time2, allocs2);
        println!("  Performance: {:.1}% (positive = split_to better)", diff_pct);
        println!();
    }
}
