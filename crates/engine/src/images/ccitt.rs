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

    decode_with_settings(data, settings, &mut sink);

    Ok(sink.finish())
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
    decode_with_settings(data, settings, &mut sink);
    Ok(sink.finish())
}

/// [`decode`] into a packed bilevel image (one bit per pixel, a set bit for
/// white; see [`RawImage::is_packed_bilevel`]) instead of one byte per
/// pixel. The pixels are those of [`decode`], an eighth of the memory: a
/// 9,000 x 12,000 scanned stencil is 13.5 MB instead of 108 MB.
pub fn decode_packed(data: &[u8], params: CcittDecodeParams) -> Result<RawImage> {
    let window = CcittDecodeWindow {
        x: 0,
        y: 0,
        width: params.columns,
        height: params.rows,
    };
    if params.columns == 0 || params.rows == 0 {
        return Ok(PackedSink::empty(window));
    }
    let settings = ccitt_settings(params)?;
    crate::images::decoder::ensure_decode_budget(params.columns, params.rows, 1)?;
    let mut sink = PackedSink::new(params.columns, params.rows, window);
    decode_with_settings(data, settings, &mut sink);
    Ok(sink.finish("CCITTFaxDecode"))
}

/// [`decode_window`] into a packed bilevel image (see [`decode_packed`]).
pub fn decode_window_packed(
    data: &[u8],
    params: CcittDecodeParams,
    window: CcittDecodeWindow,
) -> Result<RawImage> {
    validate_window(params, window)?;
    if window.width == 0 || window.height == 0 {
        return Ok(PackedSink::empty(window));
    }
    crate::images::decoder::ensure_decode_budget(window.width, window.height, 1)?;
    let settings = ccitt_settings(params)?;
    let mut sink = PackedSink::new(params.columns, params.rows, window);
    decode_with_settings(data, settings, &mut sink);
    Ok(sink.finish("CCITTFaxDecode window"))
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

/// Run the CCITT decoder into `sink`. Rows decoded before a coding error stay
/// in the sink (hayro-ccitt writes them incrementally); the sinks pad the
/// missing rows white on `finish`, as Poppler draws damaged fax images.
///
/// Group 3 streams resynchronise after a damaged row, like Poppler's
/// `CCITTFaxStream`: the damaged row is completed with white, and decoding
/// restarts at the next end-of-line marker. T.4 guarantees that no valid row
/// data contains eleven consecutive zero bits, so an EOL found in the
/// bitstream always starts a new row. hayro-ccitt cannot resume mid-stream,
/// so each restart decodes a fresh stream that begins at that EOL; a 2-D row
/// right after a restart refers to an all-white line until the next 1-D row.
fn decode_with_settings<D: hayro_ccitt::Decoder>(
    data: &[u8],
    settings: hayro_ccitt::DecodeSettings,
    sink: &mut D,
) {
    let mut rows = RowTracker::new(sink, settings.columns);
    let Err(mut error) = decode_chunk(data, settings, &mut rows) else {
        return;
    };
    // Group 4 (T.6) has no EOL codes to resynchronise on.
    if matches!(settings.encoding, hayro_ccitt::EncodingMode::Group4) {
        log::warn!("CCITTFaxDecode stopped early ({error}); keeping the rows decoded so far");
        return;
    }
    let mut stream = ShiftedBitstream::new(data);
    let mut chunk_start = 0usize;
    let mut chunk_first_row = 0u32;
    let mut damaged_rows = 0u32;
    loop {
        let chunk_rows = rows.completed - chunk_first_row;
        let threshold =
            damaged_row_threshold(stream.view_from_bit(chunk_start), settings, chunk_rows)
                .map(|bit| chunk_start + bit);
        if rows.completed < settings.rows {
            rows.pad_damaged_row();
            damaged_rows += 1;
        }
        let remaining = settings.rows.saturating_sub(rows.completed);
        let restart = threshold
            .filter(|_| remaining > 0)
            .and_then(|bit| next_eol_start(data, bit));
        let Some(restart) = restart else {
            log::warn!(
                "CCITTFaxDecode stopped early ({error}) after {damaged_rows} damaged row(s); keeping the rows decoded so far"
            );
            return;
        };
        chunk_start = restart;
        chunk_first_row = rows.completed;
        let chunk_settings = hayro_ccitt::DecodeSettings {
            rows: remaining,
            ..settings
        };
        match decode_chunk(stream.view_from_bit(restart), chunk_settings, &mut rows) {
            Ok(()) => {
                log::warn!(
                    "CCITTFaxDecode resynchronised on end-of-line markers after {damaged_rows} damaged row(s)"
                );
                return;
            }
            Err(next_error) => error = next_error,
        }
    }
}

fn decode_chunk<D: hayro_ccitt::Decoder>(
    data: &[u8],
    settings: hayro_ccitt::DecodeSettings,
    sink: &mut D,
) -> std::result::Result<(), hayro_ccitt::DecodeError> {
    let mut context = hayro_ccitt::DecoderContext::new(settings);
    hayro_ccitt::decode(data, sink, &mut context).map(|_| ())
}

/// A bit offset into `chunk` that the damaged row (row `complete_rows` of the
/// chunk) starts at or before: the EOL that ends the damaged row is the first
/// one whose final `1` bit lies at or after this offset.
///
/// With complete rows before the damage, the chunk is decoded again for just
/// those rows; hayro-ccitt then stops right after the EOL that starts the
/// damaged row and reports the byte offset rounded up. The damaged row starts
/// within the 8 bits before that byte boundary, its own EOL ends there too,
/// and the next EOL (at least 12 bits later) ends past it.
fn damaged_row_threshold(
    chunk: &[u8],
    settings: hayro_ccitt::DecodeSettings,
    complete_rows: u32,
) -> Option<usize> {
    if complete_rows == 0 {
        return Some(leading_eol_end(chunk));
    }
    let prefix = hayro_ccitt::DecodeSettings {
        rows: complete_rows,
        ..settings
    };
    let mut context = hayro_ccitt::DecoderContext::new(prefix);
    let bytes = hayro_ccitt::decode(chunk, &mut NullSink, &mut context).ok()?;
    bytes.checked_mul(8)
}

/// The bit offset just past any EOL markers (with fill bits) at the start of
/// `chunk`, read the way hayro-ccitt reads them before the first row.
fn leading_eol_end(chunk: &[u8]) -> usize {
    const MAX_FILL_BITS: usize = 24;
    let total = chunk.len().saturating_mul(8);
    let mut pos = 0usize;
    loop {
        let mut zeros = 0usize;
        while zeros < MAX_FILL_BITS && pos + zeros < total && !bit_at(chunk, pos + zeros) {
            zeros += 1;
        }
        if zeros >= 11 && pos + zeros < total && bit_at(chunk, pos + zeros) {
            pos += zeros + 1;
            continue;
        }
        return pos;
    }
}

/// The start of the first EOL (`000000000001`) whose final `1` bit is at or
/// after bit `threshold`. Decoding restarts on its eleven zero bits, so the
/// decoder reads the EOL (and any EOLs following it) itself.
fn next_eol_start(data: &[u8], threshold: usize) -> Option<usize> {
    let total = data.len().saturating_mul(8);
    let mut zeros = 0usize;
    for index in threshold.saturating_sub(11)..total {
        if !bit_at(data, index) {
            zeros += 1;
            continue;
        }
        if zeros >= 11 && index >= threshold {
            return Some(index - 11);
        }
        zeros = 0;
    }
    None
}

fn bit_at(data: &[u8], index: usize) -> bool {
    (data[index >> 3] >> (7 - (index & 7))) & 1 == 1
}

/// Byte views of a bitstream starting at any bit offset. EOL markers are not
/// byte aligned, so a restart at bit `n` reads a copy of the stream shifted
/// left by `n % 8` bits; each of the seven shifts is built at most once.
struct ShiftedBitstream<'a> {
    data: &'a [u8],
    shifted: [Option<Vec<u8>>; 8],
}

