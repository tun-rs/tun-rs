use bytes::BytesMut;
use std::time::Instant;

fn main() {
    let iterations = 10_000;
    
    println!("=== BytesCodec Reserve Pattern Benchmark ===");
    println!("Iterations: {}\n", iterations);
    
    // 情况1: reserve + resize + clone + clear
    println!("Scenario 1: reserve + resize + clone + clear");
    let start1 = Instant::now();
    {
        let mut buf = BytesMut::new();
        for _ in 0..iterations {
            buf.reserve(65536);
            // 模拟数据读取
            buf.resize(1500, 0);
            let _ = buf.clone();
            buf.clear();
        }
    }
    let time1 = start1.elapsed();
    println!("  Time: {:?}", time1);
    println!("  Throughput: {:.0} ops/sec", iterations as f64 / time1.as_secs_f64());
    println!();
    
    // 情况2: reserve + resize + split_to
    println!("Scenario 2: reserve + resize + split_to");
    let start2 = Instant::now();
    {
        let mut buf = BytesMut::new();
        for _ in 0..iterations {
            buf.reserve(65536);
            // 模拟数据读取
            buf.resize(1500, 0);
            let _ = buf.split_to(buf.len());
        }
    }
    let time2 = start2.elapsed();
    println!("  Time: {:?}", time2);
    println!("  Throughput: {:.0} ops/sec", iterations as f64 / time2.as_secs_f64());
    println!();
    
    // Comparison
    println!("=== Comparison ===");
    if time1 > time2 {
        let improvement = ((time1.as_nanos() as f64 - time2.as_nanos() as f64) / time1.as_nanos() as f64) * 100.0;
        println!("split_to is {:.1}% faster", improvement);
        println!("Time ratio: {:.2}x", time1.as_secs_f64() / time2.as_secs_f64());
    } else {
        let degradation = ((time2.as_nanos() as f64 - time1.as_nanos() as f64) / time2.as_nanos() as f64) * 100.0;
        println!("clone+clear is {:.1}% faster", degradation);
        println!("Time ratio: {:.2}x", time2.as_secs_f64() / time1.as_secs_f64());
    }
    
    println!("\n=== Detailed Analysis ===");
    
    // Test with capacity tracking
    let mut buf1 = BytesMut::new();
    let mut cap_changes1 = 0;
    let mut last_cap1 = 0;
    
    let start = Instant::now();
    for _ in 0..iterations {
        buf1.reserve(65536);
        buf1.resize(1500, 0);
        if buf1.capacity() != last_cap1 {
            cap_changes1 += 1;
            last_cap1 = buf1.capacity();
        }
        let _ = buf1.clone();
        buf1.clear();
    }
    let t1 = start.elapsed();
    
    let mut buf2 = BytesMut::new();
    let mut cap_changes2 = 0;
    let mut last_cap2 = 0;
    
    let start = Instant::now();
    for _ in 0..iterations {
        buf2.reserve(65536);
        buf2.resize(1500, 0);
        if buf2.capacity() != last_cap2 {
            cap_changes2 += 1;
            last_cap2 = buf2.capacity();
        }
        let _ = buf2.split_to(buf2.len());
    }
    let t2 = start.elapsed();
    
    println!("Scenario 1 (clone+clear):");
    println!("  Time: {:?}", t1);
    println!("  Capacity changes: {}", cap_changes1);
    println!("  Final capacity: {}", buf1.capacity());
    
    println!("\nScenario 2 (split_to):");
    println!("  Time: {:?}", t2);
    println!("  Capacity changes: {}", cap_changes2);
    println!("  Final capacity: {}", buf2.capacity());
    
    println!("\n=== Multiple runs for stability ===");
    let runs = 5;
    let mut times1 = Vec::new();
    let mut times2 = Vec::new();
    
    for run in 0..runs {
        let mut buf = BytesMut::new();
        let start = Instant::now();
        for _ in 0..iterations {
            buf.reserve(65536);
            buf.resize(1500, 0);
            let _ = buf.clone();
            buf.clear();
        }
        times1.push(start.elapsed());
        
        let mut buf = BytesMut::new();
        let start = Instant::now();
        for _ in 0..iterations {
            buf.reserve(65536);
            buf.resize(1500, 0);
            let _ = buf.split_to(buf.len());
        }
        times2.push(start.elapsed());
        
        println!("Run {}: clone+clear={:?}, split_to={:?}", 
                 run + 1, times1[run], times2[run]);
    }
    
    let avg1: f64 = times1.iter().map(|t| t.as_nanos() as f64).sum::<f64>() / runs as f64;
    let avg2: f64 = times2.iter().map(|t| t.as_nanos() as f64).sum::<f64>() / runs as f64;
    
    println!("\nAverage over {} runs:", runs);
    println!("  clone+clear: {:.2}ms", avg1 / 1_000_000.0);
    println!("  split_to:    {:.2}ms", avg2 / 1_000_000.0);
    println!("  Improvement: {:.1}%", ((avg1 - avg2) / avg1) * 100.0);
}
