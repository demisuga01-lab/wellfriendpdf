//! Native SIMD pixel-compositor kernels for Wellfriend PDF.
//!
//! The main engine crate forbids unsafe code. Architecture intrinsics require
//! unsafe, so this crate isolates that boundary behind small safe functions with
//! scalar-equivalence debug guards.
#![deny(unsafe_op_in_unsafe_fn)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimdBackend {
    Scalar,
    Sse2,
    Avx2,
    Neon,
}

impl SimdBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            SimdBackend::Scalar => "scalar",
            SimdBackend::Sse2 => "sse2",
            SimdBackend::Avx2 => "avx2",
            SimdBackend::Neon => "neon",
        }
    }
}

pub fn active_backend() -> SimdBackend {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("avx2") {
            return SimdBackend::Avx2;
        }
        if std::is_x86_feature_detected!("sse2") {
            return SimdBackend::Sse2;
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return SimdBackend::Neon;
    }
    #[cfg(all(target_arch = "arm", target_feature = "neon"))]
    {
        return SimdBackend::Neon;
    }
    SimdBackend::Scalar
}

pub fn fill_opaque_run(slice: &mut [u8], color: [u8; 4]) -> bool {
    if slice.len() < 16 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return guarded_fill_opaque_run_avx2_x86_64(slice, color);
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_fill_opaque_run_sse2_x86_64(slice, color);
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_fill_opaque_run_sse2_x86(slice, color);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return guarded_fill_opaque_run_neon_aarch64(slice, color);
    }
    #[cfg(target_arch = "wasm32")]
    {
        return fill_opaque_run_scalar(slice, color);
    }
    false
}

pub fn blend_normal_opaque_destination(slice: &mut [u8], color: [u8; 4]) -> bool {
    if slice.len() < 16 || color[3] == 0 || color[3] == 255 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return guarded_blend_normal_opaque_dst_avx2_x86_64(slice, color);
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_blend_normal_opaque_dst_sse2_x86_64(slice, color);
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_blend_normal_opaque_dst_sse2_x86(slice, color);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return guarded_blend_normal_opaque_dst_neon_aarch64(slice, color);
    }
    #[cfg(target_arch = "wasm32")]
    {
        return blend_normal_opaque_dst_scalar(slice, color);
    }
    false
}

pub fn composite_soft_mask_opaque_destination(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    if group_alpha_255 != 255 || mask_row.is_empty() {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return guarded_soft_mask_opaque_dst_avx2_x86_64(
                dst_row,
                src_row,
                mask_row,
                group_alpha_255,
            );
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_soft_mask_opaque_dst_sse2_x86_64(
                dst_row,
                src_row,
                mask_row,
                group_alpha_255,
            );
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_soft_mask_opaque_dst_sse2_x86(
                dst_row,
                src_row,
                mask_row,
                group_alpha_255,
            );
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return guarded_soft_mask_opaque_dst_neon_aarch64(
            dst_row,
            src_row,
            mask_row,
            group_alpha_255,
        );
    }
    #[cfg(target_arch = "wasm32")]
    {
        return soft_mask_opaque_dst_scalar(dst_row, src_row, mask_row, group_alpha_255);
    }
    false
}

pub fn composite_normal_opaque_destination(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let pixels = row_pixels(dst_row, src_row, &[]);
    if pixels < 4 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return guarded_composite_normal_opaque_dst_avx2_x86_64(dst_row, src_row);
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_composite_normal_opaque_dst_sse2_x86_64(dst_row, src_row);
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_composite_normal_opaque_dst_sse2_x86(dst_row, src_row);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return guarded_composite_normal_opaque_dst_neon_aarch64(dst_row, src_row);
    }
    #[cfg(target_arch = "wasm32")]
    {
        return composite_normal_opaque_dst_scalar(dst_row, src_row);
    }
    false
}

fn fill_opaque_run_scalar(slice: &mut [u8], color: [u8; 4]) -> bool {
    if slice.len() < 4 {
        return false;
    }
    for px in slice.chunks_exact_mut(4) {
        px.copy_from_slice(&color);
    }
    true
}

fn blend_normal_opaque_dst_scalar(slice: &mut [u8], color: [u8; 4]) -> bool {
    if slice.len() < 4 || color[3] == 0 {
        return false;
    }
    let alpha = u16::from(color[3]);
    let inv = 255_u16.saturating_sub(alpha);
    for px in slice.chunks_exact_mut(4) {
        for channel in 0..3 {
            let mixed = u16::from(color[channel]) * alpha + u16::from(px[channel]) * inv + 128;
            px[channel] = ((mixed + (mixed >> 8)) >> 8).min(255) as u8;
        }
        px[3] = 255;
    }
    true
}

fn soft_mask_opaque_dst_scalar(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    let pixels = dst_row
        .chunks_exact_mut(4)
        .zip(src_row.chunks_exact(4))
        .zip(mask_row.iter())
        .count();
    if pixels == 0 {
        return false;
    }
    for ((dst, src), mask) in dst_row
        .chunks_exact_mut(4)
        .zip(src_row.chunks_exact(4))
        .zip(mask_row.iter().copied())
    {
        let eff = div255_round_u16(
            div255_round_u16(u16::from(src[3]) * u16::from(mask)) * group_alpha_255,
        );
        if eff == 0 {
            continue;
        }
        let inv = 255_u16.saturating_sub(eff);
        for channel in 0..3 {
            let mixed = u16::from(src[channel]) * eff + u16::from(dst[channel]) * inv + 128;
            dst[channel] = ((mixed + (mixed >> 8)) >> 8).min(255) as u8;
        }
        dst[3] = 255;
    }
    true
}

