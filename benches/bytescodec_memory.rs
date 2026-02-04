use bytes::BytesMut;
use std::io;
use std::time::Instant;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

// Custom allocator to track allocations
struct CountingAllocator;

static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static DEALLOCATED: AtomicU64 = AtomicU64::new(0);
static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATED.fetch_add(layout.size() as u64, Ordering::SeqCst);
        ALLOC_COUNT.fetch_add(1, Ordering::SeqCst);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        DEALLOCATED.fetch_add(layout.size() as u64, Ordering::SeqCst);
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn reset_counters() {
    ALLOCATED.store(0, Ordering::SeqCst);
    DEALLOCATED.store(0, Ordering::SeqCst);
    ALLOC_COUNT.store(0, Ordering::SeqCst);
}

fn get_stats() -> (u64, u64, u64) {
    (
        ALLOCATED.load(Ordering::SeqCst),
        DEALLOCATED.load(Ordering::SeqCst),
        ALLOC_COUNT.load(Ordering::SeqCst),
    )
}

fn decode_clone_clear(buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
    if !buf.is_empty() {
        let rs = buf.clone();
        buf.clear();
        Ok(Some(rs))
    } else {
        Ok(None)
    }
}

fn decode_split_to(buf: &mut BytesMut) -> Result<Option<BytesMut>, io::Error> {
    if !buf.is_empty() {
        Ok(Some(buf.split_to(buf.len())))
    } else {
        Ok(None)
    }
}

fn benchmark_with_memory(name: &str, packet_size: usize, iterations: usize, use_split: bool) {
    reset_counters();
    
    let mut buf = BytesMut::with_capacity(packet_size);
    let data = vec![0u8; packet_size];
    
    let start = Instant::now();
    for _ in 0..iterations {
        buf.extend_from_slice(&data);
        if use_split {
            let _ = decode_split_to(&mut buf);
        } else {
            let _ = decode_clone_clear(&mut buf);
        }
    }
    let elapsed = start.elapsed();
    
    let (allocated, deallocated, alloc_count) = get_stats();
    let net_memory = allocated as i64 - deallocated as i64;
    
    println!("{} ({} bytes):", name, packet_size);
    println!("  Time: {:?}", elapsed);
    println!("  Throughput: {:.0} ops/sec", (iterations as f64) / elapsed.as_secs_f64());
    println!("  Allocations: {}", alloc_count);
    println!("  Memory allocated: {} bytes", allocated);
    println!("  Memory deallocated: {} bytes", deallocated);
    println!("  Net memory: {} bytes", net_memory);
    println!("  Avg alloc per op: {:.1} bytes", allocated as f64 / iterations as f64);
}

fn main() {
    let iterations = 10_000;
    let packet_sizes = vec![64, 512, 1500, 9000];
    
    println!("=== BytesCodec Memory Allocation Benchmark ===");
    println!("Iterations: {}\n", iterations);
    
    for &size in &packet_sizes {
        println!("--- Packet size: {} bytes ---", size);
        benchmark_with_memory("clone+clear", size, iterations, false);
        println!();
        benchmark_with_memory("split_to   ", size, iterations, true);
        println!();
    }
    
    println!("\n=== Summary ===");
    for &size in &packet_sizes {
        reset_counters();
        let mut buf1 = BytesMut::with_capacity(size);
        let data = vec![0u8; size];
        let start1 = Instant::now();
        for _ in 0..iterations {
            buf1.extend_from_slice(&data);
            let _ = decode_clone_clear(&mut buf1);
        }
        let time1 = start1.elapsed();
        let (alloc1, _, count1) = get_stats();
        
        reset_counters();
        let mut buf2 = BytesMut::with_capacity(size);
        let start2 = Instant::now();
        for _ in 0..iterations {
            buf2.extend_from_slice(&data);
            let _ = decode_split_to(&mut buf2);
        }
        let time2 = start2.elapsed();
        let (alloc2, _, count2) = get_stats();
        
        println!("{} bytes:", size);
        println!("  clone+clear: {:?}, {} allocs, {} bytes allocated", time1, count1, alloc1);
        println!("  split_to:    {:?}, {} allocs, {} bytes allocated", time2, count2, alloc2);
        
        let time_ratio = time1.as_nanos() as f64 / time2.as_nanos() as f64;
        let alloc_ratio = alloc1 as f64 / alloc2.max(1) as f64;
        
        println!("  Time ratio (clone/split): {:.2}x", time_ratio);
        println!("  Alloc ratio (clone/split): {:.2}x", alloc_ratio);
        println!();
    }
}
