use std::io::{Cursor, Read};

use crate::error::{OxideError, Result};
use crate::fonts::variations::{self, VariationRequest};
use crate::images::decoder::{ColorSpaceConverter, ImageDecoder, RawImage};
use crate::render::buffer::{rgba, PixelColor};
use crate::render::font_rasterizer::GlyphToPath;
use crate::render::path::Path;
use crate::render::transform::Transform2D;
use flate2::read::GzDecoder;
use ttf_parser::colr::{CompositeMode, Paint, Painter};
use ttf_parser::{GlyphId, RasterGlyphImage, RasterImageFormat, RgbaColor, Tag, Transform};

const MAX_COLOR_GLYPH_PIXELS: u32 = 4096 * 4096;
const MAX_COLOR_GLYPH_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLR_TRANSFORM_DEPTH: usize = 32;
const MAX_COLR_PAINT_LAYERS: usize = 256;
const MAX_SVG_BYTES: usize = 256 * 1024;
const MAX_SVG_PAINT_PATHS: usize = 512;
const MAX_SVG_PATH_COMMANDS: usize = 4096;
const MAX_SVG_GROUP_DEPTH: usize = 64;

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

#[derive(Debug, Clone)]
pub(crate) struct SvgPaintPath {
    pub path: Path,
    pub fill: Option<PixelColor>,
    pub stroke: Option<PixelColor>,
    pub stroke_width: f64,
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
        if let Some(payload) = sbix_payload(font_bytes, glyph_id, target_ppem.max(1)) {
            let image = decode_sbix_payload(&payload)?;
            return Ok(Some(DecodedRasterGlyph {
                x: payload.x,
                y: payload.y,
                pixels_per_em: payload.pixels_per_em.max(1),
                image,
            }));
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
        ("<style", "CSS style blocks are blocked"),
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
        (
            "<clippath",
            "SVG clipPath requires paint-server references and is unsupported",
        ),
        (
            "<lineargradient",
            "SVG gradient paint servers are unsupported in the static subset",
        ),
        (
            "<radialgradient",
            "SVG gradient paint servers are unsupported in the static subset",
        ),
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
        if !(-1..=MAX_SVG_GROUP_DEPTH as isize).contains(&depth) {
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
    if path_commands > MAX_SVG_PATH_COMMANDS {
        return SvgGlyphPolicy::PathLimitExceeded;
    }
    SvgGlyphPolicy::StaticSubsetCandidate
}

pub(crate) fn svg_static_glyph_paints(
    font_bytes: &[u8],
    glyph_id: GlyphId,
    foreground: PixelColor,
    graphics_alpha: u8,
) -> Result<Option<Vec<SvgPaintPath>>> {
    let face = match ttf_parser::Face::parse(font_bytes, 0) {
        Ok(face) => face,
        Err(_) => return Ok(None),
    };
    let Some(document) = face.glyph_svg_image(glyph_id) else {
        return Ok(None);
    };
    let svg = decode_svg_document(document.data)?;
    let policy = classify_svg_glyph_document(&svg);
    if policy != SvgGlyphPolicy::StaticSubsetCandidate {
        return Err(OxideError::UnsupportedFeature(format!(
            "SVG-in-OpenType color glyph blocked: glyph={} status={} reason={}",
            glyph_id.0,
            policy.status(),
            policy.reason()
        )));
    }
    let paths = parse_static_svg_paint_paths(&svg, foreground, graphics_alpha)?;
    if paths.is_empty() {
        return Err(OxideError::UnsupportedFeature(format!(
            "SVG-in-OpenType color glyph produced no supported static paint paths: glyph={}",
            glyph_id.0
        )));
    }
    Ok(Some(paths))
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
    sbix_payload_inner(sbix, face.number_of_glyphs(), glyph_id, 0, 0).map(|payload| payload.kind)
}

#[derive(Debug, Clone)]
pub(crate) struct DecodedRasterGlyph {
    pub x: i16,
    pub y: i16,
    pub pixels_per_em: u16,
    pub image: RawImage,
}

#[derive(Debug, Clone)]
struct SbixPayload<'a> {
    kind: ColorBitmapPayloadKind,
    data: &'a [u8],
    x: i16,
    y: i16,
    pixels_per_em: u16,
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

#[derive(Clone, Debug)]
struct SvgState {
    fill: Option<PixelColor>,
    stroke: Option<PixelColor>,
    stroke_width: f64,
    opacity: f64,
    fill_opacity: f64,
    stroke_opacity: f64,
    transform: Transform2D,
}

impl SvgState {
    fn root(foreground: PixelColor) -> Self {
        Self {
            fill: Some(foreground),
            stroke: None,
            stroke_width: 1.0,
            opacity: 1.0,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            transform: Transform2D::identity(),
        }
    }

    fn fill_color(&self, graphics_alpha: u8) -> Option<PixelColor> {
        self.fill
            .map(|color| color_with_alpha(color, self.opacity * self.fill_opacity, graphics_alpha))
    }

    fn stroke_color(&self, graphics_alpha: u8) -> Option<PixelColor> {
        self.stroke.map(|color| {
            color_with_alpha(color, self.opacity * self.stroke_opacity, graphics_alpha)
        })
    }
}

fn color_with_alpha(mut color: PixelColor, opacity: f64, graphics_alpha: u8) -> PixelColor {
    let opacity = opacity.clamp(0.0, 1.0);
    let alpha = f64::from(color[3]) * opacity * (f64::from(graphics_alpha) / 255.0);
    color[3] = alpha.round().clamp(0.0, 255.0) as u8;
    color
}

fn decode_svg_document(data: &[u8]) -> Result<String> {
    if data.len() > MAX_SVG_BYTES {
        return Err(OxideError::UnsupportedFeature(format!(
            "SVG-in-OpenType glyph document too large: {} bytes",
            data.len()
        )));
    }
    let mut bytes = Vec::new();
    if data.starts_with(&[0x1f, 0x8b]) {
        let decoder = GzDecoder::new(Cursor::new(data));
        decoder
            .take((MAX_SVG_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| {
                OxideError::MalformedPdf(format!("SVG-in-OpenType gzip decode failed: {e}"))
            })?;
        if bytes.len() > MAX_SVG_BYTES {
            return Err(OxideError::UnsupportedFeature(
                "SVG-in-OpenType decompressed document exceeds static subset cap".to_string(),
            ));
        }
    } else {
        bytes.extend_from_slice(data);
    }
    String::from_utf8(bytes).map_err(|e| {
        OxideError::MalformedPdf(format!("SVG-in-OpenType document is not UTF-8: {e}"))
    })
}

fn parse_static_svg_paint_paths(
    svg: &str,
    foreground: PixelColor,
    graphics_alpha: u8,
) -> Result<Vec<SvgPaintPath>> {
    let mut states = vec![SvgState::root(foreground)];
    let mut paths = Vec::new();
    let mut cursor = 0usize;
    while let Some(rel_start) = svg[cursor..].find('<') {
        let start = cursor + rel_start;
        let Some(rel_end) = svg[start..].find('>') else {
            return Err(OxideError::MalformedPdf(
                "SVG-in-OpenType static subset tag is unterminated".to_string(),
            ));
        };
        let end = start + rel_end;
        let raw_tag = svg[start + 1..end].trim();
        cursor = end + 1;
        if raw_tag.is_empty()
            || raw_tag.starts_with('!')
            || raw_tag.starts_with('?')
            || raw_tag.starts_with("!--")
        {
            continue;
        }
        if let Some(stripped) = raw_tag.strip_prefix('/') {
            let name = svg_tag_name(stripped);
            if matches!(name, "svg" | "g") && states.len() > 1 {
                states.pop();
            }
            continue;
        }

        let self_closing = raw_tag.ends_with('/');
        let tag_body = raw_tag.trim_end_matches('/').trim();
        let name = svg_tag_name(tag_body);
        let attrs = parse_svg_attrs(tag_body.get(name.len()..).unwrap_or_default())?;
        let state = apply_svg_attrs(states.last().expect("root state"), &attrs, foreground)?;

        match name {
            "svg" | "g" => {
                if !self_closing {
                    if states.len() >= MAX_SVG_GROUP_DEPTH {
                        return Err(OxideError::UnsupportedFeature(
                            "SVG-in-OpenType group depth cap exceeded".to_string(),
                        ));
                    }
                    states.push(state);
                }
            }
            "path" => {
                let d = attr(&attrs, "d").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG path element missing d attribute".to_string())
                })?;
                let path = parse_svg_path_data(d)?;
                push_svg_paint_path(&mut paths, path, &state, graphics_alpha)?;
            }
            "rect" => {
                let x = parse_attr_number(&attrs, "x").unwrap_or(0.0);
                let y = parse_attr_number(&attrs, "y").unwrap_or(0.0);
                let w = parse_attr_number(&attrs, "width").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG rect missing width".to_string())
                })?;
                let h = parse_attr_number(&attrs, "height").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG rect missing height".to_string())
                })?;
                if !(w.is_finite() && h.is_finite()) || w < 0.0 || h < 0.0 {
                    return Err(OxideError::MalformedPdf(
                        "SVG rect has invalid dimensions".to_string(),
                    ));
                }
                let mut path = Path::new();
                path.rect(x, y, w, h);
                push_svg_paint_path(&mut paths, path, &state, graphics_alpha)?;
            }
            "circle" => {
                let cx = parse_attr_number(&attrs, "cx").unwrap_or(0.0);
                let cy = parse_attr_number(&attrs, "cy").unwrap_or(0.0);
                let r = parse_attr_number(&attrs, "r")
                    .ok_or_else(|| OxideError::MalformedPdf("SVG circle missing r".to_string()))?;
                push_svg_paint_path(
                    &mut paths,
                    ellipse_path(cx, cy, r, r)?,
                    &state,
                    graphics_alpha,
                )?;
            }
            "ellipse" => {
                let cx = parse_attr_number(&attrs, "cx").unwrap_or(0.0);
                let cy = parse_attr_number(&attrs, "cy").unwrap_or(0.0);
                let rx = parse_attr_number(&attrs, "rx").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG ellipse missing rx".to_string())
                })?;
                let ry = parse_attr_number(&attrs, "ry").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG ellipse missing ry".to_string())
                })?;
                push_svg_paint_path(
                    &mut paths,
                    ellipse_path(cx, cy, rx, ry)?,
                    &state,
                    graphics_alpha,
                )?;
            }
            "line" => {
                let x1 = parse_attr_number(&attrs, "x1").unwrap_or(0.0);
                let y1 = parse_attr_number(&attrs, "y1").unwrap_or(0.0);
                let x2 = parse_attr_number(&attrs, "x2").unwrap_or(0.0);
                let y2 = parse_attr_number(&attrs, "y2").unwrap_or(0.0);
                let mut path = Path::new();
                path.move_to(x1, y1);
                path.line_to(x2, y2);
                push_svg_paint_path(&mut paths, path, &state, graphics_alpha)?;
            }
            "polyline" | "polygon" => {
                let points = attr(&attrs, "points").ok_or_else(|| {
                    OxideError::MalformedPdf("SVG polyline/polygon missing points".to_string())
                })?;
                let mut path = points_path(points)?;
                if name == "polygon" {
                    path.close();
                }
                push_svg_paint_path(&mut paths, path, &state, graphics_alpha)?;
            }
            other => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "SVG-in-OpenType static subset element unsupported: <{other}>"
                )));
            }
        }
        if paths.len() > MAX_SVG_PAINT_PATHS {
            return Err(OxideError::UnsupportedFeature(
                "SVG-in-OpenType paint path cap exceeded".to_string(),
            ));
        }
    }
    Ok(paths)
}

