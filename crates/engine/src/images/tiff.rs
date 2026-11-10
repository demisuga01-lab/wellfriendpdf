//! Minimal baseline TIFF 6.0 writer (little-endian, chunky, strip-based).
//!
//! Written by hand instead of pulling in a TIFF crate: page rasters only need
//! a handful of tags. Supported pixel layouts are 1-bit black and white
//! (MinIsWhite), 8-bit grey, RGB, RGBA (unassociated alpha) and CMYK
//! (Separated, InkSet CMYK). Compression is none, PackBits or Adobe Deflate.

use std::io::Write;

use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;
use crate::images::raster::BilevelImage;

/// TIFF strip compression.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TiffCompression {
    /// Uncompressed (Compression = 1).
    None,
    /// PackBits run-length coding (Compression = 32773).
    PackBits,
    /// Adobe Deflate / zlib (Compression = 8).
    #[default]
    Deflate,
}

impl TiffCompression {
    /// Parse a CLI-style name: `none`, `packbits`, `deflate` (alias `zip`).
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "packbits" => Some(Self::PackBits),
            "deflate" | "zip" => Some(Self::Deflate),
            _ => None,
        }
    }

    fn tag_value(self) -> u16 {
        match self {
            Self::None => 1,
            Self::PackBits => 32773,
            Self::Deflate => 8,
        }
    }
}

