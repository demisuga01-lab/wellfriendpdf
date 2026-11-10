//! Raster page/image writers with output metadata (`pdftoppm` equivalents).
//!
//! [`crate::images::encoder::ImageEncoder`] keeps its simple, metadata-free
//! entry points. This module adds what page-raster output needs on top:
//!
//! - PNG with a `pHYs` resolution chunk, an optional embedded ICC profile
//!   (`iCCP`) or `sRGB` tag, grey, grey+alpha, RGB, RGBA and 1-bit output;
//! - JPEG with JFIF density, grey, CMYK, progressive and optimised Huffman
//!   tables;
//! - PNM (PBM / PGM / PPM) writers;
//! - pixel conversions: RGB to grey, RGB to CMYK, alpha flattening, and grey
//!   to 1-bit by threshold, ordered (halftone) dithering or error diffusion.
//!
//! TIFF lives in [`crate::images::tiff`].

use crate::error::{Result, WellfriendError};
use crate::images::decoder::RawImage;

/// Output metadata shared by the raster writers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RasterMetadata {
    /// Horizontal and vertical resolution in dots per inch. Written as PNG
    /// `pHYs`, JPEG JFIF density and TIFF X/YResolution.
    pub dpi: Option<(f64, f64)>,
    /// ICC profile to embed (PNG `iCCP`, JPEG `APP2`). Takes precedence over
    /// [`srgb`](Self::srgb).
    pub icc_profile: Option<Vec<u8>>,
    /// Tag the image as sRGB (PNG `sRGB` chunk) when no ICC profile is given.
    pub srgb: bool,
}

/// PNG compression effort.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PngEffort {
    /// Fast deflate (the page-render default).
    #[default]
    Fast,
    /// Default zlib effort.
    Default,
    /// Best compression (slowest).
    Best,
}

/// JPEG writer options.
#[derive(Debug, Clone, PartialEq)]
pub struct JpegOptions {
    /// Quality 1..=100.
    pub quality: u8,
    pub progressive: bool,
    pub optimize_huffman: bool,
    pub metadata: RasterMetadata,
}

impl Default for JpegOptions {
    fn default() -> Self {
        Self {
            quality: 85,
            progressive: false,
            optimize_huffman: false,
            metadata: RasterMetadata::default(),
        }
    }
}

/// A packed 1-bit image. Rows are padded to whole bytes, the most significant
/// bit is the leftmost pixel, and a set bit means **black** (the PBM / TIFF
/// MinIsWhite convention).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BilevelImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl BilevelImage {
    /// Bytes per packed row.
    pub fn row_bytes(&self) -> usize {
        (self.width as usize).div_ceil(8)
    }

    /// True when the pixel at (x, y) is black.
    pub fn is_black(&self, x: u32, y: u32) -> bool {
        let row = self.row_bytes();
        let byte = self.data[y as usize * row + x as usize / 8];
        byte & (0x80 >> (x % 8)) != 0
    }
}

/// How grey levels are reduced to black and white.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BilevelMethod {
    /// Pixels darker than the threshold become black.
    Threshold(u8),
    /// Ordered dispersed-dot dithering with an 8x8 Bayer screen: a halftone
    /// in the spirit of Splash's default screen.
    Halftone,
    /// Floyd-Steinberg error diffusion.
    ErrorDiffusion,
}

impl Default for BilevelMethod {
    fn default() -> Self {
        Self::Halftone
    }
}

const BAYER_8X8: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

fn require_channels(image: &RawImage, allowed: &[u8], what: &str) -> Result<()> {
    if !allowed.contains(&image.channels) {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "{what}: unsupported channel count {}",
            image.channels
        )));
    }
    let expected = image.width as usize * image.height as usize * image.channels as usize;
    if image.pixels.len() != expected {
        return Err(WellfriendError::invalid_input(format!(
            "{what}: pixel buffer holds {} bytes, expected {expected}",
            image.pixels.len()
        )));
    }
    Ok(())
}

#[inline]
fn luma(r: u8, g: u8, b: u8) -> u8 {
    // ITU-R BT.601 weights in 8.8 fixed point (77 + 150 + 29 = 256).
    ((u32::from(r) * 77 + u32::from(g) * 150 + u32::from(b) * 29 + 128) >> 8) as u8
}

