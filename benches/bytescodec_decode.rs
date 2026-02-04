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

fn benchmark_decode(name: &str, packet_size: usize, iterations: usize, use_split: bool) {
    let mut total_time = std::time::Duration::ZERO;
    let mut total_allocs = 0u64;
    
    for _ in 0..iterations {
        let mut buf = BytesMut::from(vec![0u8; packet_size].as_slice());
        
        let start = Instant::now();
        if use_split {
            let _ = decode_split_to(&mut buf);
        } else {
            let _ = decode_clone_clear(&mut buf);
        }
        total_time += start.elapsed();
        
        // Count allocations by checking capacity changes
        total_allocs += 1;
    }
    
    let avg_time = total_time / iterations as u32;
    let throughput = (iterations as f64) / total_time.as_secs_f64();
    
    println!("{}: {} bytes", name, packet_size);
    println!("  Total time: {:?}", total_time);
    println!("  Average time: {:?}", avg_time);
    println!("  Throughput: {:.0} ops/sec", throughput);
    println!("  Time per GB: {:?}", 
        std::time::Duration::from_secs_f64(1_000_000_000.0 / (throughput * packet_size as f64)));
}

fn main() {
    let iterations = 1_000_000;
    let packet_sizes = vec![64, 512, 1500, 9000];
    
    println!("=== BytesCodec Decode Benchmark ===");
    println!("Iterations: {}\n", iterations);
    
    for &size in &packet_sizes {
        println!("--- Packet size: {} bytes ---", size);
        benchmark_decode("clone+clear", size, iterations, false);
        benchmark_decode("split_to   ", size, iterations, true);
        println!();
    }
    
    println!("\n=== Performance Comparison ===");
    for &size in &packet_sizes {
        let mut buf1 = BytesMut::from(vec![0u8; size].as_slice());
        let mut buf2 = BytesMut::from(vec![0u8; size].as_slice());
        
        let start1 = Instant::now();
        for _ in 0..iterations {
            buf1 = BytesMut::from(vec![0u8; size].as_slice());
            let _ = decode_clone_clear(&mut buf1);
        }
        let time1 = start1.elapsed();
        
        let start2 = Instant::now();
        for _ in 0..iterations {
            buf2 = BytesMut::from(vec![0u8; size].as_slice());
            let _ = decode_split_to(&mut buf2);
        }
        let time2 = start2.elapsed();
        
        let improvement = ((time1.as_nanos() as f64 - time2.as_nanos() as f64) / time1.as_nanos() as f64) * 100.0;
        
        println!("{} bytes: clone+clear={:?}, split_to={:?}, improvement={:.1}%", 
                 size, time1, time2, improvement);
    }
}