/// Pixels to write.
#[derive(Debug, Clone, Copy)]
pub enum TiffPixels<'a> {
    /// 1-bit, set bit = black.
    Bilevel(&'a BilevelImage),
    /// 8-bit grey (1 channel).
    Gray(&'a RawImage),
    /// 8-bit RGB (3 channels).
    Rgb(&'a RawImage),
    /// 8-bit RGBA with straight (unassociated) alpha (4 channels).
    Rgba(&'a RawImage),
    /// 8-bit CMYK (4 channels).
    Cmyk(&'a RawImage),
}

/// TIFF writer options.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TiffOptions {
    pub compression: TiffCompression,
    /// Resolution in DPI; written as X/YResolution with ResolutionUnit inch.
    pub dpi: Option<(f64, f64)>,
}

const SHORT: u16 = 3;
const LONG: u16 = 4;
const RATIONAL: u16 = 5;

struct Entry {
    tag: u16,
    kind: u16,
    count: u32,
    data: Vec<u8>,
}

fn shorts(tag: u16, values: &[u16]) -> Entry {
    Entry {
        tag,
        kind: SHORT,
        count: values.len() as u32,
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn longs(tag: u16, values: &[u32]) -> Entry {
    Entry {
        tag,
        kind: LONG,
        count: values.len() as u32,
        data: values.iter().flat_map(|v| v.to_le_bytes()).collect(),
    }
}

fn rational(tag: u16, value: f64) -> Entry {
    // Keep two decimals of precision for fractional resolutions.
    let value = if value.is_finite() && value > 0.0 {
        value
    } else {
        72.0
    };
    let denominator = 100u32;
    let numerator = (value * f64::from(denominator))
        .round()
        .clamp(1.0, u32::MAX as f64) as u32;
    let mut data = numerator.to_le_bytes().to_vec();
    data.extend_from_slice(&denominator.to_le_bytes());
    Entry {
        tag,
        kind: RATIONAL,
        count: 1,
        data,
    }
}

/// PackBits-encode one row (TIFF requires rows to be packed separately).
fn packbits_row(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        // Measure a run of identical bytes.
        let mut run = 1;
        while i + run < row.len() && run < 128 && row[i + run] == row[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1i16 - run as i16) as i8 as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // Literal run until the next repeat of two or more.
        let start = i;
        let mut len = 0;
        while i < row.len() && len < 128 {
            if i + 1 < row.len() && row[i] == row[i + 1] {
                break;
            }
            i += 1;
            len += 1;
        }
        out.push((len - 1) as u8);
        out.extend_from_slice(&row[start..start + len]);
    }
}

fn check_raw(image: &RawImage, channels: u8) -> Result<()> {
    let expected = image.width as usize * image.height as usize * channels as usize;
    if image.channels != channels || image.pixels.len() != expected {
        return Err(WellfriendError::invalid_input(format!(
            "TIFF encoding: expected {channels}-channel {}x{} pixels ({expected} bytes), got {} channels / {} bytes",
            image.width,
            image.height,
            image.channels,
            image.pixels.len()
        )));
    }
    Ok(())
}

/// Encode an image as a baseline TIFF file.
pub fn encode_tiff(pixels: TiffPixels<'_>, options: &TiffOptions) -> Result<Vec<u8>> {
    let (width, height, data, bits, samples, photometric): (u32, u32, &[u8], u16, u16, u16) =
        match pixels {
            TiffPixels::Bilevel(image) => {
                if image.data.len() != image.row_bytes() * image.height as usize {
                    return Err(WellfriendError::invalid_input(
                        "TIFF encoding: bilevel buffer does not match its dimensions",
                    ));
                }
                (image.width, image.height, &image.data, 1, 1, 0)
            }
            TiffPixels::Gray(image) => {
                check_raw(image, 1)?;
                (image.width, image.height, &image.pixels, 8, 1, 1)
            }
            TiffPixels::Rgb(image) => {
                check_raw(image, 3)?;
                (image.width, image.height, &image.pixels, 8, 3, 2)
            }
            TiffPixels::Rgba(image) => {
                check_raw(image, 4)?;
                (image.width, image.height, &image.pixels, 8, 4, 2)
            }
            TiffPixels::Cmyk(image) => {
                check_raw(image, 4)?;
                (image.width, image.height, &image.pixels, 8, 4, 5)
            }
        };
    if width == 0 || height == 0 {
        return Err(WellfriendError::invalid_input(
            "TIFF encoding: image has no pixels",
        ));
    }
    let row_bytes = if bits == 1 {
        (width as usize).div_ceil(8)
    } else {
        width as usize * samples as usize
    };
    let rows_per_strip = (64 * 1024 / row_bytes.max(1)).clamp(1, height as usize);

    let mut out: Vec<u8> = Vec::with_capacity(data.len() / 2 + 512);
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // IFD offset, patched below

    let mut strip_offsets = Vec::new();
    let mut strip_counts = Vec::new();
    for strip in data.chunks(rows_per_strip * row_bytes) {
        let encoded: Vec<u8> = match options.compression {
            TiffCompression::None => strip.to_vec(),
            TiffCompression::PackBits => {
                let mut packed = Vec::with_capacity(strip.len() / 2);
                for row in strip.chunks(row_bytes) {
                    packbits_row(row, &mut packed);
                }
                packed
            }
            TiffCompression::Deflate => {
                let mut encoder =
                    flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(6));
                encoder
                    .write_all(strip)
                    .and_then(|_| encoder.finish())
                    .map_err(|e| {
                        WellfriendError::MalformedPdf(format!("TIFF deflate failed: {e}"))
                    })?
            }
        };
        strip_offsets.push(u32::try_from(out.len()).map_err(|_| {
            WellfriendError::ResourceLimit("TIFF output exceeds 4 GiB".to_string())
        })?);
        strip_counts.push(encoded.len() as u32);
        out.extend_from_slice(&encoded);
    }
    if out.len() % 2 == 1 {
        out.push(0);
    }

    let mut entries = vec![
        longs(256, &[width]),
        longs(257, &[height]),
        shorts(258, &vec![bits; samples as usize]),
        shorts(259, &[options.compression.tag_value()]),
        shorts(262, &[photometric]),
        longs(273, &strip_offsets),
        shorts(277, &[samples]),
        longs(278, &[rows_per_strip as u32]),
        longs(279, &strip_counts),
    ];
    let (xdpi, ydpi) = options.dpi.unwrap_or((72.0, 72.0));
    entries.push(rational(282, xdpi));
    entries.push(rational(283, ydpi));
    entries.push(shorts(284, &[1]));
    entries.push(shorts(296, &[if options.dpi.is_some() { 2 } else { 1 }]));
    if photometric == 5 {
        entries.push(shorts(332, &[1])); // InkSet: CMYK
    }
    if matches!(pixels, TiffPixels::Rgba(_)) {
        entries.push(shorts(338, &[2])); // ExtraSamples: unassociated alpha
    }
    entries.sort_by_key(|entry| entry.tag);

    let ifd_offset = out.len() as u32;
    out[4..8].copy_from_slice(&ifd_offset.to_le_bytes());
    let ifd_len = 2 + entries.len() * 12 + 4;
    let mut extra_offset = ifd_offset as usize + ifd_len;
    let mut extra = Vec::new();
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for entry in &entries {
        out.extend_from_slice(&entry.tag.to_le_bytes());
        out.extend_from_slice(&entry.kind.to_le_bytes());
        out.extend_from_slice(&entry.count.to_le_bytes());
        if entry.data.len() <= 4 {
            let mut inline = [0u8; 4];
            inline[..entry.data.len()].copy_from_slice(&entry.data);
            out.extend_from_slice(&inline);
        } else {
            out.extend_from_slice(&(extra_offset as u32).to_le_bytes());
            extra.extend_from_slice(&entry.data);
            extra_offset += entry.data.len();
            if entry.data.len() % 2 == 1 {
                extra.push(0);
                extra_offset += 1;
            }
        }
    }
    out.extend_from_slice(&0u32.to_le_bytes()); // no next IFD
    out.extend_from_slice(&extra);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tiny reader for the writer's own output: returns tag -> (type, count,
    /// value-or-offset bytes) and the decoded strip data.
    fn read_tiff(bytes: &[u8]) -> (std::collections::BTreeMap<u16, Vec<u32>>, Vec<u8>) {
        assert_eq!(&bytes[0..4], b"II*\0");
        let ifd = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
        let count = u16::from_le_bytes(bytes[ifd..ifd + 2].try_into().unwrap()) as usize;
        let mut tags = std::collections::BTreeMap::new();
        for i in 0..count {
            let e = ifd + 2 + i * 12;
            let tag = u16::from_le_bytes(bytes[e..e + 2].try_into().unwrap());
            let kind = u16::from_le_bytes(bytes[e + 2..e + 4].try_into().unwrap());
            let n = u32::from_le_bytes(bytes[e + 4..e + 8].try_into().unwrap()) as usize;
            let size = match kind {
                3 => 2,
                4 => 4,
                5 => 8,
                _ => panic!("unexpected type"),
            };
            let total = size * n;
            let data = if total <= 4 {
                &bytes[e + 8..e + 8 + total]
            } else {
                let off = u32::from_le_bytes(bytes[e + 8..e + 12].try_into().unwrap()) as usize;
                &bytes[off..off + total]
            };
            let values: Vec<u32> = match kind {
                3 => data
                    .chunks(2)
                    .map(|c| u32::from(u16::from_le_bytes([c[0], c[1]])))
                    .collect(),
                _ => data
                    .chunks(4)
                    .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
                    .collect(),
            };
            tags.insert(tag, values);
        }
        let compression = tags[&259][0];
        let mut pixels = Vec::new();
        for (off, len) in tags[&273].iter().zip(tags[&279].iter()) {
            let strip = &bytes[*off as usize..(*off + *len) as usize];
            match compression {
                1 => pixels.extend_from_slice(strip),
                8 => {
                    use std::io::Read;
                    let mut d = flate2::read::ZlibDecoder::new(strip);
                    d.read_to_end(&mut pixels).unwrap();
                }
                32773 => {
                    let mut i = 0;
                    while i < strip.len() {
                        let n = strip[i] as i8;
                        i += 1;
                        if n >= 0 {
                            let len = n as usize + 1;
                            pixels.extend_from_slice(&strip[i..i + len]);
                            i += len;
                        } else if n != -128 {
                            let len = (1 - n as i16) as usize;
                            pixels.extend(std::iter::repeat_n(strip[i], len));
                            i += 1;
                        }
                    }
                }
                other => panic!("compression {other}"),
            }
        }
        (tags, pixels)
    }

    fn gradient(width: u32, height: u32, channels: u8) -> RawImage {
        let pixels = (0..width * height * u32::from(channels))
            .map(|i| ((i / 7) % 251) as u8)
            .collect();
        RawImage {
            width,
            height,
            channels,
            bits_per_sample: 8,
            pixels,
        }
    }

    #[test]
    fn rgb_round_trips_through_every_compression() {
        let image = gradient(300, 250, 3);
        for compression in [
            TiffCompression::None,
            TiffCompression::PackBits,
            TiffCompression::Deflate,
        ] {
            let bytes = encode_tiff(
                TiffPixels::Rgb(&image),
                &TiffOptions {
                    compression,
                    dpi: Some((150.0, 300.0)),
                },
            )
            .unwrap();
            let (tags, pixels) = read_tiff(&bytes);
            assert_eq!(tags[&256], vec![300]);
            assert_eq!(tags[&257], vec![250]);
            assert_eq!(tags[&258], vec![8, 8, 8]);
            assert_eq!(tags[&262], vec![2]);
            assert_eq!(tags[&296], vec![2]);
            assert_eq!(tags[&282], vec![15000, 100]);
            assert_eq!(tags[&283], vec![30000, 100]);
            assert!(tags[&273].len() > 1, "multiple strips");
            assert_eq!(pixels, image.pixels, "{compression:?}");
        }
    }

    #[test]
    fn cmyk_rgba_gray_and_bilevel_tags() {
        let cmyk = gradient(5, 3, 4);
        let (tags, pixels) =
            read_tiff(&encode_tiff(TiffPixels::Cmyk(&cmyk), &TiffOptions::default()).unwrap());
        assert_eq!(tags[&262], vec![5]);
        assert_eq!(tags[&332], vec![1]);
        assert_eq!(tags[&277], vec![4]);
        assert_eq!(pixels, cmyk.pixels);

        let (tags, _) =
            read_tiff(&encode_tiff(TiffPixels::Rgba(&cmyk), &TiffOptions::default()).unwrap());
        assert_eq!(tags[&262], vec![2]);
        assert_eq!(tags[&338], vec![2]);

        let gray = gradient(5, 3, 1);
        let (tags, pixels) =
            read_tiff(&encode_tiff(TiffPixels::Gray(&gray), &TiffOptions::default()).unwrap());
        assert_eq!(tags[&262], vec![1]);
        assert_eq!(pixels, gray.pixels);

        let bits = BilevelImage {
            width: 10,
            height: 2,
            data: vec![0xFF, 0xC0, 0x00, 0x40],
        };
        let (tags, pixels) = read_tiff(
            &encode_tiff(
                TiffPixels::Bilevel(&bits),
                &TiffOptions {
                    compression: TiffCompression::PackBits,
                    dpi: None,
                },
            )
            .unwrap(),
        );
        assert_eq!(tags[&258], vec![1]);
        assert_eq!(tags[&262], vec![0], "MinIsWhite");
        assert_eq!(pixels, bits.data);
    }

    #[test]
    fn packbits_matches_reference_vector() {
        // Example from the TIFF 6.0 specification, section 9.
        let input = [
            0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0xAA, 0xAA, 0xAA, 0xAA, 0x80, 0x00, 0x2A, 0x22,
            0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA,
        ];
        let mut out = Vec::new();
        packbits_row(&input, &mut out);
        assert_eq!(
            out,
            vec![
                0xFE, 0xAA, 0x02, 0x80, 0x00, 0x2A, 0xFD, 0xAA, 0x03, 0x80, 0x00, 0x2A, 0x22, 0xF7,
                0xAA
            ]
        );
    }
}
