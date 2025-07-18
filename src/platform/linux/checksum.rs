use byteorder::{BigEndian, ByteOrder};

/// A pure Rust scalar (non-SIMD) implementation for the checksum accumulation.
///
/// It uses a simple loop instead of manual unrolling for better clarity and maintainability.
fn checksum_no_fold_scalar(mut b: &[u8], initial: u64) -> u64 {
    let mut accumulator = initial;

    // Process the slice in 4-byte (u32) chunks.
    while b.len() >= 4 {
        accumulator += BigEndian::read_u32(&b[0..4]) as u64;
        b = &b[4..];
    }

    // Handle the remaining 1-3 bytes.
    if b.len() >= 2 {
        accumulator += BigEndian::read_u16(&b[0..2]) as u64;
        b = &b[2..];
    }
    if let Some(&byte) = b.first() {
        // For odd-length inputs, the last byte is treated as the high byte
        // of a 16-bit word (e.g., [0xAB] becomes 0xAB00), as per RFC 1071.
        accumulator += (byte as u64) << 8;
    }

    accumulator
}

/// A SIMD-accelerated (AVX2) implementation for the checksum accumulation.
///
/// # Safety
/// Caller must ensure this function is called only on CPUs that support AVX2.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn checksum_no_fold_avx2(mut b: &[u8], initial: u64) -> u64 {
    use std::arch::x86_64::*;

    let mut accumulator = initial;
    const CHUNK_SIZE: usize = 32; // AVX2 processes 32 bytes (256 bits) at a time.

    if b.len() >= CHUNK_SIZE {
        // Use a 256-bit vector to hold four 64-bit partial sums.
        let mut sums = _mm256_setzero_si256();

        // Shuffle mask to reverse byte order from Big Endian to Little Endian for each 32-bit integer.
        let shuffle_mask = _mm256_set_epi8(
            12, 13, 14, 15, 8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3, 12, 13, 14, 15, 8, 9, 10, 11, 4,
            5, 6, 7, 0, 1, 2, 3,
        );

        while b.len() >= CHUNK_SIZE {
            // Load 32 bytes of data.
            let data = _mm256_loadu_si256(b.as_ptr() as *const __m256i);
            // Swap byte order from BE to LE.
            let swapped = _mm256_shuffle_epi8(data, shuffle_mask);

            // Widen the lower 4 u32s to u64s and add them to the accumulator.
            let lower_u64 = _mm256_cvtepu32_epi64(_mm256_extracti128_si256(swapped, 0));
            sums = _mm256_add_epi64(sums, lower_u64);

            // Widen the upper 4 u32s to u64s and add them to the accumulator.
            let upper_u64 = _mm256_cvtepu32_epi64(_mm256_extracti128_si256(swapped, 1));
            sums = _mm256_add_epi64(sums, upper_u64);

            b = &b[CHUNK_SIZE..];
        }

        // Perform a horizontal sum to combine the partial sums in the vector.
        accumulator += _mm256_extract_epi64(sums, 0) as u64;
        accumulator += _mm256_extract_epi64(sums, 1) as u64;
        accumulator += _mm256_extract_epi64(sums, 2) as u64;
        accumulator += _mm256_extract_epi64(sums, 3) as u64;
    }

    // Process any remaining data using the scalar implementation.
    checksum_no_fold_scalar(b, accumulator)
}

