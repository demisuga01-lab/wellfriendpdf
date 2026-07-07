use std::io::Cursor;

use crate::error::{OxideError, Result};
use crate::fonts::variations::{self, VariationRequest};
use crate::images::decoder::RawImage;
use crate::render::buffer::{rgba, PixelColor};
use crate::render::font_rasterizer::GlyphToPath;
use crate::render::path::Path;
use crate::render::transform::Transform2D;
use ttf_parser::colr::{CompositeMode, Paint, Painter};
use ttf_parser::{GlyphId, RasterGlyphImage, RasterImageFormat, RgbaColor, Tag, Transform};

const MAX_COLOR_GLYPH_PIXELS: u32 = 4096 * 4096;
const MAX_COLOR_GLYPH_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLR_TRANSFORM_DEPTH: usize = 32;
const MAX_COLR_PAINT_LAYERS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorGlyphKind {
    None,
    ColrCpal,
    RasterBitmap,
    SvgBlocked,
    UnsupportedBitmapPayload,
}

impl ColorGlyphKind {
    pub(crate) fn cache_mode(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ColrCpal => 1,
            Self::RasterBitmap => 2,
            Self::SvgBlocked => 3,
            Self::UnsupportedBitmapPayload => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ColrLayer {
    pub glyph_id: u16,
    pub color: PixelColor,
    pub transform: Transform2D,
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
    if sbix_payload_kind(font_bytes, glyph_id).is_some() {
        return ColorGlyphKind::UnsupportedBitmapPayload;
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
) -> Result<Option<Vec<ColrLayer>>> {
    let mut face = ttf_parser::Face::parse(font_bytes, 0)
        .map_err(|_| OxideError::UnsupportedFeature("malformed COLR/CPAL font".to_string()))?;
    variations::apply_request(&mut face, variation);
    if !face.is_color_glyph(glyph_id) {
        return Ok(None);
    }

    let mut collector = SolidLayerCollector::new(graphics_alpha);
    let foreground = RgbaColor::new(foreground[0], foreground[1], foreground[2], foreground[3]);
    if face
        .paint_color_glyph(glyph_id, 0, foreground, &mut collector)
        .is_none()
    {
        return Ok(None);
    }
    if collector.unsupported {
        return Err(OxideError::UnsupportedFeature(format!(
            "COLR/CPAL paint graph contains unsupported operators for glyph {}: {}",
            glyph_id.0,
            collector.unsupported_ops.join(", ")
        )));
    }
    if collector.layers.is_empty() {
        return Ok(None);
    }
    Ok(Some(collector.layers))
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
        if let Some(kind) = sbix_payload_kind(font_bytes, glyph_id) {
            return Err(OxideError::UnsupportedFeature(format!(
                "sbix color glyph payload is not enabled for rendering: glyph={} payload={}",
                glyph_id.0,
                kind.label()
            )));
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ColorBitmapPayloadKind {
    Png,
    Jpeg,
    Tiff,
    Pdf,
    Mask,
    Dupe,
    Other(String),
}

impl ColorBitmapPayloadKind {
    pub(crate) fn label(&self) -> &str {
        match self {
            Self::Png => "sbix PNG",
            Self::Jpeg => "sbix JPEG",
            Self::Tiff => "sbix TIFF",
            Self::Pdf => "sbix PDF",
            Self::Mask => "sbix mask",
            Self::Dupe => "sbix duplicate glyph reference",
            Self::Other(tag) => tag.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SvgGlyphPolicy {
    StaticSubsetCandidate,
    UnsupportedStaticFeature(&'static str),
    BlockedSecurity(&'static str),
    PathLimitExceeded,
}

impl SvgGlyphPolicy {
    pub(crate) fn status(&self) -> &'static str {
        match self {
            Self::StaticSubsetCandidate => "static_subset_candidate",
            Self::UnsupportedStaticFeature(_) => "unsupported_static_feature",
            Self::BlockedSecurity(_) => "blocked_security_policy",
            Self::PathLimitExceeded => "blocked_path_limit",
        }
    }

    pub(crate) fn reason(&self) -> &'static str {
        match self {
            Self::StaticSubsetCandidate => "safe static SVG subset candidate",
            Self::UnsupportedStaticFeature(reason) | Self::BlockedSecurity(reason) => reason,
            Self::PathLimitExceeded => "path/depth limit exceeded",
        }
    }
}

pub(crate) fn classify_svg_glyph_document(svg: &str) -> SvgGlyphPolicy {
    const MAX_SVG_BYTES: usize = 256 * 1024;
    const MAX_PATH_COMMANDS: usize = 4096;
    const MAX_GROUP_DEPTH: isize = 64;
    if svg.len() > MAX_SVG_BYTES {
        return SvgGlyphPolicy::PathLimitExceeded;
    }
    let lower = svg.to_ascii_lowercase();
    let blocked_security = [
        ("<script", "script elements are blocked"),
        ("<foreignobject", "foreignObject is blocked"),
        ("<animate", "animation elements are blocked"),
        ("<animatetransform", "animation elements are blocked"),
        ("<set", "animation elements are blocked"),
        (" onload=", "event handler attributes are blocked"),
        (" onclick=", "event handler attributes are blocked"),
        (" onmouseover=", "event handler attributes are blocked"),
        ("javascript:", "javascript URLs are blocked"),
        ("file:", "file URLs are blocked"),
        ("http://", "network URLs are blocked"),
        ("https://", "network URLs are blocked"),
        ("@import", "CSS imports are blocked"),
        ("<font", "remote or embedded SVG fonts are blocked"),
        ("<image", "external image resources are blocked"),
        ("<filter", "SVG filters are blocked"),
        ("<mask", "SVG masks are blocked"),
    ];
    for (needle, reason) in blocked_security {
        if lower.contains(needle) {
            return SvgGlyphPolicy::BlockedSecurity(reason);
        }
    }
    let unsupported_static = [
        ("<text", "text elements require font/layout execution"),
        ("<use", "recursive references are unsupported"),
        ("url(", "paint server URL references are unsupported"),
        ("<pattern", "SVG pattern paint servers are unsupported"),
    ];
    for (needle, reason) in unsupported_static {
        if lower.contains(needle) {
            return SvgGlyphPolicy::UnsupportedStaticFeature(reason);
        }
    }
    let mut depth = 0isize;
    for token in lower.match_indices('<').map(|(idx, _)| &lower[idx..]) {
        if token.starts_with("<g") || token.starts_with("<svg") {
            depth += 1;
        } else if token.starts_with("</g") || token.starts_with("</svg") {
            depth -= 1;
        }
        if !(-1..=MAX_GROUP_DEPTH).contains(&depth) {
            return SvgGlyphPolicy::PathLimitExceeded;
        }
    }
    let path_commands = lower
        .bytes()
        .filter(|byte| {
            matches!(
                *byte,
                b'm' | b'l' | b'h' | b'v' | b'c' | b's' | b'q' | b't' | b'a' | b'z'
            )
        })
        .count();
    if path_commands > MAX_PATH_COMMANDS {
        return SvgGlyphPolicy::PathLimitExceeded;
    }
    SvgGlyphPolicy::StaticSubsetCandidate
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

    pub(crate) fn supports_colr_cpal_v1_subset(&self) -> bool {
        self.colr_version.is_some_and(|version| version > 0) && self.has_cpal
    }
}

pub(crate) fn sbix_payload_kind(
    font_bytes: &[u8],
    glyph_id: GlyphId,
) -> Option<ColorBitmapPayloadKind> {
    let face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    let raw = face.raw_face();
    let sbix = raw.table(Tag::from_bytes(b"sbix"))?;
    sbix_payload_kind_inner(sbix, face.number_of_glyphs(), glyph_id, 0)
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
    current_transform: Transform2D,
    transform_stack: Vec<Transform2D>,
    layer_depth: usize,
    layers: Vec<ColrLayer>,
    graphics_alpha: u8,
    unsupported: bool,
    unsupported_ops: Vec<&'static str>,
}

impl SolidLayerCollector {
    fn new(graphics_alpha: u8) -> Self {
        Self {
            current_glyph: None,
            current_transform: Transform2D::identity(),
            transform_stack: Vec::new(),
            layer_depth: 0,
            layers: Vec::new(),
            graphics_alpha,
            unsupported: false,
            unsupported_ops: Vec::new(),
        }
    }

    fn mark_unsupported(&mut self, op: &'static str) {
        self.unsupported = true;
        if !self.unsupported_ops.contains(&op) {
            self.unsupported_ops.push(op);
        }
    }

    fn push_transform2d(&mut self, op: &'static str, transform: Transform2D) {
        if self.transform_stack.len() >= MAX_COLR_TRANSFORM_DEPTH || !finite_transform(transform) {
            self.mark_unsupported(op);
            return;
        }
        self.transform_stack.push(self.current_transform);
        self.current_transform = self.current_transform.concat(&transform);
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
                        transform: self.current_transform,
                    });
                    if self.layers.len() > MAX_COLR_PAINT_LAYERS {
                        self.mark_unsupported("Paint layer count cap exceeded");
                    }
                }
            }
            Paint::LinearGradient(_) => self.mark_unsupported("PaintLinearGradient"),
            Paint::RadialGradient(_) => self.mark_unsupported("PaintRadialGradient"),
            Paint::SweepGradient(_) => self.mark_unsupported("PaintSweepGradient"),
        }
    }

    fn push_clip(&mut self) {
        self.mark_unsupported("PaintClip");
    }

    fn push_clip_box(&mut self, _clipbox: ttf_parser::colr::ClipBox) {
        self.mark_unsupported("PaintClipBox");
    }

    fn pop_clip(&mut self) {}

    fn push_layer(&mut self, mode: CompositeMode) {
        if mode != CompositeMode::SourceOver {
            self.mark_unsupported("PaintComposite");
            return;
        }
        self.layer_depth = self.layer_depth.saturating_add(1);
        if self.layer_depth > MAX_COLR_TRANSFORM_DEPTH {
            self.mark_unsupported("PaintComposite depth cap");
        }
    }

    fn pop_layer(&mut self) {
        self.layer_depth = self.layer_depth.saturating_sub(1);
    }

    fn push_translate(&mut self, _tx: f32, _ty: f32) {
        self.push_transform2d(
            "PaintTranslate",
            Transform2D::translation(f64::from(_tx), f64::from(_ty)),
        );
    }

    fn push_scale(&mut self, _sx: f32, _sy: f32) {
        self.push_transform2d(
            "PaintScale",
            Transform2D::scale(f64::from(_sx), f64::from(_sy)),
        );
    }

    fn push_rotate(&mut self, _angle: f32) {
        self.push_transform2d(
            "PaintRotate",
            Transform2D::rotation(f64::from(_angle) * std::f64::consts::PI),
        );
    }

    fn push_skew(&mut self, _skew_x: f32, _skew_y: f32) {
        self.push_transform2d(
            "PaintSkew",
            Transform2D::shear(
                (f64::from(-_skew_x) * std::f64::consts::PI).tan(),
                (f64::from(_skew_y) * std::f64::consts::PI).tan(),
            ),
        );
    }

    fn push_transform(&mut self, _transform: Transform) {
        self.push_transform2d(
            "PaintTransform",
            Transform2D::new(
                f64::from(_transform.a),
                f64::from(_transform.b),
                f64::from(_transform.c),
                f64::from(_transform.d),
                f64::from(_transform.e),
                f64::from(_transform.f),
            ),
        );
    }

    fn pop_transform(&mut self) {
        if let Some(transform) = self.transform_stack.pop() {
            self.current_transform = transform;
        }
    }
}

fn finite_transform(transform: Transform2D) -> bool {
    transform.a.is_finite()
        && transform.b.is_finite()
        && transform.c.is_finite()
        && transform.d.is_finite()
        && transform.e.is_finite()
        && transform.f.is_finite()
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

fn sbix_payload_kind_inner(
    sbix: &[u8],
    glyph_count: u16,
    glyph_id: GlyphId,
    depth: u8,
) -> Option<ColorBitmapPayloadKind> {
    if depth >= 10 || glyph_id.0 >= glyph_count {
        return None;
    }
    let strike_count = read_u32(sbix, 4)? as usize;
    let strike_offsets_start = 8usize;
    for strike_index in 0..strike_count {
        let strike_offset = read_u32(sbix, strike_offsets_start + strike_index * 4)? as usize;
        let offsets_start = strike_offset.checked_add(4)?;
        let glyph_offset_index = offsets_start.checked_add(usize::from(glyph_id.0) * 4)?;
        let start = read_u32(sbix, glyph_offset_index)? as usize;
        let end = read_u32(sbix, glyph_offset_index.checked_add(4)?)? as usize;
        if start == end {
            continue;
        }
        if end <= start || end.checked_sub(start)? < 8 {
            return Some(ColorBitmapPayloadKind::Other(
                "sbix malformed strike record".to_string(),
            ));
        }
        let record_start = strike_offset.checked_add(start)?;
        let tag_bytes = sbix.get(record_start.checked_add(4)?..record_start.checked_add(8)?)?;
        match tag_bytes {
            b"png " => return Some(ColorBitmapPayloadKind::Png),
            b"jpg " | b"jpeg" => return Some(ColorBitmapPayloadKind::Jpeg),
            b"tiff" | b"tif " => return Some(ColorBitmapPayloadKind::Tiff),
            b"pdf " => return Some(ColorBitmapPayloadKind::Pdf),
            b"mask" => return Some(ColorBitmapPayloadKind::Mask),
            b"dupe" => {
                let payload_start = record_start.checked_add(8)?;
                let dupe_gid = read_u16(sbix, payload_start)?;
                return sbix_payload_kind_inner(sbix, glyph_count, GlyphId(dupe_gid), depth + 1)
                    .or(Some(ColorBitmapPayloadKind::Dupe));
            }
            other => {
                return Some(ColorBitmapPayloadKind::Other(
                    String::from_utf8_lossy(other).trim().to_string(),
                ));
            }
        }
    }
    None
}

fn read_u16(data: &[u8], offset: usize) -> Option<u16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(data: &[u8], offset: usize) -> Option<u32> {
    let bytes = data.get(offset..offset.checked_add(4)?)?;
    Some(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
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