fn composite_normal_opaque_dst_scalar(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let pixels = row_pixels(dst_row, src_row, &[]);
    if pixels == 0 {
        return false;
    }
    for (dst, src) in dst_row[..pixels * 4]
        .chunks_exact_mut(4)
        .zip(src_row[..pixels * 4].chunks_exact(4))
    {
        if src[3] == 0 {
            continue;
        }
        if src[3] == 255 {
            dst.copy_from_slice(&[src[0], src[1], src[2], 255]);
            continue;
        }
        let alpha = u16::from(src[3]);
        let inv = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            let mixed = u16::from(src[channel]) * alpha + u16::from(dst[channel]) * inv + 128;
            dst[channel] = ((mixed + (mixed >> 8)) >> 8).min(255) as u8;
        }
        dst[3] = 255;
    }
    true
}

fn row_pixels(dst_row: &[u8], src_row: &[u8], mask_row: &[u8]) -> usize {
    let pixels = dst_row.len().min(src_row.len()) / 4;
    if mask_row.is_empty() {
        pixels
    } else {
        pixels.min(mask_row.len())
    }
}

#[inline]
fn div255_round_u16(value: u16) -> u16 {
    let adjusted = value.saturating_add(128);
    (adjusted + (adjusted >> 8)) >> 8
}

#[cfg(target_arch = "x86_64")]
fn guarded_fill_opaque_run_avx2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        fill_opaque_run_scalar(&mut copy, color);
        copy
    });
    // SAFETY: entered only after AVX2 runtime detection.
    let ok = unsafe { fill_opaque_run_avx2_x86_64(slice, color) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(slice, expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn fill_opaque_run_avx2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86_64::{__m256i, _mm256_set1_epi32, _mm256_storeu_si256};
    let pixel = u32::from_le_bytes(color) as i32;
    let fill = _mm256_set1_epi32(pixel);
    let simd_len = (slice.len() / 32) * 32;
    let mut offset = 0usize;
    while offset < simd_len {
        unsafe {
            _mm256_storeu_si256(slice.as_mut_ptr().add(offset) as *mut __m256i, fill);
        }
        offset += 32;
    }
    if offset < slice.len() {
        fill_opaque_run_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "x86_64")]
fn guarded_fill_opaque_run_sse2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        fill_opaque_run_scalar(&mut copy, color);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { fill_opaque_run_sse2_x86_64(slice, color) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(slice, expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn fill_opaque_run_sse2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86_64::{__m128i, _mm_set1_epi32, _mm_storeu_si128};
    let pixel = u32::from_le_bytes(color) as i32;
    let fill = _mm_set1_epi32(pixel);
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        unsafe {
            _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, fill);
        }
        offset += 16;
    }
    if offset < slice.len() {
        fill_opaque_run_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "x86")]
fn guarded_fill_opaque_run_sse2_x86(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        fill_opaque_run_scalar(&mut copy, color);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { fill_opaque_run_sse2_x86(slice, color) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(slice, expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn fill_opaque_run_sse2_x86(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86::{__m128i, _mm_set1_epi32, _mm_storeu_si128};
    let pixel = u32::from_le_bytes(color) as i32;
    let fill = _mm_set1_epi32(pixel);
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        unsafe {
            _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, fill);
        }
        offset += 16;
    }
    if offset < slice.len() {
        fill_opaque_run_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "aarch64")]
fn guarded_fill_opaque_run_neon_aarch64(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        fill_opaque_run_scalar(&mut copy, color);
        copy
    });
    // SAFETY: AArch64 guarantees Advanced SIMD.
    let ok = unsafe { fill_opaque_run_neon_aarch64(slice, color) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(slice, expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "aarch64")]
unsafe fn fill_opaque_run_neon_aarch64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::aarch64::{vdupq_n_u32, vreinterpretq_u8_u32, vst1q_u8};
    let pixel = u32::from_le_bytes(color);
    let fill = vreinterpretq_u8_u32(vdupq_n_u32(pixel));
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        unsafe {
            vst1q_u8(slice.as_mut_ptr().add(offset), fill);
        }
        offset += 16;
    }
    if offset < slice.len() {
        fill_opaque_run_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
macro_rules! guarded_blend_x86 {
    ($name:ident, $kernel:ident) => {
        fn $name(slice: &mut [u8], color: [u8; 4]) -> bool {
            let mut scalar = cfg!(debug_assertions).then(|| {
                let mut copy = slice.to_vec();
                blend_normal_opaque_dst_scalar(&mut copy, color);
                copy
            });
            let ok = unsafe { $kernel(slice, color) };
            if let Some(expected) = scalar.take() {
                debug_assert_eq!(slice, expected.as_slice());
            }
            ok
        }
    };
}

#[cfg(target_arch = "x86_64")]
guarded_blend_x86!(
    guarded_blend_normal_opaque_dst_avx2_x86_64,
    blend_normal_opaque_dst_avx2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn blend_normal_opaque_dst_avx2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi16, _mm256_loadu_si256, _mm256_mullo_epi16, _mm256_packus_epi16,
        _mm256_set1_epi16, _mm256_setzero_si256, _mm256_srli_epi16, _mm256_storeu_si256,
        _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
    };
    let alpha = color[3] as i16;
    let inv = 255i16.saturating_sub(alpha);
    let src_values = [
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
    ];
    let alpha_values = [
        alpha, alpha, alpha, 255, alpha, alpha, alpha, 255, alpha, alpha, alpha, 255, alpha, alpha,
        alpha, 255,
    ];
    let inv_values = [
        inv, inv, inv, 0, inv, inv, inv, 0, inv, inv, inv, 0, inv, inv, inv, 0,
    ];
    let src = unsafe { _mm256_loadu_si256(src_values.as_ptr() as *const __m256i) };
    let alpha_v = unsafe { _mm256_loadu_si256(alpha_values.as_ptr() as *const __m256i) };
    let inv_v = unsafe { _mm256_loadu_si256(inv_values.as_ptr() as *const __m256i) };
    let round = _mm256_set1_epi16(128);
    let zero = _mm256_setzero_si256();
    let simd_len = (slice.len() / 32) * 32;
    let mut offset = 0usize;
    while offset < simd_len {
        let dst = unsafe { _mm256_loadu_si256(slice.as_ptr().add(offset) as *const __m256i) };
        let lo = _mm256_unpacklo_epi8(dst, zero);
        let hi = _mm256_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src, alpha_v),
                _mm256_mullo_epi16(lo, inv_v),
            ),
            round,
        );
        let hi_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src, alpha_v),
                _mm256_mullo_epi16(hi, inv_v),
            ),
            round,
        );
        let lo_out = _mm256_srli_epi16(
            _mm256_add_epi16(lo_mixed, _mm256_srli_epi16(lo_mixed, 8)),
            8,
        );
        let hi_out = _mm256_srli_epi16(
            _mm256_add_epi16(hi_mixed, _mm256_srli_epi16(hi_mixed, 8)),
            8,
        );
        let packed = _mm256_packus_epi16(lo_out, hi_out);
        unsafe {
            _mm256_storeu_si256(slice.as_mut_ptr().add(offset) as *mut __m256i, packed);
        }
        for alpha_offset in (offset + 3..offset + 32).step_by(4) {
            slice[alpha_offset] = 255;
        }
        offset += 32;
    }
    if offset < slice.len() {
        blend_normal_opaque_dst_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "x86_64")]