impl<'a> ShiftedBitstream<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            shifted: Default::default(),
        }
    }

    fn view_from_bit(&mut self, bit: usize) -> &[u8] {
        let (byte, shift) = (bit / 8, bit % 8);
        if shift == 0 {
            return self.data.get(byte..).unwrap_or_default();
        }
        let data = self.data;
        let copy = self.shifted[shift].get_or_insert_with(|| {
            (0..data.len())
                .map(|index| {
                    let next = data.get(index + 1).map_or(0, |next| next >> (8 - shift));
                    (data[index] << shift) | next
                })
                .collect()
        });
        copy.get(byte..).unwrap_or_default()
    }
}

/// Forwards pixels to the real sink while counting completed rows and the
/// pixels of the row in progress, so a damaged row can be completed.
struct RowTracker<'a, D> {
    inner: &'a mut D,
    columns: u32,
    row_pixels: u32,
    completed: u32,
}

impl<'a, D: hayro_ccitt::Decoder> RowTracker<'a, D> {
    fn new(inner: &'a mut D, columns: u32) -> Self {
        Self {
            inner,
            columns,
            row_pixels: 0,
            completed: 0,
        }
    }

    /// Complete the row in progress with white pixels.
    fn pad_damaged_row(&mut self) {
        for _ in self.row_pixels..self.columns {
            self.inner.push_pixel(true);
        }
        hayro_ccitt::Decoder::next_line(self);
    }
}