fn push_svg_paint_path(
    paths: &mut Vec<SvgPaintPath>,
    path: Path,
    state: &SvgState,
    graphics_alpha: u8,
) -> Result<()> {
    if path.is_empty() {
        return Ok(());
    }
    if !finite_transform(state.transform) || !state.stroke_width.is_finite() {
        return Err(OxideError::UnsupportedFeature(
            "SVG-in-OpenType static subset contains non-finite transform or stroke width"
                .to_string(),
        ));
    }
    let fill = state.fill_color(graphics_alpha);
    let stroke = state.stroke_color(graphics_alpha);
    if fill.is_none() && stroke.is_none() {
        return Ok(());
    }
    paths.push(SvgPaintPath {
        path,
        fill,
        stroke,
        stroke_width: state.stroke_width.max(0.0),
        transform: state.transform,
    });
    Ok(())
}

fn svg_tag_name(tag_body: &str) -> &str {
    tag_body
        .split(|ch: char| ch.is_ascii_whitespace() || ch == '/')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches('/')
}

fn parse_svg_attrs(data: &str) -> Result<Vec<(String, String)>> {
    let bytes = data.as_bytes();
    let mut idx = 0usize;
    let mut attrs = Vec::new();
    while idx < bytes.len() {
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() {
            break;
        }
        let key_start = idx;
        while idx < bytes.len()
            && !bytes[idx].is_ascii_whitespace()
            && bytes[idx] != b'='
            && bytes[idx] != b'/'
        {
            idx += 1;
        }
        if key_start == idx {
            idx += 1;
            continue;
        }
        let key = data[key_start..idx].to_ascii_lowercase();
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() || bytes[idx] != b'=' {
            return Err(OxideError::UnsupportedFeature(format!(
                "SVG-in-OpenType static subset requires quoted attributes: {key}"
            )));
        }
        idx += 1;
        while idx < bytes.len() && bytes[idx].is_ascii_whitespace() {
            idx += 1;
        }
        if idx >= bytes.len() || (bytes[idx] != b'"' && bytes[idx] != b'\'') {
            return Err(OxideError::UnsupportedFeature(format!(
                "SVG-in-OpenType static subset requires quoted attribute values: {key}"
            )));
        }
        let quote = bytes[idx];
        idx += 1;
        let value_start = idx;
        while idx < bytes.len() && bytes[idx] != quote {
            idx += 1;
        }
        if idx >= bytes.len() {
            return Err(OxideError::MalformedPdf(format!(
                "SVG-in-OpenType attribute is unterminated: {key}"
            )));
        }
        attrs.push((key, decode_xml_entities(&data[value_start..idx])));
        idx += 1;
    }
    Ok(attrs)
}