guarded_blend_x86!(
    guarded_blend_normal_opaque_dst_sse2_x86_64,
    blend_normal_opaque_dst_sse2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn blend_normal_opaque_dst_sse2_x86_64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86_64::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    let alpha = color[3] as i16;
    let inv = 255i16.saturating_sub(alpha);
    let src_values = [
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
    ];
    let alpha_values = [alpha, alpha, alpha, 255, alpha, alpha, alpha, 255];
    let inv_values = [inv, inv, inv, 0, inv, inv, inv, 0];
    let src = unsafe { _mm_loadu_si128(src_values.as_ptr() as *const __m128i) };
    let alpha_v = unsafe { _mm_loadu_si128(alpha_values.as_ptr() as *const __m128i) };
    let inv_v = unsafe { _mm_loadu_si128(inv_values.as_ptr() as *const __m128i) };
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        let dst = unsafe { _mm_loadu_si128(slice.as_ptr().add(offset) as *const __m128i) };
        let lo = _mm_unpacklo_epi8(dst, zero);
        let hi = _mm_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm_add_epi16(
            _mm_add_epi16(_mm_mullo_epi16(src, alpha_v), _mm_mullo_epi16(lo, inv_v)),
            round,
        );
        let hi_mixed = _mm_add_epi16(
            _mm_add_epi16(_mm_mullo_epi16(src, alpha_v), _mm_mullo_epi16(hi, inv_v)),
            round,
        );
        let lo_out = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
        let hi_out = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
        let packed = _mm_packus_epi16(lo_out, hi_out);
        unsafe {
            _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, packed);
        }
        for alpha_offset in (offset + 3..offset + 16).step_by(4) {
            slice[alpha_offset] = 255;
        }
        offset += 16;
    }
    if offset < slice.len() {
        blend_normal_opaque_dst_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "x86")]
guarded_blend_x86!(
    guarded_blend_normal_opaque_dst_sse2_x86,
    blend_normal_opaque_dst_sse2_x86
);

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn blend_normal_opaque_dst_sse2_x86(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::x86::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    let alpha = color[3] as i16;
    let inv = 255i16.saturating_sub(alpha);
    let src_values = [
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
    ];
    let alpha_values = [alpha, alpha, alpha, 255, alpha, alpha, alpha, 255];
    let inv_values = [inv, inv, inv, 0, inv, inv, inv, 0];
    let src = unsafe { _mm_loadu_si128(src_values.as_ptr() as *const __m128i) };
    let alpha_v = unsafe { _mm_loadu_si128(alpha_values.as_ptr() as *const __m128i) };
    let inv_v = unsafe { _mm_loadu_si128(inv_values.as_ptr() as *const __m128i) };
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        let dst = unsafe { _mm_loadu_si128(slice.as_ptr().add(offset) as *const __m128i) };
        let lo = _mm_unpacklo_epi8(dst, zero);
        let hi = _mm_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm_add_epi16(
            _mm_add_epi16(_mm_mullo_epi16(src, alpha_v), _mm_mullo_epi16(lo, inv_v)),
            round,
        );
        let hi_mixed = _mm_add_epi16(
            _mm_add_epi16(_mm_mullo_epi16(src, alpha_v), _mm_mullo_epi16(hi, inv_v)),
            round,
        );
        let lo_out = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
        let hi_out = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
        let packed = _mm_packus_epi16(lo_out, hi_out);
        unsafe {
            _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, packed);
        }
        for alpha_offset in (offset + 3..offset + 16).step_by(4) {
            slice[alpha_offset] = 255;
        }
        offset += 16;
    }
    if offset < slice.len() {
        blend_normal_opaque_dst_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(target_arch = "aarch64")]
fn guarded_blend_normal_opaque_dst_neon_aarch64(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        blend_normal_opaque_dst_scalar(&mut copy, color);
        copy
    });
    let ok = unsafe { blend_normal_opaque_dst_neon_aarch64(slice, color) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(slice, expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "aarch64")]