/// Convert an RGB (3) or RGBA (4) image to grey (1) or grey+alpha (2).
/// Grey and grey+alpha input is returned unchanged.
pub fn to_gray(image: &RawImage) -> Result<RawImage> {
    require_channels(image, &[1, 2, 3, 4], "grey conversion")?;
    let pixels = match image.channels {
        1 | 2 => image.pixels.clone(),
        3 => image
            .pixels
            .chunks_exact(3)
            .map(|p| luma(p[0], p[1], p[2]))
            .collect(),
        _ => {
            let mut out = Vec::with_capacity(image.pixels.len() / 2);
            for p in image.pixels.chunks_exact(4) {
                out.push(luma(p[0], p[1], p[2]));
                out.push(p[3]);
            }
            out
        }
    };
    let channels = match image.channels {
        1 | 3 => 1,
        _ => 2,
    };
    Ok(RawImage {
        width: image.width,
        height: image.height,
        channels,
        bits_per_sample: 8,
        pixels,
    })
}

/// Composite an image with alpha (grey+alpha or RGBA, straight alpha) onto an
/// opaque background colour, dropping the alpha channel. Images without alpha
/// are returned unchanged.
pub fn flatten_alpha(image: &RawImage, background: [u8; 3]) -> Result<RawImage> {
    require_channels(image, &[1, 2, 3, 4], "alpha flattening")?;
    let blend = |c: u8, bg: u8, a: u8| -> u8 {
        let a = u32::from(a);
        ((u32::from(c) * a + u32::from(bg) * (255 - a) + 127) / 255) as u8
    };
    let (pixels, channels) = match image.channels {
        2 => {
            let bg = luma(background[0], background[1], background[2]);
            (
                image
                    .pixels
                    .chunks_exact(2)
                    .map(|p| blend(p[0], bg, p[1]))
                    .collect(),
                1,
            )
        }
        4 => {
            let mut out = Vec::with_capacity(image.pixels.len() / 4 * 3);
            for p in image.pixels.chunks_exact(4) {
                out.push(blend(p[0], background[0], p[3]));
                out.push(blend(p[1], background[1], p[3]));
                out.push(blend(p[2], background[2], p[3]));
            }
            (out, 3)
        }
        _ => (image.pixels.clone(), image.channels),
    };
    Ok(RawImage {
        width: image.width,
        height: image.height,
        channels,
        bits_per_sample: 8,
        pixels,
    })
}

/// Convert RGB (or RGBA, alpha ignored) to device CMYK with full grey
/// component replacement: `K = 1 - max(R, G, B)`. This is the uncalibrated
/// DeviceRGB to DeviceCMYK mapping of PDF 32000-1 10.3.5, not a colour-managed
/// separation.
pub fn rgb_to_cmyk(image: &RawImage) -> Result<RawImage> {
    require_channels(image, &[3, 4], "CMYK conversion")?;
    let step = image.channels as usize;
    let mut out = Vec::with_capacity(image.width as usize * image.height as usize * 4);
    for p in image.pixels.chunks_exact(step) {
        let (r, g, b) = (u32::from(p[0]), u32::from(p[1]), u32::from(p[2]));
        let max = r.max(g).max(b);
        let k = 255 - max;
        if max == 0 {
            out.extend_from_slice(&[0, 0, 0, 255]);
            continue;
        }
        let c = ((max - r) * 255 + max / 2) / max;
        let m = ((max - g) * 255 + max / 2) / max;
        let y = ((max - b) * 255 + max / 2) / max;
        out.extend_from_slice(&[c as u8, m as u8, y as u8, k as u8]);
    }
    Ok(RawImage {
        width: image.width,
        height: image.height,
        channels: 4,
        bits_per_sample: 8,
        pixels: out,
    })
}