fn decode_xml_entities(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

fn apply_svg_attrs(
    base: &SvgState,
    attrs: &[(String, String)],
    foreground: PixelColor,
) -> Result<SvgState> {
    let mut state = base.clone();
    if let Some(style) = attr(attrs, "style") {
        for item in style.split(';') {
            let Some((key, value)) = item.split_once(':') else {
                continue;
            };
            apply_svg_paint_attr(&mut state, key.trim(), value.trim(), foreground)?;
        }
    }
    for (key, value) in attrs {
        apply_svg_paint_attr(&mut state, key, value, foreground)?;
    }
    if let Some(transform) = attr(attrs, "transform") {
        let local = parse_svg_transform(transform)?;
        state.transform = local.concat(&base.transform);
    }
    Ok(state)
}

fn apply_svg_paint_attr(
    state: &mut SvgState,
    key: &str,
    value: &str,
    foreground: PixelColor,
) -> Result<()> {
    match key {
        "fill" => state.fill = parse_svg_color(value, foreground)?,
        "stroke" => state.stroke = parse_svg_color(value, foreground)?,
        "stroke-width" => state.stroke_width = parse_svg_number(value)?.max(0.0),
        "opacity" => state.opacity = parse_unit_interval(value)?,
        "fill-opacity" => state.fill_opacity = parse_unit_interval(value)?,
        "stroke-opacity" => state.stroke_opacity = parse_unit_interval(value)?,
        "transform" | "d" | "x" | "y" | "width" | "height" | "cx" | "cy" | "r" | "rx" | "ry"
        | "x1" | "y1" | "x2" | "y2" | "points" | "viewbox" | "version" | "xmlns" | "style" => {}
        "stroke-linecap" | "stroke-linejoin" | "stroke-miterlimit" => {}
        other if other.starts_with("on") => {
            return Err(OxideError::UnsupportedFeature(format!(
                "SVG-in-OpenType event attribute blocked: {other}"
            )));
        }
        other => {
            return Err(OxideError::UnsupportedFeature(format!(
                "SVG-in-OpenType static subset attribute unsupported: {other}"
            )));
        }
    }
    Ok(())
}

fn attr<'a>(attrs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn parse_attr_number(attrs: &[(String, String)], name: &str) -> Option<f64> {
    attr(attrs, name).and_then(|value| parse_svg_number(value).ok())
}

