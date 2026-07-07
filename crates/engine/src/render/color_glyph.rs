use std::io::Cursor;

use crate::error::{OxideError, Result};
use crate::fonts::variations::{self, VariationRequest};
use crate::images::decoder::RawImage;
use crate::render::buffer::{rgba, PixelColor};
use crate::render::font_rasterizer::GlyphToPath;
use crate::render::path::Path;
use ttf_parser::colr::{CompositeMode, Paint, Painter};
use ttf_parser::{GlyphId, RasterGlyphImage, RasterImageFormat, RgbaColor, Tag, Transform};

const MAX_COLOR_GLYPH_PIXELS: u32 = 4096 * 4096;
const MAX_COLOR_GLYPH_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorGlyphKind {
    None,
    ColrCpal,
    RasterBitmap,
    SvgBlocked,
}

impl ColorGlyphKind {
    pub(crate) fn cache_mode(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ColrCpal => 1,
            Self::RasterBitmap => 2,
            Self::SvgBlocked => 3,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ColrLayer {
    pub glyph_id: u16,
    pub color: PixelColor,
}

pub(crate) fn resolve_request_glyph_id(
    font_bytes: &[u8],
    is_gid: bool,
    code: u16,
    ch: char,
    glyph_name: Option<&str>,
    variation: &VariationRequest,
) -> Option<GlyphId> {
    if is_gid {
        return Some(GlyphId(code));
    }
    let mut face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    variations::apply_request(&mut face, variation);
    crate::render::glyph_outline::resolve_glyph_id_for_simple(&face, code, ch, glyph_name)
}

pub(crate) fn color_glyph_kind(font_bytes: &[u8], glyph_id: GlyphId) -> ColorGlyphKind {
    let Ok(face) = ttf_parser::Face::parse(font_bytes, 0) else {
        return ColorGlyphKind::None;
    };
    if face.is_color_glyph(glyph_id) {
        return ColorGlyphKind::ColrCpal;
    }
    let ppem = face.units_per_em().clamp(1, u16::MAX);
    if face.glyph_raster_image(glyph_id, ppem).is_some() {
        return ColorGlyphKind::RasterBitmap;
    }
    if face.glyph_svg_image(glyph_id).is_some() {
        return ColorGlyphKind::SvgBlocked;
    }
    ColorGlyphKind::None
}

pub(crate) fn colr_cpal_layers(
    font_bytes: &[u8],
    glyph_id: GlyphId,
    foreground: PixelColor,
    graphics_alpha: u8,
    variation: &VariationRequest,
) -> Option<Vec<ColrLayer>> {
    let mut face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    variations::apply_request(&mut face, variation);
    if !face.is_color_glyph(glyph_id) {
        return None;
    }

    let mut collector = SolidLayerCollector::new(graphics_alpha);
    let foreground = RgbaColor::new(foreground[0], foreground[1], foreground[2], foreground[3]);
    face.paint_color_glyph(glyph_id, 0, foreground, &mut collector)?;
    if collector.unsupported || collector.layers.is_empty() {
        return None;
    }
    Some(collector.layers)
}

pub(crate) fn decode_raster_glyph_image(
    font_bytes: &[u8],
    glyph_id: GlyphId,
    target_ppem: u16,
) -> Result<Option<DecodedRasterGlyph>> {
    let face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => return Ok(None),
    };
    let Some(image) = face.glyph_raster_image(glyph_id, target_ppem.max(1)) else {
        return Ok(None);
    };
    if u32::from(image.width).saturating_mul(u32::from(image.height)) > MAX_COLOR_GLYPH_PIXELS {
        return Err(OxideError::UnsupportedFeature(format!(
            "color glyph raster strike too large: glyph={} strike={} dimensions={}x{}",
            glyph_id.0, image.pixels_per_em, image.width, image.height
        )));
    }
    Ok(Some(DecodedRasterGlyph {
        x: image.x,
        y: image.y,
        pixels_per_em: image.pixels_per_em.max(1),
        image: decode_raster_image_payload(image)?,
    }))
}

pub(crate) fn color_font_table_summary(font_bytes: &[u8]) -> ColorFontTableSummary {
    let Ok(face) = ttf_parser::Face::parse(font_bytes, 0) else {
        return ColorFontTableSummary::default();
    };
    let raw = face.raw_face();
    ColorFontTableSummary {
        colr_version: raw
            .table(Tag::from_bytes(b"COLR"))
            .and_then(|data| data.get(0..2))
            .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]])),
        has_cpal: raw.table(Tag::from_bytes(b"CPAL")).is_some(),
        has_cbdt: raw.table(Tag::from_bytes(b"CBDT")).is_some(),
        has_cblc: raw.table(Tag::from_bytes(b"CBLC")).is_some(),
        has_sbix: raw.table(Tag::from_bytes(b"sbix")).is_some(),
        has_svg: raw.table(Tag::from_bytes(b"SVG ")).is_some(),
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ColorFontTableSummary {
    pub colr_version: Option<u16>,
    pub has_cpal: bool,
    pub has_cbdt: bool,
    pub has_cblc: bool,
    pub has_sbix: bool,
    pub has_svg: bool,
}

