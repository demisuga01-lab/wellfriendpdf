use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;

/// How many shorter segment prefixes are tried when a damaged JBIG2 stream
/// fails to decode as a whole.
const MAX_PARTIAL_DECODE_ATTEMPTS: usize = 8;

/// Decode an embedded PDF JBIG2 stream into Wellfriend's grayscale RawImage.
///
/// The underlying decoder supports PDF embedded organization, optional global
/// segments, generic regions, symbol dictionaries, text regions, halftone
/// regions, and generic refinement regions. Unsupported or malformed JBIG2
/// constructs are surfaced as decode errors rather than panics.
///
/// The decoder only emits the page bitmap after every segment succeeded, so a
/// damaged or truncated stream would lose the whole image. When the full
/// decode fails, the longest prefix of complete segments that still decodes
/// is used instead (regions after the damage are missing, the rest is drawn).
pub fn decode(data: &[u8], globals: Option<&[u8]>) -> Result<RawImage> {
    let error = match decode_complete(data, globals) {
        Ok(image) => return Ok(image),
        Err(error) => error,
    };
    if matches!(
        error,
        WellfriendError::ResourceLimit(_) | WellfriendError::Cancelled(_)
    ) {
        return Err(error);
    }
    let ends = embedded_segment_ends(data);
    for &end in ends
        .iter()
        .rev()
        .filter(|end| **end < data.len())
        .take(MAX_PARTIAL_DECODE_ATTEMPTS)
    {
        if let Ok(image) = decode_complete(&data[..end], globals) {
            log::warn!(
                "{error}; drew the JBIG2 image from its first {end} of {} bytes",
                data.len()
            );
            return Ok(image);
        }
    }
    Err(error)
}

fn decode_complete(data: &[u8], globals: Option<&[u8]>) -> Result<RawImage> {
    let image = hayro_jbig2::Image::new_embedded(data, globals)
        .map_err(|err| WellfriendError::MalformedPdf(format!("JBIG2Decode parse failed: {err}")))?;

    // H-6: bound the codestream-declared region dimensions before allocating the
    // grayscale sink, so a crafted multi-gigapixel JBIG2 page cannot force a huge
    // reservation in our sink.
    crate::images::decoder::ensure_decode_budget(image.width(), image.height(), 1)?;
    let mut sink = GrayscaleSink::new(image.width(), image.height());
    image
        .decode(&mut sink)
        .map_err(|err| WellfriendError::MalformedPdf(format!("JBIG2Decode failed: {err}")))?;
    Ok(sink.finish())
}

/// Byte offsets just past each complete segment of an embedded
/// (sequential-organization) JBIG2 stream, per T.88 section 7.2. Parsing stops
/// at the first truncated header, at a segment whose data runs past the end,
/// or at an unknown-length segment (which cannot be delimited cheaply).
fn embedded_segment_ends(data: &[u8]) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        match segment_end(data, pos) {
            Some(end) if end > pos => {
                ends.push(end);
                pos = end;
            }
            _ => break,
        }
    }
    ends
}

fn segment_end(data: &[u8], start: usize) -> Option<usize> {
    let be_u32 = |at: usize| -> Option<u32> {
        let bytes = data.get(at..at.checked_add(4)?)?;
        Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    };
    let segment_number = be_u32(start)?;
    let flags = *data.get(start + 4)?;
    let page_association_bytes = if flags & 0x40 != 0 { 4 } else { 1 };
    let mut pos = start + 5;
    let count_byte = *data.get(pos)?;
    let referred_count = match count_byte >> 5 {
        7 => {
            let count = (be_u32(pos)? & 0x1FFF_FFFF) as usize;
            pos += 4 + (count + 1).div_ceil(8);
            count
        }
        5 | 6 => return None,
        count => {
            pos += 1;
            usize::from(count)
        }
    };
    let referred_size = if segment_number <= 256 {
        1
    } else if segment_number <= 65_536 {
        2
    } else {
        4
    };
    pos = pos.checked_add(referred_count.checked_mul(referred_size)?)?;
    pos = pos.checked_add(page_association_bytes)?;
    let data_length = be_u32(pos)?;
    if data_length == u32::MAX {
        return None;
    }
    let end = (pos + 4).checked_add(data_length as usize)?;
    (end <= data.len()).then_some(end)
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
        if self.pixels.len() < expected {
            log::warn!(
                "JBIG2Decode {}x{} decoded {} pixels, expected {expected}; padding white",
                self.width,
                self.height,
                self.pixels.len()
            );
            self.pixels.resize(expected, 255);
        }
        RawImage {
            width: self.width,
            height: self.height,
            channels: 1,
            bits_per_sample: 8,
            pixels: self.pixels,
        }
    }

    fn push_gray(&mut self, black: bool, count: usize) {
        let value = if black { 0 } else { 255 };
        let expected = self.width as usize * self.height as usize;
        let remaining = expected.saturating_sub(self.pixels.len());
        self.pixels
            .extend(std::iter::repeat_n(value, count.min(remaining)));
    }
}

impl hayro_jbig2::Decoder for GrayscaleSink {
    fn push_pixel(&mut self, black: bool) {
        self.push_gray(black, 1);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.push_gray(black, chunk_count as usize * 8);
    }

    fn next_line(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_embedded_stream_returns_error() {
        let result = decode(b"not a jbig2 stream", None);
        assert!(result.is_err());
    }

    #[test]
    fn jbig2_sink_pads_short_output_white() {
        let mut sink = GrayscaleSink::new(4, 1);
        sink.push_gray(true, 2);
        assert_eq!(sink.finish().pixels, vec![0, 0, 255, 255]);
    }

    /// One embedded segment header (short form, 1-byte page association).
    fn segment(number: u32, kind: u8, referred: &[u8], payload: &[u8]) -> Vec<u8> {
        let mut out = number.to_be_bytes().to_vec();
        out.push(kind);
        out.push((referred.len() as u8) << 5);
        out.extend_from_slice(referred);
        out.push(1);
        out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// Page information for a `width` x `height` page with a white default.
    fn page_information(width: u32, height: u32) -> Vec<u8> {
        let mut payload = width.to_be_bytes().to_vec();
        payload.extend_from_slice(&height.to_be_bytes());
        payload.extend_from_slice(&[0; 8]); // resolutions
        payload.push(0); // flags: default pixel 0 (white)
        payload.extend_from_slice(&[0, 0]); // striping
        segment(0, 48, &[], &payload)
    }

    #[test]
    fn segment_scanner_delimits_complete_segments_only() {
        let mut data = page_information(8, 2);
        let first_end = data.len();
        data.extend_from_slice(&segment(1, 50, &[], &[]));
        let second_end = data.len();
        // A third segment whose payload is cut off.
        let mut truncated = segment(2, 38, &[0], &[1, 2, 3, 4]);
        truncated.truncate(truncated.len() - 2);
        data.extend_from_slice(&truncated);
        assert_eq!(embedded_segment_ends(&data), vec![first_end, second_end]);
    }

    #[test]
    fn truncated_jbig2_stream_keeps_the_decodable_prefix() {
        let mut data = page_information(8, 2);
        // An immediate generic region whose data is cut off mid-segment.
        let mut region = segment(1, 38, &[], &[0; 40]);
        region.truncate(region.len() - 30);
        data.extend_from_slice(&region);

        let image = decode(&data, None).expect("page information alone still decodes");
        assert_eq!((image.width, image.height), (8, 2));
        assert!(image.pixels.iter().all(|pixel| *pixel == 255));
    }
}