fn parse_svg_color(value: &str, foreground: PixelColor) -> Result<Option<PixelColor>> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if value.eq_ignore_ascii_case("currentcolor") || value.eq_ignore_ascii_case("context-fill") {
        return Ok(Some(foreground));
    }
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex_color(hex).map(Some);
    }
    let lower = value.to_ascii_lowercase();
    let named = match lower.as_str() {
        "black" => Some(rgba(0, 0, 0, 255)),
        "white" => Some(rgba(255, 255, 255, 255)),
        "red" => Some(rgba(255, 0, 0, 255)),
        "green" => Some(rgba(0, 128, 0, 255)),
        "blue" => Some(rgba(0, 0, 255, 255)),
        "yellow" => Some(rgba(255, 255, 0, 255)),
        "cyan" => Some(rgba(0, 255, 255, 255)),
        "magenta" => Some(rgba(255, 0, 255, 255)),
        "orange" => Some(rgba(255, 165, 0, 255)),
        "purple" => Some(rgba(128, 0, 128, 255)),
        _ => None,
    };
    if named.is_some() {
        return Ok(named);
    }
    if lower.starts_with("rgb(") && lower.ends_with(')') {
        let body = &lower[4..lower.len() - 1];
        let parts: Vec<_> = body
            .split([',', ' '])
            .filter(|part| !part.trim().is_empty())
            .collect();
        if parts.len() == 3 {
            let r = parse_color_component(parts[0])?;
            let g = parse_color_component(parts[1])?;
            let b = parse_color_component(parts[2])?;
            return Ok(Some(rgba(r, g, b, 255)));
        }
    }
    Err(OxideError::UnsupportedFeature(format!(
        "SVG-in-OpenType static subset color unsupported: {value}"
    )))
}