impl ColorFontTableSummary {
    pub(crate) fn supports_colr_cpal_v0(&self) -> bool {
        self.colr_version == Some(0) && self.has_cpal
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedRasterGlyph {
    pub x: i16,
    pub y: i16,
    pub pixels_per_em: u16,
    pub image: RawImage,
}

struct SolidLayerCollector {
    current_glyph: Option<u16>,
    layers: Vec<ColrLayer>,
    graphics_alpha: u8,
    unsupported: bool,
}

impl SolidLayerCollector {
    fn new(graphics_alpha: u8) -> Self {
        Self {
            current_glyph: None,
            layers: Vec::new(),
            graphics_alpha,
            unsupported: false,
        }
    }
}

impl<'a> Painter<'a> for SolidLayerCollector {
    fn outline_glyph(&mut self, glyph_id: GlyphId) {
        self.current_glyph = Some(glyph_id.0);
    }

    fn paint(&mut self, paint: Paint<'a>) {
        match paint {
            Paint::Solid(color) => {
                if let Some(glyph_id) = self.current_glyph {
                    self.layers.push(ColrLayer {
                        glyph_id,
                        color: rgba(
                            color.red,
                            color.green,
                            color.blue,
                            multiply_alpha(color.alpha, self.graphics_alpha),
                        ),
                    });
                }
            }
            Paint::LinearGradient(_) | Paint::RadialGradient(_) | Paint::SweepGradient(_) => {
                self.unsupported = true;
            }
        }
    }

    fn push_clip(&mut self) {
        self.unsupported = true;
    }

    fn push_clip_box(&mut self, _clipbox: ttf_parser::colr::ClipBox) {
        self.unsupported = true;
    }

    fn pop_clip(&mut self) {}

    fn push_layer(&mut self, _mode: CompositeMode) {
        self.unsupported = true;
    }

    fn pop_layer(&mut self) {}

    fn push_translate(&mut self, _tx: f32, _ty: f32) {
        self.unsupported = true;
    }

    fn push_scale(&mut self, _sx: f32, _sy: f32) {
        self.unsupported = true;
    }

    fn push_rotate(&mut self, _angle: f32) {
        self.unsupported = true;
    }

    fn push_skew(&mut self, _skew_x: f32, _skew_y: f32) {
        self.unsupported = true;
    }

    fn push_transform(&mut self, _transform: Transform) {
        self.unsupported = true;
    }

    fn pop_transform(&mut self) {}
}

pub(crate) fn outline_gid_path(
    font_bytes: &[u8],
    glyph_id: u16,
    variation: &VariationRequest,
) -> Option<Path> {
    let mut face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    variations::apply_request(&mut face, variation);
    let mut builder = GlyphToPath::new();
    face.outline_glyph(GlyphId(glyph_id), &mut builder)?;
    Some(builder.into_path())
}

fn decode_raster_image_payload(image: RasterGlyphImage<'_>) -> Result<RawImage> {
    match image.format {
        RasterImageFormat::PNG => decode_png(image.data),
        RasterImageFormat::BitmapPremulBgra32 => decode_premul_bgra32(image),
        RasterImageFormat::BitmapGray8 => decode_gray8(image),
        RasterImageFormat::BitmapGray4 => decode_gray_subbyte(image, 4, false),
        RasterImageFormat::BitmapGray4Packed => decode_gray_subbyte(image, 4, true),
        RasterImageFormat::BitmapGray2 => decode_gray_subbyte(image, 2, false),
        RasterImageFormat::BitmapGray2Packed => decode_gray_subbyte(image, 2, true),
        RasterImageFormat::BitmapMono => decode_mono(image, false),
        RasterImageFormat::BitmapMonoPacked => decode_mono(image, true),
    }
}

fn decode_png(data: &[u8]) -> Result<RawImage> {
    if data.len() > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::UnsupportedFeature(format!(
            "color glyph PNG payload too large: {} bytes",
            data.len()
        )));
    }
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = decoder.read_info().map_err(|e| {
        OxideError::MalformedPdf(format!("color glyph PNG metadata decode failed: {e}"))
    })?;
    let size = reader.output_buffer_size();
    if size > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::UnsupportedFeature(format!(
            "color glyph PNG decoded payload too large: {size} bytes"
        )));
    }
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|e| OxideError::MalformedPdf(format!("color glyph PNG decode failed: {e}")))?;
    pixels.truncate(info.buffer_size());
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => {
            pixels = gray_alpha_to_rgba(&pixels);
            4
        }
        png::ColorType::Indexed => {
            return Err(OxideError::UnsupportedFeature(
                "color glyph indexed PNG did not expand to RGB/RGBA".to_string(),
            ))
        }
    };
    Ok(RawImage {
        width: info.width,
        height: info.height,
        channels,
        bits_per_sample: 8,
        pixels,
    })
}