unsafe fn blend_normal_opaque_dst_neon_aarch64(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::aarch64::{
        vaddq_u16, vcombine_u8, vdupq_n_u16, vget_high_u8, vget_low_u8, vld1q_u8, vmovl_u8,
        vmulq_u16, vqmovn_u16, vshrq_n_u16, vst1q_u8,
    };
    let alpha = u16::from(color[3]);
    let inv = 255_u16.saturating_sub(alpha);
    let src = [
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
        u16::from(color[0]),
        u16::from(color[1]),
        u16::from(color[2]),
        255,
    ];
    let alpha_v = [alpha, alpha, alpha, 255, alpha, alpha, alpha, 255];
    let inv_v = [inv, inv, inv, 0, inv, inv, inv, 0];
    let src_v = unsafe { std::arch::aarch64::vld1q_u16(src.as_ptr()) };
    let alpha_v = unsafe { std::arch::aarch64::vld1q_u16(alpha_v.as_ptr()) };
    let inv_v = unsafe { std::arch::aarch64::vld1q_u16(inv_v.as_ptr()) };
    let round = vdupq_n_u16(128);
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        let dst = unsafe { vld1q_u8(slice.as_ptr().add(offset)) };
        let dst_lo = vmovl_u8(vget_low_u8(dst));
        let dst_hi = vmovl_u8(vget_high_u8(dst));
        let lo_mixed = vaddq_u16(
            vaddq_u16(vmulq_u16(src_v, alpha_v), vmulq_u16(dst_lo, inv_v)),
            round,
        );
        let hi_mixed = vaddq_u16(
            vaddq_u16(vmulq_u16(src_v, alpha_v), vmulq_u16(dst_hi, inv_v)),
            round,
        );
        let lo = vshrq_n_u16(vaddq_u16(lo_mixed, vshrq_n_u16(lo_mixed, 8)), 8);
        let hi = vshrq_n_u16(vaddq_u16(hi_mixed, vshrq_n_u16(hi_mixed, 8)), 8);
        let packed = vcombine_u8(vqmovn_u16(lo), vqmovn_u16(hi));
        unsafe { vst1q_u8(slice.as_mut_ptr().add(offset), packed) };
        for alpha_offset in (offset + 3..offset + 16).step_by(4) {
            slice[alpha_offset] = 255;
        }
        offset += 16;
    }
    if offset < slice.len() {
        blend_normal_opaque_dst_scalar(&mut slice[offset..], color);
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
macro_rules! guarded_soft_x86 {
    ($name:ident, $kernel:ident) => {
        fn $name(
            dst_row: &mut [u8],
            src_row: &[u8],
            mask_row: &[u8],
            group_alpha_255: u16,
        ) -> bool {
            let pixels = dst_row
                .chunks_exact(4)
                .zip(src_row.chunks_exact(4))
                .count()
                .min(mask_row.len());
            let mut scalar = cfg!(debug_assertions).then(|| {
                let mut copy = dst_row[..pixels * 4].to_vec();
                soft_mask_opaque_dst_scalar(
                    &mut copy,
                    &src_row[..pixels * 4],
                    &mask_row[..pixels],
                    group_alpha_255,
                );
                copy
            });
            let ok = unsafe { $kernel(dst_row, src_row, mask_row, group_alpha_255) };
            if ok {
                if let Some(expected) = scalar.take() {
                    debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
                }
            }
            ok
        }
    };
}

#[cfg(target_arch = "x86_64")]
guarded_soft_x86!(
    guarded_soft_mask_opaque_dst_avx2_x86_64,
    soft_mask_opaque_dst_avx2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn soft_mask_opaque_dst_avx2_x86_64(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    unsafe { soft_mask_opaque_dst_avx2_impl(dst_row, src_row, mask_row, group_alpha_255) }
}

#[cfg(target_arch = "x86_64")]
guarded_soft_x86!(
    guarded_soft_mask_opaque_dst_sse2_x86_64,
    soft_mask_opaque_dst_sse2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn soft_mask_opaque_dst_sse2_x86_64(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    unsafe { soft_mask_opaque_dst_sse2_impl(dst_row, src_row, mask_row, group_alpha_255) }
}

#[cfg(target_arch = "x86")]
guarded_soft_x86!(
    guarded_soft_mask_opaque_dst_sse2_x86,
    soft_mask_opaque_dst_sse2_x86
);

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn soft_mask_opaque_dst_sse2_x86(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    unsafe { soft_mask_opaque_dst_sse2_impl(dst_row, src_row, mask_row, group_alpha_255) }
}

#[cfg(target_arch = "aarch64")]
fn guarded_soft_mask_opaque_dst_neon_aarch64(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let pixels = row_pixels(dst_row, src_row, mask_row);
        let mut copy = dst_row[..pixels * 4].to_vec();
        soft_mask_opaque_dst_scalar(
            &mut copy,
            &src_row[..pixels * 4],
            &mask_row[..pixels],
            group_alpha_255,
        );
        copy
    });
    // SAFETY: AArch64 guarantees Advanced SIMD.
    let ok =
        unsafe { soft_mask_opaque_dst_neon_aarch64(dst_row, src_row, mask_row, group_alpha_255) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(&dst_row[..expected.len()], expected.as_slice());
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn soft_mask_opaque_dst_sse2_impl(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };

    if group_alpha_255 != 255 {
        return false;
    }
    let pixels = row_pixels(dst_row, src_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let mut alpha_lo = [0i16; 8];
        let mut inv_lo = [0i16; 8];
        let mut alpha_hi = [0i16; 8];
        let mut inv_hi = [0i16; 8];
        for lane in 0..2 {
            let idx = pixel + lane;
            let alpha =
                div255_round_u16(u16::from(src_row[idx * 4 + 3]) * u16::from(mask_row[idx])) as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = lane * 4;
            alpha_lo[base] = alpha;
            alpha_lo[base + 1] = alpha;
            alpha_lo[base + 2] = alpha;
            alpha_lo[base + 3] = 255;
            inv_lo[base] = inv;
            inv_lo[base + 1] = inv;
            inv_lo[base + 2] = inv;
            inv_lo[base + 3] = 0;
        }
        for lane in 0..2 {
            let idx = pixel + lane + 2;
            let alpha =
                div255_round_u16(u16::from(src_row[idx * 4 + 3]) * u16::from(mask_row[idx])) as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = lane * 4;
            alpha_hi[base] = alpha;
            alpha_hi[base + 1] = alpha;
            alpha_hi[base + 2] = alpha;
            alpha_hi[base + 3] = 255;
            inv_hi[base] = inv;
            inv_hi[base + 1] = inv;
            inv_hi[base + 2] = inv;
            inv_hi[base + 3] = 0;
        }
        let alpha_lo = unsafe { _mm_loadu_si128(alpha_lo.as_ptr() as *const __m128i) };
        let inv_lo = unsafe { _mm_loadu_si128(inv_lo.as_ptr() as *const __m128i) };
        let alpha_hi = unsafe { _mm_loadu_si128(alpha_hi.as_ptr() as *const __m128i) };
        let inv_hi = unsafe { _mm_loadu_si128(inv_hi.as_ptr() as *const __m128i) };
        let src = unsafe { _mm_loadu_si128(src_row.as_ptr().add(offset) as *const __m128i) };
        let dst = unsafe { _mm_loadu_si128(dst_row.as_ptr().add(offset) as *const __m128i) };
        let src_lo = _mm_unpacklo_epi8(src, zero);
        let src_hi = _mm_unpackhi_epi8(src, zero);
        let dst_lo = _mm_unpacklo_epi8(dst, zero);
        let dst_hi = _mm_unpackhi_epi8(dst, zero);
        let lo = _mm_srli_epi16(
            _mm_add_epi16(
                _mm_add_epi16(
                    _mm_add_epi16(
                        _mm_mullo_epi16(src_lo, alpha_lo),
                        _mm_mullo_epi16(dst_lo, inv_lo),
                    ),
                    round,
                ),
                _mm_srli_epi16(
                    _mm_add_epi16(
                        _mm_add_epi16(
                            _mm_mullo_epi16(src_lo, alpha_lo),
                            _mm_mullo_epi16(dst_lo, inv_lo),
                        ),
                        round,
                    ),
                    8,
                ),
            ),
            8,
        );
        let hi = _mm_srli_epi16(
            _mm_add_epi16(
                _mm_add_epi16(
                    _mm_add_epi16(
                        _mm_mullo_epi16(src_hi, alpha_hi),
                        _mm_mullo_epi16(dst_hi, inv_hi),
                    ),
                    round,
                ),
                _mm_srli_epi16(
                    _mm_add_epi16(
                        _mm_add_epi16(
                            _mm_mullo_epi16(src_hi, alpha_hi),
                            _mm_mullo_epi16(dst_hi, inv_hi),
                        ),
                        round,
                    ),
                    8,
                ),
            ),
            8,
        );
        let packed = _mm_packus_epi16(lo, hi);
        unsafe { _mm_storeu_si128(dst_row.as_mut_ptr().add(offset) as *mut __m128i, packed) };
        for alpha_offset in (offset + 3..offset + 16).step_by(4) {
            dst_row[alpha_offset] = 255;
        }
        pixel += 4;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        soft_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            group_alpha_255,
        );
    }
    true
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn soft_mask_opaque_dst_avx2_impl(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi16, _mm256_loadu_si256, _mm256_mullo_epi16, _mm256_packus_epi16,
        _mm256_set1_epi16, _mm256_setzero_si256, _mm256_srli_epi16, _mm256_storeu_si256,
        _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
    };

    if group_alpha_255 != 255 {
        return false;
    }
    let pixels = row_pixels(dst_row, src_row, mask_row);
    let simd_pixels = (pixels / 8) * 8;
    if simd_pixels == 0 {
        return unsafe {
            soft_mask_opaque_dst_sse2_impl(dst_row, src_row, mask_row, group_alpha_255)
        };
    }
    let round = _mm256_set1_epi16(128);
    let zero = _mm256_setzero_si256();
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let mut alpha_lo = [0i16; 16];
        let mut inv_lo = [0i16; 16];
        let mut alpha_hi = [0i16; 16];
        let mut inv_hi = [0i16; 16];
        for (slot, rel_pixel) in [0usize, 1, 4, 5].into_iter().enumerate() {
            let idx = pixel + rel_pixel;
            let alpha =
                div255_round_u16(u16::from(src_row[idx * 4 + 3]) * u16::from(mask_row[idx])) as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = slot * 4;
            alpha_lo[base] = alpha;
            alpha_lo[base + 1] = alpha;
            alpha_lo[base + 2] = alpha;
            alpha_lo[base + 3] = 255;
            inv_lo[base] = inv;
            inv_lo[base + 1] = inv;
            inv_lo[base + 2] = inv;
            inv_lo[base + 3] = 0;
        }
        for (slot, rel_pixel) in [2usize, 3, 6, 7].into_iter().enumerate() {
            let idx = pixel + rel_pixel;
            let alpha =
                div255_round_u16(u16::from(src_row[idx * 4 + 3]) * u16::from(mask_row[idx])) as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = slot * 4;
            alpha_hi[base] = alpha;
            alpha_hi[base + 1] = alpha;
            alpha_hi[base + 2] = alpha;
            alpha_hi[base + 3] = 255;
            inv_hi[base] = inv;
            inv_hi[base + 1] = inv;
            inv_hi[base + 2] = inv;
            inv_hi[base + 3] = 0;
        }
        let alpha_lo = unsafe { _mm256_loadu_si256(alpha_lo.as_ptr() as *const __m256i) };
        let inv_lo = unsafe { _mm256_loadu_si256(inv_lo.as_ptr() as *const __m256i) };
        let alpha_hi = unsafe { _mm256_loadu_si256(alpha_hi.as_ptr() as *const __m256i) };
        let inv_hi = unsafe { _mm256_loadu_si256(inv_hi.as_ptr() as *const __m256i) };
        let src = unsafe { _mm256_loadu_si256(src_row.as_ptr().add(offset) as *const __m256i) };
        let dst = unsafe { _mm256_loadu_si256(dst_row.as_ptr().add(offset) as *const __m256i) };
        let src_lo = _mm256_unpacklo_epi8(src, zero);
        let src_hi = _mm256_unpackhi_epi8(src, zero);
        let dst_lo = _mm256_unpacklo_epi8(dst, zero);
        let dst_hi = _mm256_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src_lo, alpha_lo),
                _mm256_mullo_epi16(dst_lo, inv_lo),
            ),
            round,
        );
        let hi_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src_hi, alpha_hi),
                _mm256_mullo_epi16(dst_hi, inv_hi),
            ),
            round,
        );
        let lo = _mm256_srli_epi16(
            _mm256_add_epi16(lo_mixed, _mm256_srli_epi16(lo_mixed, 8)),
            8,
        );
        let hi = _mm256_srli_epi16(
            _mm256_add_epi16(hi_mixed, _mm256_srli_epi16(hi_mixed, 8)),
            8,
        );
        let packed = _mm256_packus_epi16(lo, hi);
        unsafe { _mm256_storeu_si256(dst_row.as_mut_ptr().add(offset) as *mut __m256i, packed) };
        for alpha_offset in (offset + 3..offset + 32).step_by(4) {
            dst_row[alpha_offset] = 255;
        }
        pixel += 8;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        soft_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            group_alpha_255,
        );
    }
    true
}