impl<D: hayro_ccitt::Decoder> hayro_ccitt::Decoder for RowTracker<'_, D> {
    fn push_pixel(&mut self, white: bool) {
        self.row_pixels = self.row_pixels.saturating_add(1);
        self.inner.push_pixel(white);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.row_pixels = self
            .row_pixels
            .saturating_add(chunk_count.saturating_mul(8));
        self.inner.push_pixel_chunk(white, chunk_count);
    }

    fn next_line(&mut self) {
        self.row_pixels = 0;
        self.completed = self.completed.saturating_add(1);
        self.inner.next_line();
    }
}

struct NullSink;

impl hayro_ccitt::Decoder for NullSink {
    fn push_pixel(&mut self, _white: bool) {}

    fn push_pixel_chunk(&mut self, _white: bool, _chunk_count: u32) {}

    fn next_line(&mut self) {}
}

/// Fill the missing tail of a short CCITT bitmap with white samples.
fn pad_white(pixels: &mut Vec<u8>, expected: usize, label: &str) {
    if pixels.len() < expected {
        log::warn!(
            "{label} decoded {} of {expected} pixels; padding the rest white",
            pixels.len()
        );
        pixels.resize(expected, 255);
    }
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

    fn finish(mut self) -> RawImage {
        let expected = self.width as usize * self.height as usize;
        pad_white(&mut self.pixels, expected, "CCITTFaxDecode");
        RawImage {
            width: self.width,
            height: self.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: self.pixels,
        }
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

    fn finish(mut self) -> RawImage {
        let expected = self.window.width as usize * self.window.height as usize;
        pad_white(&mut self.pixels, expected, "CCITTFaxDecode window");
        RawImage {
            width: self.window.width,
            height: self.window.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: self.pixels,
        }
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

/// The packed counterpart of [`GrayscaleSink`] and [`WindowedGrayscaleSink`]:
/// it reads the decoder's pixels as one stream of `image_width` pixels per
/// row exactly as they do (the decoder's `next_line` is not needed), keeps
/// the `window` rectangle at one bit per pixel and starts white, so pixels the
/// stream never reaches stay white as those sinks pad them.
struct PackedSink {
    image_width: usize,
    image_height: usize,
    window: CcittDecodeWindow,
    row_bytes: usize,
    /// Position of the next decoded pixel in the full image.
    row: usize,
    column: usize,
    /// Window pixels the stream has reached.
    covered: usize,
    pixels: Vec<u8>,
}

impl PackedSink {
    fn new(image_width: u32, image_height: u32, window: CcittDecodeWindow) -> Self {
        let row_bytes = (window.width as usize).div_ceil(8);
        Self {
            image_width: image_width as usize,
            image_height: image_height as usize,
            window,
            row_bytes,
            row: 0,
            column: 0,
            covered: 0,
            pixels: vec![0xFF; row_bytes * window.height as usize],
        }
    }

    fn empty(window: CcittDecodeWindow) -> RawImage {
        RawImage {
            width: window.width,
            height: window.height,
            channels: 1,
            bits_per_sample: 1,
            pixels: Vec::new(),
        }
    }

    fn finish(self, label: &str) -> RawImage {
        let expected = self.window.width as usize * self.window.height as usize;
        if self.covered < expected {
            log::warn!(
                "{label} decoded {} of {expected} pixels; padding the rest white",
                self.covered
            );
        }
        RawImage {
            width: self.window.width,
            height: self.window.height,
            channels: 1,
            bits_per_sample: 1,
            pixels: self.pixels,
        }
    }

    fn push(&mut self, white: bool, count: usize) {
        let x0 = self.window.x as usize;
        let x1 = x0 + self.window.width as usize;
        let y0 = self.window.y as usize;
        let y1 = y0 + self.window.height as usize;
        let mut remaining = count;
        while remaining > 0 && self.row < self.image_height {
            let take = remaining.min(self.image_width - self.column);
            if self.row >= y0 && self.row < y1 {
                let start = self.column.max(x0);
                let end = (self.column + take).min(x1);
                if start < end {
                    self.covered += end - start;
                    if !white {
                        let offset = (self.row - y0) * self.row_bytes;
                        clear_bits(
                            &mut self.pixels[offset..offset + self.row_bytes],
                            start - x0,
                            end - x0,
                        );
                    }
                }
            }
            self.column += take;
            if self.column == self.image_width {
                self.column = 0;
                self.row += 1;
            }
            remaining -= take;
        }
    }
}

/// Clear bits `start..end` of a most-significant-bit-first row.
fn clear_bits(row: &mut [u8], start: usize, end: usize) {
    let (first, last) = (start >> 3, (end - 1) >> 3);
    let head = 0xFFu8 >> (start & 7);
    let tail = 0xFFu8 << (7 - ((end - 1) & 7));
    if first == last {
        row[first] &= !(head & tail);
        return;
    }
    row[first] &= !head;
    row[first + 1..last].fill(0);
    row[last] &= !tail;
}

impl hayro_ccitt::Decoder for PackedSink {
    fn push_pixel(&mut self, white: bool) {
        self.push(white, 1);
    }

    fn push_pixel_chunk(&mut self, white: bool, chunk_count: u32) {
        self.push(white, chunk_count as usize * 8);
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
    fn ccitt_full_sink_pads_short_output_white() {
        let mut sink = GrayscaleSink::new(4, 1);
        sink.push_gray(false, 2);
        assert_eq!(sink.finish().pixels, vec![0, 0, 255, 255]);
    }

    #[test]
    fn damaged_ccitt_keeps_rows_decoded_before_the_error() {
        // Row 1: white 2, black 3, white 3. Then an invalid all-zero code.
        let data = pack_bits("0111 10 1000 000000000000 000000000000");
        let image = decode(&data, params(0, 8, 3)).unwrap();
        assert_eq!(image.width, 8);
        assert_eq!(image.height, 3);
        assert_eq!(&image.pixels[..8], &[255, 255, 0, 0, 0, 255, 255, 255]);
        assert!(image.pixels[8..].iter().all(|pixel| *pixel == 255));
    }

    const EOL: &str = "000000000001";
    /// White 2, black 3, white 3.
    const ROW_A: &str = "0111 10 1000";
    const ROW_A_PIXELS: [u8; 8] = [255, 255, 0, 0, 0, 255, 255, 255];

    fn with_eols(params: CcittDecodeParams) -> CcittDecodeParams {
        CcittDecodeParams {
            end_of_line: true,
            ..params
        }
    }

    #[test]
    fn damaged_group3_1d_row_resynchronises_on_the_next_eol() {
        // Row 1 is white 2 followed by sixteen zero bits: neither a white nor
        // a black code, so decoding fails mid-row. The following EOL starts
        // row 2, which must still be drawn.
        let bits = format!("{EOL} {ROW_A} {EOL} 0111 0000000000000000 {EOL} {ROW_A}");
        let image = decode(&pack_bits(&bits), with_eols(params(0, 8, 3))).unwrap();
        assert_eq!(&image.pixels[..8], &ROW_A_PIXELS, "row before the damage");
        assert_eq!(&image.pixels[8..16], &[255; 8], "damaged row is white");
        assert_eq!(&image.pixels[16..], &ROW_A_PIXELS, "row after the EOL");
    }

    #[test]
    fn damaged_group3_2d_row_resynchronises_on_the_next_eol() {
        // K = 2: rows 0 and 2 are 1-D coded (tag 1); row 1 is a 2-D row: V0
        // (two white pixels), then seven zero bits that are not a mode code.
        // (The final 1 keeps the zeros from reading as EOL fill bits.)
        let bits = format!("{EOL} 1 {ROW_A} {EOL} 0 1 0000000 1 {EOL} 1 {ROW_A}");
        let image = decode(&pack_bits(&bits), with_eols(params(2, 8, 3))).unwrap();
        assert_eq!(&image.pixels[..8], &ROW_A_PIXELS);
        assert_eq!(&image.pixels[8..16], &[255; 8]);
        assert_eq!(&image.pixels[16..], &ROW_A_PIXELS);
    }

    #[test]
    fn several_damaged_rows_each_resynchronise() {
        let damaged = "0111 0000000000000000";
        let bits =
            format!("{EOL} {damaged} {EOL} {ROW_A} {EOL} {damaged} {EOL} {damaged} {EOL} {ROW_A}");
        let image = decode(&pack_bits(&bits), with_eols(params(0, 8, 5))).unwrap();
        let rows: Vec<&[u8]> = image.pixels.chunks(8).collect();
        assert_eq!(rows[0], &[255; 8]);
        assert_eq!(rows[1], &ROW_A_PIXELS);
        assert_eq!(rows[2], &[255; 8]);
        assert_eq!(rows[3], &[255; 8]);
        assert_eq!(rows[4], &ROW_A_PIXELS);
    }

    #[test]
    fn resynchronised_rows_land_in_a_source_window() {
        let bits = format!("{EOL} {ROW_A} {EOL} 0111 0000000000000000 {EOL} {ROW_A}");
        let image = decode_window(
            &pack_bits(&bits),
            with_eols(params(0, 8, 3)),
            CcittDecodeWindow {
                x: 1,
                y: 2,
                width: 5,
                height: 1,
            },
        )
        .unwrap();
        assert_eq!(image.pixels, vec![255, 0, 0, 0, 255]);
    }

    #[test]
    fn eol_scanner_finds_markers_at_any_bit_offset() {
        let data = pack_bits(&format!("101 {EOL} 0 {EOL}"));
        assert_eq!(next_eol_start(&data, 0), Some(3));
        // The first EOL ends at bit 14; asking for one ending later skips it.
        assert_eq!(next_eol_start(&data, 15), Some(16));
        assert_eq!(next_eol_start(&data, 28), None);
        assert_eq!(leading_eol_end(&pack_bits(&format!("{EOL}{EOL} 1"))), 24);
        assert_eq!(leading_eol_end(&pack_bits("1")), 0);
    }

    #[test]
    fn shifted_bitstream_reads_from_any_bit() {
        let data = [0b1010_1100, 0b0101_0011];
        let mut stream = ShiftedBitstream::new(&data);
        assert_eq!(stream.view_from_bit(0), &data);
        assert_eq!(stream.view_from_bit(3), &[0b0110_0010, 0b1001_1000]);
        assert_eq!(stream.view_from_bit(11), &[0b1001_1000]);
        assert_eq!(stream.view_from_bit(16), &[] as &[u8]);
    }

    #[test]
    fn truncated_ccitt_window_pads_missing_rows_white() {
        // Only the first of three rows is present.
        let image = decode_window(
            &pack_bits("0111 10 1000"),
            params(0, 8, 3),
            CcittDecodeWindow {
                x: 1,
                y: 0,
                width: 5,
                height: 3,
            },
        )
        .unwrap();
        assert_eq!(&image.pixels[..5], &[255, 0, 0, 0, 255]);
        assert!(image.pixels[5..].iter().all(|pixel| *pixel == 255));
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
    fn ccitt_window_sink_pads_short_output_white() {
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
        assert_eq!(sink.finish().pixels, vec![255, 0, 255]);
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
    fn packed_decodes_hold_the_grayscale_pixels() {
        let damaged = "0111 0000000000000000";
        let mut black_is_1 = params(0, 8, 1);
        black_is_1.black_is_1 = true;
        let mut aligned = params(0, 8, 2);
        aligned.encoded_byte_align = true;
        let cases = [
            (pack_bits("0111 10 1000"), params(0, 8, 1)),
            (pack_bits("0111 10 1000"), black_is_1),
            // Truncated: rows two and three are padded white.
            (pack_bits("0111 10 1000"), params(0, 8, 3)),
            (pack_bits("0111 10 1000"), params(0, 13, 2)),
            (
                pack_bits(&format!(
                    "{EOL} {damaged} {EOL} {ROW_A} {EOL} {damaged} {EOL} {ROW_A}"
                )),
                with_eols(params(0, 8, 4)),
            ),
            (pack_bits("1 10011 0 1"), params(2, 8, 2)),
            (pack_bits("1 1"), params(-1, 8, 2)),
            (pack_bits("10011 000 10011"), aligned),
            (
                pack_bits("0111 10 1000 0111 10 1000 0111 10 1000"),
                params(0, 8, 3),
            ),
        ];
        for (data, params) in &cases {
            let gray = decode(data, *params).unwrap();
            let packed = decode_packed(data, *params).unwrap();
            assert!(packed.is_packed_bilevel());
            assert_eq!((packed.width, packed.height), (gray.width, gray.height));
            assert_eq!(packed.pixels.len(), packed.byte_count());
            assert_eq!(packed.unpack_bilevel().pixels, gray.pixels, "{params:?}");
            for window in [
                CcittDecodeWindow {
                    x: 1,
                    y: 0,
                    width: 5,
                    height: params.rows,
                },
                CcittDecodeWindow {
                    x: 0,
                    y: params.rows - 1,
                    width: params.columns,
                    height: 1,
                },
                CcittDecodeWindow {
                    x: params.columns - 1,
                    y: 0,
                    width: 1,
                    height: 1,
                },
            ] {
                let gray = decode_window(data, *params, window).unwrap();
                let packed = decode_window_packed(data, *params, window).unwrap();
                assert_eq!(
                    packed.unpack_bilevel().pixels,
                    gray.pixels,
                    "{params:?} {window:?}"
                );
            }
        }
        let empty = decode_packed(&[], params(0, 0, 4)).unwrap();
        assert!(empty.pixels.is_empty() && empty.is_packed_bilevel());
        assert!(decode_window_packed(
            &pack_bits("10011"),
            params(0, 8, 1),
            CcittDecodeWindow {
                x: 7,
                y: 0,
                width: 2,
                height: 1,
            },
        )
        .is_err());
    }

    #[test]
    fn clear_bits_clears_exactly_the_range() {
        for width in 1..=24usize {
            for start in 0..width {
                for end in start + 1..=width {
                    let mut row = vec![0xFF; width.div_ceil(8)];
                    clear_bits(&mut row, start, end);
                    for x in 0..row.len() * 8 {
                        let set = row[x >> 3] & (0x80 >> (x & 7)) != 0;
                        assert_eq!(
                            set,
                            !(start..end).contains(&x),
                            "{width} {start}..{end} {x}"
                        );
                    }
                }
            }
        }
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