fn decode_premul_bgra32(image: RasterGlyphImage<'_>) -> Result<RawImage> {
    let expected = usize::from(image.width)
        .saturating_mul(usize::from(image.height))
        .saturating_mul(4);
    if image.data.len() < expected || expected > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::MalformedPdf(format!(
            "color glyph BGRA payload length {} does not match {}x{}",
            image.data.len(),
            image.width,
            image.height
        )));
    }
    let mut pixels = Vec::with_capacity(expected);
    for chunk in image.data[..expected].chunks_exact(4) {
        let b = unpremultiply(chunk[0], chunk[3]);
        let g = unpremultiply(chunk[1], chunk[3]);
        let r = unpremultiply(chunk[2], chunk[3]);
        let a = chunk[3];
        pixels.extend_from_slice(&[r, g, b, a]);
    }
    Ok(RawImage {
        width: u32::from(image.width),
        height: u32::from(image.height),
        channels: 4,
        bits_per_sample: 8,
        pixels,
    })
}

fn decode_gray8(image: RasterGlyphImage<'_>) -> Result<RawImage> {
    let row_len = usize::from(image.width);
    let expected = row_len.saturating_mul(usize::from(image.height));
    if image.data.len() < expected || expected > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::MalformedPdf(
            "color glyph gray8 payload is truncated".to_string(),
        ));
    }
    Ok(RawImage {
        width: u32::from(image.width),
        height: u32::from(image.height),
        channels: 1,
        bits_per_sample: 8,
        pixels: image.data[..expected].to_vec(),
    })
}

fn decode_gray_subbyte(image: RasterGlyphImage<'_>, bits: u8, packed: bool) -> Result<RawImage> {
    let width = usize::from(image.width);
    let height = usize::from(image.height);
    let row_bits = width.saturating_mul(usize::from(bits));
    let row_bytes = row_bits.div_ceil(8);
    let expected = if packed {
        width
            .saturating_mul(height)
            .saturating_mul(usize::from(bits))
            .div_ceil(8)
    } else {
        row_bytes.saturating_mul(height)
    };
    if packed && row_bits % 8 != 0 {
        return Err(OxideError::UnsupportedFeature(
            "color glyph packed gray rows with non-byte-aligned width are unsupported".to_string(),
        ));
    }
    if image.data.len() < expected || expected > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::MalformedPdf(
            "color glyph gray payload is truncated".to_string(),
        ));
    }
    let mut pixels = Vec::with_capacity(width.saturating_mul(height));
    let mask = (1u8 << bits) - 1;
    let max = u16::from(mask);
    for row in 0..height {
        let start = row * row_bytes;
        for x in 0..width {
            let bit_index = x * usize::from(bits);
            let byte = image.data[start + bit_index / 8];
            let shift = 8 - usize::from(bits) - (bit_index % 8);
            let value = (byte >> shift) & mask;
            pixels.push(((u16::from(value) * 255) / max) as u8);
        }
    }
    Ok(RawImage {
        width: u32::from(image.width),
        height: u32::from(image.height),
        channels: 1,
        bits_per_sample: 8,
        pixels,
    })
}

fn decode_mono(image: RasterGlyphImage<'_>, packed: bool) -> Result<RawImage> {
    let width = usize::from(image.width);
    let height = usize::from(image.height);
    let row_bytes = width.div_ceil(8);
    let expected = if packed {
        width.saturating_mul(height).div_ceil(8)
    } else {
        row_bytes.saturating_mul(height)
    };
    if packed && width % 8 != 0 {
        return Err(OxideError::UnsupportedFeature(
            "color glyph packed mono rows with non-byte-aligned width are unsupported".to_string(),
        ));
    }
    if image.data.len() < expected || expected > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::MalformedPdf(
            "color glyph mono payload is truncated".to_string(),
        ));
    }
    let mut pixels = Vec::with_capacity(width.saturating_mul(height));
    for row in 0..height {
        let start = row * row_bytes;
        for x in 0..width {
            let byte = image.data[start + x / 8];
            let bit = 7 - (x % 8);
            pixels.push(if (byte >> bit) & 1 == 1 { 0 } else { 255 });
        }
    }
    Ok(RawImage {
        width: u32::from(image.width),
        height: u32::from(image.height),
        channels: 1,
        bits_per_sample: 8,
        pixels,
    })
}

fn gray_alpha_to_rgba(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() * 2);
    for chunk in data.chunks_exact(2) {
        out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
    }
    out
}

fn unpremultiply(value: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        0
    } else {
        ((u16::from(value) * 255 + u16::from(alpha) / 2) / u16::from(alpha)).min(255) as u8
    }
}

fn multiply_alpha(a: u8, b: u8) -> u8 {
    ((u16::from(a) * u16::from(b) + 127) / 255) as u8
}