#[cfg(target_arch = "aarch64")]
unsafe fn soft_mask_opaque_dst_neon_aarch64(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    if group_alpha_255 != 255 {
        return false;
    }
    let pixels = row_pixels(dst_row, src_row, mask_row);
    let simd_pixels = (pixels / 2) * 2;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(2) {
        let offset = pixel * 4;
        let eff0 = div255_round_u16(u16::from(src_row[offset + 3]) * u16::from(mask_row[pixel]));
        let eff1 =
            div255_round_u16(u16::from(src_row[offset + 7]) * u16::from(mask_row[pixel + 1]));
        neon_mix_two_opaque_dst(dst_row, src_row, offset, eff0, eff1);
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        soft_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            group_alpha_255,
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
macro_rules! guarded_row_x86 {
    ($name:ident, $kernel:ident) => {
        fn $name(dst_row: &mut [u8], src_row: &[u8]) -> bool {
            let pixels = row_pixels(dst_row, src_row, &[]);
            let mut scalar = cfg!(debug_assertions).then(|| {
                let mut copy = dst_row[..pixels * 4].to_vec();
                composite_normal_opaque_dst_scalar(&mut copy, &src_row[..pixels * 4]);
                copy
            });
            let ok = unsafe { $kernel(dst_row, src_row) };
            if ok {
                if let Some(expected) = scalar.take() {
                    debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
                }
            }
            ok
        }
    };
}

#[cfg(target_arch = "x86_64")]
guarded_row_x86!(
    guarded_composite_normal_opaque_dst_avx2_x86_64,
    composite_normal_opaque_dst_avx2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn composite_normal_opaque_dst_avx2_x86_64(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_add_epi16, _mm256_loadu_si256, _mm256_mullo_epi16, _mm256_packus_epi16,
        _mm256_set1_epi16, _mm256_setzero_si256, _mm256_srli_epi16, _mm256_storeu_si256,
        _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
    };
    let pixels = row_pixels(dst_row, src_row, &[]);
    let simd_pixels = (pixels / 8) * 8;
    if simd_pixels == 0 {
        return false;
    }
    let round = _mm256_set1_epi16(128);
    let zero = _mm256_setzero_si256();
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let mut alpha_lo = [0i16; 16];
        let mut inv_lo = [0i16; 16];
        let mut alpha_hi = [0i16; 16];
        let mut inv_hi = [0i16; 16];
        for (slot, rel_pixel) in [0usize, 1, 4, 5].into_iter().enumerate() {
            let alpha = src_row[offset + rel_pixel * 4 + 3] as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = slot * 4;
            alpha_lo[base] = alpha;
            alpha_lo[base + 1] = alpha;
            alpha_lo[base + 2] = alpha;
            alpha_lo[base + 3] = 255;
            inv_lo[base] = inv;
            inv_lo[base + 1] = inv;
            inv_lo[base + 2] = inv;
            inv_lo[base + 3] = 0;
        }
        for (slot, rel_pixel) in [2usize, 3, 6, 7].into_iter().enumerate() {
            let alpha = src_row[offset + rel_pixel * 4 + 3] as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = slot * 4;
            alpha_hi[base] = alpha;
            alpha_hi[base + 1] = alpha;
            alpha_hi[base + 2] = alpha;
            alpha_hi[base + 3] = 255;
            inv_hi[base] = inv;
            inv_hi[base + 1] = inv;
            inv_hi[base + 2] = inv;
            inv_hi[base + 3] = 0;
        }
        let alpha_lo = unsafe { _mm256_loadu_si256(alpha_lo.as_ptr() as *const __m256i) };
        let inv_lo = unsafe { _mm256_loadu_si256(inv_lo.as_ptr() as *const __m256i) };
        let alpha_hi = unsafe { _mm256_loadu_si256(alpha_hi.as_ptr() as *const __m256i) };
        let inv_hi = unsafe { _mm256_loadu_si256(inv_hi.as_ptr() as *const __m256i) };
        let src = unsafe { _mm256_loadu_si256(src_row.as_ptr().add(offset) as *const __m256i) };
        let dst = unsafe { _mm256_loadu_si256(dst_row.as_ptr().add(offset) as *const __m256i) };
        let src_lo = _mm256_unpacklo_epi8(src, zero);
        let src_hi = _mm256_unpackhi_epi8(src, zero);
        let dst_lo = _mm256_unpacklo_epi8(dst, zero);
        let dst_hi = _mm256_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src_lo, alpha_lo),
                _mm256_mullo_epi16(dst_lo, inv_lo),
            ),
            round,
        );
        let hi_mixed = _mm256_add_epi16(
            _mm256_add_epi16(
                _mm256_mullo_epi16(src_hi, alpha_hi),
                _mm256_mullo_epi16(dst_hi, inv_hi),
            ),
            round,
        );
        let lo = _mm256_srli_epi16(
            _mm256_add_epi16(lo_mixed, _mm256_srli_epi16(lo_mixed, 8)),
            8,
        );
        let hi = _mm256_srli_epi16(
            _mm256_add_epi16(hi_mixed, _mm256_srli_epi16(hi_mixed, 8)),
            8,
        );
        let packed = _mm256_packus_epi16(lo, hi);
        unsafe { _mm256_storeu_si256(dst_row.as_mut_ptr().add(offset) as *mut __m256i, packed) };
        for alpha_offset in (offset + 3..offset + 32).step_by(4) {
            dst_row[alpha_offset] = 255;
        }
        pixel += 8;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        composite_normal_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
        );
    }
    true
}