fn parse_hex_color(hex: &str) -> Result<PixelColor> {
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).map_err(svg_color_err)?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).map_err(svg_color_err)?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).map_err(svg_color_err)?;
            Ok(rgba(r, g, b, 255))
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(svg_color_err)?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(svg_color_err)?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(svg_color_err)?;
            Ok(rgba(r, g, b, 255))
        }
        _ => Err(OxideError::UnsupportedFeature(format!(
            "SVG-in-OpenType static subset hex color unsupported: #{hex}"
        ))),
    }
}

fn svg_color_err(err: std::num::ParseIntError) -> OxideError {
    OxideError::MalformedPdf(format!("SVG-in-OpenType color parse failed: {err}"))
}

fn parse_color_component(value: &str) -> Result<u8> {
    let value = value.trim();
    if let Some(percent) = value.strip_suffix('%') {
        let pct = parse_svg_number(percent)?;
        Ok(((pct.clamp(0.0, 100.0) / 100.0) * 255.0).round() as u8)
    } else {
        Ok(parse_svg_number(value)?.round().clamp(0.0, 255.0) as u8)
    }
}

fn parse_unit_interval(value: &str) -> Result<f64> {
    Ok(parse_svg_number(value)?.clamp(0.0, 1.0))
}

fn parse_svg_number(value: &str) -> Result<f64> {
    let value = value
        .trim()
        .trim_end_matches("px")
        .trim_end_matches("pt")
        .trim_end_matches("em");
    let parsed = value.parse::<f64>().map_err(|e| {
        OxideError::MalformedPdf(format!("SVG-in-OpenType number parse failed: {e}"))
    })?;
    if parsed.is_finite() {
        Ok(parsed)
    } else {
        Err(OxideError::UnsupportedFeature(
            "SVG-in-OpenType non-finite number blocked".to_string(),
        ))
    }
}

fn parse_svg_transform(value: &str) -> Result<Transform2D> {
    let mut current = Transform2D::identity();
    let mut rest = value.trim();
    while !rest.is_empty() {
        let Some(open) = rest.find('(') else {
            return Err(OxideError::MalformedPdf(format!(
                "SVG transform missing '(': {rest}"
            )));
        };
        let name = rest[..open].trim().to_ascii_lowercase();
        let Some(close_rel) = rest[open + 1..].find(')') else {
            return Err(OxideError::MalformedPdf(format!(
                "SVG transform missing ')': {rest}"
            )));
        };
        let close = open + 1 + close_rel;
        let args = parse_number_list(&rest[open + 1..close])?;
        let next = match name.as_str() {
            "matrix" if args.len() == 6 => {
                Transform2D::new(args[0], args[1], args[2], args[3], args[4], args[5])
            }
            "translate" if !args.is_empty() && args.len() <= 2 => {
                Transform2D::translation(args[0], args.get(1).copied().unwrap_or(0.0))
            }
            "scale" if !args.is_empty() && args.len() <= 2 => {
                Transform2D::scale(args[0], args.get(1).copied().unwrap_or(args[0]))
            }
            "rotate" if !args.is_empty() && args.len() <= 3 => {
                let rotate = Transform2D::rotation(args[0].to_radians());
                if args.len() == 3 {
                    Transform2D::translation(args[1], args[2])
                        .concat(&rotate)
                        .concat(&Transform2D::translation(-args[1], -args[2]))
                } else {
                    rotate
                }
            }
            "skewx" if args.len() == 1 => Transform2D::shear(args[0].to_radians().tan(), 0.0),
            "skewy" if args.len() == 1 => Transform2D::shear(0.0, args[0].to_radians().tan()),
            _ => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "SVG-in-OpenType transform unsupported or malformed: {name}"
                )))
            }
        };
        if !finite_transform(next) {
            return Err(OxideError::UnsupportedFeature(
                "SVG-in-OpenType non-finite transform blocked".to_string(),
            ));
        }
        current = next.concat(&current);
        rest =
            rest[close + 1..].trim_start_matches(|ch: char| ch.is_ascii_whitespace() || ch == ',');
    }
    Ok(current)
}

fn parse_number_list(value: &str) -> Result<Vec<f64>> {
    value
        .split([',', ' ', '\t', '\r', '\n'])
        .filter(|part| !part.trim().is_empty())
        .map(parse_svg_number)
        .collect()
}