/// Reduce an image to 1 bit per pixel. Colour and alpha input is converted to
/// grey first (alpha composited onto white).
pub fn to_bilevel(image: &RawImage, method: BilevelMethod) -> Result<BilevelImage> {
    let flat = flatten_alpha(image, [255, 255, 255])?;
    let gray = to_gray(&flat)?;
    let width = gray.width as usize;
    let height = gray.height as usize;
    let row_bytes = width.div_ceil(8);
    let mut data = vec![0u8; row_bytes * height];
    let mut set_black = |x: usize, y: usize| {
        data[y * row_bytes + x / 8] |= 0x80 >> (x % 8);
    };
    match method {
        BilevelMethod::Threshold(level) => {
            for y in 0..height {
                for x in 0..width {
                    if gray.pixels[y * width + x] < level {
                        set_black(x, y);
                    }
                }
            }
        }
        BilevelMethod::Halftone => {
            for y in 0..height {
                for x in 0..width {
                    // Threshold in (0, 255]: cell value v maps to 4v + 2.
                    let t = u16::from(BAYER_8X8[y % 8][x % 8]) * 4 + 2;
                    if u16::from(gray.pixels[y * width + x]) < t {
                        set_black(x, y);
                    }
                }
            }
        }
        BilevelMethod::ErrorDiffusion => {
            let mut current: Vec<i32> = vec![0; width + 2];
            let mut next: Vec<i32> = vec![0; width + 2];
            for y in 0..height {
                for x in 0..width {
                    let value = i32::from(gray.pixels[y * width + x]) + current[x + 1];
                    let (out, err) = if value < 128 {
                        set_black(x, y);
                        (0, value)
                    } else {
                        (255, value - 255)
                    };
                    let _ = out;
                    current[x + 2] += err * 7 / 16;
                    next[x] += err * 3 / 16;
                    next[x + 1] += err * 5 / 16;
                    next[x + 2] += err / 16;
                }
                std::mem::swap(&mut current, &mut next);
                next.iter_mut().for_each(|v| *v = 0);
            }
        }
    }
    Ok(BilevelImage {
        width: gray.width,
        height: gray.height,
        data,
    })
}

fn dots_per_meter(dpi: f64) -> u32 {
    if !dpi.is_finite() || dpi <= 0.0 {
        return 0;
    }
    (dpi / 0.0254).round().clamp(1.0, u32::MAX as f64) as u32
}

fn png_compression(effort: PngEffort) -> png::Compression {
    match effort {
        PngEffort::Fast => png::Compression::Fast,
        PngEffort::Default => png::Compression::Default,
        PngEffort::Best => png::Compression::Best,
    }
}

fn png_write(
    width: u32,
    height: u32,
    color: png::ColorType,
    depth: png::BitDepth,
    data: &[u8],
    metadata: &RasterMetadata,
    effort: PngEffort,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(color);
        encoder.set_depth(depth);
        encoder.set_compression(png_compression(effort));
        if let Some((x, y)) = metadata.dpi {
            let (xppu, yppu) = (dots_per_meter(x), dots_per_meter(y));
            if xppu > 0 && yppu > 0 {
                encoder.set_pixel_dims(Some(png::PixelDimensions {
                    xppu,
                    yppu,
                    unit: png::Unit::Meter,
                }));
            }
        }
        if metadata.icc_profile.is_none() && metadata.srgb {
            encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        }
        let mut writer = encoder
            .write_header()
            .map_err(|e| WellfriendError::MalformedPdf(format!("PNG header error: {e}")))?;
        if let Some(profile) = metadata.icc_profile.as_deref() {
            writer
                .write_chunk(png::chunk::iCCP, &iccp_chunk_payload(profile))
                .map_err(|e| WellfriendError::MalformedPdf(format!("PNG iCCP error: {e}")))?;
        }
        writer
            .write_image_data(data)
            .map_err(|e| WellfriendError::MalformedPdf(format!("PNG encode error: {e}")))?;
    }
    Ok(out)
}

/// `iCCP` payload: profile name, NUL, compression method 0, zlib data.
fn iccp_chunk_payload(profile: &[u8]) -> Vec<u8> {
    let mut payload = b"ICC Profile\0\0".to_vec();
    payload.extend_from_slice(&crate::filters::flate_encode(profile, 6));
    payload
}