#[cfg(target_arch = "x86_64")]
guarded_row_x86!(
    guarded_composite_normal_opaque_dst_sse2_x86_64,
    composite_normal_opaque_dst_sse2_x86_64
);

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn composite_normal_opaque_dst_sse2_x86_64(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    unsafe { composite_normal_opaque_dst_sse2_impl(dst_row, src_row) }
}

#[cfg(target_arch = "x86")]
guarded_row_x86!(
    guarded_composite_normal_opaque_dst_sse2_x86,
    composite_normal_opaque_dst_sse2_x86
);

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn composite_normal_opaque_dst_sse2_x86(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    unsafe { composite_normal_opaque_dst_sse2_impl(dst_row, src_row) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn composite_normal_opaque_dst_sse2_impl(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    let pixels = row_pixels(dst_row, src_row, &[]);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let mut alpha_lo = [0i16; 8];
        let mut inv_lo = [0i16; 8];
        let mut alpha_hi = [0i16; 8];
        let mut inv_hi = [0i16; 8];
        for lane in 0..2 {
            let alpha = src_row[offset + lane * 4 + 3] as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = lane * 4;
            alpha_lo[base] = alpha;
            alpha_lo[base + 1] = alpha;
            alpha_lo[base + 2] = alpha;
            alpha_lo[base + 3] = 255;
            inv_lo[base] = inv;
            inv_lo[base + 1] = inv;
            inv_lo[base + 2] = inv;
            inv_lo[base + 3] = 0;
        }
        for lane in 0..2 {
            let alpha = src_row[offset + (lane + 2) * 4 + 3] as i16;
            let inv = 255i16.saturating_sub(alpha);
            let base = lane * 4;
            alpha_hi[base] = alpha;
            alpha_hi[base + 1] = alpha;
            alpha_hi[base + 2] = alpha;
            alpha_hi[base + 3] = 255;
            inv_hi[base] = inv;
            inv_hi[base + 1] = inv;
            inv_hi[base + 2] = inv;
            inv_hi[base + 3] = 0;
        }
        let alpha_lo = unsafe { _mm_loadu_si128(alpha_lo.as_ptr() as *const __m128i) };
        let inv_lo = unsafe { _mm_loadu_si128(inv_lo.as_ptr() as *const __m128i) };
        let alpha_hi = unsafe { _mm_loadu_si128(alpha_hi.as_ptr() as *const __m128i) };
        let inv_hi = unsafe { _mm_loadu_si128(inv_hi.as_ptr() as *const __m128i) };
        let src = unsafe { _mm_loadu_si128(src_row.as_ptr().add(offset) as *const __m128i) };
        let dst = unsafe { _mm_loadu_si128(dst_row.as_ptr().add(offset) as *const __m128i) };
        let src_lo = _mm_unpacklo_epi8(src, zero);
        let src_hi = _mm_unpackhi_epi8(src, zero);
        let dst_lo = _mm_unpacklo_epi8(dst, zero);
        let dst_hi = _mm_unpackhi_epi8(dst, zero);
        let lo_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_lo, alpha_lo),
                _mm_mullo_epi16(dst_lo, inv_lo),
            ),
            round,
        );
        let hi_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_hi, alpha_hi),
                _mm_mullo_epi16(dst_hi, inv_hi),
            ),
            round,
        );
        let lo = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
        let hi = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
        let packed = _mm_packus_epi16(lo, hi);
        unsafe { _mm_storeu_si128(dst_row.as_mut_ptr().add(offset) as *mut __m128i, packed) };
        for alpha_offset in (offset + 3..offset + 16).step_by(4) {
            dst_row[alpha_offset] = 255;
        }
        pixel += 4;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        composite_normal_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
        );
    }
    true
}