fn ellipse_path(cx: f64, cy: f64, rx: f64, ry: f64) -> Result<Path> {
    if !(cx.is_finite() && cy.is_finite() && rx.is_finite() && ry.is_finite())
        || rx < 0.0
        || ry < 0.0
    {
        return Err(OxideError::MalformedPdf(
            "SVG ellipse has invalid geometry".to_string(),
        ));
    }
    let k = 0.552_284_749_830_793_6;
    let mut path = Path::new();
    path.move_to(cx + rx, cy);
    path.curve_to(cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry);
    path.curve_to(cx - k * rx, cy + ry, cx - rx, cy + k * ry, cx - rx, cy);
    path.curve_to(cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, cy - ry);
    path.curve_to(cx + k * rx, cy - ry, cx + rx, cy - k * ry, cx + rx, cy);
    path.close();
    Ok(path)
}

fn points_path(points: &str) -> Result<Path> {
    let numbers = parse_number_list(points)?;
    if numbers.len() < 4 || numbers.len() % 2 != 0 {
        return Err(OxideError::MalformedPdf(
            "SVG points list must contain x/y pairs".to_string(),
        ));
    }
    let mut path = Path::new();
    path.move_to(numbers[0], numbers[1]);
    for pair in numbers[2..].chunks_exact(2) {
        path.line_to(pair[0], pair[1]);
    }
    Ok(path)
}

#[derive(Clone, Debug)]
struct SvgPathParser<'a> {
    data: &'a [u8],
    idx: usize,
}

impl<'a> SvgPathParser<'a> {
    fn new(data: &'a str) -> Self {
        Self {
            data: data.as_bytes(),
            idx: 0,
        }
    }

    fn eof(&self) -> bool {
        self.idx >= self.data.len()
    }

    fn skip_sep(&mut self) {
        while !self.eof()
            && (self.data[self.idx].is_ascii_whitespace() || self.data[self.idx] == b',')
        {
            self.idx += 1;
        }
    }

    fn next_command(&mut self) -> Option<char> {
        self.skip_sep();
        if self.eof() {
            return None;
        }
        let byte = self.data[self.idx];
        if byte.is_ascii_alphabetic() {
            self.idx += 1;
            Some(byte as char)
        } else {
            None
        }
    }

    fn has_number(&mut self) -> bool {
        self.skip_sep();
        !self.eof() && matches!(self.data[self.idx], b'+' | b'-' | b'.' | b'0'..=b'9')
    }

    fn number(&mut self) -> Result<f64> {
        self.skip_sep();
        let start = self.idx;
        if !self.eof() && matches!(self.data[self.idx], b'+' | b'-') {
            self.idx += 1;
        }
        while !self.eof() && self.data[self.idx].is_ascii_digit() {
            self.idx += 1;
        }
        if !self.eof() && self.data[self.idx] == b'.' {
            self.idx += 1;
            while !self.eof() && self.data[self.idx].is_ascii_digit() {
                self.idx += 1;
            }
        }
        if !self.eof() && matches!(self.data[self.idx], b'e' | b'E') {
            self.idx += 1;
            if !self.eof() && matches!(self.data[self.idx], b'+' | b'-') {
                self.idx += 1;
            }
            while !self.eof() && self.data[self.idx].is_ascii_digit() {
                self.idx += 1;
            }
        }
        if start == self.idx {
            return Err(OxideError::MalformedPdf(
                "SVG path expected number".to_string(),
            ));
        }
        let s = std::str::from_utf8(&self.data[start..self.idx])
            .map_err(|e| OxideError::MalformedPdf(format!("SVG path number is not UTF-8: {e}")))?;
        parse_svg_number(s)
    }
}

