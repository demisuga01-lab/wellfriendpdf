use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;

/// How many whole-stream decodes may be spent isolating damaged segments
/// when a JBIG2 stream fails to decode as a whole.
const MAX_PARTIAL_DECODE_ATTEMPTS: usize = 24;

/// Decode an embedded PDF JBIG2 stream into Wellfriend's grayscale RawImage.
///
/// The underlying decoder supports PDF embedded organization, optional global
/// segments, generic regions, symbol dictionaries, text regions, halftone
/// regions, and generic refinement regions. Unsupported or malformed JBIG2
/// constructs are surfaced as decode errors rather than panics.
///
/// The decoder only emits the page bitmap after every segment succeeded, so a
/// damaged or truncated stream would lose the whole image. When the full
/// decode fails, the damaged segments are left out and the others are
/// composed onto the page, as Poppler's JBIG2Stream skips a segment it cannot
/// read: region segments are independent, so regions after a damaged one are
/// still drawn. Segments that refer to a left-out segment (a text region
/// using a damaged symbol dictionary, say) are left out with it, and a
/// truncated tail that cannot be delimited is dropped.
pub fn decode(data: &[u8], globals: Option<&[u8]>) -> Result<RawImage> {
    let error = match decode_complete(data, globals) {
        Ok(image) => return Ok(image),
        Err(error) => error,
    };
    if is_fatal(&error) {
        return Err(error);
    }
    let segments = embedded_segments(data);
    match decode_without_damaged_segments(data, globals, &segments)? {
        Some(partial) => {
            log::warn!(
                "{error}; drew the JBIG2 image without {} damaged or dependent segment(s) of {} ({} of {} bytes delimited)",
                partial.left_out,
                segments.len(),
                segments.last().map_or(0, |segment| segment.end),
                data.len()
            );
            Ok(partial.image)
        }
        None => Err(error),
    }
}

fn is_fatal(error: &WellfriendError) -> bool {
    matches!(
        error,
        WellfriendError::ResourceLimit(_) | WellfriendError::Cancelled(_)
    )
}

struct PartialDecode {
    image: RawImage,
    left_out: usize,
}

enum Attempt {
    Decoded(RawImage),
    Failed,
    Exhausted,
}

/// Decodes chosen subsets of a stream's segments within an attempt budget.
struct SegmentDecoder<'a> {
    data: &'a [u8],
    globals: Option<&'a [u8]>,
    segments: &'a [SegmentSpan],
    budget: usize,
}

impl SegmentDecoder<'_> {
    fn attempt(&mut self, indices: &[usize]) -> Result<Attempt> {
        // The whole delimited stream is the input that already failed.
        let whole = indices.len() == self.segments.len()
            && self.segments.last().map(|segment| segment.end) == Some(self.data.len());
        if whole {
            return Ok(Attempt::Failed);
        }
        if self.budget == 0 {
            return Ok(Attempt::Exhausted);
        }
        self.budget -= 1;
        let mut bytes = Vec::new();
        for &index in indices {
            let segment = &self.segments[index];
            bytes.extend_from_slice(&self.data[segment.start..segment.end]);
        }
        match decode_complete(&bytes, self.globals) {
            Ok(image) => Ok(Attempt::Decoded(image)),
            Err(error) if is_fatal(&error) => Err(error),
            Err(_) => Ok(Attempt::Failed),
        }
    }
}

/// Find the damaged segments one at a time and decode the rest.
///
/// Segments are composed onto the page in order, so a decode of a segment
/// prefix fails exactly when the prefix contains a damaged segment: the first
/// damaged one is found by bisecting the undecided segments, then it (and
/// every segment referring to it) is left out and the search continues after
/// it. The result is the decode of all kept segments, or the largest verified
/// subset when the attempt budget runs out.
fn decode_without_damaged_segments(
    data: &[u8],
    globals: Option<&[u8]>,
    segments: &[SegmentSpan],
) -> Result<Option<PartialDecode>> {
    let mut decoder = SegmentDecoder {
        data,
        globals,
        segments,
        budget: MAX_PARTIAL_DECODE_ATTEMPTS,
    };
    let mut kept: Vec<usize> = Vec::new();
    let mut left_out: Vec<u32> = Vec::new();
    let mut best: Option<RawImage> = None;
    let mut next = 0usize;
    let finish = |image: Option<RawImage>, kept: usize| {
        image.map(|image| PartialDecode {
            image,
            left_out: segments.len() - kept,
        })
    };
    loop {
        let mut remaining = Vec::new();
        for (index, segment) in segments.iter().enumerate().skip(next) {
            if segment
                .referred
                .iter()
                .any(|number| left_out.contains(number))
            {
                if !left_out.contains(&segment.number) {
                    left_out.push(segment.number);
                }
                continue;
            }
            remaining.push(index);
        }
        if remaining.is_empty() {
            return Ok(finish(best, kept.len()));
        }
        let mut candidate = kept.clone();
        candidate.extend_from_slice(&remaining);
        match decoder.attempt(&candidate)? {
            Attempt::Decoded(image) => return Ok(finish(Some(image), candidate.len())),
            Attempt::Exhausted => return Ok(finish(best, kept.len())),
            Attempt::Failed => {}
        }
        // Invariant: kept + remaining[..good] decodes, kept + remaining[..bad]
        // does not.
        let (mut good, mut bad) = (0usize, remaining.len());
        while bad - good > 1 {
            let mid = good + (bad - good) / 2;
            candidate.truncate(kept.len());
            candidate.extend_from_slice(&remaining[..mid]);
            match decoder.attempt(&candidate)? {
                Attempt::Decoded(image) => {
                    good = mid;
                    best = Some(image);
                }
                Attempt::Failed => bad = mid,
                Attempt::Exhausted => {
                    kept.extend_from_slice(&remaining[..good]);
                    return Ok(finish(best, kept.len()));
                }
            }
        }
        let damaged = remaining[bad - 1];
        kept.extend_from_slice(&remaining[..bad - 1]);
        left_out.push(segments[damaged].number);
        next = damaged + 1;
    }
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

/// One complete segment of an embedded JBIG2 stream.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SegmentSpan {
    start: usize,
    end: usize,
    number: u32,
    referred: Vec<u32>,
}

