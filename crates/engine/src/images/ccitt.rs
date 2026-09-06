use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;

/// PDF /CCITTFaxDecode parameters relevant to bi-level image decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcittDecodeParams {
    pub k: i64,
    pub columns: u32,
    pub rows: u32,
    pub black_is_1: bool,
    pub encoded_byte_align: bool,
    pub end_of_line: bool,
    pub end_of_block: bool,
}

/// Source rectangle for bounded CCITT grayscale output.
///
/// The CCITT bitstream is still decoded sequentially because Group 3/4 coding
/// depends on prior row state, but the sink allocates and retains only this
/// requested sample window instead of materializing the full bitmap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CcittDecodeWindow {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Decode a CCITT Group 3/Group 4 fax stream into Wellfriend's grayscale RawImage.
pub fn decode(data: &[u8], params: CcittDecodeParams) -> Result<RawImage> {
    if params.columns == 0 || params.rows == 0 {
        return Ok(RawImage {
            width: params.columns,
            height: params.rows,
            channels: 1,
            bits_per_sample: 8,
            pixels: Vec::new(),
        });
    }

    let settings = ccitt_settings(params)?;
    // H-5: bound /Columns × /Rows before allocating the grayscale sink, so a
    // crafted `/Columns 100000 /Rows 100000` cannot force a ~10 GB reservation.
    crate::images::decoder::ensure_decode_budget(params.columns, params.rows, 1)?;
    let mut sink = GrayscaleSink::new(params.columns, params.rows);

    decode_with_settings(data, settings, &mut sink)?;

    sink.finish()
}

/// Decode a bounded source window from a CCITT image.
///
/// This is exact for grayscale output and bounds retained pixels to the
/// requested window. Callers that paint the cropped result must adjust the
/// image CTM to the same source rectangle.
pub fn decode_window(
    data: &[u8],
    params: CcittDecodeParams,
    window: CcittDecodeWindow,
) -> Result<RawImage> {
    validate_window(params, window)?;
    if window.width == 0 || window.height == 0 {
        return Ok(RawImage {
            width: window.width,
            height: window.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: Vec::new(),
        });
    }
    crate::images::decoder::ensure_decode_budget(window.width, window.height, 1)?;
    let settings = ccitt_settings(params)?;
    let mut sink = WindowedGrayscaleSink::new(params.columns, params.rows, window);
    decode_with_settings(data, settings, &mut sink)?;
    sink.finish()
}

fn validate_window(params: CcittDecodeParams, window: CcittDecodeWindow) -> Result<()> {
    let end_x = window.x.checked_add(window.width).ok_or_else(|| {
        WellfriendError::MalformedPdf("CCITTFaxDecode source window x range overflows".to_string())
    })?;
    let end_y = window.y.checked_add(window.height).ok_or_else(|| {
        WellfriendError::MalformedPdf("CCITTFaxDecode source window y range overflows".to_string())
    })?;
    if end_x > params.columns || end_y > params.rows {
        return Err(WellfriendError::MalformedPdf(format!(
            "CCITTFaxDecode source window {}:{} {}x{} exceeds image {}x{}",
            window.x, window.y, window.width, window.height, params.columns, params.rows
        )));
    }
    Ok(())
}

fn ccitt_settings(params: CcittDecodeParams) -> Result<hayro_ccitt::DecodeSettings> {
    let encoding = if params.k < 0 {
        hayro_ccitt::EncodingMode::Group4
    } else if params.k == 0 {
        hayro_ccitt::EncodingMode::Group3_1D
    } else {
        hayro_ccitt::EncodingMode::Group3_2D {
            k: u32::try_from(params.k).map_err(|_| {
                WellfriendError::MalformedPdf("CCITTFaxDecode /K is too large".to_string())
            })?,
        }
    };

    Ok(hayro_ccitt::DecodeSettings {
        columns: params.columns,
        rows: params.rows,
        end_of_block: params.end_of_block,
        end_of_line: params.end_of_line,
        rows_are_byte_aligned: params.encoded_byte_align,
        encoding,
        invert_black: params.black_is_1,
    })
}

fn decode_with_settings<D: hayro_ccitt::Decoder>(
    data: &[u8],
    settings: hayro_ccitt::DecodeSettings,
    sink: &mut D,
) -> Result<()> {
    let mut context = hayro_ccitt::DecoderContext::new(settings);
    hayro_ccitt::decode(data, sink, &mut context)
        .map(|_| ())
        .map_err(|err| WellfriendError::MalformedPdf(format!("CCITTFaxDecode failed: {err}")))
}