fn parse_svg_path_data(data: &str) -> Result<Path> {
    let mut parser = SvgPathParser::new(data);
    let mut path = Path::new();
    let mut command = ' ';
    let mut current = (0.0, 0.0);
    let mut subpath_start = (0.0, 0.0);
    let mut commands = 0usize;

    while !parser.eof() {
        if let Some(next) = parser.next_command() {
            command = next;
        } else if command == ' ' {
            return Err(OxideError::MalformedPdf(
                "SVG path data starts without command".to_string(),
            ));
        }
        commands += 1;
        if commands > MAX_SVG_PATH_COMMANDS {
            return Err(OxideError::UnsupportedFeature(
                "SVG-in-OpenType path command cap exceeded".to_string(),
            ));
        }

        let relative = command.is_ascii_lowercase();
        match command.to_ascii_uppercase() {
            'M' => {
                let mut first = true;
                while parser.has_number() {
                    let (x, y) = svg_point(&mut parser, current, relative)?;
                    if first {
                        path.move_to(x, y);
                        subpath_start = (x, y);
                        first = false;
                    } else {
                        path.line_to(x, y);
                    }
                    current = (x, y);
                }
                command = if relative { 'l' } else { 'L' };
            }
            'L' => {
                while parser.has_number() {
                    let (x, y) = svg_point(&mut parser, current, relative)?;
                    path.line_to(x, y);
                    current = (x, y);
                }
            }
            'H' => {
                while parser.has_number() {
                    let mut x = parser.number()?;
                    if relative {
                        x += current.0;
                    }
                    path.line_to(x, current.1);
                    current.0 = x;
                }
            }
            'V' => {
                while parser.has_number() {
                    let mut y = parser.number()?;
                    if relative {
                        y += current.1;
                    }
                    path.line_to(current.0, y);
                    current.1 = y;
                }
            }
            'C' => {
                while parser.has_number() {
                    let (x1, y1) = svg_point(&mut parser, current, relative)?;
                    let (x2, y2) = svg_point(&mut parser, current, relative)?;
                    let (x, y) = svg_point(&mut parser, current, relative)?;
                    path.curve_to(x1, y1, x2, y2, x, y);
                    current = (x, y);
                }
            }
            'Q' => {
                while parser.has_number() {
                    let (qx, qy) = svg_point(&mut parser, current, relative)?;
                    let (x, y) = svg_point(&mut parser, current, relative)?;
                    let c1 = (
                        current.0 + (2.0 / 3.0) * (qx - current.0),
                        current.1 + (2.0 / 3.0) * (qy - current.1),
                    );
                    let c2 = (x + (2.0 / 3.0) * (qx - x), y + (2.0 / 3.0) * (qy - y));
                    path.curve_to(c1.0, c1.1, c2.0, c2.1, x, y);
                    current = (x, y);
                }
            }
            'Z' => {
                path.close();
                current = subpath_start;
            }
            other => {
                return Err(OxideError::UnsupportedFeature(format!(
                    "SVG-in-OpenType path command unsupported: {other}"
                )));
            }
        }
    }
    Ok(path)
}

fn svg_point(
    parser: &mut SvgPathParser<'_>,
    current: (f64, f64),
    relative: bool,
) -> Result<(f64, f64)> {
    let mut x = parser.number()?;
    let mut y = parser.number()?;
    if relative {
        x += current.0;
        y += current.1;
    }
    if x.is_finite() && y.is_finite() {
        Ok((x, y))
    } else {
        Err(OxideError::UnsupportedFeature(
            "SVG-in-OpenType non-finite path coordinate blocked".to_string(),
        ))
    }
}

fn sbix_payload<'a>(
    font_bytes: &'a [u8],
    glyph_id: GlyphId,
    target_ppem: u16,
) -> Option<SbixPayload<'a>> {
    let face = ttf_parser::Face::parse(font_bytes, 0).ok()?;
    let raw = face.raw_face();
    let sbix = raw.table(Tag::from_bytes(b"sbix"))?;
    sbix_payload_inner(sbix, face.number_of_glyphs(), glyph_id, target_ppem, 0)
}

fn sbix_payload_inner<'a>(
    sbix: &'a [u8],
    glyph_count: u16,
    glyph_id: GlyphId,
    target_ppem: u16,
    depth: u8,
) -> Option<SbixPayload<'a>> {
    if depth >= 10 || glyph_id.0 >= glyph_count {
        return None;
    }
    let strike_count = read_u32(sbix, 4)? as usize;
    let strike_offsets_start = 8usize;
    let mut best: Option<SbixPayload<'a>> = None;
    let mut best_distance = u16::MAX;
    for strike_index in 0..strike_count {
        let strike_offset = read_u32(sbix, strike_offsets_start + strike_index * 4)? as usize;
        let pixels_per_em = read_u16(sbix, strike_offset).unwrap_or(0).max(1);
        let offsets_start = strike_offset.checked_add(4)?;
        let glyph_offset_index = offsets_start.checked_add(usize::from(glyph_id.0) * 4)?;
        let start = read_u32(sbix, glyph_offset_index)? as usize;
        let end = read_u32(sbix, glyph_offset_index.checked_add(4)?)? as usize;
        if start == end {
            continue;
        }
        if end <= start || end.checked_sub(start)? < 8 {
            return Some(SbixPayload {
                kind: ColorBitmapPayloadKind::Other("sbix malformed strike record".to_string()),
                data: &[],
                x: 0,
                y: 0,
                pixels_per_em,
            });
        }
        let record_start = strike_offset.checked_add(start)?;
        let record_end = strike_offset.checked_add(end)?;
        let x = read_i16(sbix, record_start).unwrap_or(0);
        let y = read_i16(sbix, record_start.checked_add(2)?).unwrap_or(0);
        let tag_bytes = sbix.get(record_start.checked_add(4)?..record_start.checked_add(8)?)?;
        let payload_start = record_start.checked_add(8)?;
        let data = sbix.get(payload_start..record_end)?;
        let kind = match tag_bytes {
            b"png " => ColorBitmapPayloadKind::Png,
            b"jpg " | b"jpeg" => ColorBitmapPayloadKind::Jpeg,
            b"tiff" | b"tif " => ColorBitmapPayloadKind::Tiff,
            b"pdf " => ColorBitmapPayloadKind::Pdf,
            b"mask" => ColorBitmapPayloadKind::Mask,
            b"dupe" => {
                let dupe_gid = read_u16(sbix, payload_start)?;
                return sbix_payload_inner(
                    sbix,
                    glyph_count,
                    GlyphId(dupe_gid),
                    target_ppem,
                    depth + 1,
                )
                .or(Some(SbixPayload {
                    kind: ColorBitmapPayloadKind::Dupe,
                    data,
                    x,
                    y,
                    pixels_per_em,
                }));
            }
            other => {
                ColorBitmapPayloadKind::Other(String::from_utf8_lossy(other).trim().to_string())
            }
        };
        let distance = pixels_per_em.abs_diff(target_ppem.max(1));
        if best.is_none() || distance < best_distance {
            best_distance = distance;
            best = Some(SbixPayload {
                kind,
                data,
                x,
                y,
                pixels_per_em,
            });
        }
    }
    best
}