/// The complete segments of an embedded (sequential-organization) JBIG2
/// stream, per T.88 section 7.2. Parsing stops at the first truncated header,
/// at a segment whose data runs past the end, or at an unknown-length segment
/// (which cannot be delimited cheaply).
fn embedded_segments(data: &[u8]) -> Vec<SegmentSpan> {
    let mut segments = Vec::new();
    let mut pos = 0usize;
    while pos < data.len() {
        match parse_segment_span(data, pos) {
            Some(segment) if segment.end > pos => {
                pos = segment.end;
                segments.push(segment);
            }
            _ => break,
        }
    }
    segments
}

fn parse_segment_span(data: &[u8], start: usize) -> Option<SegmentSpan> {
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
    let referred_end = pos.checked_add(referred_count.checked_mul(referred_size)?)?;
    let referred = data
        .get(pos..referred_end)?
        .chunks_exact(referred_size)
        .map(|bytes| bytes.iter().fold(0u32, |n, byte| n << 8 | u32::from(*byte)))
        .collect();
    pos = referred_end.checked_add(page_association_bytes)?;
    let data_length = be_u32(pos)?;
    if data_length == u32::MAX {
        return None;
    }
    let end = (pos + 4).checked_add(data_length as usize)?;
    (end <= data.len()).then_some(SegmentSpan {
        start,
        end,
        number: segment_number,
        referred,
    })
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
        let segments = embedded_segments(&data);
        let ends: Vec<usize> = segments.iter().map(|segment| segment.end).collect();
        assert_eq!(ends, vec![first_end, second_end]);
        assert_eq!(segments[1].start, first_end);
        assert_eq!(segments[1].number, 1);
    }

    #[test]
    fn segment_scanner_reads_referred_segment_numbers() {
        let mut data = page_information(8, 2);
        data.extend_from_slice(&segment(3, 6, &[1, 2], &[]));
        let segments = embedded_segments(&data);
        assert_eq!(segments[1].referred, vec![1, 2]);
        assert!(segments[0].referred.is_empty());
    }

    /// An immediate MMR generic region painting one black 8-pixel row at `y`.
    fn black_row_region(number: u32, y: u32, referred: &[u8]) -> Vec<u8> {
        let mut payload = 8u32.to_be_bytes().to_vec(); // width
        payload.extend_from_slice(&1u32.to_be_bytes()); // height
        payload.extend_from_slice(&0u32.to_be_bytes()); // x
        payload.extend_from_slice(&y.to_be_bytes()); // y
        payload.push(0); // region flags: OR
        payload.push(1); // generic region flags: MMR

        // MMR row: horizontal mode (001), white 0 (00110101), black 8 (000101).
        payload.extend_from_slice(&[0x26, 0xA2, 0x80]);
        segment(number, 38, referred, &payload)
    }

    /// A generic region whose region information is cut short.
    fn damaged_region(number: u32) -> Vec<u8> {
        segment(number, 38, &[], &[0, 0, 0])
    }

    fn rows(image: &RawImage) -> Vec<Vec<u8>> {
        image
            .pixels
            .chunks(image.width as usize)
            .map(<[u8]>::to_vec)
            .collect()
    }

    #[test]
    fn regions_after_a_damaged_middle_segment_are_still_drawn() {
        let mut data = page_information(8, 2);
        data.extend_from_slice(&black_row_region(1, 0, &[]));
        data.extend_from_slice(&damaged_region(2));
        data.extend_from_slice(&black_row_region(3, 1, &[]));
        decode_complete(&data, None).expect_err("the damaged region fails the whole decode");

        let image = decode(&data, None).expect("partial decode");
        assert_eq!(rows(&image), vec![vec![0; 8], vec![0; 8]]);
    }

    #[test]
    fn segments_referring_to_a_damaged_segment_are_left_out() {
        let mut data = page_information(8, 3);
        data.extend_from_slice(&black_row_region(1, 0, &[]));
        data.extend_from_slice(&damaged_region(2));
        data.extend_from_slice(&black_row_region(3, 1, &[2]));
        data.extend_from_slice(&black_row_region(4, 2, &[]));

        let image = decode(&data, None).expect("partial decode");
        assert_eq!(rows(&image), vec![vec![0; 8], vec![255; 8], vec![0; 8]]);
    }

    #[test]
    fn several_damaged_segments_are_each_skipped() {
        let mut data = page_information(8, 4);
        data.extend_from_slice(&damaged_region(1));
        data.extend_from_slice(&black_row_region(2, 1, &[]));
        data.extend_from_slice(&damaged_region(3));
        data.extend_from_slice(&damaged_region(4));
        data.extend_from_slice(&black_row_region(5, 3, &[]));

        let image = decode(&data, None).expect("partial decode");
        assert_eq!(
            rows(&image),
            vec![vec![255; 8], vec![0; 8], vec![255; 8], vec![0; 8]]
        );
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