struct GrayscaleSink {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl GrayscaleSink {
    fn new(width: u32, height: u32) -> Self {
        let expected = width as usize * height as usize;
        Self {
            width,
            height,
            pixels: Vec::with_capacity(expected),
        }
    }

    fn finish(self) -> Result<RawImage> {
        let expected = self.width as usize * self.height as usize;
        if self.pixels.len() != expected {
            return Err(WellfriendError::MalformedPdf(format!(
                "CCITTFaxDecode {}x{} decoded {} pixels, expected {}",
                self.width,
                self.height,
                self.pixels.len(),
                expected
            )));
        }
        Ok(RawImage {
            width: self.width,
            height: self.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: self.pixels,
        })
    }

    fn push_gray(&mut self, white: bool, count: usize) {
        let value = if white { 255 } else { 0 };
        let expected = self.width as usize * self.height as usize;
        let remaining = expected.saturating_sub(self.pixels.len());
        self.pixels
            .extend(std::iter::repeat_n(value, count.min(remaining)));
    }
}

impl hayro_ccitt::Decoder for GrayscaleSink {
    fn push_pixel(&mut self, white: bool) {
        self.push_gray(white, 1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.push_gray(white, chunk_count as usize * 8);
    }

    fn next_line(&mut self) {}
}

struct WindowedGrayscaleSink {
    image_width: u32,
    image_height: u32,
    window: CcittDecodeWindow,
    next_sample: usize,
    pixels: Vec<u8>,
}

impl WindowedGrayscaleSink {
    fn new(image_width: u32, image_height: u32, window: CcittDecodeWindow) -> Self {
        let expected = window.width as usize * window.height as usize;
        Self {
            image_width,
            image_height,
            window,
            next_sample: 0,
            pixels: Vec::with_capacity(expected),
        }
    }

    fn finish(self) -> Result<RawImage> {
        let expected = self.window.width as usize * self.window.height as usize;
        if self.pixels.len() != expected {
            return Err(WellfriendError::MalformedPdf(format!(
                "CCITTFaxDecode window {}:{} {}x{} from {}x{} decoded {} pixels, expected {}",
                self.window.x,
                self.window.y,
                self.window.width,
                self.window.height,
                self.image_width,
                self.image_height,
                self.pixels.len(),
                expected
            )));
        }
        Ok(RawImage {
            width: self.window.width,
            height: self.window.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: self.pixels,
        })
    }