/// Encode a grey (1), grey+alpha (2), RGB (3) or RGBA (4, straight alpha)
/// image as an 8-bit PNG with optional resolution and colour metadata.
pub fn encode_png(
    image: &RawImage,
    metadata: &RasterMetadata,
    effort: PngEffort,
) -> Result<Vec<u8>> {
    require_channels(image, &[1, 2, 3, 4], "PNG encoding")?;
    let color = match image.channels {
        1 => png::ColorType::Grayscale,
        2 => png::ColorType::GrayscaleAlpha,
        3 => png::ColorType::Rgb,
        _ => png::ColorType::Rgba,
    };
    png_write(
        image.width,
        image.height,
        color,
        png::BitDepth::Eight,
        &image.pixels,
        metadata,
        effort,
    )
}

/// Encode a 1-bit image as a 1-bit greyscale PNG (black stays black).
pub fn encode_png_bilevel(
    image: &BilevelImage,
    metadata: &RasterMetadata,
    effort: PngEffort,
) -> Result<Vec<u8>> {
    // PNG grey: 0 is black, so invert the PBM-style "1 = black" bits.
    let inverted: Vec<u8> = image.data.iter().map(|b| !b).collect();
    png_write(
        image.width,
        image.height,
        png::ColorType::Grayscale,
        png::BitDepth::One,
        &inverted,
        metadata,
        effort,
    )
}

/// Encode grey (1), RGB (3) or CMYK (4) as JPEG. CMYK is written as Adobe
/// YCCK, the form Poppler's `-jpegcmyk` produces and viewers read.
pub fn encode_jpeg(image: &RawImage, cmyk: bool, options: &JpegOptions) -> Result<Vec<u8>> {
    use jpeg_encoder::{ColorType, Density, Encoder};

    require_channels(image, &[1, 3, 4], "JPEG encoding")?;
    let color = match (image.channels, cmyk) {
        (1, false) => ColorType::Luma,
        (3, false) => ColorType::Rgb,
        (4, true) => ColorType::CmykAsYcck,
        (n, _) => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "JPEG encoding: {n} channels with cmyk={cmyk} is not a supported combination"
            )))
        }
    };
    let width = u16::try_from(image.width).map_err(|_| {
        WellfriendError::UnsupportedFeature(format!(
            "JPEG encoding: image width {} exceeds 65535",
            image.width
        ))
    })?;
    let height = u16::try_from(image.height).map_err(|_| {
        WellfriendError::UnsupportedFeature(format!(
            "JPEG encoding: image height {} exceeds 65535",
            image.height
        ))
    })?;
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, options.quality.clamp(1, 100));
    if let Some((x, y)) = options.metadata.dpi {
        let clamp = |v: f64| -> u16 {
            if v.is_finite() {
                v.round().clamp(1.0, f64::from(u16::MAX)) as u16
            } else {
                72
            }
        };
        encoder.set_density(Density::Inch {
            x: clamp(x),
            y: clamp(y),
        });
    }
    encoder.set_progressive(options.progressive);
    encoder.set_optimized_huffman_tables(options.optimize_huffman);
    if let Some(profile) = options.metadata.icc_profile.as_deref() {
        encoder
            .add_icc_profile(profile)
            .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG ICC profile error: {e}")))?;
    }
    encoder
        .encode(&image.pixels, width, height, color)
        .map_err(|e| WellfriendError::MalformedPdf(format!("JPEG encode error: {e}")))?;
    Ok(out)
}

/// Binary PBM (`P4`): 1 = black, rows padded to bytes.
pub fn encode_pbm(image: &BilevelImage) -> Vec<u8> {
    let mut out = format!("P4\n{} {}\n", image.width, image.height).into_bytes();
    out.extend_from_slice(&image.data);
    out
}

