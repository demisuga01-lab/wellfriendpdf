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
    Ssse3,
    Avx2,
    Neon,
    WasmSimd,
}

impl SimdBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            SimdBackend::Scalar => "scalar",
            SimdBackend::Sse2 => "sse2",
            SimdBackend::Ssse3 => "ssse3",
            SimdBackend::Avx2 => "avx2",
            SimdBackend::Neon => "neon",
            SimdBackend::WasmSimd => "wasm_simd",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparableBlendMode {
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
}

pub fn active_backend() -> SimdBackend {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("avx2") {
            return SimdBackend::Avx2;
        }
        if std::is_x86_feature_detected!("ssse3") {
            return SimdBackend::Ssse3;
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
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return SimdBackend::WasmSimd;
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
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
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_fill_opaque_run_wasm_simd128(slice, color);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        fill_opaque_run_scalar(slice, color)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn fill_alpha_run(slice: &mut [u8], color: [u8; 4]) -> bool {
    if color[3] == 0 || color[3] == 255 {
        return false;
    }
    blend_normal_opaque_destination(slice, color)
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
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_blend_normal_opaque_dst_wasm_simd128(slice, color);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        blend_normal_opaque_dst_scalar(slice, color)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn blend_alpha_mask_opaque_destination(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    if pixels < 4 || color[3] == 0 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_blend_alpha_mask_opaque_dst_sse2(dst_row, mask_row, color);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_blend_alpha_mask_opaque_dst_wasm_simd128(dst_row, mask_row, color);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        blend_alpha_mask_opaque_dst_scalar(dst_row, mask_row, color)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn blend_alpha_mask_normal(dst_row: &mut [u8], mask_row: &[u8], color: [u8; 4]) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    if pixels < 4 || color[3] == 0 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_blend_alpha_mask_normal_sse2(dst_row, mask_row, color);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_blend_alpha_mask_normal_wasm_simd128(dst_row, mask_row, color);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        blend_alpha_mask_normal_scalar(dst_row, mask_row, color)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn multiply_alpha_rows(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    let pixels = alpha_row.len().min(mask_row.len());
    if pixels < 16 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        #[cfg(target_arch = "x86_64")]
        {
            if std::is_x86_feature_detected!("avx2") {
                return guarded_multiply_alpha_rows_avx2_x86_64(alpha_row, mask_row);
            }
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_multiply_alpha_rows_sse2(alpha_row, mask_row);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_multiply_alpha_rows_wasm_simd128(alpha_row, mask_row);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        multiply_alpha_rows_scalar(alpha_row, mask_row)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn blend_separable_opaque_destination(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    if dst_row.len() < 8 || color[3] != 255 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_blend_separable_opaque_dst_sse2(dst_row, color, blend_mode);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_blend_separable_opaque_dst_wasm_simd128(dst_row, color, blend_mode);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        blend_separable_opaque_dst_scalar(dst_row, color, blend_mode)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = blend_mode;
        false
    }
}

pub fn premultiply_rgba(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_premultiply_rgba_sse2(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_premultiply_rgba_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        premultiply_rgba_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn premultiply_bgra8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_premultiply_bgra8_sse2(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_premultiply_bgra8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        premultiply_bgra8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn unpremultiply_rgba(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_unpremultiply_rgba_sse2(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_unpremultiply_rgba_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        unpremultiply_rgba_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn copy_rgba(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_copy_rgba_sse2(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_copy_rgba_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        copy_rgba_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_opaque_rgba(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_rgba_to_opaque_rgba_sse2(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_opaque_rgba_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_opaque_rgba_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn reverse_4byte_words_in_place(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    if words < 4 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("avx2") {
            return guarded_reverse_4byte_words_avx2_x86_64(slice);
        }
        if std::is_x86_feature_detected!("sse2") {
            return guarded_reverse_4byte_words_sse2_x86_64(slice);
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_reverse_4byte_words_sse2_x86(slice);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        return guarded_reverse_4byte_words_neon_aarch64(slice);
    }
    #[cfg(all(target_arch = "arm", target_feature = "neon"))]
    {
        return guarded_reverse_4byte_words_neon_arm(slice);
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_reverse_4byte_words_wasm_simd128(slice);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        reverse_4byte_words_scalar(slice)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn composite_soft_mask_opaque_destination(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    if group_alpha_255 > 255 || mask_row.is_empty() {
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
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_soft_mask_opaque_dst_wasm_simd128(
            dst_row,
            src_row,
            mask_row,
            group_alpha_255,
        );
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        soft_mask_opaque_dst_scalar(dst_row, src_row, mask_row, group_alpha_255)
    }
    #[cfg(not(target_arch = "wasm32"))]
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
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_composite_normal_opaque_dst_wasm_simd128(dst_row, src_row);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        composite_normal_opaque_dst_scalar(dst_row, src_row)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn flatten_opaque_background(data: &mut [u8], background: [u8; 4]) -> bool {
    if data.len() < 16 || background[3] != 255 {
        return false;
    }
    #[cfg(target_arch = "x86_64")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_flatten_opaque_background_sse2_x86_64(data, background);
        }
    }
    #[cfg(target_arch = "x86")]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_flatten_opaque_background_sse2_x86(data, background);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_flatten_opaque_background_wasm_simd128(data, background);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        flatten_opaque_background_scalar(data, background)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_gray8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_gray_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_gray8_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_gray8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_gray8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_gray_rgb8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_gray_rgb8_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_gray_rgb8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_gray_rgb8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_gray_rgba8(
    source: &[u8],
    destination: &mut [u8],
    _force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_gray_rgba8_ssse3(source, destination, _force_opaque_alpha);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_gray_rgba8_wasm_simd128(source, destination, _force_opaque_alpha);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_gray_rgba8_scalar(source, destination, _force_opaque_alpha)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_gray_bgra8(
    source: &[u8],
    destination: &mut [u8],
    _force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_gray_bgra8_ssse3(source, destination, _force_opaque_alpha);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_gray_bgra8_wasm_simd128(source, destination, _force_opaque_alpha);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_gray_bgra8_scalar(source, destination, _force_opaque_alpha)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_premultiplied_gray_rgba8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_premultiplied_gray_rgba8_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_premultiplied_gray_rgba8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_premultiplied_gray_rgba8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_premultiplied_gray_bgra8(source: &[u8], destination: &mut [u8]) -> bool {
    rgba_to_premultiplied_gray_rgba8(source, destination)
}

pub fn rgba_to_rgb8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_rgb8_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_rgb8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_rgb8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgb8_to_opaque_rgba(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgb_rgba_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgb8_to_opaque_rgba_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgb8_to_opaque_rgba_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgb8_to_opaque_rgba_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_bgr8(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("ssse3") {
            return guarded_rgba_to_bgr8_ssse3(source, destination);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_bgr8_wasm_simd128(source, destination);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_bgr8_scalar(source, destination)
    }
    #[cfg(not(target_arch = "wasm32"))]
    false
}

pub fn rgba_to_bgra8(source: &[u8], destination: &mut [u8], _force_opaque_alpha: bool) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels < 4 {
        return false;
    }
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        if std::is_x86_feature_detected!("sse2") {
            return guarded_rgba_to_bgra8_sse2(source, destination, _force_opaque_alpha);
        }
    }
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        return guarded_rgba_to_bgra8_wasm_simd128(source, destination, _force_opaque_alpha);
    }
    #[cfg(all(target_arch = "wasm32", not(target_feature = "simd128")))]
    {
        rgba_to_bgra8_scalar(source, destination, _force_opaque_alpha)
    }
    #[cfg(not(target_arch = "wasm32"))]
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

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn blend_alpha_mask_opaque_dst_scalar(dst_row: &mut [u8], mask_row: &[u8], color: [u8; 4]) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    if pixels == 0 || color[3] == 0 {
        return false;
    }
    let color_alpha = u16::from(color[3]);
    for (dst, mask) in dst_row[..pixels * 4]
        .chunks_exact_mut(4)
        .zip(mask_row.iter().copied())
    {
        let alpha = ((color_alpha * u16::from(mask) + 127) / 255).min(255);
        if alpha == 0 {
            continue;
        }
        let inv = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            let mixed = u16::from(color[channel]) * alpha + u16::from(dst[channel]) * inv + 128;
            dst[channel] = ((mixed + (mixed >> 8)) >> 8).min(255) as u8;
        }
        dst[3] = 255;
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn blend_alpha_mask_normal_scalar(dst_row: &mut [u8], mask_row: &[u8], color: [u8; 4]) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    if pixels == 0 || color[3] == 0 {
        return false;
    }
    let color_alpha = u16::from(color[3]);
    for (dst, mask) in dst_row[..pixels * 4]
        .chunks_exact_mut(4)
        .zip(mask_row.iter().copied())
    {
        let src_alpha_byte = ((color_alpha * u16::from(mask) + 127) / 255).min(255) as u8;
        blend_alpha_mask_normal_pixel_scalar(dst, color, src_alpha_byte);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn multiply_alpha_rows_scalar(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    let pixels = alpha_row.len().min(mask_row.len());
    if pixels == 0 {
        return false;
    }
    for (alpha, mask) in alpha_row
        .iter_mut()
        .zip(mask_row.iter().copied())
        .take(pixels)
    {
        *alpha = div255_round_u16(u16::from(*alpha) * u16::from(mask)).min(255) as u8;
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn blend_alpha_mask_normal_pixel_scalar(dst: &mut [u8], color: [u8; 4], src_alpha_byte: u8) {
    if dst.len() < 4 || src_alpha_byte == 0 {
        return;
    }
    let src_a = f32::from(src_alpha_byte) / 255.0;
    let dst_a = f32::from(dst[3]) / 255.0;
    let out_a = src_a + dst_a * (1.0 - src_a);
    if out_a < 1e-6 {
        dst[..4].copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    let inv_alpha = 1.0 / out_a;
    for channel in 0..3 {
        let src = f32::from(color[channel]) / 255.0;
        let old = f32::from(dst[channel]) / 255.0;
        let out = (src * src_a + old * dst_a * (1.0 - src_a)) * inv_alpha;
        dst[channel] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).clamp(0.0, 255.0) as u8;
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn blend_separable_opaque_dst_scalar(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    if dst_row.len() < 4 || color[3] != 255 {
        return false;
    }
    for dst in dst_row.chunks_exact_mut(4) {
        for channel in 0..3 {
            dst[channel] = blend_separable_channel(color[channel], dst[channel], blend_mode);
        }
        dst[3] = 255;
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn blend_separable_channel(src: u8, dst: u8, blend_mode: SeparableBlendMode) -> u8 {
    match blend_mode {
        SeparableBlendMode::Multiply => div255_round_u16(u16::from(src) * u16::from(dst)) as u8,
        SeparableBlendMode::Screen => {
            let src = u16::from(src);
            let dst = u16::from(dst);
            (src + dst - div255_round_u16(src * dst)).min(255) as u8
        }
        SeparableBlendMode::Overlay => hard_light_channel(dst, src),
        SeparableBlendMode::Darken => src.min(dst),
        SeparableBlendMode::Lighten => src.max(dst),
        SeparableBlendMode::ColorDodge => color_dodge_channel(src, dst),
        SeparableBlendMode::ColorBurn => color_burn_channel(src, dst),
        SeparableBlendMode::HardLight => hard_light_channel(src, dst),
        SeparableBlendMode::SoftLight => soft_light_channel(src, dst),
        SeparableBlendMode::Difference => src.abs_diff(dst),
        SeparableBlendMode::Exclusion => exclusion_channel(src, dst),
    }
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn hard_light_channel(src: u8, dst: u8) -> u8 {
    let src = normalized_channel(src);
    let dst = normalized_channel(dst);
    let out = if src <= 0.5 {
        2.0 * src * dst
    } else {
        1.0 - 2.0 * (1.0 - src) * (1.0 - dst)
    };
    denormalize_channel(out)
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn color_dodge_channel(src: u8, dst: u8) -> u8 {
    let src = normalized_channel(src);
    let dst = normalized_channel(dst);
    denormalize_channel(if dst <= 0.0 {
        0.0
    } else if src >= 1.0 {
        1.0
    } else {
        (dst / (1.0 - src)).min(1.0)
    })
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn color_burn_channel(src: u8, dst: u8) -> u8 {
    let src = normalized_channel(src);
    let dst = normalized_channel(dst);
    denormalize_channel(if dst >= 1.0 {
        1.0
    } else if src <= 0.0 {
        0.0
    } else {
        1.0 - ((1.0 - dst) / src).min(1.0)
    })
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn soft_light_channel(src: u8, dst: u8) -> u8 {
    let src = normalized_channel(src);
    let dst = normalized_channel(dst);
    let out = if src <= 0.5 {
        dst - (1.0 - 2.0 * src) * dst * (1.0 - dst)
    } else {
        let d = if dst <= 0.25 {
            ((16.0 * dst - 12.0) * dst + 4.0) * dst
        } else {
            dst.sqrt()
        };
        dst + (2.0 * src - 1.0) * (d - dst)
    };
    denormalize_channel(out)
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn normalized_channel(value: u8) -> f32 {
    f32::from(value) / 255.0
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn denormalize_channel(value: f32) -> u8 {
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn exclusion_channel(src: u8, dst: u8) -> u8 {
    let src = u32::from(src);
    let dst = u32::from(dst);
    let product = src.saturating_mul(dst);
    let doubled_scaled = (product.saturating_mul(4).saturating_add(255)) / 510;
    src.saturating_add(dst)
        .saturating_sub(doubled_scaled)
        .min(255) as u8
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn premultiply_rgba_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (src, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let alpha = u16::from(src[3]);
        dst[0] = premultiply_channel(src[0], alpha);
        dst[1] = premultiply_channel(src[1], alpha);
        dst[2] = premultiply_channel(src[2], alpha);
        dst[3] = src[3];
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn premultiply_bgra8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (src, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let alpha = u16::from(src[3]);
        dst[0] = premultiply_channel(src[2], alpha);
        dst[1] = premultiply_channel(src[1], alpha);
        dst[2] = premultiply_channel(src[0], alpha);
        dst[3] = src[3];
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn unpremultiply_rgba_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (src, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let alpha = src[3];
        if alpha == 0 {
            dst.copy_from_slice(&[0, 0, 0, 0]);
        } else if alpha == 255 {
            dst.copy_from_slice(src);
        } else {
            let alpha = u16::from(alpha);
            dst[0] = unpremultiply_channel(src[0], alpha);
            dst[1] = unpremultiply_channel(src[1], alpha);
            dst[2] = unpremultiply_channel(src[2], alpha);
            dst[3] = src[3];
        }
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn copy_rgba_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    destination[..pixels * 4].copy_from_slice(&source[..pixels * 4]);
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_opaque_rgba_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        dst.copy_from_slice(&[rgba[0], rgba[1], rgba[2], 255]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64",
    target_arch = "aarch64",
    all(target_arch = "arm", target_feature = "neon")
))]
fn reverse_4byte_words_scalar(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    if words == 0 {
        return false;
    }
    for word in slice[..words * 4].chunks_exact_mut(4) {
        word.reverse();
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
        let eff = soft_mask_effective_alpha(src[3], mask, group_alpha_255);
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

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn flatten_opaque_background_scalar(data: &mut [u8], background: [u8; 4]) -> bool {
    let pixels = data.len() / 4;
    if pixels == 0 || background[3] != 255 {
        return false;
    }
    for chunk in data[..pixels * 4].chunks_exact_mut(4) {
        let src_a = chunk[3];
        if src_a == 255 {
            chunk[3] = 255;
            continue;
        }
        if src_a == 0 {
            chunk.copy_from_slice(&background);
            continue;
        }
        let alpha = u16::from(src_a);
        let inv_alpha = 255_u16.saturating_sub(alpha);
        for channel in 0..3 {
            chunk[channel] = ((u16::from(chunk[channel]) * alpha
                + u16::from(background[channel]) * inv_alpha
                + 127)
                / 255) as u8;
        }
        chunk[3] = 255;
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_gray8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_gray_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, gray) in source
        .chunks_exact(4)
        .take(pixels)
        .zip(destination.iter_mut())
    {
        *gray = rgba_luma_byte(rgba[0], rgba[1], rgba[2]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_gray_rgb8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 3].chunks_exact_mut(3))
    {
        let gray = rgba_luma_byte(rgba[0], rgba[1], rgba[2]);
        dst.copy_from_slice(&[gray, gray, gray]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_gray_rgba8_scalar(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let gray = rgba_luma_byte(rgba[0], rgba[1], rgba[2]);
        let alpha = if force_opaque_alpha { 255 } else { rgba[3] };
        dst.copy_from_slice(&[gray, gray, gray, alpha]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_gray_bgra8_scalar(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let gray = rgba_luma_byte(rgba[0], rgba[1], rgba[2]);
        let alpha = if force_opaque_alpha { 255 } else { rgba[3] };
        dst.copy_from_slice(&[gray, gray, gray, alpha]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_premultiplied_gray_rgba8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, dst) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        let alpha = u16::from(rgba[3]);
        let gray = premultiply_channel(rgba_luma_byte(rgba[0], rgba[1], rgba[2]), alpha);
        dst.copy_from_slice(&[gray, gray, gray, rgba[3]]);
    }
    true
}

#[cfg(test)]
fn rgba_to_premultiplied_gray_bgra8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    rgba_to_premultiplied_gray_rgba8_scalar(source, destination)
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_rgb8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, rgb) in source
        .chunks_exact(4)
        .take(pixels)
        .zip(destination[..pixels * 3].chunks_exact_mut(3))
    {
        rgb.copy_from_slice(&rgba[..3]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgb8_to_opaque_rgba_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgb_rgba_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgb, rgba) in source[..pixels * 3]
        .chunks_exact(3)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        rgba.copy_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_bgr8_scalar(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, bgr) in source
        .chunks_exact(4)
        .take(pixels)
        .zip(destination[..pixels * 3].chunks_exact_mut(3))
    {
        bgr.copy_from_slice(&[rgba[2], rgba[1], rgba[0]]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
fn rgba_to_bgra8_scalar(source: &[u8], destination: &mut [u8], force_opaque_alpha: bool) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    if pixels == 0 {
        return false;
    }
    for (rgba, bgra) in source[..pixels * 4]
        .chunks_exact(4)
        .zip(destination[..pixels * 4].chunks_exact_mut(4))
    {
        bgra.copy_from_slice(&[
            rgba[2],
            rgba[1],
            rgba[0],
            if force_opaque_alpha { 255 } else { rgba[3] },
        ]);
    }
    true
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn rgba_luma_byte(red: u8, green: u8, blue: u8) -> u8 {
    ((u16::from(red) * 77 + u16::from(green) * 150 + u16::from(blue) * 29 + 128) >> 8) as u8
}

fn alpha_mask_pixels(dst_row: &[u8], mask_row: &[u8]) -> usize {
    (dst_row.len() / 4).min(mask_row.len())
}

fn rgba_row_pixels(source: &[u8], destination: &[u8]) -> usize {
    (source.len() / 4).min(destination.len() / 4)
}

fn row_pixels(dst_row: &[u8], src_row: &[u8], mask_row: &[u8]) -> usize {
    let pixels = dst_row.len().min(src_row.len()) / 4;
    if mask_row.is_empty() {
        pixels
    } else {
        pixels.min(mask_row.len())
    }
}

fn rgba_gray_pixels(source: &[u8], destination: &[u8]) -> usize {
    (source.len() / 4).min(destination.len())
}

fn rgba_rgb_pixels(source: &[u8], destination: &[u8]) -> usize {
    (source.len() / 4).min(destination.len() / 3)
}

fn rgb_rgba_pixels(source: &[u8], destination: &[u8]) -> usize {
    (source.len() / 3).min(destination.len() / 4)
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn div255_round_u16(value: u16) -> u16 {
    let adjusted = value.saturating_add(128);
    (adjusted + (adjusted >> 8)) >> 8
}

#[inline]
fn soft_mask_effective_alpha(src_alpha: u8, mask: u8, group_alpha_255: u16) -> u16 {
    let product = u32::from(src_alpha)
        .saturating_mul(u32::from(mask))
        .saturating_mul(u32::from(group_alpha_255.min(255)));
    ((product + 32_512) / 65_025).min(255) as u16
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn premultiply_channel(value: u8, alpha: u16) -> u8 {
    div255_round_u16(u16::from(value) * alpha).min(255) as u8
}

#[cfg(any(
    test,
    target_arch = "wasm32",
    target_arch = "x86",
    target_arch = "x86_64"
))]
#[inline]
fn unpremultiply_channel(value: u8, alpha: u16) -> u8 {
    ((u16::from(value) * 255 + (alpha / 2)) / alpha).min(255) as u8
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_gray8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_gray_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = destination[..pixels].to_vec();
        rgba_to_gray8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: this function is compiled only for wasm32 with simd128 enabled.
    let ok = unsafe { rgba_to_gray8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_gray8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{v128, v128_load};

    let pixels = rgba_gray_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let rgba = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let gray = wasm_luma4_bytes(rgba);
        unsafe { wasm_store_low_4_bytes(destination.as_mut_ptr().add(pixel), gray) };
    }

    if simd_pixels < pixels {
        rgba_to_gray8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels..pixels],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_luma4_bytes(raw: std::arch::wasm32::v128) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        i16x8_add, i16x8_mul, i16x8_splat, i32x4_splat, i8x16_shuffle, u16x8_extend_low_u8x16,
        u16x8_shr, u8x16_narrow_i16x8,
    };

    let zero = i32x4_splat(0);
    let red =
        i8x16_shuffle::<0, 4, 8, 12, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero);
    let green =
        i8x16_shuffle::<1, 5, 9, 13, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero);
    let blue =
        i8x16_shuffle::<2, 6, 10, 14, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero);
    let red = u16x8_extend_low_u8x16(red);
    let green = u16x8_extend_low_u8x16(green);
    let blue = u16x8_extend_low_u8x16(blue);
    let weighted = i16x8_add(
        i16x8_add(
            i16x8_mul(red, i16x8_splat(77)),
            i16x8_mul(green, i16x8_splat(150)),
        ),
        i16x8_mul(blue, i16x8_splat(29)),
    );
    let gray = u16x8_shr(i16x8_add(weighted, i16x8_splat(128)), 8);
    u8x16_narrow_i16x8(gray, i16x8_splat(0))
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
unsafe fn wasm_store_low_12_bytes(destination: *mut u8, value: std::arch::wasm32::v128) {
    use std::arch::wasm32::{v128_store32_lane, v128_store64_lane};

    unsafe {
        v128_store64_lane::<0>(value, destination as *mut u64);
        v128_store32_lane::<2>(value, destination.add(8) as *mut u32);
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
unsafe fn wasm_store_low_4_bytes(destination: *mut u8, value: std::arch::wasm32::v128) {
    use std::arch::wasm32::v128_store32_lane;

    unsafe {
        v128_store32_lane::<0>(value, destination as *mut u32);
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_gray_rgb8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = destination[..pixels * 3].to_vec();
        rgba_to_gray_rgb8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_gray_rgb8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_gray_rgba8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_gray_rgba8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_gray_rgba8_wasm_simd128(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_gray_bgra8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_gray_bgra8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_gray_bgra8_wasm_simd128(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_gray_rgb8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let gray = wasm_luma4_bytes(raw);
        let expanded = i8x16_shuffle::<0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 0, 0, 0, 0>(gray, gray);
        unsafe { wasm_store_low_12_bytes(destination.as_mut_ptr().add(pixel * 3), expanded) };
    }
    if simd_pixels < pixels {
        rgba_to_gray_rgb8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_gray_rgba8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let opaque = [255u8; 16];
    let opaque_raw = unsafe { v128_load(opaque.as_ptr() as *const v128) };
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let gray = wasm_luma4_bytes(raw);
        let out = if force_opaque_alpha {
            i8x16_shuffle::<0, 0, 0, 16, 1, 1, 1, 17, 2, 2, 2, 18, 3, 3, 3, 19>(gray, opaque_raw)
        } else {
            i8x16_shuffle::<0, 0, 0, 19, 1, 1, 1, 23, 2, 2, 2, 27, 3, 3, 3, 31>(gray, raw)
        };
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, out) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_gray_rgba8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
            force_opaque_alpha,
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_gray_bgra8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    unsafe { rgba_to_gray_rgba8_wasm_simd128(source, destination, force_opaque_alpha) }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_premultiplied_gray_rgba8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_premultiplied_gray_rgba8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_premultiplied_gray_rgba8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_premultiplied_gray_rgba8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
) -> bool {
    use std::arch::wasm32::{i16x8_splat, i8x16_shuffle, v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let round = i16x8_splat(128);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let gray = wasm_luma4_bytes(raw);
        let gray_rgba =
            i8x16_shuffle::<0, 0, 0, 19, 1, 1, 1, 23, 2, 2, 2, 27, 3, 3, 3, 31>(gray, raw);
        let out = wasm_premultiply_rgba_group(
            gray_rgba,
            source[offset + 3] as i16,
            source[offset + 7] as i16,
            source[offset + 11] as i16,
            source[offset + 15] as i16,
            round,
        );
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, out) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_premultiplied_gray_rgba8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_rgb8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = destination[..pixels * 3].to_vec();
        rgba_to_rgb8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_rgb8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_bgr8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = destination[..pixels * 3].to_vec();
        rgba_to_bgr8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_bgr8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_bgra8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_bgra8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_bgra8_wasm_simd128(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_rgb8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 4;
        let dst_offset = pixel * 3;
        let rgba = unsafe { v128_load(source.as_ptr().add(src_offset) as *const v128) };
        let rgb = i8x16_shuffle::<0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, 0, 0, 0, 0>(rgba, rgba);
        unsafe { wasm_store_low_12_bytes(destination.as_mut_ptr().add(dst_offset), rgb) };
    }
    if simd_pixels < pixels {
        rgba_to_rgb8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgb8_to_opaque_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgb_rgba_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgb8_to_opaque_rgba_scalar(&source[..pixels * 3], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgb8_to_opaque_rgba_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgb8_to_opaque_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{
        i8x16_shuffle, v128, v128_load, v128_load32_zero, v128_load64_zero, v128_store,
    };

    let pixels = rgb_rgba_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let opaque = [255u8; 16];
    let opaque_raw = unsafe { v128_load(opaque.as_ptr() as *const v128) };
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 3;
        let dst_offset = pixel * 4;
        let rgb_low = unsafe { v128_load64_zero(source.as_ptr().add(src_offset) as *const u64) };
        let rgb_high =
            unsafe { v128_load32_zero(source.as_ptr().add(src_offset + 8) as *const u32) };
        let raw = i8x16_shuffle::<0, 1, 2, 3, 4, 5, 6, 7, 16, 17, 18, 19, 16, 16, 16, 16>(
            rgb_low, rgb_high,
        );
        let out =
            i8x16_shuffle::<0, 1, 2, 16, 3, 4, 5, 17, 6, 7, 8, 18, 9, 10, 11, 19>(raw, opaque_raw);
        unsafe { v128_store(destination.as_mut_ptr().add(dst_offset) as *mut v128, out) };
    }
    if simd_pixels < pixels {
        rgb8_to_opaque_rgba_scalar(
            &source[simd_pixels * 3..pixels * 3],
            &mut destination[simd_pixels * 4..pixels * 4],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_bgr8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 4;
        let dst_offset = pixel * 3;
        let rgba = unsafe { v128_load(source.as_ptr().add(src_offset) as *const v128) };
        let bgr = i8x16_shuffle::<2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, 0, 0, 0, 0>(rgba, rgba);
        unsafe { wasm_store_low_12_bytes(destination.as_mut_ptr().add(dst_offset), bgr) };
    }
    if simd_pixels < pixels {
        rgba_to_bgr8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_bgra8_wasm_simd128(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let opaque = [255u8; 16];
    let opaque_raw = unsafe { v128_load(opaque.as_ptr() as *const v128) };
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let rgba = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let bgra = if force_opaque_alpha {
            i8x16_shuffle::<2, 1, 0, 16, 6, 5, 4, 17, 10, 9, 8, 18, 14, 13, 12, 19>(
                rgba, opaque_raw,
            )
        } else {
            i8x16_shuffle::<2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15>(rgba, rgba)
        };
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, bgra) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_bgra8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
            force_opaque_alpha,
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_fill_opaque_run_wasm_simd128(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        fill_opaque_run_scalar(&mut copy, color);
        copy
    });
    // SAFETY: this function is compiled only for wasm simd128 targets.
    let ok = unsafe { fill_opaque_run_wasm_simd128(slice, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(slice, expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn fill_opaque_run_wasm_simd128(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::wasm32::{i32x4_splat, v128, v128_store};

    let fill = i32x4_splat(i32::from_le_bytes(color));
    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        // SAFETY: offset stays below the rounded-down slice length and storeu
        // semantics accept an arbitrary byte alignment.
        unsafe {
            v128_store(slice.as_mut_ptr().add(offset) as *mut v128, fill);
        }
        offset += 16;
    }
    if offset < slice.len() {
        fill_opaque_run_scalar(&mut slice[offset..], color);
    }
    true
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSE2: premultiply RGBA/BGRA
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86")]
type NativeM128i = std::arch::x86::__m128i;
#[cfg(target_arch = "x86_64")]
type NativeM128i = std::arch::x86_64::__m128i;
#[cfg(target_arch = "x86")]
type NativeM128 = std::arch::x86::__m128;
#[cfg(target_arch = "x86_64")]
type NativeM128 = std::arch::x86_64::__m128;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn native_store_low_12_bytes_sse2(destination: *mut u8, value: NativeM128i) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_cvtsi128_si32, _mm_srli_si128, _mm_storel_epi64};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{_mm_cvtsi128_si32, _mm_srli_si128, _mm_storel_epi64};

    unsafe {
        _mm_storel_epi64(destination as *mut NativeM128i, value);
        std::ptr::write_unaligned(
            destination.add(8) as *mut u32,
            _mm_cvtsi128_si32(_mm_srli_si128::<8>(value)) as u32,
        );
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn native_store_low_4_bytes_sse2(destination: *mut u8, value: NativeM128i) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::_mm_cvtsi128_si32;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::_mm_cvtsi128_si32;

    unsafe {
        std::ptr::write_unaligned(destination as *mut u32, _mm_cvtsi128_si32(value) as u32);
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn native_load_low_12_bytes_sse2(source: *const u8) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_cvtsi32_si128, _mm_loadl_epi64, _mm_or_si128, _mm_slli_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{_mm_cvtsi32_si128, _mm_loadl_epi64, _mm_or_si128, _mm_slli_si128};

    let low = unsafe { _mm_loadl_epi64(source as *const NativeM128i) };
    let high = _mm_cvtsi32_si128(unsafe { std::ptr::read_unaligned(source.add(8) as *const i32) });
    _mm_or_si128(low, _mm_slli_si128::<8>(high))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_premultiply_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        premultiply_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { premultiply_rgba_sse2(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn premultiply_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let premultiplied = unsafe {
            premultiply_rgba_group_sse2(
                raw,
                i16::from(source[offset + 3]),
                i16::from(source[offset + 7]),
                i16::from(source[offset + 11]),
                i16::from(source[offset + 15]),
            )
        };
        unsafe {
            _mm_storeu_si128(
                destination.as_mut_ptr().add(offset) as *mut __m128i,
                premultiplied,
            )
        };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        premultiply_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_premultiply_bgra8_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        premultiply_bgra8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { premultiply_bgra8_sse2(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn premultiply_bgra8_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let premultiplied = unsafe {
            premultiply_rgba_group_sse2(
                raw,
                i16::from(source[offset + 3]),
                i16::from(source[offset + 7]),
                i16::from(source[offset + 11]),
                i16::from(source[offset + 15]),
            )
        };
        let bgra = unsafe { rgba_words_to_bgra_sse2(premultiplied) };
        unsafe { _mm_storeu_si128(destination.as_mut_ptr().add(offset) as *mut __m128i, bgra) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        premultiply_bgra8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_unpremultiply_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        unpremultiply_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { unpremultiply_rgba_sse2(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// SSE2 batched unpremultiplication from associated RGBA to straight RGBA.
///
/// The exact reciprocal/division step remains scalar per pixel because SSE2 has
/// no integer division lanes. This still removes the native scalar-only public
/// dispatch by batching four-pixel loads/stores and preserving the scalar oracle.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn unpremultiply_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let mut lane = [0u8; 16];
        unsafe { _mm_storeu_si128(lane.as_mut_ptr() as *mut __m128i, raw) };
        for px in lane.chunks_exact_mut(4) {
            let alpha = px[3];
            if alpha == 0 {
                px.copy_from_slice(&[0, 0, 0, 0]);
            } else if alpha != 255 {
                let alpha = u16::from(alpha);
                px[0] = unpremultiply_channel(px[0], alpha);
                px[1] = unpremultiply_channel(px[1], alpha);
                px[2] = unpremultiply_channel(px[2], alpha);
            }
        }
        let out = unsafe { _mm_loadu_si128(lane.as_ptr() as *const __m128i) };
        unsafe { _mm_storeu_si128(destination.as_mut_ptr().add(offset) as *mut __m128i, out) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        unpremultiply_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn premultiply_rgba_group_sse2(
    raw: NativeM128i,
    a0: i16,
    a1: i16,
    a2: i16,
    a3: i16,
) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_add_epi16, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16, _mm_set_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_add_epi16, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16, _mm_set_epi16,
        _mm_setzero_si128, _mm_srli_epi16, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };

    let zero = _mm_setzero_si128();
    let round = _mm_set1_epi16(128);
    let src_lo = _mm_unpacklo_epi8(raw, zero);
    let src_hi = _mm_unpackhi_epi8(raw, zero);
    let alpha_lo = _mm_set_epi16(255, a1, a1, a1, 255, a0, a0, a0);
    let alpha_hi = _mm_set_epi16(255, a3, a3, a3, 255, a2, a2, a2);
    let lo_mixed = _mm_add_epi16(_mm_mullo_epi16(src_lo, alpha_lo), round);
    let hi_mixed = _mm_add_epi16(_mm_mullo_epi16(src_hi, alpha_hi), round);
    let lo_out = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
    let hi_out = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
    _mm_packus_epi16(lo_out, hi_out)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn rgba_words_to_bgra_sse2(raw: NativeM128i) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_and_si128, _mm_or_si128, _mm_set1_epi32, _mm_slli_epi32, _mm_srli_epi32,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_and_si128, _mm_or_si128, _mm_set1_epi32, _mm_slli_epi32, _mm_srli_epi32,
    };

    let red = _mm_and_si128(raw, _mm_set1_epi32(0x0000_00ff));
    let green_alpha = _mm_and_si128(raw, _mm_set1_epi32(0xff00_ff00_u32 as i32));
    let blue = _mm_and_si128(raw, _mm_set1_epi32(0x00ff_0000));
    _mm_or_si128(
        green_alpha,
        _mm_or_si128(_mm_slli_epi32(red, 16), _mm_srli_epi32(blue, 16)),
    )
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_bgra8_sse2(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_bgra8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { rgba_to_bgra8_sse2(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn rgba_to_bgra8_sse2(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_storeu_si128,
    };

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let alpha_mask = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let rgba = if force_opaque_alpha {
            _mm_or_si128(raw, alpha_mask)
        } else {
            raw
        };
        let bgra = unsafe { rgba_words_to_bgra_sse2(rgba) };
        unsafe { _mm_storeu_si128(destination.as_mut_ptr().add(offset) as *mut __m128i, bgra) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_bgra8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
            force_opaque_alpha,
        );
    }
    true
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSSE3: grayscale caller-surface conversion
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_gray8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_gray_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels];
        rgba_to_gray8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_gray8_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_gray8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128};

    let pixels = rgba_gray_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let gray = unsafe { rgba_luma4_ssse3(raw) };
        unsafe { native_store_low_4_bytes_sse2(destination.as_mut_ptr().add(pixel), gray) };
    }
    if simd_pixels < pixels {
        rgba_to_gray8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels..pixels],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_gray_rgb8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 3];
        rgba_to_gray_rgb8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_gray_rgb8_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_gray_rgb8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let gray_rgb_shuffle =
        _mm_setr_epi8(0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, -128, -128, -128, -128);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let gray = unsafe { rgba_luma4_ssse3(raw) };
        let expanded = _mm_shuffle_epi8(gray, gray_rgb_shuffle);
        let dst_offset = pixel * 3;
        unsafe {
            native_store_low_12_bytes_sse2(destination.as_mut_ptr().add(dst_offset), expanded)
        };
    }
    if simd_pixels < pixels {
        rgba_to_gray_rgb8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_gray_rgba8_ssse3(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_gray_rgba8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_gray_rgba8_ssse3(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_gray_rgba8_ssse3(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_setr_epi8, _mm_shuffle_epi8,
        _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_setr_epi8, _mm_shuffle_epi8,
        _mm_storeu_si128,
    };

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let gray_rgb_shuffle =
        _mm_setr_epi8(0, 0, 0, -128, 1, 1, 1, -128, 2, 2, 2, -128, 3, 3, 3, -128);
    let source_alpha_shuffle = _mm_setr_epi8(
        -128, -128, -128, 3, -128, -128, -128, 7, -128, -128, -128, 11, -128, -128, -128, 15,
    );
    let opaque_alpha = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let gray = unsafe { rgba_luma4_ssse3(raw) };
        let gray_rgb = _mm_shuffle_epi8(gray, gray_rgb_shuffle);
        let alpha = if force_opaque_alpha {
            opaque_alpha
        } else {
            _mm_shuffle_epi8(raw, source_alpha_shuffle)
        };
        let expanded = _mm_or_si128(gray_rgb, alpha);
        unsafe {
            _mm_storeu_si128(
                destination.as_mut_ptr().add(offset) as *mut __m128i,
                expanded,
            )
        };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_gray_rgba8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
            force_opaque_alpha,
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_gray_bgra8_ssse3(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_gray_bgra8_scalar(&source[..pixels * 4], &mut copy, force_opaque_alpha);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_gray_bgra8_ssse3(source, destination, force_opaque_alpha) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_gray_bgra8_ssse3(
    source: &[u8],
    destination: &mut [u8],
    force_opaque_alpha: bool,
) -> bool {
    unsafe { rgba_to_gray_rgba8_ssse3(source, destination, force_opaque_alpha) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_premultiplied_gray_rgba8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_premultiplied_gray_rgba8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_premultiplied_gray_rgba8_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_premultiplied_gray_rgba8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
    };

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let gray_rgb_shuffle =
        _mm_setr_epi8(0, 0, 0, -128, 1, 1, 1, -128, 2, 2, 2, -128, 3, 3, 3, -128);
    let source_alpha_shuffle = _mm_setr_epi8(
        -128, -128, -128, 3, -128, -128, -128, 7, -128, -128, -128, 11, -128, -128, -128, 15,
    );
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let gray = unsafe { rgba_luma4_ssse3(raw) };
        let gray_rgba = _mm_or_si128(
            _mm_shuffle_epi8(gray, gray_rgb_shuffle),
            _mm_shuffle_epi8(raw, source_alpha_shuffle),
        );
        let premultiplied = unsafe {
            premultiply_rgba_group_sse2(
                gray_rgba,
                source[offset + 3] as i16,
                source[offset + 7] as i16,
                source[offset + 11] as i16,
                source[offset + 15] as i16,
            )
        };
        unsafe {
            _mm_storeu_si128(
                destination.as_mut_ptr().add(offset) as *mut __m128i,
                premultiplied,
            )
        };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_premultiplied_gray_rgba8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_luma4_ssse3(raw: NativeM128i) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_add_epi16, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16, _mm_setr_epi8,
        _mm_setzero_si128, _mm_shuffle_epi8, _mm_srli_epi16, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_add_epi16, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16, _mm_setr_epi8,
        _mm_setzero_si128, _mm_shuffle_epi8, _mm_srli_epi16, _mm_unpacklo_epi8,
    };

    let zero = _mm_setzero_si128();
    let red = _mm_unpacklo_epi8(
        _mm_shuffle_epi8(
            raw,
            _mm_setr_epi8(
                0, 4, 8, 12, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128,
            ),
        ),
        zero,
    );
    let green = _mm_unpacklo_epi8(
        _mm_shuffle_epi8(
            raw,
            _mm_setr_epi8(
                1, 5, 9, 13, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128,
            ),
        ),
        zero,
    );
    let blue = _mm_unpacklo_epi8(
        _mm_shuffle_epi8(
            raw,
            _mm_setr_epi8(
                2, 6, 10, 14, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128, -128,
                -128,
            ),
        ),
        zero,
    );
    let weighted = _mm_add_epi16(
        _mm_add_epi16(
            _mm_mullo_epi16(red, _mm_set1_epi16(77)),
            _mm_mullo_epi16(green, _mm_set1_epi16(150)),
        ),
        _mm_add_epi16(
            _mm_mullo_epi16(blue, _mm_set1_epi16(29)),
            _mm_set1_epi16(128),
        ),
    );
    let gray16 = _mm_srli_epi16(weighted, 8);
    _mm_packus_epi16(gray16, zero)
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSSE3: packed RGB/BGR caller-surface and image-span conversion
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_rgb8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 3];
        rgba_to_rgb8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_rgb8_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_rgb8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let shuffle = _mm_setr_epi8(
        0, 1, 2, 4, 5, 6, 8, 9, 10, 12, 13, 14, -128, -128, -128, -128,
    );
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 4;
        let dst_offset = pixel * 3;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(src_offset) as *const __m128i) };
        let rgb = _mm_shuffle_epi8(raw, shuffle);
        unsafe { native_store_low_12_bytes_sse2(destination.as_mut_ptr().add(dst_offset), rgb) };
    }
    if simd_pixels < pixels {
        rgba_to_rgb8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_bgr8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_rgb_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 3];
        rgba_to_bgr8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgba_to_bgr8_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 3], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgba_to_bgr8_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_setr_epi8, _mm_shuffle_epi8};

    let pixels = rgba_rgb_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let shuffle = _mm_setr_epi8(
        2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -128, -128, -128, -128,
    );
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 4;
        let dst_offset = pixel * 3;
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(src_offset) as *const __m128i) };
        let bgr = _mm_shuffle_epi8(raw, shuffle);
        unsafe { native_store_low_12_bytes_sse2(destination.as_mut_ptr().add(dst_offset), bgr) };
    }
    if simd_pixels < pixels {
        rgba_to_bgr8_scalar(
            &source[simd_pixels * 4..pixels * 4],
            &mut destination[simd_pixels * 3..pixels * 3],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgb8_to_opaque_rgba_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgb_rgba_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgb8_to_opaque_rgba_scalar(&source[..pixels * 3], &mut copy);
        copy
    });
    // SAFETY: entered only after SSSE3 runtime detection.
    let ok = unsafe { rgb8_to_opaque_rgba_ssse3(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "ssse3")]
unsafe fn rgb8_to_opaque_rgba_ssse3(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_or_si128, _mm_set1_epi32, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_or_si128, _mm_set1_epi32, _mm_setr_epi8, _mm_shuffle_epi8, _mm_storeu_si128,
    };

    let pixels = rgb_rgba_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let shuffle = _mm_setr_epi8(0, 1, 2, -128, 3, 4, 5, -128, 6, 7, 8, -128, 9, 10, 11, -128);
    let alpha_mask = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for pixel in (0..simd_pixels).step_by(4) {
        let src_offset = pixel * 3;
        let dst_offset = pixel * 4;
        let raw = unsafe { native_load_low_12_bytes_sse2(source.as_ptr().add(src_offset)) };
        let expanded = _mm_or_si128(_mm_shuffle_epi8(raw, shuffle), alpha_mask);
        unsafe {
            _mm_storeu_si128(
                destination.as_mut_ptr().add(dst_offset) as *mut __m128i,
                expanded,
            )
        };
    }
    if simd_pixels < pixels {
        rgb8_to_opaque_rgba_scalar(
            &source[simd_pixels * 3..pixels * 3],
            &mut destination[simd_pixels * 4..pixels * 4],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSE2: RGBA copy and opaque-alpha conversion
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_copy_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        copy_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { copy_rgba_sse2(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn copy_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

    let pixels = rgba_row_pixels(source, destination);
    let bytes = pixels * 4;
    let simd_bytes = (bytes / 16) * 16;
    if simd_bytes == 0 {
        return false;
    }
    for offset in (0..simd_bytes).step_by(16) {
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        unsafe { _mm_storeu_si128(destination.as_mut_ptr().add(offset) as *mut __m128i, raw) };
    }
    if simd_bytes < bytes {
        copy_rgba_scalar(
            &source[simd_bytes..bytes],
            &mut destination[simd_bytes..bytes],
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_rgba_to_opaque_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_opaque_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { rgba_to_opaque_rgba_sse2(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn rgba_to_opaque_rgba_sse2(source: &[u8], destination: &mut [u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_storeu_si128,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_storeu_si128,
    };

    let pixels = rgba_row_pixels(source, destination);
    let bytes = pixels * 4;
    let simd_bytes = (bytes / 16) * 16;
    if simd_bytes == 0 {
        return false;
    }
    let alpha_mask = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for offset in (0..simd_bytes).step_by(16) {
        let raw = unsafe { _mm_loadu_si128(source.as_ptr().add(offset) as *const __m128i) };
        let opaque = _mm_or_si128(raw, alpha_mask);
        unsafe { _mm_storeu_si128(destination.as_mut_ptr().add(offset) as *mut __m128i, opaque) };
    }
    if simd_bytes < bytes {
        rgba_to_opaque_rgba_scalar(
            &source[simd_bytes..bytes],
            &mut destination[simd_bytes..bytes],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSE2: blend_alpha_mask_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_blend_alpha_mask_opaque_dst_sse2(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_alpha_mask_opaque_dst_scalar(&mut copy, &mask_row[..pixels], color);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { blend_alpha_mask_opaque_dst_sse2(dst_row, mask_row, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// SSE2 alpha-mask paint over an opaque destination row.
///
/// The effective source alpha is `round(color.a * mask / 255)`, matching the
/// scalar glyph/image-mask compositor. The alpha channel is forced to opaque.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn blend_alpha_mask_opaque_dst_sse2(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_set_epi16, _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8,
        _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_set1_epi16,
        _mm_set_epi16, _mm_setzero_si128, _mm_srli_epi16, _mm_storeu_si128, _mm_unpackhi_epi8,
        _mm_unpacklo_epi8,
    };

    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let color_alpha = u16::from(color[3]);
    let src_v = _mm_set_epi16(
        255,
        i16::from(color[2]),
        i16::from(color[1]),
        i16::from(color[0]),
        255,
        i16::from(color[2]),
        i16::from(color[1]),
        i16::from(color[0]),
    );
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();

    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let a0 = ((color_alpha * u16::from(mask_row[pixel]) + 127) / 255) as i16;
        let a1 = ((color_alpha * u16::from(mask_row[pixel + 1]) + 127) / 255) as i16;
        let a2 = ((color_alpha * u16::from(mask_row[pixel + 2]) + 127) / 255) as i16;
        let a3 = ((color_alpha * u16::from(mask_row[pixel + 3]) + 127) / 255) as i16;
        let inv0 = 255_i16.saturating_sub(a0);
        let inv1 = 255_i16.saturating_sub(a1);
        let inv2 = 255_i16.saturating_sub(a2);
        let inv3 = 255_i16.saturating_sub(a3);

        let alpha_lo = _mm_set_epi16(255, a1, a1, a1, 255, a0, a0, a0);
        let inv_lo = _mm_set_epi16(0, inv1, inv1, inv1, 0, inv0, inv0, inv0);
        let alpha_hi = _mm_set_epi16(255, a3, a3, a3, 255, a2, a2, a2);
        let inv_hi = _mm_set_epi16(0, inv3, inv3, inv3, 0, inv2, inv2, inv2);

        let dst_raw = unsafe { _mm_loadu_si128(dst_row.as_ptr().add(offset) as *const __m128i) };
        let dst_lo = _mm_unpacklo_epi8(dst_raw, zero);
        let dst_hi = _mm_unpackhi_epi8(dst_raw, zero);

        let lo_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_v, alpha_lo),
                _mm_mullo_epi16(dst_lo, inv_lo),
            ),
            round,
        );
        let hi_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_v, alpha_hi),
                _mm_mullo_epi16(dst_hi, inv_hi),
            ),
            round,
        );
        let lo_out = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
        let hi_out = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
        let packed = _mm_packus_epi16(lo_out, hi_out);
        unsafe { _mm_storeu_si128(dst_row.as_mut_ptr().add(offset) as *mut __m128i, packed) };
    }

    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        blend_alpha_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            color,
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_blend_alpha_mask_normal_sse2(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_alpha_mask_normal_scalar(&mut copy, &mask_row[..pixels], color);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { blend_alpha_mask_normal_sse2(dst_row, mask_row, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// SSE2 alpha-mask paint over arbitrary normal Compat destination rows.
///
/// Mixed-destination alpha uses the exact scalar source-over formula. SSE2 is
/// used for four-pixel unaligned load/store groups so the public native path no
/// longer declines while preserving the established floating-point byte result.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn blend_alpha_mask_normal_sse2(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_loadu_si128, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_loadu_si128, _mm_storeu_si128};

    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let mut group = [0u8; 16];
    let color_alpha = u16::from(color[3]);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(dst_row.as_ptr().add(offset) as *const __m128i) };
        unsafe { _mm_storeu_si128(group.as_mut_ptr() as *mut __m128i, raw) };
        for idx in 0..4 {
            let dst = &mut group[idx * 4..idx * 4 + 4];
            let src_alpha_byte =
                ((color_alpha * u16::from(mask_row[pixel + idx]) + 127) / 255).min(255) as u8;
            blend_alpha_mask_normal_pixel_scalar(dst, color, src_alpha_byte);
        }
        let out = unsafe { _mm_loadu_si128(group.as_ptr() as *const __m128i) };
        unsafe { _mm_storeu_si128(dst_row.as_mut_ptr().add(offset) as *mut __m128i, out) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        blend_alpha_mask_normal_scalar(
            &mut dst_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            color,
        );
    }
    true
}

#[cfg(target_arch = "x86_64")]
fn guarded_multiply_alpha_rows_avx2_x86_64(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    let pixels = alpha_row.len().min(mask_row.len());
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = alpha_row[..pixels].to_vec();
        multiply_alpha_rows_scalar(&mut copy, &mask_row[..pixels]);
        copy
    });
    // SAFETY: entered only after AVX2 runtime detection.
    let ok = unsafe { multiply_alpha_rows_avx2_x86_64(alpha_row, mask_row) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&alpha_row[..pixels], expected.as_slice());
        }
    }
    ok
}

/// AVX2 clip/soft-mask alpha fusion over 32-byte row groups.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn multiply_alpha_rows_avx2_x86_64(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_loadu_si256, _mm256_mullo_epi16, _mm256_packus_epi16, _mm256_setzero_si256,
        _mm256_storeu_si256, _mm256_unpackhi_epi8, _mm256_unpacklo_epi8,
    };

    let pixels = alpha_row.len().min(mask_row.len());
    let simd_pixels = (pixels / 32) * 32;
    if simd_pixels == 0 {
        return false;
    }
    let zero = _mm256_setzero_si256();
    for offset in (0..simd_pixels).step_by(32) {
        let alpha = unsafe { _mm256_loadu_si256(alpha_row.as_ptr().add(offset) as *const __m256i) };
        let mask = unsafe { _mm256_loadu_si256(mask_row.as_ptr().add(offset) as *const __m256i) };
        let alpha_lo = _mm256_unpacklo_epi8(alpha, zero);
        let alpha_hi = _mm256_unpackhi_epi8(alpha, zero);
        let mask_lo = _mm256_unpacklo_epi8(mask, zero);
        let mask_hi = _mm256_unpackhi_epi8(mask, zero);
        let out_lo = div255_round_u16x16_avx2(_mm256_mullo_epi16(alpha_lo, mask_lo));
        let out_hi = div255_round_u16x16_avx2(_mm256_mullo_epi16(alpha_hi, mask_hi));
        let packed = _mm256_packus_epi16(out_lo, out_hi);
        unsafe { _mm256_storeu_si256(alpha_row.as_mut_ptr().add(offset) as *mut __m256i, packed) };
    }
    if simd_pixels < pixels {
        multiply_alpha_rows_scalar(
            &mut alpha_row[simd_pixels..pixels],
            &mask_row[simd_pixels..pixels],
        );
    }
    true
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
fn div255_round_u16x16_avx2(value: std::arch::x86_64::__m256i) -> std::arch::x86_64::__m256i {
    use std::arch::x86_64::{_mm256_add_epi16, _mm256_set1_epi16, _mm256_srli_epi16};
    let rounded = _mm256_add_epi16(value, _mm256_set1_epi16(128));
    _mm256_srli_epi16(_mm256_add_epi16(rounded, _mm256_srli_epi16(rounded, 8)), 8)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_multiply_alpha_rows_sse2(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    let pixels = alpha_row.len().min(mask_row.len());
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = alpha_row[..pixels].to_vec();
        multiply_alpha_rows_scalar(&mut copy, &mask_row[..pixels]);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { multiply_alpha_rows_sse2(alpha_row, mask_row) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&alpha_row[..pixels], expected.as_slice());
        }
    }
    ok
}

/// SSE2 clip/soft-mask alpha fusion.
///
/// Each output byte is `round(alpha * mask / 255)`, matching the scalar clip
/// and soft-mask product used by the engine.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn multiply_alpha_rows_sse2(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_setzero_si128,
        _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_loadu_si128, _mm_mullo_epi16, _mm_packus_epi16, _mm_setzero_si128,
        _mm_storeu_si128, _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };

    let pixels = alpha_row.len().min(mask_row.len());
    let simd_pixels = (pixels / 16) * 16;
    if simd_pixels == 0 {
        return false;
    }
    let zero = _mm_setzero_si128();
    for offset in (0..simd_pixels).step_by(16) {
        let alpha = unsafe { _mm_loadu_si128(alpha_row.as_ptr().add(offset) as *const __m128i) };
        let mask = unsafe { _mm_loadu_si128(mask_row.as_ptr().add(offset) as *const __m128i) };
        let alpha_lo = _mm_unpacklo_epi8(alpha, zero);
        let alpha_hi = _mm_unpackhi_epi8(alpha, zero);
        let mask_lo = _mm_unpacklo_epi8(mask, zero);
        let mask_hi = _mm_unpackhi_epi8(mask, zero);
        let out_lo = unsafe { div255_round_u16x8_sse2(_mm_mullo_epi16(alpha_lo, mask_lo)) };
        let out_hi = unsafe { div255_round_u16x8_sse2(_mm_mullo_epi16(alpha_hi, mask_hi)) };
        let packed = _mm_packus_epi16(out_lo, out_hi);
        unsafe { _mm_storeu_si128(alpha_row.as_mut_ptr().add(offset) as *mut __m128i, packed) };
    }
    if simd_pixels < pixels {
        multiply_alpha_rows_scalar(
            &mut alpha_row[simd_pixels..pixels],
            &mask_row[simd_pixels..pixels],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: blend_alpha_mask_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_blend_alpha_mask_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_alpha_mask_opaque_dst_scalar(&mut copy, &mask_row[..pixels], color);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { blend_alpha_mask_opaque_dst_wasm_simd128(dst_row, mask_row, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 alpha-mask paint over an opaque destination row.
///
/// The effective source alpha is `round(color.a * mask / 255)`, matching the
/// renderer glyph/image-mask path before the final source-over step.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn blend_alpha_mask_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    use std::arch::wasm32::{
        i16x8, i16x8_add, i16x8_mul, i16x8_splat, u16x8_shr, u8x16_narrow_i16x8, v128, v128_store,
    };

    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let color_alpha = u16::from(color[3]);
    let src_v = i16x8(
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
    );
    let round = i16x8_splat(128);

    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let a0 = ((color_alpha * u16::from(mask_row[pixel]) + 127) / 255) as i16;
        let a1 = ((color_alpha * u16::from(mask_row[pixel + 1]) + 127) / 255) as i16;
        let a2 = ((color_alpha * u16::from(mask_row[pixel + 2]) + 127) / 255) as i16;
        let a3 = ((color_alpha * u16::from(mask_row[pixel + 3]) + 127) / 255) as i16;
        let inv0 = 255i16.saturating_sub(a0);
        let inv1 = 255i16.saturating_sub(a1);
        let inv2 = 255i16.saturating_sub(a2);
        let inv3 = 255i16.saturating_sub(a3);

        let alpha_lo = i16x8(a0, a0, a0, 255, a1, a1, a1, 255);
        let inv_lo = i16x8(inv0, inv0, inv0, 0, inv1, inv1, inv1, 0);
        let alpha_hi = i16x8(a2, a2, a2, 255, a3, a3, a3, 255);
        let inv_hi = i16x8(inv2, inv2, inv2, 0, inv3, inv3, inv3, 0);

        let dst_raw =
            unsafe { std::arch::wasm32::v128_load(dst_row.as_ptr().add(offset) as *const v128) };
        let dst_lo = std::arch::wasm32::u16x8_extend_low_u8x16(dst_raw);
        let dst_hi = std::arch::wasm32::u16x8_extend_high_u8x16(dst_raw);

        let lo_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_v, alpha_lo), i16x8_mul(dst_lo, inv_lo)),
            round,
        );
        let hi_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_v, alpha_hi), i16x8_mul(dst_hi, inv_hi)),
            round,
        );
        let lo_out = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
        let hi_out = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);
        let packed = u8x16_narrow_i16x8(lo_out, hi_out);
        unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, packed) };

        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        dst_row[offset + 11] = 255;
        dst_row[offset + 15] = 255;
        pixel += 4;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        blend_alpha_mask_opaque_dst_scalar(
            &mut dst_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            color,
        );
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: blend_alpha_mask_normal
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_blend_alpha_mask_normal_wasm_simd128(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_alpha_mask_normal_scalar(&mut copy, &mask_row[..pixels], color);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { blend_alpha_mask_normal_wasm_simd128(dst_row, mask_row, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 alpha-mask paint over arbitrary normal Compat destination rows.
///
/// This covers mixed/non-opaque destination alpha for glyph/image-mask rows. The
/// per-pixel source-over math intentionally calls the scalar-equivalent formula
/// after SIMD load/store grouping so alpha rounding stays identical to the
/// engine's normal Compat row compositor.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn blend_alpha_mask_normal_wasm_simd128(
    dst_row: &mut [u8],
    mask_row: &[u8],
    color: [u8; 4],
) -> bool {
    use std::arch::wasm32::{v128, v128_load, v128_store};

    let pixels = alpha_mask_pixels(dst_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let mut group = [0u8; 16];
    let color_alpha = u16::from(color[3]);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(dst_row.as_ptr().add(offset) as *const v128) };
        unsafe { v128_store(group.as_mut_ptr() as *mut v128, raw) };
        for idx in 0..4 {
            let dst = &mut group[idx * 4..idx * 4 + 4];
            let src_alpha_byte =
                ((color_alpha * u16::from(mask_row[pixel + idx]) + 127) / 255).min(255) as u8;
            blend_alpha_mask_normal_pixel_scalar(dst, color, src_alpha_byte);
        }
        let out = unsafe { v128_load(group.as_ptr() as *const v128) };
        unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        blend_alpha_mask_normal_scalar(
            &mut dst_row[offset..pixels * 4],
            &mask_row[simd_pixels..pixels],
            color,
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_multiply_alpha_rows_wasm_simd128(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    let pixels = alpha_row.len().min(mask_row.len());
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = alpha_row[..pixels].to_vec();
        multiply_alpha_rows_scalar(&mut copy, &mask_row[..pixels]);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { multiply_alpha_rows_wasm_simd128(alpha_row, mask_row) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&alpha_row[..pixels], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 clip/soft-mask alpha fusion.
///
/// Each output byte is `round(alpha * mask / 255)`, matching the scalar clip
/// and soft-mask product used by the engine.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn multiply_alpha_rows_wasm_simd128(alpha_row: &mut [u8], mask_row: &[u8]) -> bool {
    use std::arch::wasm32::{
        u16x8_extend_high_u8x16, u16x8_extend_low_u8x16, u8x16_narrow_i16x8, v128, v128_load,
        v128_store,
    };

    let pixels = alpha_row.len().min(mask_row.len());
    let simd_pixels = (pixels / 16) * 16;
    if simd_pixels == 0 {
        return false;
    }
    for offset in (0..simd_pixels).step_by(16) {
        let alpha = unsafe { v128_load(alpha_row.as_ptr().add(offset) as *const v128) };
        let mask = unsafe { v128_load(mask_row.as_ptr().add(offset) as *const v128) };
        let alpha_lo = u16x8_extend_low_u8x16(alpha);
        let alpha_hi = u16x8_extend_high_u8x16(alpha);
        let mask_lo = u16x8_extend_low_u8x16(mask);
        let mask_hi = u16x8_extend_high_u8x16(mask);
        let out_lo = wasm_div255_round_i16x8(std::arch::wasm32::i16x8_mul(alpha_lo, mask_lo));
        let out_hi = wasm_div255_round_i16x8(std::arch::wasm32::i16x8_mul(alpha_hi, mask_hi));
        let packed = u8x16_narrow_i16x8(out_lo, out_hi);
        unsafe { v128_store(alpha_row.as_mut_ptr().add(offset) as *mut v128, packed) };
    }
    if simd_pixels < pixels {
        multiply_alpha_rows_scalar(
            &mut alpha_row[simd_pixels..pixels],
            &mask_row[simd_pixels..pixels],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// x86/x86_64 SSE2: blend_separable_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn guarded_blend_separable_opaque_dst_sse2(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    let pixels = dst_row.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_separable_opaque_dst_scalar(&mut copy, color, blend_mode);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { blend_separable_opaque_dst_sse2(dst_row, color, blend_mode) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// SSE2 grouped separable blend over opaque destination rows.
///
/// Multiply, Screen, Overlay, Darken, Lighten, HardLight, Difference, and
/// Exclusion use exact integer SIMD lane math. ColorDodge, ColorBurn, and
/// SoftLight use SSE float lanes with grouped gather/scatter and scalar oracle
/// checks so their PDF byte contracts stay identical.
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn blend_separable_opaque_dst_sse2(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_max_epu8, _mm_min_epu8, _mm_mullo_epi16,
        _mm_packus_epi16, _mm_set1_epi16, _mm_storeu_si128, _mm_sub_epi16, _mm_subs_epu8,
        _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        __m128i, _mm_add_epi16, _mm_loadu_si128, _mm_max_epu8, _mm_min_epu8, _mm_mullo_epi16,
        _mm_packus_epi16, _mm_set1_epi16, _mm_storeu_si128, _mm_sub_epi16, _mm_subs_epu8,
        _mm_unpackhi_epi8, _mm_unpacklo_epi8,
    };

    let pixels = dst_row.len() / 4;
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 || color[3] != 255 {
        return false;
    }
    let color_bytes = [
        color[0], color[1], color[2], 255, color[0], color[1], color[2], 255, color[0], color[1],
        color[2], 255, color[0], color[1], color[2], 255,
    ];
    let color_raw = unsafe { _mm_loadu_si128(color_bytes.as_ptr() as *const __m128i) };
    let zero = _mm_set1_epi16(0);
    let color_lo = _mm_unpacklo_epi8(color_raw, zero);
    let color_hi = _mm_unpackhi_epi8(color_raw, zero);
    let mut group = [0u8; 16];
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(dst_row.as_ptr().add(offset) as *const __m128i) };
        let out = match blend_mode {
            SeparableBlendMode::Multiply | SeparableBlendMode::Screen => {
                let dst_lo = _mm_unpacklo_epi8(raw, zero);
                let dst_hi = _mm_unpackhi_epi8(raw, zero);
                let product_lo =
                    unsafe { div255_round_u16x8_sse2(_mm_mullo_epi16(color_lo, dst_lo)) };
                let product_hi =
                    unsafe { div255_round_u16x8_sse2(_mm_mullo_epi16(color_hi, dst_hi)) };
                let out_lo = if matches!(blend_mode, SeparableBlendMode::Screen) {
                    _mm_sub_epi16(_mm_add_epi16(color_lo, dst_lo), product_lo)
                } else {
                    product_lo
                };
                let out_hi = if matches!(blend_mode, SeparableBlendMode::Screen) {
                    _mm_sub_epi16(_mm_add_epi16(color_hi, dst_hi), product_hi)
                } else {
                    product_hi
                };
                _mm_packus_epi16(out_lo, out_hi)
            }
            SeparableBlendMode::Darken => _mm_min_epu8(color_raw, raw),
            SeparableBlendMode::Lighten => _mm_max_epu8(color_raw, raw),
            SeparableBlendMode::Difference => {
                _mm_subs_epu8(_mm_max_epu8(color_raw, raw), _mm_min_epu8(color_raw, raw))
            }
            SeparableBlendMode::Exclusion => {
                let dst_lo = _mm_unpacklo_epi8(raw, zero);
                let dst_hi = _mm_unpackhi_epi8(raw, zero);
                let product_lo = _mm_mullo_epi16(color_lo, dst_lo);
                let product_hi = _mm_mullo_epi16(color_hi, dst_hi);
                let scaled_lo = unsafe { double_product_over_255_i16x8_sse2(product_lo) };
                let scaled_hi = unsafe { double_product_over_255_i16x8_sse2(product_hi) };
                let out_lo = _mm_sub_epi16(_mm_add_epi16(color_lo, dst_lo), scaled_lo);
                let out_hi = _mm_sub_epi16(_mm_add_epi16(color_hi, dst_hi), scaled_hi);
                _mm_packus_epi16(out_lo, out_hi)
            }
            SeparableBlendMode::Overlay | SeparableBlendMode::HardLight => {
                let dst_lo = _mm_unpacklo_epi8(raw, zero);
                let dst_hi = _mm_unpackhi_epi8(raw, zero);
                let (src_lo, src_hi, backdrop_lo, backdrop_hi) =
                    if matches!(blend_mode, SeparableBlendMode::Overlay) {
                        (dst_lo, dst_hi, color_lo, color_hi)
                    } else {
                        (color_lo, color_hi, dst_lo, dst_hi)
                    };
                let out_lo = unsafe { hard_light_i16x8_sse2(src_lo, backdrop_lo) };
                let out_hi = unsafe { hard_light_i16x8_sse2(src_hi, backdrop_hi) };
                _mm_packus_epi16(out_lo, out_hi)
            }
            SeparableBlendMode::ColorDodge | SeparableBlendMode::ColorBurn => {
                unsafe { _mm_storeu_si128(group.as_mut_ptr() as *mut __m128i, raw) };
                let red =
                    unsafe { division_blend_channel_i32x4_sse2(&group, color[0], 0, blend_mode) };
                let green =
                    unsafe { division_blend_channel_i32x4_sse2(&group, color[1], 1, blend_mode) };
                let blue =
                    unsafe { division_blend_channel_i32x4_sse2(&group, color[2], 2, blend_mode) };
                unsafe { store_rgba_channels_from_i32x4_sse2(&mut group, red, green, blue) };
                unsafe { _mm_loadu_si128(group.as_ptr() as *const __m128i) }
            }
            SeparableBlendMode::SoftLight => {
                unsafe { _mm_storeu_si128(group.as_mut_ptr() as *mut __m128i, raw) };
                let red = unsafe { soft_light_channel_i32x4_sse2(&group, color[0], 0) };
                let green = unsafe { soft_light_channel_i32x4_sse2(&group, color[1], 1) };
                let blue = unsafe { soft_light_channel_i32x4_sse2(&group, color[2], 2) };
                unsafe { store_rgba_channels_from_i32x4_sse2(&mut group, red, green, blue) };
                unsafe { _mm_loadu_si128(group.as_ptr() as *const __m128i) }
            }
        };
        unsafe { _mm_storeu_si128(dst_row.as_mut_ptr().add(offset) as *mut __m128i, out) };
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        dst_row[offset + 11] = 255;
        dst_row[offset + 15] = 255;
    }
    if simd_pixels < pixels {
        blend_separable_opaque_dst_scalar(
            &mut dst_row[simd_pixels * 4..pixels * 4],
            color,
            blend_mode,
        );
    }
    true
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn div255_round_u16x8_sse2(value: NativeM128i) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_add_epi16, _mm_set1_epi16, _mm_srli_epi16};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{_mm_add_epi16, _mm_set1_epi16, _mm_srli_epi16};

    let rounded = _mm_add_epi16(value, _mm_set1_epi16(128));
    _mm_srli_epi16(_mm_add_epi16(rounded, _mm_srli_epi16(rounded, 8)), 8)
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn hard_light_i16x8_sse2(source: NativeM128i, backdrop: NativeM128i) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_and_si128, _mm_andnot_si128, _mm_cmpgt_epi16, _mm_mullo_epi16, _mm_or_si128,
        _mm_set1_epi16, _mm_sub_epi16,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_and_si128, _mm_andnot_si128, _mm_cmpgt_epi16, _mm_mullo_epi16, _mm_or_si128,
        _mm_set1_epi16, _mm_sub_epi16,
    };

    let dark = unsafe { double_product_over_255_i16x8_sse2(_mm_mullo_epi16(source, backdrop)) };
    let inverse_source = _mm_sub_epi16(_mm_set1_epi16(255), source);
    let inverse_backdrop = _mm_sub_epi16(_mm_set1_epi16(255), backdrop);
    let light_scale = unsafe {
        double_product_over_255_i16x8_sse2(_mm_mullo_epi16(inverse_source, inverse_backdrop))
    };
    let light = _mm_sub_epi16(_mm_set1_epi16(255), light_scale);
    let dark_mask = _mm_cmpgt_epi16(_mm_set1_epi16(128), source);
    _mm_or_si128(
        _mm_and_si128(dark_mask, dark),
        _mm_andnot_si128(dark_mask, light),
    )
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn double_product_over_255_i16x8_sse2(product: NativeM128i) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_add_epi16, _mm_and_si128, _mm_cmpgt_epi16, _mm_mullo_epi16, _mm_set1_epi16,
        _mm_srli_epi16, _mm_sub_epi16,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_add_epi16, _mm_and_si128, _mm_cmpgt_epi16, _mm_mullo_epi16, _mm_set1_epi16,
        _mm_srli_epi16, _mm_sub_epi16,
    };

    let one = _mm_set1_epi16(1);
    let product_plus_one = _mm_add_epi16(product, one);
    let quotient = _mm_srli_epi16(
        _mm_add_epi16(product_plus_one, _mm_srli_epi16(product_plus_one, 8)),
        8,
    );
    let remainder = _mm_sub_epi16(product, _mm_mullo_epi16(quotient, _mm_set1_epi16(255)));
    let add_one = _mm_and_si128(_mm_cmpgt_epi16(remainder, _mm_set1_epi16(63)), one);
    let add_two = _mm_and_si128(_mm_cmpgt_epi16(remainder, _mm_set1_epi16(191)), one);

    _mm_add_epi16(
        _mm_add_epi16(quotient, quotient),
        _mm_add_epi16(add_one, add_two),
    )
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn division_blend_channel_i32x4_sse2(
    group: &[u8; 16],
    source_channel: u8,
    channel: usize,
    blend_mode: SeparableBlendMode,
) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_cmple_ps, _mm_div_ps, _mm_min_ps, _mm_set1_ps, _mm_sub_ps};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{_mm_cmple_ps, _mm_div_ps, _mm_min_ps, _mm_set1_ps, _mm_sub_ps};

    let zero = _mm_set1_ps(0.0);
    let one = _mm_set1_ps(1.0);
    let source = _mm_set1_ps(f32::from(source_channel) / 255.0);
    let dst = unsafe { rgba_channel_f32x4_sse2(group, channel) };
    let blended = match blend_mode {
        SeparableBlendMode::ColorDodge => {
            let divided = _mm_min_ps(_mm_div_ps(dst, _mm_sub_ps(one, source)), one);
            let source_is_one = _mm_cmple_ps(one, source);
            let dst_is_zero = _mm_cmple_ps(dst, zero);
            let saturated = unsafe { select_ps_sse2(source_is_one, one, divided) };
            unsafe { select_ps_sse2(dst_is_zero, zero, saturated) }
        }
        SeparableBlendMode::ColorBurn => {
            let inverse_dst = _mm_sub_ps(one, dst);
            let divided = _mm_min_ps(_mm_div_ps(inverse_dst, source), one);
            let burned = _mm_sub_ps(one, divided);
            let source_is_zero = _mm_cmple_ps(source, zero);
            let dst_is_one = _mm_cmple_ps(one, dst);
            let zeroed = unsafe { select_ps_sse2(source_is_zero, zero, burned) };
            unsafe { select_ps_sse2(dst_is_one, one, zeroed) }
        }
        _ => unreachable!("division blend helper only handles ColorDodge/ColorBurn"),
    };
    unsafe { denormalize_unit_f32x4_sse2(blended) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn soft_light_channel_i32x4_sse2(
    group: &[u8; 16],
    source_channel: u8,
    channel: usize,
) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_add_ps, _mm_cmple_ps, _mm_mul_ps, _mm_set1_ps, _mm_sqrt_ps, _mm_sub_ps,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_add_ps, _mm_cmple_ps, _mm_mul_ps, _mm_set1_ps, _mm_sqrt_ps, _mm_sub_ps,
    };

    let one = _mm_set1_ps(1.0);
    let source = _mm_set1_ps(f32::from(source_channel) / 255.0);
    let dst = unsafe { rgba_channel_f32x4_sse2(group, channel) };
    let two_source = _mm_mul_ps(_mm_set1_ps(2.0), source);
    let source_dark_scale = _mm_sub_ps(one, two_source);
    let inverse_dst = _mm_sub_ps(one, dst);
    let dark = _mm_sub_ps(
        dst,
        _mm_mul_ps(_mm_mul_ps(source_dark_scale, dst), inverse_dst),
    );
    let polynomial = _mm_mul_ps(
        _mm_add_ps(
            _mm_mul_ps(
                _mm_sub_ps(_mm_mul_ps(_mm_set1_ps(16.0), dst), _mm_set1_ps(12.0)),
                dst,
            ),
            _mm_set1_ps(4.0),
        ),
        dst,
    );
    let sqrt = _mm_sqrt_ps(dst);
    let d = unsafe { select_ps_sse2(_mm_cmple_ps(dst, _mm_set1_ps(0.25)), polynomial, sqrt) };
    let light = _mm_add_ps(
        dst,
        _mm_mul_ps(_mm_sub_ps(two_source, one), _mm_sub_ps(d, dst)),
    );
    let blended = unsafe { select_ps_sse2(_mm_cmple_ps(source, _mm_set1_ps(0.5)), dark, light) };
    unsafe { denormalize_unit_f32x4_sse2(blended) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn rgba_channel_f32x4_sse2(group: &[u8; 16], channel: usize) -> NativeM128 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::_mm_set_ps;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::_mm_set_ps;

    _mm_set_ps(
        f32::from(group[12 + channel]) / 255.0,
        f32::from(group[8 + channel]) / 255.0,
        f32::from(group[4 + channel]) / 255.0,
        f32::from(group[channel]) / 255.0,
    )
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn select_ps_sse2(
    mask: NativeM128,
    true_value: NativeM128,
    false_value: NativeM128,
) -> NativeM128 {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{_mm_and_ps, _mm_andnot_ps, _mm_or_ps};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{_mm_and_ps, _mm_andnot_ps, _mm_or_ps};

    _mm_or_ps(
        _mm_and_ps(mask, true_value),
        _mm_andnot_ps(mask, false_value),
    )
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn denormalize_unit_f32x4_sse2(value: NativeM128) -> NativeM128i {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{
        _mm_add_ps, _mm_cvttps_epi32, _mm_max_ps, _mm_min_ps, _mm_mul_ps, _mm_set1_ps,
    };
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{
        _mm_add_ps, _mm_cvttps_epi32, _mm_max_ps, _mm_min_ps, _mm_mul_ps, _mm_set1_ps,
    };

    let clamped = _mm_max_ps(_mm_set1_ps(0.0), _mm_min_ps(value, _mm_set1_ps(1.0)));
    _mm_cvttps_epi32(_mm_add_ps(
        _mm_mul_ps(clamped, _mm_set1_ps(255.0)),
        _mm_set1_ps(0.5),
    ))
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn store_rgba_channels_from_i32x4_sse2(
    group: &mut [u8; 16],
    red: NativeM128i,
    green: NativeM128i,
    blue: NativeM128i,
) {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::{__m128i, _mm_storeu_si128};
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::{__m128i, _mm_storeu_si128};

    let mut red_lanes = [0i32; 4];
    let mut green_lanes = [0i32; 4];
    let mut blue_lanes = [0i32; 4];
    unsafe { _mm_storeu_si128(red_lanes.as_mut_ptr() as *mut __m128i, red) };
    unsafe { _mm_storeu_si128(green_lanes.as_mut_ptr() as *mut __m128i, green) };
    unsafe { _mm_storeu_si128(blue_lanes.as_mut_ptr() as *mut __m128i, blue) };
    for pixel in 0..4 {
        let base = pixel * 4;
        group[base] = red_lanes[pixel] as u8;
        group[base + 1] = green_lanes[pixel] as u8;
        group[base + 2] = blue_lanes[pixel] as u8;
        group[base + 3] = 255;
    }
}

// ---------------------------------------------------------------------------
// WASM SIMD128: blend_separable_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_blend_separable_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    let pixels = dst_row.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        blend_separable_opaque_dst_scalar(&mut copy, color, blend_mode);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { blend_separable_opaque_dst_wasm_simd128(dst_row, color, blend_mode) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn blend_separable_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    color: [u8; 4],
    blend_mode: SeparableBlendMode,
) -> bool {
    use std::arch::wasm32::{
        i16x8_add, i16x8_mul, i16x8_sub, u16x8_extend_high_u8x16, u16x8_extend_low_u8x16,
        u8x16_max, u8x16_min, u8x16_narrow_i16x8, u8x16_sub_sat, v128, v128_load, v128_store,
    };

    let pixels = dst_row.len() / 4;
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 || color[3] != 255 {
        return false;
    }
    let color_bytes = [
        color[0], color[1], color[2], 255, color[0], color[1], color[2], 255, color[0], color[1],
        color[2], 255, color[0], color[1], color[2], 255,
    ];
    let color_raw = unsafe { v128_load(color_bytes.as_ptr() as *const v128) };
    let color_lo = u16x8_extend_low_u8x16(color_raw);
    let color_hi = u16x8_extend_high_u8x16(color_raw);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(dst_row.as_ptr().add(offset) as *const v128) };
        match blend_mode {
            SeparableBlendMode::Multiply | SeparableBlendMode::Screen => {
                let dst_lo = u16x8_extend_low_u8x16(raw);
                let dst_hi = u16x8_extend_high_u8x16(raw);
                let product_lo = i16x8_mul(color_lo, dst_lo);
                let product_hi = i16x8_mul(color_hi, dst_hi);
                let product_lo = wasm_div255_round_i16x8(product_lo);
                let product_hi = wasm_div255_round_i16x8(product_hi);
                let out_lo = if matches!(blend_mode, SeparableBlendMode::Screen) {
                    i16x8_sub(i16x8_add(color_lo, dst_lo), product_lo)
                } else {
                    product_lo
                };
                let out_hi = if matches!(blend_mode, SeparableBlendMode::Screen) {
                    i16x8_sub(i16x8_add(color_hi, dst_hi), product_hi)
                } else {
                    product_hi
                };
                let out = u8x16_narrow_i16x8(out_lo, out_hi);
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::Darken => {
                let out = u8x16_min(color_raw, raw);
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::Lighten => {
                let out = u8x16_max(color_raw, raw);
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::Difference => {
                let out = u8x16_sub_sat(u8x16_max(color_raw, raw), u8x16_min(color_raw, raw));
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::Overlay | SeparableBlendMode::HardLight => {
                let dst_lo = u16x8_extend_low_u8x16(raw);
                let dst_hi = u16x8_extend_high_u8x16(raw);
                let (src_lo, src_hi, backdrop_lo, backdrop_hi) =
                    if matches!(blend_mode, SeparableBlendMode::Overlay) {
                        (dst_lo, dst_hi, color_lo, color_hi)
                    } else {
                        (color_lo, color_hi, dst_lo, dst_hi)
                    };
                let out_lo = wasm_hard_light_i16x8(src_lo, backdrop_lo);
                let out_hi = wasm_hard_light_i16x8(src_hi, backdrop_hi);
                let out = u8x16_narrow_i16x8(out_lo, out_hi);
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::Exclusion => {
                let dst_lo = u16x8_extend_low_u8x16(raw);
                let dst_hi = u16x8_extend_high_u8x16(raw);
                let product_lo = i16x8_mul(color_lo, dst_lo);
                let product_hi = i16x8_mul(color_hi, dst_hi);
                let scaled_lo = wasm_double_product_over_255_i16x8(product_lo);
                let scaled_hi = wasm_double_product_over_255_i16x8(product_hi);
                let out_lo = i16x8_sub(i16x8_add(color_lo, dst_lo), scaled_lo);
                let out_hi = i16x8_sub(i16x8_add(color_hi, dst_hi), scaled_hi);
                let out = u8x16_narrow_i16x8(out_lo, out_hi);
                unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, out) };
            }
            SeparableBlendMode::ColorDodge | SeparableBlendMode::ColorBurn => {
                let red = wasm_division_blend_channel_u32x4(raw, color[0], 0, blend_mode);
                let green = wasm_division_blend_channel_u32x4(raw, color[1], 1, blend_mode);
                let blue = wasm_division_blend_channel_u32x4(raw, color[2], 2, blend_mode);
                wasm_store_rgba_channels_from_u32x4(dst_row, offset, red, green, blue);
            }
            SeparableBlendMode::SoftLight => {
                let red = wasm_soft_light_channel_u32x4(raw, color[0], 0);
                let green = wasm_soft_light_channel_u32x4(raw, color[1], 1);
                let blue = wasm_soft_light_channel_u32x4(raw, color[2], 2);
                wasm_store_rgba_channels_from_u32x4(dst_row, offset, red, green, blue);
            }
        }
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        dst_row[offset + 11] = 255;
        dst_row[offset + 15] = 255;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        blend_separable_opaque_dst_scalar(&mut dst_row[offset..pixels * 4], color, blend_mode);
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_division_blend_channel_u32x4(
    raw: std::arch::wasm32::v128,
    source_channel: u8,
    channel: usize,
    blend_mode: SeparableBlendMode,
) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        f32x4_convert_u32x4, f32x4_div, f32x4_ge, f32x4_le, f32x4_min, f32x4_splat, f32x4_sub,
        v128_bitselect,
    };

    let zero = f32x4_splat(0.0);
    let one = f32x4_splat(1.0);
    let dst = f32x4_div(
        f32x4_convert_u32x4(wasm_rgba_channel_u32x4(raw, channel)),
        f32x4_splat(255.0),
    );
    let source = f32::from(source_channel) / 255.0;
    let out = match blend_mode {
        SeparableBlendMode::ColorDodge => {
            let candidate = if source_channel == 255 {
                one
            } else {
                f32x4_min(f32x4_div(dst, f32x4_splat(1.0 - source)), one)
            };
            v128_bitselect(zero, candidate, f32x4_le(dst, zero))
        }
        SeparableBlendMode::ColorBurn => {
            let candidate = if source_channel == 0 {
                zero
            } else {
                f32x4_sub(
                    one,
                    f32x4_min(f32x4_div(f32x4_sub(one, dst), f32x4_splat(source)), one),
                )
            };
            v128_bitselect(one, candidate, f32x4_ge(dst, one))
        }
        _ => unreachable!("division blend helper only handles ColorDodge/ColorBurn"),
    };

    wasm_denormalize_unit_f32x4(out)
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_soft_light_channel_u32x4(
    raw: std::arch::wasm32::v128,
    source_channel: u8,
    channel: usize,
) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        f32x4_add, f32x4_convert_u32x4, f32x4_div, f32x4_le, f32x4_mul, f32x4_splat, f32x4_sqrt,
        f32x4_sub, v128_bitselect,
    };

    let one = f32x4_splat(1.0);
    let dst = f32x4_div(
        f32x4_convert_u32x4(wasm_rgba_channel_u32x4(raw, channel)),
        f32x4_splat(255.0),
    );
    let source = f32::from(source_channel) / 255.0;
    let out = if source <= 0.5 {
        f32x4_sub(
            dst,
            f32x4_mul(
                f32x4_mul(f32x4_splat(1.0 - 2.0 * source), dst),
                f32x4_sub(one, dst),
            ),
        )
    } else {
        let polynomial = f32x4_mul(
            f32x4_add(
                f32x4_mul(
                    f32x4_sub(f32x4_mul(f32x4_splat(16.0), dst), f32x4_splat(12.0)),
                    dst,
                ),
                f32x4_splat(4.0),
            ),
            dst,
        );
        let curve = v128_bitselect(
            polynomial,
            f32x4_sqrt(dst),
            f32x4_le(dst, f32x4_splat(0.25)),
        );
        f32x4_add(
            dst,
            f32x4_mul(f32x4_splat(2.0 * source - 1.0), f32x4_sub(curve, dst)),
        )
    };

    wasm_denormalize_unit_f32x4(out)
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_rgba_channel_u32x4(
    raw: std::arch::wasm32::v128,
    channel: usize,
) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        i32x4_splat, u16x8_extend_low_u8x16, u32x4_extend_low_u16x8, u8x16_shuffle,
    };

    let zero = i32x4_splat(0);
    let gathered = match channel {
        0 => {
            u8x16_shuffle::<0, 4, 8, 12, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero)
        }
        1 => {
            u8x16_shuffle::<1, 5, 9, 13, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero)
        }
        2 => {
            u8x16_shuffle::<2, 6, 10, 14, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16>(raw, zero)
        }
        _ => unreachable!("RGBA channel must be 0, 1, or 2"),
    };
    u32x4_extend_low_u16x8(u16x8_extend_low_u8x16(gathered))
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_denormalize_unit_f32x4(value: std::arch::wasm32::v128) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        f32x4_add, f32x4_floor, f32x4_max, f32x4_min, f32x4_mul, f32x4_splat, u32x4_trunc_sat_f32x4,
    };

    let clamped = f32x4_max(f32x4_splat(0.0), f32x4_min(value, f32x4_splat(1.0)));
    u32x4_trunc_sat_f32x4(f32x4_floor(f32x4_add(
        f32x4_mul(clamped, f32x4_splat(255.0)),
        f32x4_splat(0.5),
    )))
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_store_rgba_channels_from_u32x4(
    dst_row: &mut [u8],
    offset: usize,
    red: std::arch::wasm32::v128,
    green: std::arch::wasm32::v128,
    blue: std::arch::wasm32::v128,
) {
    use std::arch::wasm32::{v128, v128_store};

    let mut red_lanes = [0u32; 4];
    let mut green_lanes = [0u32; 4];
    let mut blue_lanes = [0u32; 4];
    unsafe { v128_store(red_lanes.as_mut_ptr() as *mut v128, red) };
    unsafe { v128_store(green_lanes.as_mut_ptr() as *mut v128, green) };
    unsafe { v128_store(blue_lanes.as_mut_ptr() as *mut v128, blue) };
    for pixel in 0..4 {
        let base = offset + pixel * 4;
        dst_row[base] = red_lanes[pixel] as u8;
        dst_row[base + 1] = green_lanes[pixel] as u8;
        dst_row[base + 2] = blue_lanes[pixel] as u8;
        dst_row[base + 3] = 255;
    }
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_div255_round_i16x8(value: std::arch::wasm32::v128) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{i16x8_add, i16x8_splat, u16x8_shr};

    let rounded = i16x8_add(value, i16x8_splat(128));
    u16x8_shr(i16x8_add(rounded, u16x8_shr(rounded, 8)), 8)
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_double_product_over_255_i16x8(product: std::arch::wasm32::v128) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        i16x8_add, i16x8_mul, i16x8_splat, i16x8_sub, u16x8_ge, u16x8_shr, v128_and,
    };

    let one = i16x8_splat(1);
    let product_plus_one = i16x8_add(product, one);
    let quotient = u16x8_shr(
        i16x8_add(product_plus_one, u16x8_shr(product_plus_one, 8)),
        8,
    );
    let remainder = i16x8_sub(product, i16x8_mul(quotient, i16x8_splat(255)));
    let add_one = v128_and(u16x8_ge(remainder, i16x8_splat(64)), one);
    let add_two = v128_and(u16x8_ge(remainder, i16x8_splat(192)), one);

    i16x8_add(i16x8_add(quotient, quotient), i16x8_add(add_one, add_two))
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_hard_light_i16x8(
    source: std::arch::wasm32::v128,
    backdrop: std::arch::wasm32::v128,
) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{i16x8_mul, i16x8_splat, i16x8_sub, u16x8_le, v128_bitselect};

    let dark = wasm_double_product_over_255_i16x8(i16x8_mul(source, backdrop));
    let inverse_source = i16x8_sub(i16x8_splat(255), source);
    let inverse_backdrop = i16x8_sub(i16x8_splat(255), backdrop);
    let light_scale =
        wasm_double_product_over_255_i16x8(i16x8_mul(inverse_source, inverse_backdrop));
    let light = i16x8_sub(i16x8_splat(255), light_scale);
    let dark_branch = u16x8_le(source, i16x8_splat(127));

    v128_bitselect(dark, light, dark_branch)
}

// ---------------------------------------------------------------------------
// WASM SIMD128: premultiply_rgba
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_premultiply_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        premultiply_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { premultiply_rgba_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 premultiplication from straight RGBA to premultiplied RGBA.
///
/// Processes four pixels per vector. Alpha lanes are preserved exactly; RGB
/// lanes use the renderer's `round(value * alpha / 255)` convention.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn premultiply_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i16x8_splat, v128, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let round = i16x8_splat(128);
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let src_raw =
            unsafe { std::arch::wasm32::v128_load(source.as_ptr().add(offset) as *const v128) };
        let packed = wasm_premultiply_rgba_group(
            src_raw,
            source[offset + 3] as i16,
            source[offset + 7] as i16,
            source[offset + 11] as i16,
            source[offset + 15] as i16,
            round,
        );
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, packed) };
        pixel += 4;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        premultiply_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[inline]
fn wasm_premultiply_rgba_group(
    raw: std::arch::wasm32::v128,
    a0: i16,
    a1: i16,
    a2: i16,
    a3: i16,
    round: std::arch::wasm32::v128,
) -> std::arch::wasm32::v128 {
    use std::arch::wasm32::{
        i16x8, i16x8_add, i16x8_mul, u16x8_extend_high_u8x16, u16x8_extend_low_u8x16, u16x8_shr,
        u8x16_narrow_i16x8,
    };

    let src_lo = u16x8_extend_low_u8x16(raw);
    let src_hi = u16x8_extend_high_u8x16(raw);
    let alpha_lo = i16x8(a0, a0, a0, 255, a1, a1, a1, 255);
    let alpha_hi = i16x8(a2, a2, a2, 255, a3, a3, a3, 255);
    let lo_mixed = i16x8_add(i16x8_mul(src_lo, alpha_lo), round);
    let hi_mixed = i16x8_add(i16x8_mul(src_hi, alpha_hi), round);
    let lo_out = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
    let hi_out = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);
    u8x16_narrow_i16x8(lo_out, hi_out)
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_premultiply_bgra8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        premultiply_bgra8_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { premultiply_bgra8_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn premultiply_bgra8_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i16x8_splat, i8x16_shuffle, v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let round = i16x8_splat(128);
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let premultiplied = wasm_premultiply_rgba_group(
            raw,
            source[offset + 3] as i16,
            source[offset + 7] as i16,
            source[offset + 11] as i16,
            source[offset + 15] as i16,
            round,
        );
        let bgra = i8x16_shuffle::<2, 1, 0, 3, 6, 5, 4, 7, 10, 9, 8, 11, 14, 13, 12, 15>(
            premultiplied,
            premultiplied,
        );
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, bgra) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        premultiply_bgra8_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: unpremultiply_rgba
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_unpremultiply_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        unpremultiply_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { unpremultiply_rgba_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 unpremultiplication from associated RGBA to straight RGBA.
///
/// The lane keeps exact scalar rounding for each RGB/alpha pair. It uses SIMD
/// loads and stores for four-pixel groups, then performs the reciprocal step
/// per pixel because WebAssembly SIMD does not provide integer division lanes.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn unpremultiply_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let mut lane = [0u8; 16];
        unsafe { v128_store(lane.as_mut_ptr() as *mut v128, raw) };
        for px in lane.chunks_exact_mut(4) {
            let alpha = px[3];
            if alpha == 0 {
                px.copy_from_slice(&[0, 0, 0, 0]);
            } else if alpha != 255 {
                let alpha = u16::from(alpha);
                px[0] = unpremultiply_channel(px[0], alpha);
                px[1] = unpremultiply_channel(px[1], alpha);
                px[2] = unpremultiply_channel(px[2], alpha);
            }
        }
        let out = unsafe { v128_load(lane.as_ptr() as *const v128) };
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, out) };
        pixel += 4;
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        unpremultiply_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: copy_rgba / rgba_to_opaque_rgba
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_copy_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        copy_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { copy_rgba_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_rgba_to_opaque_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    let pixels = rgba_row_pixels(source, destination);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = vec![0u8; pixels * 4];
        rgba_to_opaque_rgba_scalar(&source[..pixels * 4], &mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { rgba_to_opaque_rgba_wasm_simd128(source, destination) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&destination[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn copy_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let raw = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, raw) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        copy_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn rgba_to_opaque_rgba_wasm_simd128(source: &[u8], destination: &mut [u8]) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load, v128_store};

    let pixels = rgba_row_pixels(source, destination);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }
    let opaque = [255u8; 16];
    let opaque_raw = unsafe { v128_load(opaque.as_ptr() as *const v128) };
    for pixel in (0..simd_pixels).step_by(4) {
        let offset = pixel * 4;
        let rgba = unsafe { v128_load(source.as_ptr().add(offset) as *const v128) };
        let forced = i8x16_shuffle::<0, 1, 2, 16, 4, 5, 6, 17, 8, 9, 10, 18, 12, 13, 14, 19>(
            rgba, opaque_raw,
        );
        unsafe { v128_store(destination.as_mut_ptr().add(offset) as *mut v128, forced) };
    }
    if simd_pixels < pixels {
        let offset = simd_pixels * 4;
        rgba_to_opaque_rgba_scalar(
            &source[offset..pixels * 4],
            &mut destination[offset..pixels * 4],
        );
    }
    true
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_reverse_4byte_words_wasm_simd128(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { reverse_4byte_words_wasm_simd128(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn reverse_4byte_words_wasm_simd128(slice: &mut [u8]) -> bool {
    use std::arch::wasm32::{i8x16_shuffle, v128, v128_load, v128_store};

    let words = slice.len() / 4;
    let simd_words = (words / 4) * 4;
    if simd_words == 0 {
        return false;
    }
    for word in (0..simd_words).step_by(4) {
        let offset = word * 4;
        let raw = unsafe { v128_load(slice.as_ptr().add(offset) as *const v128) };
        let reversed =
            i8x16_shuffle::<3, 2, 1, 0, 7, 6, 5, 4, 11, 10, 9, 8, 15, 14, 13, 12>(raw, raw);
        unsafe { v128_store(slice.as_mut_ptr().add(offset) as *mut v128, reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

#[cfg(target_arch = "aarch64")]
fn guarded_reverse_4byte_words_neon_aarch64(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: AArch64 guarantees Advanced SIMD.
    let ok = unsafe { reverse_4byte_words_neon_aarch64(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "aarch64")]
unsafe fn reverse_4byte_words_neon_aarch64(slice: &mut [u8]) -> bool {
    use std::arch::aarch64::{vld1q_u8, vrev32q_u8, vst1q_u8};

    let words = slice.len() / 4;
    let simd_words = (words / 4) * 4;
    if simd_words == 0 {
        return reverse_4byte_words_scalar(slice);
    }
    for word in (0..simd_words).step_by(4) {
        let offset = word * 4;
        let raw = unsafe { vld1q_u8(slice.as_ptr().add(offset)) };
        let reversed = vrev32q_u8(raw);
        unsafe { vst1q_u8(slice.as_mut_ptr().add(offset), reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

#[cfg(all(target_arch = "arm", target_feature = "neon"))]
fn guarded_reverse_4byte_words_neon_arm(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: compiled only when NEON is enabled for the target.
    let ok = unsafe { reverse_4byte_words_neon_arm(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "arm", target_feature = "neon"))]
#[target_feature(enable = "neon")]
unsafe fn reverse_4byte_words_neon_arm(slice: &mut [u8]) -> bool {
    use std::arch::arm::{vld1q_u8, vrev32q_u8, vst1q_u8};

    let words = slice.len() / 4;
    let simd_words = (words / 4) * 4;
    if simd_words == 0 {
        return reverse_4byte_words_scalar(slice);
    }
    for word in (0..simd_words).step_by(4) {
        let offset = word * 4;
        let raw = unsafe { vld1q_u8(slice.as_ptr().add(offset)) };
        let reversed = vrev32q_u8(raw);
        unsafe { vst1q_u8(slice.as_mut_ptr().add(offset), reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

#[cfg(target_arch = "x86_64")]
fn guarded_reverse_4byte_words_avx2_x86_64(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: entered only after AVX2 runtime detection.
    let ok = unsafe { reverse_4byte_words_avx2_x86_64(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn reverse_4byte_words_avx2_x86_64(slice: &mut [u8]) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_and_si256, _mm256_loadu_si256, _mm256_or_si256, _mm256_set1_epi32,
        _mm256_slli_epi32, _mm256_srli_epi32, _mm256_storeu_si256,
    };

    let words = slice.len() / 4;
    let simd_words = (words / 8) * 8;
    let low_byte = _mm256_set1_epi32(0x0000_00ff);
    let second_byte = _mm256_set1_epi32(0x0000_ff00);
    let third_byte = _mm256_set1_epi32(0x00ff_0000);
    let high_byte = _mm256_set1_epi32(0xff00_0000_u32 as i32);
    for word in (0..simd_words).step_by(8) {
        let offset = word * 4;
        let raw = unsafe { _mm256_loadu_si256(slice.as_ptr().add(offset) as *const __m256i) };
        let b0 = _mm256_slli_epi32(_mm256_and_si256(raw, low_byte), 24);
        let b1 = _mm256_slli_epi32(_mm256_and_si256(raw, second_byte), 8);
        let b2 = _mm256_srli_epi32(_mm256_and_si256(raw, third_byte), 8);
        let b3 = _mm256_srli_epi32(_mm256_and_si256(raw, high_byte), 24);
        let reversed = _mm256_or_si256(_mm256_or_si256(b0, b1), _mm256_or_si256(b2, b3));
        unsafe { _mm256_storeu_si256(slice.as_mut_ptr().add(offset) as *mut __m256i, reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

#[cfg(target_arch = "x86_64")]
fn guarded_reverse_4byte_words_sse2_x86_64(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { reverse_4byte_words_sse2_x86_64(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn reverse_4byte_words_sse2_x86_64(slice: &mut [u8]) -> bool {
    use std::arch::x86_64::{
        __m128i, _mm_and_si128, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_slli_epi32,
        _mm_srli_epi32, _mm_storeu_si128,
    };

    let words = slice.len() / 4;
    let simd_words = (words / 4) * 4;
    if simd_words == 0 {
        return reverse_4byte_words_scalar(slice);
    }
    let low_byte = _mm_set1_epi32(0x0000_00ff);
    let second_byte = _mm_set1_epi32(0x0000_ff00);
    let third_byte = _mm_set1_epi32(0x00ff_0000);
    let high_byte = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for word in (0..simd_words).step_by(4) {
        let offset = word * 4;
        let raw = unsafe { _mm_loadu_si128(slice.as_ptr().add(offset) as *const __m128i) };
        let b0 = _mm_slli_epi32(_mm_and_si128(raw, low_byte), 24);
        let b1 = _mm_slli_epi32(_mm_and_si128(raw, second_byte), 8);
        let b2 = _mm_srli_epi32(_mm_and_si128(raw, third_byte), 8);
        let b3 = _mm_srli_epi32(_mm_and_si128(raw, high_byte), 24);
        let reversed = _mm_or_si128(_mm_or_si128(b0, b1), _mm_or_si128(b2, b3));
        unsafe { _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

#[cfg(target_arch = "x86")]
fn guarded_reverse_4byte_words_sse2_x86(slice: &mut [u8]) -> bool {
    let words = slice.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice[..words * 4].to_vec();
        reverse_4byte_words_scalar(&mut copy);
        copy
    });
    // SAFETY: entered only after SSE2 runtime detection.
    let ok = unsafe { reverse_4byte_words_sse2_x86(slice) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&slice[..words * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn reverse_4byte_words_sse2_x86(slice: &mut [u8]) -> bool {
    use std::arch::x86::{
        __m128i, _mm_and_si128, _mm_loadu_si128, _mm_or_si128, _mm_set1_epi32, _mm_slli_epi32,
        _mm_srli_epi32, _mm_storeu_si128,
    };

    let words = slice.len() / 4;
    let simd_words = (words / 4) * 4;
    if simd_words == 0 {
        return reverse_4byte_words_scalar(slice);
    }
    let low_byte = _mm_set1_epi32(0x0000_00ff);
    let second_byte = _mm_set1_epi32(0x0000_ff00);
    let third_byte = _mm_set1_epi32(0x00ff_0000);
    let high_byte = _mm_set1_epi32(0xff00_0000_u32 as i32);
    for word in (0..simd_words).step_by(4) {
        let offset = word * 4;
        let raw = unsafe { _mm_loadu_si128(slice.as_ptr().add(offset) as *const __m128i) };
        let b0 = _mm_slli_epi32(_mm_and_si128(raw, low_byte), 24);
        let b1 = _mm_slli_epi32(_mm_and_si128(raw, second_byte), 8);
        let b2 = _mm_srli_epi32(_mm_and_si128(raw, third_byte), 8);
        let b3 = _mm_srli_epi32(_mm_and_si128(raw, high_byte), 24);
        let reversed = _mm_or_si128(_mm_or_si128(b0, b1), _mm_or_si128(b2, b3));
        unsafe { _mm_storeu_si128(slice.as_mut_ptr().add(offset) as *mut __m128i, reversed) };
    }
    if simd_words < words {
        reverse_4byte_words_scalar(&mut slice[simd_words * 4..words * 4]);
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: blend_normal_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_blend_normal_opaque_dst_wasm_simd128(slice: &mut [u8], color: [u8; 4]) -> bool {
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = slice.to_vec();
        blend_normal_opaque_dst_scalar(&mut copy, color);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { blend_normal_opaque_dst_wasm_simd128(slice, color) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(slice, expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 blend: src_color * alpha + dst * (255 - alpha), dst_alpha forced to 255.
/// Processes 4 pixels (16 bytes) per iteration using 128-bit SIMD lanes widened to 16-bit.
///
/// Rounding: uses the same `(x + 128 + ((x + 128) >> 8)) >> 8` formula as the scalar path,
/// which produces bit-exact results for all u8 inputs.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn blend_normal_opaque_dst_wasm_simd128(slice: &mut [u8], color: [u8; 4]) -> bool {
    use std::arch::wasm32::{
        i16x8_add, i16x8_mul, i16x8_splat, u16x8_shr, u8x16_narrow_i16x8, v128, v128_load,
        v128_store,
    };

    let alpha = color[3] as i16;
    let inv = 255i16.saturating_sub(alpha);

    // Source color values (replicated for 2 pixels per i16x8 half)
    let src_v = std::arch::wasm32::i16x8(
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
        color[0] as i16,
        color[1] as i16,
        color[2] as i16,
        255,
    );

    // Alpha multiplier per lane: alpha for color channels, 255 for the alpha channel
    let alpha_v = std::arch::wasm32::i16x8(alpha, alpha, alpha, 255, alpha, alpha, alpha, 255);

    // Inverse multiplier per lane: inv for color channels, 0 for the alpha channel
    let inv_v = std::arch::wasm32::i16x8(inv, inv, inv, 0, inv, inv, inv, 0);

    let round = i16x8_splat(128);

    let simd_len = (slice.len() / 16) * 16;
    let mut offset = 0usize;
    while offset < simd_len {
        let dst_raw = unsafe { v128_load(slice.as_ptr().add(offset) as *const v128) };

        // Widen low 8 bytes (pixels 0-1) → i16x8
        let dst_lo = std::arch::wasm32::u16x8_extend_low_u8x16(dst_raw);
        // Widen high 8 bytes (pixels 2-3) → i16x8
        let dst_hi = std::arch::wasm32::u16x8_extend_high_u8x16(dst_raw);

        // mixed = src * alpha + dst * inv + 128
        // Note: i16x8_mul gives low 16 bits of product (same as _mm_mullo_epi16).
        // For src[ch]*alpha, max is 255*254=64770 which fits u16 (wraps in i16 but
        // the bit pattern is correct for the subsequent add/shift logic).
        let lo_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_v, alpha_v), i16x8_mul(dst_lo, inv_v)),
            round,
        );
        let hi_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_v, alpha_v), i16x8_mul(dst_hi, inv_v)),
            round,
        );

        // (x + (x >> 8)) >> 8   (u16x8_shr is logical/unsigned shift)
        let lo_out = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
        let hi_out = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);

        // Narrow back to u8x16
        let packed = u8x16_narrow_i16x8(lo_out, hi_out);
        unsafe { v128_store(slice.as_mut_ptr().add(offset) as *mut v128, packed) };

        // Force alpha channels to 255 (alpha slot computation is not meaningful)
        slice[offset + 3] = 255;
        slice[offset + 7] = 255;
        slice[offset + 11] = 255;
        slice[offset + 15] = 255;

        offset += 16;
    }
    if offset < slice.len() {
        blend_normal_opaque_dst_scalar(&mut slice[offset..], color);
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: composite_normal_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_composite_normal_opaque_dst_wasm_simd128(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    let pixels = row_pixels(dst_row, src_row, &[]);
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = dst_row[..pixels * 4].to_vec();
        composite_normal_opaque_dst_scalar(&mut copy, &src_row[..pixels * 4]);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { composite_normal_opaque_dst_wasm_simd128(dst_row, src_row) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 source-over composite with per-pixel alpha from src, opaque dst.
/// Processes 2 pixels per iteration (widened to i16x8 for multiply/accumulate).
///
/// Rounding: bit-exact with scalar `(x + 128 + ((x + 128) >> 8)) >> 8`.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn composite_normal_opaque_dst_wasm_simd128(dst_row: &mut [u8], src_row: &[u8]) -> bool {
    use std::arch::wasm32::{
        i16x8, i16x8_add, i16x8_mul, i16x8_splat, u16x8_shr, u8x16_narrow_i16x8, v128, v128_store,
    };

    let pixels = row_pixels(dst_row, src_row, &[]);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let round = i16x8_splat(128);

    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;

        // Build per-pixel alpha/inv vectors for 2 pixels at a time (lo half, hi half)
        let a0 = src_row[offset + 3] as i16;
        let a1 = src_row[offset + 7] as i16;
        let a2 = src_row[offset + 11] as i16;
        let a3 = src_row[offset + 15] as i16;
        let inv0 = 255i16.saturating_sub(a0);
        let inv1 = 255i16.saturating_sub(a1);
        let inv2 = 255i16.saturating_sub(a2);
        let inv3 = 255i16.saturating_sub(a3);

        let alpha_lo = i16x8(a0, a0, a0, 255, a1, a1, a1, 255);
        let inv_lo = i16x8(inv0, inv0, inv0, 0, inv1, inv1, inv1, 0);
        let alpha_hi = i16x8(a2, a2, a2, 255, a3, a3, a3, 255);
        let inv_hi = i16x8(inv2, inv2, inv2, 0, inv3, inv3, inv3, 0);

        // Load src and dst as raw bytes, widen to 16-bit
        let src_raw =
            unsafe { std::arch::wasm32::v128_load(src_row.as_ptr().add(offset) as *const v128) };
        let dst_raw =
            unsafe { std::arch::wasm32::v128_load(dst_row.as_ptr().add(offset) as *const v128) };

        let src_lo = std::arch::wasm32::u16x8_extend_low_u8x16(src_raw);
        let src_hi = std::arch::wasm32::u16x8_extend_high_u8x16(src_raw);
        let dst_lo = std::arch::wasm32::u16x8_extend_low_u8x16(dst_raw);
        let dst_hi = std::arch::wasm32::u16x8_extend_high_u8x16(dst_raw);

        // mixed = src * alpha + dst * inv + 128
        let lo_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_lo, alpha_lo), i16x8_mul(dst_lo, inv_lo)),
            round,
        );
        let hi_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_hi, alpha_hi), i16x8_mul(dst_hi, inv_hi)),
            round,
        );

        // (x + (x >> 8)) >> 8
        let lo_out = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
        let hi_out = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);

        let packed = u8x16_narrow_i16x8(lo_out, hi_out);
        unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, packed) };

        // Force alpha to 255
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        dst_row[offset + 11] = 255;
        dst_row[offset + 15] = 255;

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

// ---------------------------------------------------------------------------
// WASM SIMD128: flatten_opaque_background
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_flatten_opaque_background_wasm_simd128(data: &mut [u8], background: [u8; 4]) -> bool {
    let pixels = data.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = data[..pixels * 4].to_vec();
        flatten_opaque_background_scalar(&mut copy, background);
        copy
    });
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok = unsafe { flatten_opaque_background_wasm_simd128(data, background) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&data[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn flatten_opaque_background_wasm_simd128(data: &mut [u8], background: [u8; 4]) -> bool {
    use std::arch::wasm32::{
        i16x8, i16x8_add, i16x8_mul, i16x8_splat, u16x8_extend_high_u8x16, u16x8_extend_low_u8x16,
        u16x8_shr, u8x16_narrow_i16x8, v128, v128_load, v128_store,
    };

    let pixels = data.len() / 4;
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 || background[3] != 255 {
        return false;
    }

    let background_v = i16x8(
        i16::from(background[0]),
        i16::from(background[1]),
        i16::from(background[2]),
        255,
        i16::from(background[0]),
        i16::from(background[1]),
        i16::from(background[2]),
        255,
    );
    let round = i16x8_splat(128);

    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let a0 = i16::from(data[offset + 3]);
        let a1 = i16::from(data[offset + 7]);
        let a2 = i16::from(data[offset + 11]);
        let a3 = i16::from(data[offset + 15]);
        let inv0 = 255i16.saturating_sub(a0);
        let inv1 = 255i16.saturating_sub(a1);
        let inv2 = 255i16.saturating_sub(a2);
        let inv3 = 255i16.saturating_sub(a3);

        let alpha_lo = i16x8(a0, a0, a0, 255, a1, a1, a1, 255);
        let inv_lo = i16x8(inv0, inv0, inv0, 0, inv1, inv1, inv1, 0);
        let alpha_hi = i16x8(a2, a2, a2, 255, a3, a3, a3, 255);
        let inv_hi = i16x8(inv2, inv2, inv2, 0, inv3, inv3, inv3, 0);

        let raw = unsafe { v128_load(data.as_ptr().add(offset) as *const v128) };
        let src_lo = u16x8_extend_low_u8x16(raw);
        let src_hi = u16x8_extend_high_u8x16(raw);

        let lo_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_lo, alpha_lo), i16x8_mul(background_v, inv_lo)),
            round,
        );
        let hi_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_hi, alpha_hi), i16x8_mul(background_v, inv_hi)),
            round,
        );
        let lo = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
        let hi = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);
        let packed = u8x16_narrow_i16x8(lo, hi);
        unsafe { v128_store(data.as_mut_ptr().add(offset) as *mut v128, packed) };
        data[offset + 3] = 255;
        data[offset + 7] = 255;
        data[offset + 11] = 255;
        data[offset + 15] = 255;

        pixel += 4;
    }
    if simd_pixels < pixels {
        flatten_opaque_background_scalar(&mut data[simd_pixels * 4..pixels * 4], background);
    }
    true
}

// ---------------------------------------------------------------------------
// WASM SIMD128: composite_soft_mask_opaque_destination
// ---------------------------------------------------------------------------

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
fn guarded_soft_mask_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    let pixels = row_pixels(dst_row, src_row, mask_row);
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
    // SAFETY: compiled only for wasm32 with simd128 target feature.
    let ok =
        unsafe { soft_mask_opaque_dst_wasm_simd128(dst_row, src_row, mask_row, group_alpha_255) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&dst_row[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

/// WASM SIMD128 soft-mask composite: effective alpha = round(src_a * mask * group_alpha / 255^2).
/// Processes 4 pixels per iteration with scalar-computed per-pixel alpha fed into SIMD blending.
///
/// Rounding: bit-exact with the engine scalar row path. Only the final
/// src*eff + dst*inv blending step runs through SIMD with the identical
/// `(x + 128 + ((x + 128) >> 8)) >> 8` formula.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
#[target_feature(enable = "simd128")]
unsafe fn soft_mask_opaque_dst_wasm_simd128(
    dst_row: &mut [u8],
    src_row: &[u8],
    mask_row: &[u8],
    group_alpha_255: u16,
) -> bool {
    use std::arch::wasm32::{
        i16x8, i16x8_add, i16x8_mul, i16x8_splat, u16x8_shr, u8x16_narrow_i16x8, v128, v128_store,
    };

    let pixels = row_pixels(dst_row, src_row, mask_row);
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 {
        return false;
    }

    let round = i16x8_splat(128);

    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;

        let eff0 =
            soft_mask_effective_alpha(src_row[offset + 3], mask_row[pixel], group_alpha_255) as i16;
        let eff1 =
            soft_mask_effective_alpha(src_row[offset + 7], mask_row[pixel + 1], group_alpha_255)
                as i16;
        let eff2 =
            soft_mask_effective_alpha(src_row[offset + 11], mask_row[pixel + 2], group_alpha_255)
                as i16;
        let eff3 =
            soft_mask_effective_alpha(src_row[offset + 15], mask_row[pixel + 3], group_alpha_255)
                as i16;

        let inv0 = 255i16.saturating_sub(eff0);
        let inv1 = 255i16.saturating_sub(eff1);
        let inv2 = 255i16.saturating_sub(eff2);
        let inv3 = 255i16.saturating_sub(eff3);

        let alpha_lo = i16x8(eff0, eff0, eff0, 255, eff1, eff1, eff1, 255);
        let inv_lo = i16x8(inv0, inv0, inv0, 0, inv1, inv1, inv1, 0);
        let alpha_hi = i16x8(eff2, eff2, eff2, 255, eff3, eff3, eff3, 255);
        let inv_hi = i16x8(inv2, inv2, inv2, 0, inv3, inv3, inv3, 0);

        let src_raw =
            unsafe { std::arch::wasm32::v128_load(src_row.as_ptr().add(offset) as *const v128) };
        let dst_raw =
            unsafe { std::arch::wasm32::v128_load(dst_row.as_ptr().add(offset) as *const v128) };

        let src_lo = std::arch::wasm32::u16x8_extend_low_u8x16(src_raw);
        let src_hi = std::arch::wasm32::u16x8_extend_high_u8x16(src_raw);
        let dst_lo = std::arch::wasm32::u16x8_extend_low_u8x16(dst_raw);
        let dst_hi = std::arch::wasm32::u16x8_extend_high_u8x16(dst_raw);

        let lo_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_lo, alpha_lo), i16x8_mul(dst_lo, inv_lo)),
            round,
        );
        let hi_mixed = i16x8_add(
            i16x8_add(i16x8_mul(src_hi, alpha_hi), i16x8_mul(dst_hi, inv_hi)),
            round,
        );

        let lo_out = u16x8_shr(i16x8_add(lo_mixed, u16x8_shr(lo_mixed, 8)), 8);
        let hi_out = u16x8_shr(i16x8_add(hi_mixed, u16x8_shr(hi_mixed, 8)), 8);

        let packed = u8x16_narrow_i16x8(lo_out, hi_out);
        unsafe { v128_store(dst_row.as_mut_ptr().add(offset) as *mut v128, packed) };

        // Force alpha to 255
        dst_row[offset + 3] = 255;
        dst_row[offset + 7] = 255;
        dst_row[offset + 11] = 255;
        dst_row[offset + 15] = 255;

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
                soft_mask_effective_alpha(src_row[idx * 4 + 3], mask_row[idx], group_alpha_255)
                    as i16;
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
                soft_mask_effective_alpha(src_row[idx * 4 + 3], mask_row[idx], group_alpha_255)
                    as i16;
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
                soft_mask_effective_alpha(src_row[idx * 4 + 3], mask_row[idx], group_alpha_255)
                    as i16;
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
                soft_mask_effective_alpha(src_row[idx * 4 + 3], mask_row[idx], group_alpha_255)
                    as i16;
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
    let pixels = row_pixels(dst_row, src_row, mask_row);
    let simd_pixels = (pixels / 2) * 2;
    if simd_pixels == 0 {
        return false;
    }
    for pixel in (0..simd_pixels).step_by(2) {
        let offset = pixel * 4;
        let eff0 = soft_mask_effective_alpha(src_row[offset + 3], mask_row[pixel], group_alpha_255);
        let eff1 =
            soft_mask_effective_alpha(src_row[offset + 7], mask_row[pixel + 1], group_alpha_255);
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

#[cfg(target_arch = "x86_64")]
fn guarded_flatten_opaque_background_sse2_x86_64(data: &mut [u8], background: [u8; 4]) -> bool {
    let pixels = data.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = data[..pixels * 4].to_vec();
        flatten_opaque_background_scalar(&mut copy, background);
        copy
    });
    let ok = unsafe { flatten_opaque_background_sse2_x86_64(data, background) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&data[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn flatten_opaque_background_sse2_x86_64(data: &mut [u8], background: [u8; 4]) -> bool {
    unsafe { flatten_opaque_background_sse2_impl(data, background) }
}

#[cfg(target_arch = "x86")]
fn guarded_flatten_opaque_background_sse2_x86(data: &mut [u8], background: [u8; 4]) -> bool {
    let pixels = data.len() / 4;
    let mut scalar = cfg!(debug_assertions).then(|| {
        let mut copy = data[..pixels * 4].to_vec();
        flatten_opaque_background_scalar(&mut copy, background);
        copy
    });
    let ok = unsafe { flatten_opaque_background_sse2_x86(data, background) };
    if ok {
        if let Some(expected) = scalar.take() {
            debug_assert_eq!(&data[..pixels * 4], expected.as_slice());
        }
    }
    ok
}

#[cfg(target_arch = "x86")]
#[target_feature(enable = "sse2")]
unsafe fn flatten_opaque_background_sse2_x86(data: &mut [u8], background: [u8; 4]) -> bool {
    unsafe { flatten_opaque_background_sse2_impl(data, background) }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "sse2")]
unsafe fn flatten_opaque_background_sse2_impl(data: &mut [u8], background: [u8; 4]) -> bool {
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

    let pixels = data.len() / 4;
    let simd_pixels = (pixels / 4) * 4;
    if simd_pixels == 0 || background[3] != 255 {
        return false;
    }
    let bg_values = [
        i16::from(background[0]),
        i16::from(background[1]),
        i16::from(background[2]),
        255,
        i16::from(background[0]),
        i16::from(background[1]),
        i16::from(background[2]),
        255,
    ];
    let background_v = unsafe { _mm_loadu_si128(bg_values.as_ptr() as *const __m128i) };
    let round = _mm_set1_epi16(128);
    let zero = _mm_setzero_si128();
    let mut pixel = 0usize;
    while pixel < simd_pixels {
        let offset = pixel * 4;
        let raw = unsafe { _mm_loadu_si128(data.as_ptr().add(offset) as *const __m128i) };
        let src_lo = _mm_unpacklo_epi8(raw, zero);
        let src_hi = _mm_unpackhi_epi8(raw, zero);
        let mut alpha_lo = [0i16; 8];
        let mut inv_lo = [0i16; 8];
        let mut alpha_hi = [0i16; 8];
        let mut inv_hi = [0i16; 8];
        for lane in 0..2 {
            let alpha = i16::from(data[offset + lane * 4 + 3]);
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
            let alpha = i16::from(data[offset + (lane + 2) * 4 + 3]);
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
        let lo_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_lo, alpha_lo),
                _mm_mullo_epi16(background_v, inv_lo),
            ),
            round,
        );
        let hi_mixed = _mm_add_epi16(
            _mm_add_epi16(
                _mm_mullo_epi16(src_hi, alpha_hi),
                _mm_mullo_epi16(background_v, inv_hi),
            ),
            round,
        );
        let lo = _mm_srli_epi16(_mm_add_epi16(lo_mixed, _mm_srli_epi16(lo_mixed, 8)), 8);
        let hi = _mm_srli_epi16(_mm_add_epi16(hi_mixed, _mm_srli_epi16(hi_mixed, 8)), 8);
        let packed = _mm_packus_epi16(lo, hi);
        unsafe { _mm_storeu_si128(data.as_mut_ptr().add(offset) as *mut __m128i, packed) };
        data[offset + 3] = 255;
        data[offset + 7] = 255;
        data[offset + 11] = 255;
        data[offset + 15] = 255;
        pixel += 4;
    }
    if simd_pixels < pixels {
        flatten_opaque_background_scalar(&mut data[simd_pixels * 4..pixels * 4], background);
    }
    true
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

    fn mixed_alpha_destination(pixels: usize) -> Vec<u8> {
        (0..pixels)
            .flat_map(|index| {
                let index = index as u8;
                [
                    index.wrapping_mul(23).wrapping_add(13),
                    index.wrapping_mul(41).wrapping_add(29),
                    index.wrapping_mul(61).wrapping_add(31),
                    [0, 64, 171, 255][index as usize % 4],
                ]
            })
            .collect()
    }

    fn separable_blend_modes() -> [SeparableBlendMode; 11] {
        [
            SeparableBlendMode::Multiply,
            SeparableBlendMode::Screen,
            SeparableBlendMode::Overlay,
            SeparableBlendMode::Darken,
            SeparableBlendMode::Lighten,
            SeparableBlendMode::ColorDodge,
            SeparableBlendMode::ColorBurn,
            SeparableBlendMode::HardLight,
            SeparableBlendMode::SoftLight,
            SeparableBlendMode::Difference,
            SeparableBlendMode::Exclusion,
        ]
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

            let mut alpha_fill_backing = vec![0u8; len + 2];
            alpha_fill_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
            let alpha_fill = &mut alpha_fill_backing[1..=len];
            let alpha_fill_before = alpha_fill.to_vec();
            let mut alpha_fill_expected = alpha_fill_before.clone();
            let _ = blend_normal_opaque_dst_scalar(&mut alpha_fill_expected, color);
            let handled = fill_alpha_run(alpha_fill, color);
            if handled {
                assert_eq!(
                    alpha_fill,
                    alpha_fill_expected.as_slice(),
                    "alpha fill pixels={pixels}"
                );
            } else {
                assert_eq!(
                    alpha_fill,
                    alpha_fill_before.as_slice(),
                    "alpha fill fallback pixels={pixels}"
                );
            }

            let alpha_mask: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 19, 128, 255][index % 4])
                .collect();
            let mut alpha_backing = vec![0u8; len + 2];
            alpha_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
            let alpha_dst = &mut alpha_backing[1..=len];
            let alpha_before = alpha_dst.to_vec();
            let mut alpha_expected = alpha_before.clone();
            let color = [211, 37, 97, 191];
            let _ = blend_alpha_mask_opaque_dst_scalar(&mut alpha_expected, &alpha_mask, color);
            let handled = blend_alpha_mask_opaque_destination(alpha_dst, &alpha_mask, color);
            if handled {
                assert_eq!(
                    alpha_dst,
                    alpha_expected.as_slice(),
                    "alpha mask pixels={pixels}"
                );
            } else {
                assert_eq!(
                    alpha_dst,
                    alpha_before.as_slice(),
                    "alpha mask fallback pixels={pixels}"
                );
            }

            let mut alpha_mixed_backing = vec![0u8; len + 2];
            alpha_mixed_backing[1..=len].copy_from_slice(&mixed_alpha_destination(pixels));
            let alpha_mixed_dst = &mut alpha_mixed_backing[1..=len];
            let alpha_mixed_before = alpha_mixed_dst.to_vec();
            let mut alpha_mixed_expected = alpha_mixed_before.clone();
            let _ = blend_alpha_mask_normal_scalar(&mut alpha_mixed_expected, &alpha_mask, color);
            let handled = blend_alpha_mask_normal(alpha_mixed_dst, &alpha_mask, color);
            if handled {
                assert_eq!(
                    alpha_mixed_dst,
                    alpha_mixed_expected.as_slice(),
                    "alpha mask normal pixels={pixels}"
                );
            } else {
                assert_eq!(
                    alpha_mixed_dst,
                    alpha_mixed_before.as_slice(),
                    "alpha mask normal fallback pixels={pixels}"
                );
            }

            let mut alpha_product_backing = vec![0xadu8; pixels + 2];
            for (idx, value) in alpha_product_backing[1..=pixels].iter_mut().enumerate() {
                *value = (idx as u8).wrapping_mul(17);
            }
            let alpha_product = &mut alpha_product_backing[1..=pixels];
            let alpha_product_before = alpha_product.to_vec();
            let alpha_product_mask: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 31, 127, 255][index % 4])
                .collect();
            let mut alpha_product_expected = alpha_product_before.clone();
            let _ = multiply_alpha_rows_scalar(&mut alpha_product_expected, &alpha_product_mask);
            let handled = multiply_alpha_rows(alpha_product, &alpha_product_mask);
            if handled {
                assert_eq!(
                    alpha_product,
                    alpha_product_expected.as_slice(),
                    "alpha row product pixels={pixels}"
                );
            } else {
                assert_eq!(
                    alpha_product,
                    alpha_product_before.as_slice(),
                    "alpha row product fallback pixels={pixels}"
                );
            }

            for blend_mode in separable_blend_modes() {
                let mut separable_backing = vec![0u8; len + 2];
                separable_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
                let separable_dst = &mut separable_backing[1..=len];
                let separable_before = separable_dst.to_vec();
                let mut separable_expected = separable_before.clone();
                let color = [163, 47, 219, 255];
                let _ =
                    blend_separable_opaque_dst_scalar(&mut separable_expected, color, blend_mode);
                let handled = blend_separable_opaque_destination(separable_dst, color, blend_mode);
                if handled {
                    assert_eq!(
                        separable_dst,
                        separable_expected.as_slice(),
                        "separable blend {blend_mode:?} pixels={pixels}"
                    );
                } else {
                    assert_eq!(
                        separable_dst,
                        separable_before.as_slice(),
                        "separable blend fallback {blend_mode:?} pixels={pixels}"
                    );
                }
            }

            let src = source_row(pixels);
            let mask: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 17, 127, 255][index % 4])
                .collect();
            for group_alpha in [255_u16, 193] {
                let mut soft_backing = vec![0u8; len + 2];
                soft_backing[1..=len].copy_from_slice(&opaque_destination(pixels));
                let soft = &mut soft_backing[1..=len];
                let soft_before = soft.to_vec();
                let mut soft_expected = soft_before.clone();
                let _ = soft_mask_opaque_dst_scalar(&mut soft_expected, &src, &mask, group_alpha);
                let handled =
                    composite_soft_mask_opaque_destination(soft, &src, &mask, group_alpha);
                if handled {
                    assert_eq!(
                        soft,
                        soft_expected.as_slice(),
                        "soft mask pixels={pixels} group_alpha={group_alpha}"
                    );
                } else {
                    assert_eq!(
                        soft,
                        soft_before.as_slice(),
                        "soft fallback pixels={pixels} group_alpha={group_alpha}"
                    );
                }
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

            let mut flatten_backing = vec![0u8; len + 2];
            flatten_backing[1..=len].copy_from_slice(&mixed_alpha_destination(pixels));
            let flatten = &mut flatten_backing[1..=len];
            let flatten_before = flatten.to_vec();
            let mut flatten_expected = flatten_before.clone();
            let background = [13, 71, 149, 255];
            let _ = flatten_opaque_background_scalar(&mut flatten_expected, background);
            let handled = flatten_opaque_background(flatten, background);
            if handled {
                assert_eq!(
                    flatten,
                    flatten_expected.as_slice(),
                    "flatten pixels={pixels}"
                );
            } else {
                assert_eq!(
                    flatten,
                    flatten_before.as_slice(),
                    "flatten fallback pixels={pixels}"
                );
            }

            let mut gray_backing = vec![0xadu8; pixels + 2];
            let gray = &mut gray_backing[1..=pixels];
            let gray_before = gray.to_vec();
            let mut gray_expected = gray_before.clone();
            let expected_handled = rgba_to_gray8_scalar(&src, &mut gray_expected);
            let handled = rgba_to_gray8(&src, gray);
            if handled {
                assert_eq!(gray, gray_expected.as_slice(), "gray pixels={pixels}");
            } else {
                assert_eq!(
                    gray,
                    gray_before.as_slice(),
                    "gray fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut gray_rgb_backing = vec![0xadu8; pixels * 3 + 2];
            let gray_rgb = &mut gray_rgb_backing[1..=pixels * 3];
            let gray_rgb_before = gray_rgb.to_vec();
            let mut gray_rgb_expected = gray_rgb_before.clone();
            let expected_handled = rgba_to_gray_rgb8_scalar(&src, &mut gray_rgb_expected);
            let handled = rgba_to_gray_rgb8(&src, gray_rgb);
            if handled {
                assert_eq!(
                    gray_rgb,
                    gray_rgb_expected.as_slice(),
                    "gray rgb pixels={pixels}"
                );
            } else {
                assert_eq!(
                    gray_rgb,
                    gray_rgb_before.as_slice(),
                    "gray rgb fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut gray_rgba_backing = vec![0xadu8; len + 2];
            let gray_rgba = &mut gray_rgba_backing[1..=len];
            let gray_rgba_before = gray_rgba.to_vec();
            let mut gray_rgba_expected = vec![0u8; len];
            let expected_handled = rgba_to_gray_rgba8_scalar(&src, &mut gray_rgba_expected, false);
            let handled = rgba_to_gray_rgba8(&src, gray_rgba, false);
            if handled {
                assert_eq!(
                    gray_rgba,
                    gray_rgba_expected.as_slice(),
                    "gray rgba pixels={pixels}"
                );
            } else {
                assert_eq!(
                    gray_rgba,
                    gray_rgba_before.as_slice(),
                    "gray rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut opaque_gray_rgba_backing = vec![0xadu8; len + 2];
            let opaque_gray_rgba = &mut opaque_gray_rgba_backing[1..=len];
            let opaque_gray_rgba_before = opaque_gray_rgba.to_vec();
            let mut opaque_gray_rgba_expected = vec![0u8; len];
            let expected_handled =
                rgba_to_gray_rgba8_scalar(&src, &mut opaque_gray_rgba_expected, true);
            let handled = rgba_to_gray_rgba8(&src, opaque_gray_rgba, true);
            if handled {
                assert_eq!(
                    opaque_gray_rgba,
                    opaque_gray_rgba_expected.as_slice(),
                    "opaque gray rgba pixels={pixels}"
                );
            } else {
                assert_eq!(
                    opaque_gray_rgba,
                    opaque_gray_rgba_before.as_slice(),
                    "opaque gray rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut gray_bgra_backing = vec![0xadu8; len + 2];
            let gray_bgra = &mut gray_bgra_backing[1..=len];
            let gray_bgra_before = gray_bgra.to_vec();
            let mut gray_bgra_expected = vec![0u8; len];
            let expected_handled = rgba_to_gray_bgra8_scalar(&src, &mut gray_bgra_expected, false);
            let handled = rgba_to_gray_bgra8(&src, gray_bgra, false);
            if handled {
                assert_eq!(
                    gray_bgra,
                    gray_bgra_expected.as_slice(),
                    "gray bgra pixels={pixels}"
                );
            } else {
                assert_eq!(
                    gray_bgra,
                    gray_bgra_before.as_slice(),
                    "gray bgra fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut premultiplied_gray_backing = vec![0xadu8; len + 2];
            let premultiplied_gray = &mut premultiplied_gray_backing[1..=len];
            let premultiplied_gray_before = premultiplied_gray.to_vec();
            let mut premultiplied_gray_expected = vec![0u8; len];
            let expected_handled =
                rgba_to_premultiplied_gray_rgba8_scalar(&src, &mut premultiplied_gray_expected);
            let handled = rgba_to_premultiplied_gray_rgba8(&src, premultiplied_gray);
            if handled {
                assert_eq!(
                    premultiplied_gray,
                    premultiplied_gray_expected.as_slice(),
                    "premultiplied gray rgba pixels={pixels}"
                );
            } else {
                assert_eq!(
                    premultiplied_gray,
                    premultiplied_gray_before.as_slice(),
                    "premultiplied gray rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut premultiplied_gray_bgra_backing = vec![0xadu8; len + 2];
            let premultiplied_gray_bgra = &mut premultiplied_gray_bgra_backing[1..=len];
            let premultiplied_gray_bgra_before = premultiplied_gray_bgra.to_vec();
            let mut premultiplied_gray_bgra_expected = vec![0u8; len];
            let expected_handled = rgba_to_premultiplied_gray_bgra8_scalar(
                &src,
                &mut premultiplied_gray_bgra_expected,
            );
            let handled = rgba_to_premultiplied_gray_bgra8(&src, premultiplied_gray_bgra);
            if handled {
                assert_eq!(
                    premultiplied_gray_bgra,
                    premultiplied_gray_bgra_expected.as_slice(),
                    "premultiplied gray bgra pixels={pixels}"
                );
            } else {
                assert_eq!(
                    premultiplied_gray_bgra,
                    premultiplied_gray_bgra_before.as_slice(),
                    "premultiplied gray bgra fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut copy_backing = vec![0xadu8; len + 2];
            let copy = &mut copy_backing[1..=len];
            let copy_before = copy.to_vec();
            let mut copy_expected = vec![0u8; len];
            let expected_handled = copy_rgba_scalar(&src, &mut copy_expected);
            let handled = copy_rgba(&src, copy);
            if handled {
                assert_eq!(copy, copy_expected.as_slice(), "copy rgba pixels={pixels}");
            } else {
                assert_eq!(
                    copy,
                    copy_before.as_slice(),
                    "copy rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut opaque_rgba_backing = vec![0xadu8; len + 2];
            let opaque_rgba = &mut opaque_rgba_backing[1..=len];
            let opaque_rgba_before = opaque_rgba.to_vec();
            let mut opaque_rgba_expected = vec![0u8; len];
            let expected_handled = rgba_to_opaque_rgba_scalar(&src, &mut opaque_rgba_expected);
            let handled = rgba_to_opaque_rgba(&src, opaque_rgba);
            if handled {
                assert_eq!(
                    opaque_rgba,
                    opaque_rgba_expected.as_slice(),
                    "opaque rgba pixels={pixels}"
                );
            } else {
                assert_eq!(
                    opaque_rgba,
                    opaque_rgba_before.as_slice(),
                    "opaque rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut reverse_backing = vec![0xadu8; len + 2];
            reverse_backing[1..=len].copy_from_slice(&src);
            let reverse = &mut reverse_backing[1..=len];
            let reverse_before = reverse.to_vec();
            let mut reverse_expected = reverse_before.clone();
            let expected_handled = reverse_4byte_words_scalar(&mut reverse_expected);
            let handled = reverse_4byte_words_in_place(reverse);
            if handled {
                assert_eq!(
                    reverse,
                    reverse_expected.as_slice(),
                    "reverse 4-byte words pixels={pixels}"
                );
            } else {
                assert_eq!(
                    reverse,
                    reverse_before.as_slice(),
                    "reverse 4-byte words fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut rgb_backing = vec![0xadu8; pixels * 3 + 2];
            let rgb = &mut rgb_backing[1..=pixels * 3];
            let rgb_before = rgb.to_vec();
            let mut rgb_expected = rgb_before.clone();
            let expected_handled = rgba_to_rgb8_scalar(&src, &mut rgb_expected);
            let handled = rgba_to_rgb8(&src, rgb);
            if handled {
                assert_eq!(rgb, rgb_expected.as_slice(), "rgb pixels={pixels}");
            } else {
                assert_eq!(rgb, rgb_before.as_slice(), "rgb fallback pixels={pixels}");
            }
            assert!(expected_handled);

            let rgb_source = rgb_expected.clone();
            let mut opaque_from_rgb_backing = vec![0xadu8; len + 2];
            let opaque_from_rgb = &mut opaque_from_rgb_backing[1..=len];
            let opaque_from_rgb_before = opaque_from_rgb.to_vec();
            let mut opaque_from_rgb_expected = vec![0u8; len];
            let expected_handled =
                rgb8_to_opaque_rgba_scalar(&rgb_source, &mut opaque_from_rgb_expected);
            let handled = rgb8_to_opaque_rgba(&rgb_source, opaque_from_rgb);
            if handled {
                assert_eq!(
                    opaque_from_rgb,
                    opaque_from_rgb_expected.as_slice(),
                    "rgb to opaque rgba pixels={pixels}"
                );
            } else {
                assert_eq!(
                    opaque_from_rgb,
                    opaque_from_rgb_before.as_slice(),
                    "rgb to opaque rgba fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut bgr_backing = vec![0xadu8; pixels * 3 + 2];
            let bgr = &mut bgr_backing[1..=pixels * 3];
            let bgr_before = bgr.to_vec();
            let mut bgr_expected = bgr_before.clone();
            let expected_handled = rgba_to_bgr8_scalar(&src, &mut bgr_expected);
            let handled = rgba_to_bgr8(&src, bgr);
            if handled {
                assert_eq!(bgr, bgr_expected.as_slice(), "bgr pixels={pixels}");
            } else {
                assert_eq!(bgr, bgr_before.as_slice(), "bgr fallback pixels={pixels}");
            }
            assert!(expected_handled);

            let mut bgra_backing = vec![0xadu8; len + 2];
            let bgra = &mut bgra_backing[1..=len];
            let bgra_before = bgra.to_vec();
            let mut bgra_expected = vec![0u8; len];
            let expected_handled = rgba_to_bgra8_scalar(&src, &mut bgra_expected, false);
            let handled = rgba_to_bgra8(&src, bgra, false);
            if handled {
                assert_eq!(bgra, bgra_expected.as_slice(), "bgra pixels={pixels}");
            } else {
                assert_eq!(
                    bgra,
                    bgra_before.as_slice(),
                    "bgra fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut opaque_bgra_backing = vec![0xadu8; len + 2];
            let opaque_bgra = &mut opaque_bgra_backing[1..=len];
            let opaque_bgra_before = opaque_bgra.to_vec();
            let mut opaque_bgra_expected = vec![0u8; len];
            let expected_handled = rgba_to_bgra8_scalar(&src, &mut opaque_bgra_expected, true);
            let handled = rgba_to_bgra8(&src, opaque_bgra, true);
            if handled {
                assert_eq!(
                    opaque_bgra,
                    opaque_bgra_expected.as_slice(),
                    "opaque bgra pixels={pixels}"
                );
            } else {
                assert_eq!(
                    opaque_bgra,
                    opaque_bgra_before.as_slice(),
                    "opaque bgra fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut premultiply_backing = vec![0xadu8; len + 2];
            let premultiply = &mut premultiply_backing[1..=len];
            let premultiply_before = premultiply.to_vec();
            let mut premultiply_expected = vec![0u8; len];
            let expected_handled = premultiply_rgba_scalar(&src, &mut premultiply_expected);
            let handled = premultiply_rgba(&src, premultiply);
            if handled {
                assert_eq!(
                    premultiply,
                    premultiply_expected.as_slice(),
                    "premultiply pixels={pixels}"
                );
            } else {
                assert_eq!(
                    premultiply,
                    premultiply_before.as_slice(),
                    "premultiply fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut premultiply_bgra_backing = vec![0xadu8; len + 2];
            let premultiply_bgra = &mut premultiply_bgra_backing[1..=len];
            let premultiply_bgra_before = premultiply_bgra.to_vec();
            let mut premultiply_bgra_expected = vec![0u8; len];
            let expected_handled = premultiply_bgra8_scalar(&src, &mut premultiply_bgra_expected);
            let handled = premultiply_bgra8(&src, premultiply_bgra);
            if handled {
                assert_eq!(
                    premultiply_bgra,
                    premultiply_bgra_expected.as_slice(),
                    "premultiply bgra pixels={pixels}"
                );
            } else {
                assert_eq!(
                    premultiply_bgra,
                    premultiply_bgra_before.as_slice(),
                    "premultiply bgra fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);

            let mut unpremultiply_backing = vec![0xadu8; len + 2];
            let unpremultiply = &mut unpremultiply_backing[1..=len];
            let unpremultiply_before = unpremultiply.to_vec();
            let mut associated = premultiply_expected.clone();
            if pixels > 0 {
                associated[0] = 99;
                associated[3] = 0;
            }
            let mut unpremultiply_expected = vec![0u8; len];
            let expected_handled =
                unpremultiply_rgba_scalar(&associated, &mut unpremultiply_expected);
            let handled = unpremultiply_rgba(&associated, unpremultiply);
            if handled {
                assert_eq!(
                    unpremultiply,
                    unpremultiply_expected.as_slice(),
                    "unpremultiply pixels={pixels}"
                );
            } else {
                assert_eq!(
                    unpremultiply,
                    unpremultiply_before.as_slice(),
                    "unpremultiply fallback pixels={pixels}"
                );
            }
            assert!(expected_handled);
        }
    }

    #[test]
    fn blend_alpha_mask_opaque_dst_scalar_varied_masks() {
        let pixels = 16;
        let mut dst = opaque_destination(pixels);
        let before = dst.clone();
        let masks: Vec<u8> = (0..pixels)
            .map(|i| (i as u8).wrapping_mul(23).wrapping_add(5))
            .collect();
        let color = [17, 223, 91, 173];
        assert!(blend_alpha_mask_opaque_dst_scalar(&mut dst, &masks, color));

        let mut dst2 = before.clone();
        blend_alpha_mask_opaque_dst_scalar(&mut dst2, &masks, color);
        assert_eq!(dst, dst2);
        assert_ne!(dst, before);
        for px in dst.chunks_exact(4) {
            assert_eq!(px[3], 255);
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn blend_alpha_mask_opaque_dst_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let mut dst = opaque_destination(pixels);
            let mut expected = dst.clone();
            let masks: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 3, 64, 127, 192, 255][index % 6])
                .collect();
            let color = [37, 191, 83, 219];

            assert!(blend_alpha_mask_opaque_dst_scalar(
                &mut expected,
                &masks,
                color
            ));
            assert!(blend_alpha_mask_opaque_destination(&mut dst, &masks, color));
            assert_eq!(dst, expected, "pixels={pixels}");
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn blend_alpha_mask_normal_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let mut dst = source_row(pixels);
            for (index, px) in dst.chunks_exact_mut(4).enumerate() {
                px[3] = [0u8, 17, 64, 127, 203, 255][index % 6];
            }
            let mut expected = dst.clone();
            let masks: Vec<u8> = (0..pixels)
                .map(|index| [0u8, 5, 73, 129, 211, 255][index % 6])
                .collect();
            let color = [41, 199, 83, 211];

            assert!(blend_alpha_mask_normal_scalar(&mut expected, &masks, color));
            assert!(blend_alpha_mask_normal(&mut dst, &masks, color));
            assert_eq!(dst, expected, "pixels={pixels}");
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn blend_separable_opaque_dst_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            for blend_mode in separable_blend_modes() {
                let color = [163, 47, 219, 255];
                let mut expected = opaque_destination(pixels);
                assert!(blend_separable_opaque_dst_scalar(
                    &mut expected,
                    color,
                    blend_mode
                ));

                let mut actual = opaque_destination(pixels);
                assert!(blend_separable_opaque_destination(
                    &mut actual,
                    color,
                    blend_mode
                ));
                assert_eq!(actual, expected, "pixels={pixels} blend={blend_mode:?}");
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn flatten_opaque_background_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let mut expected = mixed_alpha_destination(pixels);
            let background = [29, 113, 197, 255];
            assert!(flatten_opaque_background_scalar(&mut expected, background));

            let mut actual = mixed_alpha_destination(pixels);
            assert!(flatten_opaque_background(&mut actual, background));
            assert_eq!(actual, expected, "pixels={pixels}");
            for px in actual.chunks_exact(4) {
                assert_eq!(px[3], 255);
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn rgba_copy_and_opaque_public_use_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let source = source_row(pixels);

            let mut expected_copy = vec![0u8; pixels * 4];
            assert!(copy_rgba_scalar(&source, &mut expected_copy));
            let mut actual_copy = vec![0u8; pixels * 4];
            assert!(copy_rgba(&source, &mut actual_copy));
            assert_eq!(actual_copy, expected_copy, "copy pixels={pixels}");

            let mut expected_opaque = vec![0u8; pixels * 4];
            assert!(rgba_to_opaque_rgba_scalar(&source, &mut expected_opaque));
            let mut actual_opaque = vec![0u8; pixels * 4];
            assert!(rgba_to_opaque_rgba(&source, &mut actual_opaque));
            assert_eq!(actual_opaque, expected_opaque, "opaque pixels={pixels}");
            for px in actual_opaque.chunks_exact(4) {
                assert_eq!(px[3], 255);
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn premultiply_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let source = source_row(pixels);

            let mut expected_rgba = vec![0u8; pixels * 4];
            assert!(premultiply_rgba_scalar(&source, &mut expected_rgba));
            let mut actual_rgba = vec![0u8; pixels * 4];
            assert!(premultiply_rgba(&source, &mut actual_rgba));
            assert_eq!(actual_rgba, expected_rgba, "rgba pixels={pixels}");

            let mut expected_bgra = vec![0u8; pixels * 4];
            assert!(premultiply_bgra8_scalar(&source, &mut expected_bgra));
            let mut actual_bgra = vec![0u8; pixels * 4];
            assert!(premultiply_bgra8(&source, &mut actual_bgra));
            assert_eq!(actual_bgra, expected_bgra, "bgra pixels={pixels}");
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn unpremultiply_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let mut source = source_row(pixels);
            for (index, px) in source.chunks_exact_mut(4).enumerate() {
                px[3] = match index % 5 {
                    0 => 0,
                    1 => 1,
                    2 => 64,
                    3 => 173,
                    _ => 255,
                };
            }
            if pixels > 0 {
                source[0] = 99;
                source[1] = 57;
                source[2] = 13;
                source[3] = 0;
            }

            let mut expected = vec![0u8; pixels * 4];
            assert!(unpremultiply_rgba_scalar(&source, &mut expected));
            let mut actual = vec![0u8; pixels * 4];
            assert!(unpremultiply_rgba(&source, &mut actual));
            assert_eq!(actual, expected, "pixels={pixels}");
            if pixels > 0 {
                assert_eq!(&actual[..4], &[0, 0, 0, 0]);
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn rgba_to_bgra_public_uses_sse2_when_available() {
        if !std::is_x86_feature_detected!("sse2") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let source = source_row(pixels);
            for opaque in [false, true] {
                let mut expected = vec![0u8; pixels * 4];
                assert!(rgba_to_bgra8_scalar(&source, &mut expected, opaque));
                let mut actual = vec![0u8; pixels * 4];
                assert!(rgba_to_bgra8(&source, &mut actual, opaque));
                assert_eq!(actual, expected, "pixels={pixels} opaque={opaque}");
                if opaque {
                    for px in actual.chunks_exact(4) {
                        assert_eq!(px[3], 255);
                    }
                }
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn rgb_bgr_channel_public_uses_ssse3_when_available() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let source = source_row(pixels);

            let mut expected_rgb = vec![0u8; pixels * 3];
            assert!(rgba_to_rgb8_scalar(&source, &mut expected_rgb));
            let mut actual_rgb = vec![0u8; pixels * 3];
            assert!(rgba_to_rgb8(&source, &mut actual_rgb));
            assert_eq!(actual_rgb, expected_rgb, "rgb pixels={pixels}");

            let mut expected_bgr = vec![0u8; pixels * 3];
            assert!(rgba_to_bgr8_scalar(&source, &mut expected_bgr));
            let mut actual_bgr = vec![0u8; pixels * 3];
            assert!(rgba_to_bgr8(&source, &mut actual_bgr));
            assert_eq!(actual_bgr, expected_bgr, "bgr pixels={pixels}");

            let mut expected_rgba = vec![0u8; pixels * 4];
            assert!(rgb8_to_opaque_rgba_scalar(
                &expected_rgb,
                &mut expected_rgba
            ));
            let mut actual_rgba = vec![0u8; pixels * 4];
            assert!(rgb8_to_opaque_rgba(&expected_rgb, &mut actual_rgba));
            assert_eq!(actual_rgba, expected_rgba, "opaque rgba pixels={pixels}");
            for px in actual_rgba.chunks_exact(4) {
                assert_eq!(px[3], 255);
            }
        }
    }

    #[test]
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    fn grayscale_public_uses_ssse3_when_available() {
        if !std::is_x86_feature_detected!("ssse3") {
            return;
        }
        for pixels in [4usize, 5, 8, 9, 16, 17] {
            let source = source_row(pixels);

            let mut expected_gray = vec![0u8; pixels];
            assert!(rgba_to_gray8_scalar(&source, &mut expected_gray));
            let mut actual_gray = vec![0u8; pixels];
            assert!(rgba_to_gray8(&source, &mut actual_gray));
            assert_eq!(actual_gray, expected_gray, "gray pixels={pixels}");

            let mut expected_rgb = vec![0u8; pixels * 3];
            assert!(rgba_to_gray_rgb8_scalar(&source, &mut expected_rgb));
            let mut actual_rgb = vec![0u8; pixels * 3];
            assert!(rgba_to_gray_rgb8(&source, &mut actual_rgb));
            assert_eq!(actual_rgb, expected_rgb, "gray rgb pixels={pixels}");

            for opaque in [false, true] {
                let mut expected_rgba = vec![0u8; pixels * 4];
                assert!(rgba_to_gray_rgba8_scalar(
                    &source,
                    &mut expected_rgba,
                    opaque
                ));
                let mut actual_rgba = vec![0u8; pixels * 4];
                assert!(rgba_to_gray_rgba8(&source, &mut actual_rgba, opaque));
                assert_eq!(
                    actual_rgba, expected_rgba,
                    "gray rgba pixels={pixels} opaque={opaque}"
                );

                let mut expected_bgra = vec![0u8; pixels * 4];
                assert!(rgba_to_gray_bgra8_scalar(
                    &source,
                    &mut expected_bgra,
                    opaque
                ));
                let mut actual_bgra = vec![0u8; pixels * 4];
                assert!(rgba_to_gray_bgra8(&source, &mut actual_bgra, opaque));
                assert_eq!(
                    actual_bgra, expected_bgra,
                    "gray bgra pixels={pixels} opaque={opaque}"
                );
            }

            let mut expected_premultiplied = vec![0u8; pixels * 4];
            assert!(rgba_to_premultiplied_gray_rgba8_scalar(
                &source,
                &mut expected_premultiplied
            ));
            let mut actual_premultiplied = vec![0u8; pixels * 4];
            assert!(rgba_to_premultiplied_gray_rgba8(
                &source,
                &mut actual_premultiplied
            ));
            assert_eq!(
                actual_premultiplied, expected_premultiplied,
                "premultiplied gray rgba pixels={pixels}"
            );

            let mut expected_premultiplied_bgra = vec![0u8; pixels * 4];
            assert!(rgba_to_premultiplied_gray_bgra8_scalar(
                &source,
                &mut expected_premultiplied_bgra
            ));
            let mut actual_premultiplied_bgra = vec![0u8; pixels * 4];
            assert!(rgba_to_premultiplied_gray_bgra8(
                &source,
                &mut actual_premultiplied_bgra
            ));
            assert_eq!(
                actual_premultiplied_bgra, expected_premultiplied_bgra,
                "premultiplied gray bgra pixels={pixels}"
            );
        }
    }

    /// Scalar-equivalence test for blend_normal_opaque_dst covering all alpha values.
    /// Runs on any host and exercises the scalar path used as the equivalence reference.
    #[test]
    fn blend_normal_opaque_dst_scalar_exhaustive_alpha() {
        let base_dst = opaque_destination(8);
        for alpha in 1u8..=254 {
            let color = [200, 100, 50, alpha];
            let mut dst = base_dst.clone();
            let ok = blend_normal_opaque_dst_scalar(&mut dst, color);
            assert!(ok);
            // Verify alpha channel forced to 255
            for px in dst.chunks_exact(4) {
                assert_eq!(px[3], 255);
            }
            // Verify deterministic rounding: run again and compare
            let mut dst2 = base_dst.clone();
            blend_normal_opaque_dst_scalar(&mut dst2, color);
            assert_eq!(dst, dst2);
        }
    }

    /// Scalar-equivalence test for composite_normal_opaque_dst with varied per-pixel alpha.
    #[test]
    fn composite_normal_opaque_dst_scalar_varied_alpha() {
        let pixels = 16;
        let src: Vec<u8> = (0..pixels)
            .flat_map(|i| {
                let i = i as u8;
                [
                    i.wrapping_mul(37),
                    i.wrapping_mul(59),
                    i.wrapping_mul(73),
                    i.wrapping_mul(17),
                ]
            })
            .collect();
        let mut dst = opaque_destination(pixels);
        let dst_copy = dst.clone();
        let ok = composite_normal_opaque_dst_scalar(&mut dst, &src);
        assert!(ok);
        // Verify determinism
        let mut dst2 = dst_copy.clone();
        composite_normal_opaque_dst_scalar(&mut dst2, &src);
        assert_eq!(dst, dst2);
        // Verify alpha channel preservation
        for px in dst.chunks_exact(4) {
            assert_eq!(px[3], 255);
        }
    }

    /// Scalar-equivalence test for soft_mask_opaque_dst with diverse mask values.
    #[test]
    fn soft_mask_opaque_dst_scalar_diverse_masks() {
        let pixels = 16;
        let src = source_row(pixels);
        let masks: Vec<u8> = (0..pixels).map(|i| (i as u8).wrapping_mul(17)).collect();
        let dst_copy = opaque_destination(pixels);
        for group_alpha in [255_u16, 193] {
            let mut dst = dst_copy.clone();
            let ok = soft_mask_opaque_dst_scalar(&mut dst, &src, &masks, group_alpha);
            assert!(ok);
            // Determinism
            let mut dst2 = dst_copy.clone();
            soft_mask_opaque_dst_scalar(&mut dst2, &src, &masks, group_alpha);
            assert_eq!(dst, dst2, "group_alpha={group_alpha}");
            // Alpha forced to 255
            for px in dst.chunks_exact(4) {
                assert_eq!(px[3], 255, "group_alpha={group_alpha}");
            }
        }
    }

    #[test]
    fn rgba_to_gray8_scalar_matches_contract_luma_weights() {
        let src = [
            0, 0, 0, 255, 255, 255, 255, 128, 255, 0, 0, 7, 0, 255, 0, 9, 0, 0, 255, 11,
        ];
        let mut gray = [0u8; 5];
        assert!(rgba_to_gray8_scalar(&src, &mut gray));
        assert_eq!(gray, [0, 255, 77, 149, 29]);

        let mut gray_rgb = [0u8; 15];
        let mut gray_rgba = [0u8; 20];
        let mut opaque_gray_rgba = [0u8; 20];
        let mut gray_bgra = [0u8; 20];
        let mut premultiplied_gray = [0u8; 20];
        let mut premultiplied_gray_bgra = [0u8; 20];
        assert!(rgba_to_gray_rgb8_scalar(&src, &mut gray_rgb));
        assert!(rgba_to_gray_rgba8_scalar(&src, &mut gray_rgba, false));
        assert!(rgba_to_gray_rgba8_scalar(&src, &mut opaque_gray_rgba, true));
        assert!(rgba_to_gray_bgra8_scalar(&src, &mut gray_bgra, false));
        assert!(rgba_to_premultiplied_gray_rgba8_scalar(
            &src,
            &mut premultiplied_gray
        ));
        assert!(rgba_to_premultiplied_gray_bgra8_scalar(
            &src,
            &mut premultiplied_gray_bgra
        ));
        assert_eq!(
            gray_rgb,
            [0, 0, 0, 255, 255, 255, 77, 77, 77, 149, 149, 149, 29, 29, 29]
        );
        assert_eq!(
            gray_rgba,
            [0, 0, 0, 255, 255, 255, 255, 128, 77, 77, 77, 7, 149, 149, 149, 9, 29, 29, 29, 11]
        );
        assert_eq!(
            opaque_gray_rgba,
            [
                0, 0, 0, 255, 255, 255, 255, 255, 77, 77, 77, 255, 149, 149, 149, 255, 29, 29, 29,
                255
            ]
        );
        assert_eq!(gray_bgra, gray_rgba);
        assert_eq!(
            premultiplied_gray,
            [0, 0, 0, 255, 128, 128, 128, 128, 2, 2, 2, 7, 5, 5, 5, 9, 1, 1, 1, 11]
        );
        assert_eq!(premultiplied_gray_bgra, premultiplied_gray);
    }

    #[test]
    fn rgb_bgr_bgra_scalar_channel_conversions_match_contract_order() {
        let src = [
            1, 2, 3, 4, 10, 20, 30, 40, 100, 110, 120, 130, 200, 210, 220, 230,
        ];
        let mut rgba = [0u8; 16];
        let mut opaque_rgba = [0u8; 16];
        let mut rgb = [0u8; 12];
        let mut opaque_from_rgb = [0u8; 16];
        let mut bgr = [0u8; 12];
        let mut bgra = [0u8; 16];
        let mut premultiplied_bgra = [0u8; 16];
        let mut opaque_bgra = [0u8; 16];

        assert!(copy_rgba_scalar(&src, &mut rgba));
        assert!(rgba_to_opaque_rgba_scalar(&src, &mut opaque_rgba));
        assert!(rgba_to_rgb8_scalar(&src, &mut rgb));
        assert!(rgb8_to_opaque_rgba_scalar(&rgb, &mut opaque_from_rgb));
        assert!(rgba_to_bgr8_scalar(&src, &mut bgr));
        assert!(rgba_to_bgra8_scalar(&src, &mut bgra, false));
        assert!(premultiply_bgra8_scalar(&src, &mut premultiplied_bgra));
        assert!(rgba_to_bgra8_scalar(&src, &mut opaque_bgra, true));

        assert_eq!(rgba, src);
        assert_eq!(
            opaque_rgba,
            [1, 2, 3, 255, 10, 20, 30, 255, 100, 110, 120, 255, 200, 210, 220, 255]
        );
        assert_eq!(rgb, [1, 2, 3, 10, 20, 30, 100, 110, 120, 200, 210, 220]);
        assert_eq!(
            opaque_from_rgb,
            [1, 2, 3, 255, 10, 20, 30, 255, 100, 110, 120, 255, 200, 210, 220, 255]
        );
        assert_eq!(bgr, [3, 2, 1, 30, 20, 10, 120, 110, 100, 220, 210, 200]);
        assert_eq!(
            bgra,
            [3, 2, 1, 4, 30, 20, 10, 40, 120, 110, 100, 130, 220, 210, 200, 230]
        );
        assert_eq!(
            premultiplied_bgra,
            [0, 0, 0, 4, 5, 3, 2, 40, 61, 56, 51, 130, 198, 189, 180, 230]
        );
        assert_eq!(
            opaque_bgra,
            [3, 2, 1, 255, 30, 20, 10, 255, 120, 110, 100, 255, 220, 210, 200, 255]
        );
    }

    #[test]
    fn separable_blend_scalar_common_modes_match_channel_contract() {
        let base = [10, 80, 200, 255, 200, 40, 90, 255];
        let color = [128, 220, 30, 255];
        for blend_mode in separable_blend_modes() {
            let mut dst = base;
            assert!(blend_separable_opaque_dst_scalar(
                &mut dst, color, blend_mode
            ));
            for (out, old) in dst.chunks_exact(4).zip(base.chunks_exact(4)) {
                for channel in 0..3 {
                    assert_eq!(
                        out[channel],
                        blend_separable_channel(color[channel], old[channel], blend_mode),
                        "{blend_mode:?} channel {channel}"
                    );
                }
                assert_eq!(out[3], 255);
            }
        }
    }

    #[test]
    fn separable_blend_extended_modes_match_pdf_byte_contract() {
        for src in 0..=255u8 {
            for dst in 0..=255u8 {
                for blend_mode in [
                    SeparableBlendMode::Overlay,
                    SeparableBlendMode::ColorDodge,
                    SeparableBlendMode::ColorBurn,
                    SeparableBlendMode::HardLight,
                    SeparableBlendMode::SoftLight,
                ] {
                    let src_f = f32::from(src) / 255.0;
                    let dst_f = f32::from(dst) / 255.0;
                    let expected = match blend_mode {
                        SeparableBlendMode::Overlay => {
                            if dst_f <= 0.5 {
                                2.0 * dst_f * src_f
                            } else {
                                1.0 - 2.0 * (1.0 - dst_f) * (1.0 - src_f)
                            }
                        }
                        SeparableBlendMode::ColorDodge => {
                            if dst_f <= 0.0 {
                                0.0
                            } else if src_f >= 1.0 {
                                1.0
                            } else {
                                (dst_f / (1.0 - src_f)).min(1.0)
                            }
                        }
                        SeparableBlendMode::ColorBurn => {
                            if dst_f >= 1.0 {
                                1.0
                            } else if src_f <= 0.0 {
                                0.0
                            } else {
                                1.0 - ((1.0 - dst_f) / src_f).min(1.0)
                            }
                        }
                        SeparableBlendMode::HardLight => {
                            if src_f <= 0.5 {
                                2.0 * src_f * dst_f
                            } else {
                                1.0 - 2.0 * (1.0 - src_f) * (1.0 - dst_f)
                            }
                        }
                        SeparableBlendMode::SoftLight => {
                            if src_f <= 0.5 {
                                dst_f - (1.0 - 2.0 * src_f) * dst_f * (1.0 - dst_f)
                            } else {
                                let d = if dst_f <= 0.25 {
                                    ((16.0 * dst_f - 12.0) * dst_f + 4.0) * dst_f
                                } else {
                                    dst_f.sqrt()
                                };
                                dst_f + (2.0 * src_f - 1.0) * (d - dst_f)
                            }
                        }
                        _ => unreachable!("test only covers extended separable modes"),
                    };
                    let expected = (expected * 255.0).round().clamp(0.0, 255.0) as u8;
                    assert_eq!(
                        blend_separable_channel(src, dst, blend_mode),
                        expected,
                        "{blend_mode:?} src={src} dst={dst}"
                    );
                }
            }
        }
    }

    #[test]
    fn separable_blend_difference_exclusion_match_byte_contract() {
        for src in 0..=255u8 {
            for dst in 0..=255u8 {
                assert_eq!(
                    blend_separable_channel(src, dst, SeparableBlendMode::Difference),
                    src.abs_diff(dst),
                    "difference src={src} dst={dst}"
                );
                let expected = ((f32::from(src) / 255.0 + f32::from(dst) / 255.0
                    - 2.0 * (f32::from(src) / 255.0) * (f32::from(dst) / 255.0))
                    * 255.0)
                    .round()
                    .clamp(0.0, 255.0) as u8;
                assert_eq!(
                    blend_separable_channel(src, dst, SeparableBlendMode::Exclusion),
                    expected,
                    "exclusion src={src} dst={dst}"
                );
            }
        }
    }

    #[test]
    fn premultiply_rgba_scalar_preserves_alpha_and_scales_rgb() {
        let src = [
            200, 100, 50, 128, 9, 19, 29, 0, 7, 11, 13, 255, 1, 254, 127, 64,
        ];
        let mut dst = [0u8; 16];

        assert!(premultiply_rgba_scalar(&src, &mut dst));

        assert_eq!(
            dst,
            [100, 50, 25, 128, 0, 0, 0, 0, 7, 11, 13, 255, 0, 64, 32, 64]
        );
    }

    #[test]
    fn reverse_4byte_words_scalar_reverses_words_and_leaves_tail() {
        let mut row = [1, 2, 3, 4, 10, 20, 30, 40, 99];

        assert!(reverse_4byte_words_scalar(&mut row));

        assert_eq!(row, [4, 3, 2, 1, 40, 30, 20, 10, 99]);
    }

    #[test]
    fn reverse_4byte_words_public_uses_available_backend() {
        let original = source_row(8);
        let mut expected = original.clone();
        reverse_4byte_words_scalar(&mut expected);
        let mut actual = original.clone();

        let handled = reverse_4byte_words_in_place(&mut actual);

        let backend_should_handle =
            matches!(
                active_backend(),
                SimdBackend::Avx2 | SimdBackend::Ssse3 | SimdBackend::Sse2 | SimdBackend::WasmSimd
            ) || cfg!(all(target_arch = "wasm32", not(target_feature = "simd128")));
        if backend_should_handle {
            assert!(handled, "backend {:?} should reverse row", active_backend());
            assert_eq!(actual, expected);
        } else if handled {
            assert_eq!(actual, expected);
        } else {
            assert_eq!(actual, original);
        }
    }

    #[test]
    fn unpremultiply_rgba_scalar_preserves_alpha_and_scales_rgb() {
        let src = [
            100, 50, 25, 128, 9, 19, 29, 0, 7, 11, 13, 255, 0, 64, 32, 64,
        ];
        let mut dst = [0u8; 16];

        assert!(unpremultiply_rgba_scalar(&src, &mut dst));

        assert_eq!(
            dst,
            [199, 100, 50, 128, 0, 0, 0, 0, 7, 11, 13, 255, 0, 255, 128, 64]
        );
    }

    /// Verifies that the WASM SIMD kernels are selected when compiling for wasm32+simd128.
    /// On non-wasm hosts this test verifies the compile gates are correct by checking
    /// that the active backend is not WasmSimd.
    #[test]
    fn backend_detection_consistent() {
        let backend = active_backend();
        #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
        assert_eq!(backend, SimdBackend::WasmSimd);
        #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
        assert_ne!(backend, SimdBackend::WasmSimd);
    }

    /// WASM SIMD128 direct kernel test (only compiled for wasm32+simd128 targets).
    /// When cross-compiled with `--target wasm32-unknown-unknown -C target-feature=+simd128`
    /// and run with a wasm test runner, this exercises the actual SIMD paths.
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_blend_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let color = [180, 90, 45, 128];
            let mut expected = opaque_destination(pixels);
            blend_normal_opaque_dst_scalar(&mut expected, color);

            let mut actual = opaque_destination(pixels);
            let ok = unsafe { blend_normal_opaque_dst_wasm_simd128(&mut actual, color) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "blend wasm simd128 pixels={pixels}");
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_alpha_mask_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let masks: Vec<u8> = (0..pixels)
                .map(|i| (i as u8).wrapping_mul(29).wrapping_add(3))
                .collect();
            let color = [41, 199, 83, 211];
            let mut expected = opaque_destination(pixels);
            blend_alpha_mask_opaque_dst_scalar(&mut expected, &masks, color);

            let mut actual = opaque_destination(pixels);
            let ok =
                unsafe { blend_alpha_mask_opaque_dst_wasm_simd128(&mut actual, &masks, color) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "alpha-mask wasm simd128 pixels={pixels}");
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_multiply_alpha_rows_matches_scalar() {
        for pixels in [16usize, 17, 31, 32] {
            let mut expected: Vec<u8> = (0..pixels)
                .map(|i| (i as u8).wrapping_mul(17).wrapping_add(5))
                .collect();
            let masks: Vec<u8> = (0..pixels)
                .map(|i| (i as u8).wrapping_mul(29).wrapping_add(3))
                .collect();
            multiply_alpha_rows_scalar(&mut expected, &masks);

            let mut actual: Vec<u8> = (0..pixels)
                .map(|i| (i as u8).wrapping_mul(17).wrapping_add(5))
                .collect();
            let ok = unsafe { multiply_alpha_rows_wasm_simd128(&mut actual, &masks) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "alpha-row wasm simd128 pixels={pixels}");
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_separable_blend_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            for blend_mode in separable_blend_modes() {
                let color = [163, 47, 219, 255];
                let mut expected = opaque_destination(pixels);
                blend_separable_opaque_dst_scalar(&mut expected, color, blend_mode);

                let mut actual = opaque_destination(pixels);
                let ok = unsafe {
                    blend_separable_opaque_dst_wasm_simd128(&mut actual, color, blend_mode)
                };
                assert!(ok, "pixels={pixels} blend={blend_mode:?}");
                assert_eq!(
                    actual, expected,
                    "separable blend wasm simd128 pixels={pixels} blend={blend_mode:?}"
                );
            }
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_composite_normal_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let src = source_row(pixels);
            let mut expected = opaque_destination(pixels);
            composite_normal_opaque_dst_scalar(&mut expected, &src);

            let mut actual = opaque_destination(pixels);
            let ok = unsafe { composite_normal_opaque_dst_wasm_simd128(&mut actual, &src) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "composite wasm simd128 pixels={pixels}");
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_flatten_opaque_background_matches_scalar() {
        for pixels in [4usize, 5, 8, 9, 16, 17, 20] {
            let background = [29, 113, 197, 255];
            let mut expected = mixed_alpha_destination(pixels);
            flatten_opaque_background_scalar(&mut expected, background);

            let mut actual = mixed_alpha_destination(pixels);
            let ok = unsafe { flatten_opaque_background_wasm_simd128(&mut actual, background) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "flatten wasm simd128 pixels={pixels}");
            for px in actual.chunks_exact(4) {
                assert_eq!(px[3], 255);
            }
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_soft_mask_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let src = source_row(pixels);
            let masks: Vec<u8> = (0..pixels)
                .map(|i| (i as u8).wrapping_mul(31).wrapping_add(7))
                .collect();
            for group_alpha in [255_u16, 137] {
                let mut expected = opaque_destination(pixels);
                soft_mask_opaque_dst_scalar(&mut expected, &src, &masks, group_alpha);

                let mut actual = opaque_destination(pixels);
                let ok = unsafe {
                    soft_mask_opaque_dst_wasm_simd128(&mut actual, &src, &masks, group_alpha)
                };
                assert!(ok, "pixels={pixels} group_alpha={group_alpha}");
                assert_eq!(
                    actual, expected,
                    "soft mask wasm simd128 pixels={pixels} group_alpha={group_alpha}"
                );
            }
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_rgba_to_gray8_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let src = source_row(pixels);
            let mut expected = vec![0u8; pixels];
            rgba_to_gray8_scalar(&src, &mut expected);

            let mut actual = vec![0u8; pixels];
            let ok = unsafe { rgba_to_gray8_wasm_simd128(&src, &mut actual) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(actual, expected, "gray wasm simd128 pixels={pixels}");

            let mut expected_rgb = vec![0u8; pixels * 3];
            rgba_to_gray_rgb8_scalar(&src, &mut expected_rgb);
            let mut actual_rgb = vec![0u8; pixels * 3];
            let ok = unsafe { rgba_to_gray_rgb8_wasm_simd128(&src, &mut actual_rgb) };
            assert!(ok, "gray rgb pixels={pixels}");
            assert_eq!(
                actual_rgb, expected_rgb,
                "gray rgb wasm simd128 pixels={pixels}"
            );

            for opaque in [false, true] {
                let mut expected_rgba = vec![0u8; pixels * 4];
                rgba_to_gray_rgba8_scalar(&src, &mut expected_rgba, opaque);
                let mut actual_rgba = vec![0u8; pixels * 4];
                let ok = unsafe { rgba_to_gray_rgba8_wasm_simd128(&src, &mut actual_rgba, opaque) };
                assert!(ok, "gray rgba pixels={pixels} opaque={opaque}");
                assert_eq!(
                    actual_rgba, expected_rgba,
                    "gray rgba wasm simd128 pixels={pixels} opaque={opaque}"
                );

                let mut expected_bgra = vec![0u8; pixels * 4];
                rgba_to_gray_bgra8_scalar(&src, &mut expected_bgra, opaque);
                let mut actual_bgra = vec![0u8; pixels * 4];
                let ok = unsafe { rgba_to_gray_bgra8_wasm_simd128(&src, &mut actual_bgra, opaque) };
                assert!(ok, "gray bgra pixels={pixels} opaque={opaque}");
                assert_eq!(
                    actual_bgra, expected_bgra,
                    "gray bgra wasm simd128 pixels={pixels} opaque={opaque}"
                );
            }

            let mut expected_premultiplied = vec![0u8; pixels * 4];
            rgba_to_premultiplied_gray_rgba8_scalar(&src, &mut expected_premultiplied);
            let mut actual_premultiplied = vec![0u8; pixels * 4];
            let ok = unsafe {
                rgba_to_premultiplied_gray_rgba8_wasm_simd128(&src, &mut actual_premultiplied)
            };
            assert!(ok, "premultiplied gray pixels={pixels}");
            assert_eq!(
                actual_premultiplied, expected_premultiplied,
                "premultiplied gray wasm simd128 pixels={pixels}"
            );

            let mut actual_premultiplied_bgra = vec![0u8; pixels * 4];
            assert!(
                rgba_to_premultiplied_gray_bgra8(&src, &mut actual_premultiplied_bgra),
                "premultiplied gray bgra public wrapper pixels={pixels}"
            );
            assert_eq!(
                actual_premultiplied_bgra, expected_premultiplied,
                "premultiplied gray bgra public wrapper pixels={pixels}"
            );
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_rgb_bgr_bgra_conversions_match_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let src = source_row(pixels);

            let mut expected_rgba = vec![0u8; pixels * 4];
            copy_rgba_scalar(&src, &mut expected_rgba);
            let mut actual_rgba = vec![0u8; pixels * 4];
            assert!(unsafe { copy_rgba_wasm_simd128(&src, &mut actual_rgba) });
            assert_eq!(
                actual_rgba, expected_rgba,
                "copy rgba wasm simd128 pixels={pixels}"
            );

            let mut expected_opaque_rgba = vec![0u8; pixels * 4];
            rgba_to_opaque_rgba_scalar(&src, &mut expected_opaque_rgba);
            let mut actual_opaque_rgba = vec![0u8; pixels * 4];
            assert!(unsafe { rgba_to_opaque_rgba_wasm_simd128(&src, &mut actual_opaque_rgba) });
            assert_eq!(
                actual_opaque_rgba, expected_opaque_rgba,
                "opaque rgba wasm simd128 pixels={pixels}"
            );

            let mut expected_reversed = src.clone();
            reverse_4byte_words_scalar(&mut expected_reversed);
            let mut actual_reversed = src.clone();
            assert!(unsafe { reverse_4byte_words_wasm_simd128(&mut actual_reversed) });
            assert_eq!(
                actual_reversed, expected_reversed,
                "reverse 4-byte words wasm simd128 pixels={pixels}"
            );

            let mut expected_rgb = vec![0u8; pixels * 3];
            rgba_to_rgb8_scalar(&src, &mut expected_rgb);
            let mut actual_rgb = vec![0u8; pixels * 3];
            assert!(unsafe { rgba_to_rgb8_wasm_simd128(&src, &mut actual_rgb) });
            assert_eq!(actual_rgb, expected_rgb, "rgb wasm simd128 pixels={pixels}");

            let mut expected_opaque_from_rgb = vec![0u8; pixels * 4];
            rgb8_to_opaque_rgba_scalar(&expected_rgb, &mut expected_opaque_from_rgb);
            let mut actual_opaque_from_rgb = vec![0u8; pixels * 4];
            assert!(unsafe {
                rgb8_to_opaque_rgba_wasm_simd128(&expected_rgb, &mut actual_opaque_from_rgb)
            });
            assert_eq!(
                actual_opaque_from_rgb, expected_opaque_from_rgb,
                "rgb to opaque rgba wasm simd128 pixels={pixels}"
            );

            let mut expected_bgr = vec![0u8; pixels * 3];
            rgba_to_bgr8_scalar(&src, &mut expected_bgr);
            let mut actual_bgr = vec![0u8; pixels * 3];
            assert!(unsafe { rgba_to_bgr8_wasm_simd128(&src, &mut actual_bgr) });
            assert_eq!(actual_bgr, expected_bgr, "bgr wasm simd128 pixels={pixels}");

            for opaque in [false, true] {
                let mut expected_bgra = vec![0u8; pixels * 4];
                rgba_to_bgra8_scalar(&src, &mut expected_bgra, opaque);
                let mut actual_bgra = vec![0u8; pixels * 4];
                assert!(unsafe { rgba_to_bgra8_wasm_simd128(&src, &mut actual_bgra, opaque) });
                assert_eq!(
                    actual_bgra, expected_bgra,
                    "bgra wasm simd128 pixels={pixels} opaque={opaque}"
                );
            }
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_premultiply_rgba_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let src = source_row(pixels);
            let mut expected = vec![0u8; pixels * 4];
            premultiply_rgba_scalar(&src, &mut expected);

            let mut actual = vec![0u8; pixels * 4];
            let ok = unsafe { premultiply_rgba_wasm_simd128(&src, &mut actual) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(
                actual, expected,
                "premultiply rgba wasm simd128 pixels={pixels}"
            );

            let mut expected_bgra = vec![0u8; pixels * 4];
            premultiply_bgra8_scalar(&src, &mut expected_bgra);
            let mut actual_bgra = vec![0u8; pixels * 4];
            let ok = unsafe { premultiply_bgra8_wasm_simd128(&src, &mut actual_bgra) };
            assert!(ok, "bgra pixels={pixels}");
            assert_eq!(
                actual_bgra, expected_bgra,
                "premultiply bgra wasm simd128 pixels={pixels}"
            );
        }
    }

    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    #[test]
    fn wasm_simd128_unpremultiply_rgba_matches_scalar() {
        for pixels in [4usize, 8, 12, 16, 20] {
            let straight = source_row(pixels);
            let mut src = vec![0u8; pixels * 4];
            premultiply_rgba_scalar(&straight, &mut src);
            if pixels > 0 {
                src[0] = 211;
                src[3] = 0;
            }
            let mut expected = vec![0u8; pixels * 4];
            unpremultiply_rgba_scalar(&src, &mut expected);

            let mut actual = vec![0u8; pixels * 4];
            let ok = unsafe { unpremultiply_rgba_wasm_simd128(&src, &mut actual) };
            assert!(ok, "pixels={pixels}");
            assert_eq!(
                actual, expected,
                "unpremultiply rgba wasm simd128 pixels={pixels}"
            );
        }
    }
}