    fn push_gray(&mut self, white: bool, count: usize) {
        let value = if white { 255 } else { 0 };
        let image_width = self.image_width as usize;
        let image_height = self.image_height as usize;
        let total = image_width.saturating_mul(image_height);
        let mut start = self.next_sample.min(total);
        let end = start.saturating_add(count).min(total);
        self.next_sample = self.next_sample.saturating_add(count);
        if start >= end || self.window.width == 0 || self.window.height == 0 {
            return;
        }

        let x0 = self.window.x as usize;
        let x1 = x0 + self.window.width as usize;
        let y0 = self.window.y as usize;
        let y1 = y0 + self.window.height as usize;
        while start < end {
            let row = start / image_width;
            if row >= image_height {
                break;
            }
            let row_end = ((row + 1) * image_width).min(end);
            if row >= y0 && row < y1 {
                let col0 = start % image_width;
                let segment_end_col = if row_end == (row + 1) * image_width {
                    image_width
                } else {
                    row_end % image_width
                };
                let write_start = col0.max(x0);
                let write_end = segment_end_col.min(x1);
                if write_start < write_end {
                    self.pixels
                        .extend(std::iter::repeat_n(value, write_end - write_start));
                }
            }
            start = row_end;
        }
    }
}

impl hayro_ccitt::Decoder for WindowedGrayscaleSink {
    fn push_pixel(&mut self, white: bool) {
        self.push_gray(white, 1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.push_gray(white, chunk_count as usize * 8);
    }

    fn next_line(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_bits(bits: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let mut byte = 0u8;
        let mut bit_count = 0u8;

        for bit in bits.bytes().filter(|b| *b == b'0' || *b == b'1') {
            byte <<= 1;
            if bit == b'1' {
                byte |= 1;
            }
            bit_count += 1;
            if bit_count == 8 {
                out.push(byte);
                byte = 0;
                bit_count = 0;
            }
        }

        if bit_count > 0 {
            byte <<= 8 - bit_count;
            out.push(byte);
        }

        out
    }

    fn params(k: i64, columns: u32, rows: u32) -> CcittDecodeParams {
        CcittDecodeParams {
            k,
            columns,
            rows,
            black_is_1: false,
            encoded_byte_align: false,
            end_of_line: false,
            end_of_block: false,
        }
    }

    #[test]
    fn decodes_group3_1d_all_white_line() {
        let image = decode(&pack_bits("10011"), params(0, 8, 1)).unwrap();
        assert_eq!(image.width, 8);
        assert_eq!(image.height, 1);
        assert_eq!(image.pixels, vec![255; 8]);
    }

    #[test]
    fn decodes_group3_1d_known_run_lengths() {
        // White run 2 (0111), black run 3 (10), white run 3 (1000).
        let image = decode(&pack_bits("0111 10 1000"), params(0, 8, 1)).unwrap();
        assert_eq!(image.pixels, vec![255, 255, 0, 0, 0, 255, 255, 255]);
    }

    #[test]
    fn ccitt_full_sink_refuses_short_output() {
        let mut sink = GrayscaleSink::new(4, 1);
        sink.push_gray(true, 2);
        let err = sink
            .finish()
            .expect_err("short CCITT output must fail typed");
        assert!(format!("{err}").contains("CCITTFaxDecode 4x1 decoded 2 pixels, expected 4"));
    }

    #[test]
    fn decodes_group3_1d_source_window_without_full_output_allocation() {
        // Three identical rows: white 2, black 3, white 3.
        let data = pack_bits("0111 10 1000 0111 10 1000 0111 10 1000");
        let image = decode_window(
            &data,
            params(0, 8, 3),
            CcittDecodeWindow {
                x: 1,
                y: 1,
                width: 5,
                height: 1,
            },
        )
        .unwrap();
        assert_eq!(image.width, 5);
        assert_eq!(image.height, 1);
        assert_eq!(image.pixels, vec![255, 0, 0, 0, 255]);
    }

    #[test]
    fn ccitt_window_sink_refuses_short_output() {
        let mut sink = WindowedGrayscaleSink::new(
            8,
            2,
            CcittDecodeWindow {
                x: 1,
                y: 0,
                width: 3,
                height: 1,
            },
        );
        sink.pixels.extend_from_slice(&[255, 0]);
        let err = sink
            .finish()
            .expect_err("short windowed CCITT output must fail typed");
        assert!(format!("{err}")
            .contains("CCITTFaxDecode window 1:0 3x1 from 8x2 decoded 2 pixels, expected 3"));
    }

    #[test]
    fn source_window_outside_ccitt_image_is_refused() {
        let err = decode_window(
            &pack_bits("10011"),
            params(0, 8, 1),
            CcittDecodeWindow {
                x: 7,
                y: 0,
                width: 2,
                height: 1,
            },
        )
        .unwrap_err();
        assert!(
            matches!(err, WellfriendError::MalformedPdf(_)),
            "out-of-range CCITT source windows must fail closed, got {err:?}"
        );
    }

    #[test]
    fn black_is_1_inverts_output_pixels() {
        let mut options = params(0, 8, 1);
        options.black_is_1 = true;

        let image = decode(&pack_bits("0111 10 1000"), options).unwrap();
        assert_eq!(image.pixels, vec![0, 0, 255, 255, 255, 0, 0, 0]);
    }

    #[test]
    fn decodes_group3_2d_mixed_lines() {
        // First line: tag 1 + 1D white run 8. Second line: tag 0 + 2D V0.
        let image = decode(&pack_bits("1 10011 0 1"), params(2, 8, 2)).unwrap();
        assert_eq!(image.pixels, vec![255; 16]);
    }

    #[test]
    fn decodes_group4_vertical_mode_lines() {
        // With an all-white reference line, V0 (1) emits an all-white line.
        let image = decode(&pack_bits("1 1"), params(-1, 8, 2)).unwrap();
        assert_eq!(image.pixels, vec![255; 16]);
    }

    #[test]
    fn decodes_group3_1d_byte_aligned_rows() {
        let mut options = params(0, 8, 2);
        options.encoded_byte_align = true;

        // Row 1: white run 8 (10011), padded to a byte boundary.
        // Row 2 starts at the next byte: white run 8 (10011).
        let image = decode(&pack_bits("10011 000 10011"), options).unwrap();
        assert_eq!(image.pixels, vec![255; 16]);
    }

    #[test]
    fn h5_rejects_oversized_columns_rows_before_allocating() {
        // /Columns 100000 /Rows 100000 = 1e10 pixels would, before the cap,
        // force a ~10 GB Vec::with_capacity in the grayscale sink. It must now
        // fail closed with a clean error from a tiny input.
        let err = decode(&[0u8; 8], params(-1, 100_000, 100_000)).unwrap_err();
        assert!(
            matches!(err, WellfriendError::MalformedPdf(_)),
            "oversized CCITT dims must be a clean error, got {err:?}"
        );
    }
}