fn decode_sbix_payload(payload: &SbixPayload<'_>) -> Result<RawImage> {
    match payload.kind {
        ColorBitmapPayloadKind::Png => decode_png(payload.data),
        ColorBitmapPayloadKind::Jpeg => decode_jpeg(payload.data),
        ColorBitmapPayloadKind::Tiff
        | ColorBitmapPayloadKind::Pdf
        | ColorBitmapPayloadKind::Mask
        | ColorBitmapPayloadKind::Dupe
        | ColorBitmapPayloadKind::Other(_) => Err(OxideError::UnsupportedFeature(format!(
            "sbix color glyph payload unsupported by safe decoder: payload={}",
            payload.kind.label()
        ))),
    }
}

fn decode_jpeg(data: &[u8]) -> Result<RawImage> {
    if data.len() > MAX_COLOR_GLYPH_BYTES {
        return Err(OxideError::UnsupportedFeature(format!(
            "color glyph JPEG payload too large: {} bytes",
            data.len()
        )));
    }
    let (mut pixels, width, height, channels) = ImageDecoder::decode_jpeg_with_info(data)?;
    if width.saturating_mul(height) > MAX_COLOR_GLYPH_PIXELS {
        return Err(OxideError::UnsupportedFeature(format!(
            "color glyph JPEG decoded dimensions too large: {width}x{height}"
        )));
    }
    let channels = if channels == 4 {
        pixels = ColorSpaceConverter::cmyk_to_rgb(&pixels);
        3
    } else {
        channels
    };
    Ok(RawImage {
        width,
        height,
        channels,
        bits_per_sample: 8,
        pixels,
    })
}

fn read_i16(data: &[u8], offset: usize) -> Option<i16> {
    let bytes = data.get(offset..offset.checked_add(2)?)?;
    Some(i16::from_be_bytes([bytes[0], bytes[1]]))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_static_subset_parses_path_shape_transform_and_opacity() {
        let svg = r##"
            <svg viewBox="0 0 1000 1000">
              <g transform="translate(10 20) scale(2)" opacity="0.5">
                <path d="M10 10 L40 10 L40 40 Z" fill="#336699"/>
                <rect x="100" y="100" width="50" height="40" fill="none" stroke="red" stroke-width="3"/>
              </g>
            </svg>
        "##;
        let paths = parse_static_svg_paint_paths(svg, rgba(0, 0, 0, 255), 255).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].fill.unwrap()[3], 128);
        assert!(paths[1].fill.is_none());
        assert!(paths[1].stroke.is_some());
        assert_eq!(paths[1].stroke_width, 3.0);
        assert!(!paths[0].transform.is_identity());
    }

    #[test]
    fn svg_security_classifier_blocks_active_content() {
        let policy =
            classify_svg_glyph_document(r#"<svg><path onclick="alert(1)" d="M0 0 L1 1"/></svg>"#);
        assert_eq!(policy.status(), "blocked_security_policy");
        assert!(policy.reason().contains("event"));
    }

    #[test]
    fn svg_path_parser_rejects_arc_commands_precisely() {
        let err = parse_svg_path_data("M0 0 A10 10 0 0 1 20 20").unwrap_err();
        assert!(err.to_string().contains("path command unsupported: A"));
    }
}