#[cfg(target_arch = "aarch64")]
fn guarded_composite_normal_opaque_dst_neon_aarch64(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let pixels = row_pixels(dst_row, src_row, &[]);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        composite_normal_opaque_dst_scalar(&mut copy, &src_row[..pixels * 4]);
        copy
    });
    // SAFETY: AArch64 guarantees Advanced SIMD.
    let ok = unsafe { composite_normal_opaque_dst_neon_aarch64(dst_row, src_row) };
    if let Some(expected) = scalar.take() {
        debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
    }
    ok
}

#[cfg(target_arch = "aarch64")]
unsafe fn composite_normal_opaque_dst_neon_aarch64(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let pixels = row_pixels(dst_row, src_row, &[]);
    let simd_pixels = (pixels / 2) * 2;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(2) {
        let offset = pixel * 4;
        let eff0 = u16::from(src_row[offset + 3]);
        let eff1 = u16::from(src_row[offset + 7]);
        neon_mix_two_opaque_dst(dst_row, src_row, offset, eff0, eff1);
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        composite_normal_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &src_row[offset..pixels * 4],
        );
    }
    true
}

#[cfg(target_arch = "aarch64")]
fn neon_mix_two_opaque_dst(
    dst_row: &mut [u8],
    src_row: &[u8],
    offset: usize,
    eff0: u16,
    eff1: u16,
) {
    use std::arch::aarch64::{
        vaddq_u16, vdupq_n_u16, vld1q_u16, vmulq_u16, vqmovn_u16, vshrq_n_u16, vst1_u8,
    };
    let inv0 = 255_u16.saturating_sub(eff0);
    let inv1 = 255_u16.saturating_sub(eff1);
    let src = [
        u16::from(src_row[offset]),
        u16::from(src_row[offset + 1]),
        u16::from(src_row[offset + 2]),
        255,
        u16::from(src_row[offset + 4]),
        u16::from(src_row[offset + 5]),
        u16::from(src_row[offset + 6]),
        255,
    ];
    let dst = [
        u16::from(dst_row[offset]),
        u16::from(dst_row[offset + 1]),
        u16::from(dst_row[offset + 2]),
        255,
        u16::from(dst_row[offset + 4]),
        u16::from(dst_row[offset + 5]),
        u16::from(dst_row[offset + 6]),
        255,
    ];
    let eff = [eff0, eff0, eff0, 255, eff1, eff1, eff1, 255];
    let inv = [inv0, inv0, inv0, 0, inv1, inv1, inv1, 0];
    let src = unsafe { vld1q_u16(src.as_ptr()) };
    let dst = unsafe { vld1q_u16(dst.as_ptr()) };
    let eff = unsafe { vld1q_u16(eff.as_ptr()) };
    let inv = unsafe { vld1q_u16(inv.as_ptr()) };
    let mixed = vaddq_u16(
        vaddq_u16(vmulq_u16(src, eff), vmulq_u16(dst, inv)),
        vdupq_n_u16(128),
    );
    let out = vshrq_n_u16(vaddq_u16(mixed, vshrq_n_u16(mixed, 8)), 8);
    let packed = vqmovn_u16(out);
    unsafe { vst1_u8(dst_row.as_mut_ptr().add(offset), packed) };
    dst_row[offset + 3] = 255;
    dst_row[offset + 7] = 255;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source_row(pixels: usize) -> Vec<u8> {
        (0..pixels)
            .flat_map(|index| {
                let index = index as u8;
                [
                    index.wrapping_mul(47).wrapping_add(3),
                    index.wrapping_mul(29).wrapping_add(17),
                    index.wrapping_mul(13).wrapping_add(91),
                    index.wrapping_mul(53).wrapping_add(1),
                ]
            })
            .collect()
    }

    fn opaque_destination(pixels: usize) -> Vec<u8> {
        (0..pixels)
            .flat_map(|index| {
                let index = index as u8;
                [
                    index.wrapping_mul(19).wrapping_add(11),
                    index.wrapping_mul(31).wrapping_add(7),
                    index.wrapping_mul(43).wrapping_add(5),
                    255,
                ]
            })
            .collect()
    }

    #[test]
    fn public_kernels_match_scalar_or_cleanly_decline_unaligned_rows() {
        for pixels in [3usize, 4, 5, 8, 9, 16, 17] {
            let len = pixels * 4;

            let mut fill_backing = vec![0u8; len + 2];
            let fill = &mut fill_backing[1..=len];
            let fill_before = fill.to_vec();
            let mut fill_expected = fill_before.clone();
            let color = [23, 101, 211, 255];
            let expected_handled = fill_opaque_run_scalar(&mut fill_expected, color);
            let handled = fill_opaque_run(fill, color);
            if handled {
                assert_eq!(fill, fill_expected.as_slice(), "fill pixels={pixels}");
            } else {
                assert_eq!(
                    fill,
                    fill_before.as_slice(),
                    "fill fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut blend_backing = vec![0u8; len + 2];
            blend_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
            let blend = &mut blend_backing[1..=len];
            let blend_before = blend.to_vec();
            let mut blend_expected = blend_before.clone();
            let color = [151, 73, 29, 127];
            let _ = blend_normal_opaque_dst_scalar(&mut blend_expected, color);
            let handled = blend_normal_opaque_destination(blend, color);
            if handled {
                assert_eq!(blend, blend_expected.as_slice(), "blend pixels={pixels}");
            } else {
                assert_eq!(
                    blend,
                    blend_before.as_slice(),
                    "blend fallback pixels={pixels}"
                );
            }

            let src = source_row(pixels);
            let mask: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 17, 127, 255][index % 4])
                .collect();
            let mut soft_backing = vec![0u8; len + 2];
            soft_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
            let soft = &mut soft_backing[1..=len];
            let soft_before = soft.to_vec();
            let mut soft_expected = soft_before.clone();
            let _ = soft_mask_opaque_dst_scalar(&mut soft_expected, &src, &mask, 255);
            let handled = composite_soft_mask_opaque_destination(soft, &src, &mask, 255);
            if handled {
                assert_eq!(soft, soft_expected.as_slice(), "soft mask pixels={pixels}");
            } else {
                assert_eq!(
                    soft,
                    soft_before.as_slice(),
                    "soft fallback pixels={pixels}"
                );
            }

            let mut composite_backing = vec![0u8; len + 2];
            composite_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
            let composite = &mut composite_backing[1..=len];
            let composite_before = composite.to_vec();
            let mut composite_expected = composite_before.clone();
            let _ = composite_normal_opaque_dst_scalar(&mut composite_expected, &src);
            let handled = composite_normal_opaque_destination(composite, &src);
            if handled {
                assert_eq!(
                    composite,
                    composite_expected.as_slice(),
                    "source-over pixels={pixels}"
                );
            } else {
                assert_eq!(
                    composite,
                    composite_before.as_slice(),
                    "source-over fallback pixels={pixels}"
                );
            }
        }
    }
}