/// Binary PGM (`P5`, maxval 255) from grey, or PPM (`P6`) from RGB. Alpha
/// input must be flattened first.
pub fn encode_pnm(image: &RawImage) -> Result<Vec<u8>> {
    require_channels(image, &[1, 3], "PNM encoding")?;
    let magic = if image.channels == 1 { "P5" } else { "P6" };
    let mut out = format!("{magic}\n{} {}\n255\n", image.width, image.height).into_bytes();
    out.extend_from_slice(&image.pixels);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(width: u32, height: u32, pixels: Vec<u8>) -> RawImage {
        RawImage {
            width,
            height,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        }
    }

    fn find_chunk<'a>(png_bytes: &'a [u8], name: &[u8; 4]) -> Option<&'a [u8]> {
        let mut pos = 8;
        while pos + 8 <= png_bytes.len() {
            let len = u32::from_be_bytes(png_bytes[pos..pos + 4].try_into().unwrap()) as usize;
            let kind = &png_bytes[pos + 4..pos + 8];
            if kind == name {
                return Some(&png_bytes[pos + 8..pos + 8 + len]);
            }
            pos += 12 + len;
        }
        None
    }

    #[test]
    fn png_writes_phys_for_dpi() {
        let image = rgb(1, 1, vec![1, 2, 3]);
        let meta = RasterMetadata {
            dpi: Some((150.0, 300.0)),
            ..Default::default()
        };
        let bytes = encode_png(&image, &meta, PngEffort::Fast).unwrap();
        let phys = find_chunk(&bytes, b"pHYs").expect("pHYs chunk");
        assert_eq!(u32::from_be_bytes(phys[0..4].try_into().unwrap()), 5906);
        assert_eq!(u32::from_be_bytes(phys[4..8].try_into().unwrap()), 11811);
        assert_eq!(phys[8], 1, "unit is metre");
    }

    #[test]
    fn png_writes_iccp_or_srgb() {
        let image = rgb(1, 1, vec![1, 2, 3]);
        let srgb = encode_png(
            &image,
            &RasterMetadata {
                srgb: true,
                ..Default::default()
            },
            PngEffort::Fast,
        )
        .unwrap();
        assert!(find_chunk(&srgb, b"sRGB").is_some());
        assert!(find_chunk(&srgb, b"iCCP").is_none());

        let icc = encode_png(
            &image,
            &RasterMetadata {
                icc_profile: Some(vec![7u8; 200]),
                srgb: true,
                ..Default::default()
            },
            PngEffort::Fast,
        )
        .unwrap();
        let chunk = find_chunk(&icc, b"iCCP").expect("iCCP chunk");
        assert!(chunk.starts_with(b"ICC Profile\0\0"));
        assert!(find_chunk(&icc, b"sRGB").is_none());
        // The png decoder must accept the file and expose the profile.
        let decoder = png::Decoder::new(std::io::Cursor::new(&icc));
        let reader = decoder.read_info().unwrap();
        assert_eq!(reader.info().icc_profile.as_deref(), Some(&[7u8; 200][..]));
    }

    #[test]
    fn png_rgba_and_gray_alpha_round_trip() {
        let rgba = RawImage {
            width: 2,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 0, 0, 0, 255, 128],
        };
        let bytes = encode_png(&rgba, &RasterMetadata::default(), PngEffort::Fast).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[..8], &rgba.pixels[..]);

        let ga = to_gray(&rgba).unwrap();
        assert_eq!(ga.channels, 2);
        assert_eq!(ga.pixels[1], 0);
        assert_eq!(ga.pixels[3], 128);
        let bytes = encode_png(&ga, &RasterMetadata::default(), PngEffort::Fast).unwrap();
        let reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().color_type, png::ColorType::GrayscaleAlpha);
    }

    #[test]
    fn bilevel_png_keeps_black_black() {
        let image = rgb(3, 1, vec![0, 0, 0, 255, 255, 255, 10, 10, 10]);
        let bits = to_bilevel(&image, BilevelMethod::Threshold(128)).unwrap();
        assert!(bits.is_black(0, 0));
        assert!(!bits.is_black(1, 0));
        assert!(bits.is_black(2, 0));
        let bytes = encode_png_bilevel(&bits, &RasterMetadata::default(), PngEffort::Fast).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(&bytes))
            .read_info()
            .unwrap();
        assert_eq!(reader.info().bit_depth, png::BitDepth::One);
        let mut buf = vec![0; reader.output_buffer_size()];
        reader.next_frame(&mut buf).unwrap();
        // 0 = black in PNG grey: pixel 0 and 2 black, pixel 1 white.
        assert_eq!(buf[0] & 0b1110_0000, 0b0100_0000);
    }

    #[test]
    fn halftone_reproduces_mid_grey_density() {
        let image = RawImage {
            width: 64,
            height: 64,
            channels: 1,
            bits_per_sample: 8,
            pixels: vec![128; 64 * 64],
        };
        for method in [BilevelMethod::Halftone, BilevelMethod::ErrorDiffusion] {
            let bits = to_bilevel(&image, method).unwrap();
            let black: u32 = bits.data.iter().map(|b| b.count_ones()).sum();
            let ratio = f64::from(black) / 4096.0;
            assert!((ratio - 0.5).abs() < 0.05, "{method:?} gave {ratio}");
        }
        let white = RawImage {
            pixels: vec![255; 64 * 64],
            ..image.clone()
        };
        let bits = to_bilevel(&white, BilevelMethod::Halftone).unwrap();
        assert!(bits.data.iter().all(|&b| b == 0), "white stays white");
        let black = RawImage {
            pixels: vec![0; 64 * 64],
            ..image
        };
        let bits = to_bilevel(&black, BilevelMethod::Halftone).unwrap();
        assert!(bits.data.iter().all(|&b| b == 0xFF), "black stays black");
    }

    #[test]
    fn jpeg_writes_jfif_density_gray_and_cmyk() {
        let image = rgb(8, 8, vec![90; 8 * 8 * 3]);
        let options = JpegOptions {
            metadata: RasterMetadata {
                dpi: Some((300.0, 150.0)),
                ..Default::default()
            },
            ..Default::default()
        };
        let bytes = encode_jpeg(&image, false, &options).unwrap();
        // APP0 JFIF: FFD8 FFE0 len(2) "JFIF\0" ver(2) units(1) xd(2) yd(2)
        assert_eq!(&bytes[6..11], b"JFIF\0");
        assert_eq!(bytes[13], 1, "density unit is inch");
        assert_eq!(u16::from_be_bytes([bytes[14], bytes[15]]), 300);
        assert_eq!(u16::from_be_bytes([bytes[16], bytes[17]]), 150);

        let gray = to_gray(&image).unwrap();
        let gray_jpeg = encode_jpeg(&gray, false, &options).unwrap();
        let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(&gray_jpeg));
        decoder.decode().unwrap();
        assert_eq!(
            decoder.info().unwrap().pixel_format,
            jpeg_decoder::PixelFormat::L8
        );

        let cmyk = rgb_to_cmyk(&image).unwrap();
        let cmyk_jpeg = encode_jpeg(&cmyk, true, &options).unwrap();
        let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(&cmyk_jpeg));
        decoder.decode().unwrap();
        assert_eq!(
            decoder.info().unwrap().pixel_format,
            jpeg_decoder::PixelFormat::CMYK32
        );
    }

    #[test]
    fn cmyk_conversion_uses_full_gcr() {
        let image = rgb(3, 1, vec![255, 255, 255, 0, 0, 0, 255, 0, 0]);
        let cmyk = rgb_to_cmyk(&image).unwrap();
        assert_eq!(&cmyk.pixels[0..4], &[0, 0, 0, 0]);
        assert_eq!(&cmyk.pixels[4..8], &[0, 0, 0, 255]);
        assert_eq!(&cmyk.pixels[8..12], &[0, 255, 255, 0]);
    }

    #[test]
    fn pnm_headers() {
        let image = rgb(2, 1, vec![1, 2, 3, 4, 5, 6]);
        let ppm = encode_pnm(&image).unwrap();
        assert!(ppm.starts_with(b"P6\n2 1\n255\n"));
        assert_eq!(&ppm[ppm.len() - 6..], &[1, 2, 3, 4, 5, 6]);
        let pgm = encode_pnm(&to_gray(&image).unwrap()).unwrap();
        assert!(pgm.starts_with(b"P5\n2 1\n255\n"));
        let bits = to_bilevel(&image, BilevelMethod::Threshold(128)).unwrap();
        let pbm = encode_pbm(&bits);
        assert_eq!(pbm, b"P4\n2 1\n\xC0".to_vec());
    }

    #[test]
    fn flatten_alpha_composites_onto_background() {
        let rgba = RawImage {
            width: 1,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![0, 0, 0, 0],
        };
        let flat = flatten_alpha(&rgba, [255, 255, 255]).unwrap();
        assert_eq!(flat.pixels, vec![255, 255, 255]);
    }
}