/// A SIMD-accelerated (SSE4.1) implementation for the checksum accumulation.
///
/// # Safety
/// Caller must ensure this function is called only on CPUs that support SSE4.1.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn checksum_no_fold_sse41(mut b: &[u8], initial: u64) -> u64 {
    use std::arch::x86_64::*;

    let mut accumulator = initial;
    const CHUNK_SIZE: usize = 16; // SSE processes 16 bytes (128 bits) at a time.

    if b.len() >= CHUNK_SIZE {
        // Use a 128-bit vector to hold two 64-bit partial sums.
        let mut sums = _mm_setzero_si128();

        // Shuffle mask to reverse byte order from Big Endian to Little Endian for each 32-bit integer.
        let shuffle_mask = _mm_set_epi8(12, 13, 14, 15, 8, 9, 10, 11, 4, 5, 6, 7, 0, 1, 2, 3);

        while b.len() >= CHUNK_SIZE {
            // Load 16 bytes of data.
            let data = _mm_loadu_si128(b.as_ptr() as *const __m128i);
            // Swap byte order from BE to LE.
            let swapped = _mm_shuffle_epi8(data, shuffle_mask);

            // Widen the lower 2 u32s to u64s and add them to the accumulator.
            let lower_u64 = _mm_cvtepu32_epi64(swapped);
            sums = _mm_add_epi64(sums, lower_u64);

            // Widen the upper 2 u32s to u64s and add them to the accumulator.
            let upper_u64 = _mm_cvtepu32_epi64(_mm_bsrli_si128(swapped, 8));
            sums = _mm_add_epi64(sums, upper_u64);

            b = &b[CHUNK_SIZE..];
        }

        // Horizontal sum of the two 64-bit lanes.
        accumulator += _mm_cvtsi128_si64(sums) as u64;
        accumulator += _mm_extract_epi64(sums, 1) as u64;
    }

    // Process any remaining data using the scalar implementation.
    checksum_no_fold_scalar(b, accumulator)
}

/// Calculates a checksum accumulator over a byte slice without the final fold.
///
/// This function dispatches to the optimal implementation at runtime (AVX2, SSE4.1,
/// or scalar) based on CPU feature detection. The algorithm is consistent with the
/// WireGuard-Go implementation: it treats the input as a sequence of big-endian u32s,
/// accumulates them as u64s, and handles the remainder.
#[inline]
pub fn checksum_no_fold(b: &[u8], initial: u64) -> u64 {
    // Dispatch to the best available implementation based on runtime CPU feature detection.
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: We have just checked that the CPU supports AVX2.
            return unsafe { checksum_no_fold_avx2(b, initial) };
        }
        if is_x86_feature_detected!("sse4.1") {
            // SAFETY: We have just checked that the CPU supports SSE4.1.
            return unsafe { checksum_no_fold_sse41(b, initial) };
        }
    }

    // TODO: AArch64 (ARM) NEON SIMD optimization could be added here.
    // #[cfg(target_arch = "aarch64")] { ... }

    // Fall back to the scalar implementation if no SIMD features are available.
    checksum_no_fold_scalar(b, initial)
}

/// Calculates the final 16-bit internet checksum.
///
/// This performs the standard one's complement sum fold-down of a 64-bit accumulator
/// into a 16-bit value. The loop ensures correctness regardless of the initial magnitude
/// of the accumulator.
pub fn checksum(b: &[u8], initial: u64) -> u16 {
    let mut accumulator = checksum_no_fold(b, initial);

    // Fold the 64-bit accumulator into 16 bits.
    while accumulator > 0xFFFF {
        accumulator = (accumulator >> 16) + (accumulator & 0xFFFF);
    }

    accumulator as u16
}

/// Calculates the checksum accumulator for a TCP/UDP pseudo-header.
///
/// This function also benefits from the `checksum_no_fold` optimizations.
pub fn pseudo_header_checksum_no_fold(
    protocol: u8,
    src_addr: &[u8],
    dst_addr: &[u8],
    total_len: u16,
) -> u64 {
    // Accumulate the source and destination addresses.
    let sum = checksum_no_fold(src_addr, 0);
    let sum = checksum_no_fold(dst_addr, sum);

    // The pseudo-header trailer consists of {0, protocol, total_len}.
    // We construct this 4-byte sequence and add its checksum to the sum.
    let len_bytes = total_len.to_be_bytes();
    let trailer = [0, protocol, len_bytes[0], len_bytes[1]];
    checksum_no_fold(&trailer, sum)
}
