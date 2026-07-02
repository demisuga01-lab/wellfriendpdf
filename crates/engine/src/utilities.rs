//! High-level document utilities layered on the existing renderer, authoring,
//! editing, and writer modules.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::authoring::{PageSize as AuthorPageSize, PdfBuilder};
use crate::content::Color;
use crate::crypto::EncryptParams;
use crate::editing::{
    EditMode, EditTextStyle, ImageRect, ImageStampOptions, OverlayLayer, PdfEditor,
    WatermarkOptions,
};
use crate::engine::ContentEngine;
use crate::error::{OxideError, Result};
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::images::encoder::{ImageEncoder, ImageOutputFormat};
use crate::signature::SignatureReport;
use crate::writer::{build_merged, build_subset, write_document_roundtrip};
use crate::{attachments::Attachment, fonts_report::FontInfo};

/// Raster output format for full-page rendering exports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RasterImageFormat {
    Jpeg,
    Png,
}

impl RasterImageFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
        }
    }

    fn output_format(self) -> ImageOutputFormat {
        match self {
            Self::Jpeg => ImageOutputFormat::Jpeg,
            Self::Png => ImageOutputFormat::Png,
        }
    }
}

/// One rendered page export result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPageResult {
    pub page: usize,
    pub path: Option<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub ok: bool,
    pub error: Option<String>,
}

/// Render a single page to JPEG or PNG bytes using the existing page renderer.
pub fn render_page_image(
    engine: &ContentEngine,
    page: usize,
    dpi: u32,
    format: RasterImageFormat,
    quality: u8,
) -> Result<(Vec<u8>, u32, u32)> {
    let dpi = dpi.max(1);
    let quality = quality.clamp(1, 100);
    let buffer = engine.render_page(page, dpi)?;
    let width = buffer.width;
    let height = buffer.height;
    let bytes = ImageEncoder::encode(
        &buffer.to_raw_image(),
        &format.output_format(),
        Some(quality),
    )?;
    Ok((bytes, width, height))
}

/// Render pages to individual image files. Each page is rendered and written
/// before the next is started, so callers never accumulate all page buffers.
pub fn export_pdf_pages_to_images(
    engine: &ContentEngine,
    out_dir: impl AsRef<Path>,
    pages: &[usize],
    dpi: u32,
    format: RasterImageFormat,
    quality: u8,
    stem: &str,
) -> Result<Vec<RasterPageResult>> {
    let total = engine.page_count()?;
    let selected = normalize_pages(total, pages)?;
    let out_dir = out_dir.as_ref();
    fs::create_dir_all(out_dir)?;
    let width = selected
        .iter()
        .copied()
        .max()
        .unwrap_or(1)
        .max(total)
        .to_string()
        .len()
        .max(3);
    let mut results = Vec::with_capacity(selected.len());
    for page in selected {
        let path = out_dir.join(format!(
            "{stem}-{page:0width$}.{}",
            format.extension(),
            width = width
        ));
        match render_page_image(engine, page, dpi, format, quality) {
            Ok((bytes, image_width, image_height)) => {
                fs::write(&path, bytes)?;
                results.push(RasterPageResult {
                    page,
                    path: Some(path),
                    width: image_width,
                    height: image_height,
                    ok: true,
                    error: None,
                });
            }
            Err(err) => results.push(RasterPageResult {
                page,
                path: Some(path),
                width: 0,
                height: 0,
                ok: false,
                error: Some(err.to_string()),
            }),
        }
    }
    Ok(results)
}

/// Page-size policy for image-to-PDF conversion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ImagePdfPageSize {
    A4,
    Letter,
    SizeToImage,
    Custom { width: f64, height: f64 },
}

impl ImagePdfPageSize {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "a4" => Some(Self::A4),
            "letter" => Some(Self::Letter),
            "image" | "size-to-image" | "size_to_image" => Some(Self::SizeToImage),
            _ => None,
        }
    }

    fn resolve(self, image_width: u32, image_height: u32) -> Result<AuthorPageSize> {
        let size = match self {
            Self::A4 => AuthorPageSize::A4,
            Self::Letter => AuthorPageSize::LETTER,
            Self::SizeToImage => AuthorPageSize::custom(image_width as f64, image_height as f64),
            Self::Custom { width, height } => {
                if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
                    return Err(OxideError::MalformedPdf(
                        "image-to-pdf: custom page size must be positive finite points".to_string(),
                    ));
                }
                AuthorPageSize::custom(width, height)
            }
        };
        Ok(size)
    }
}

/// Options for image-to-PDF conversion.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ImageToPdfOptions {
    pub page_size: ImagePdfPageSize,
    pub margin_points: f64,
}

impl Default for ImageToPdfOptions {
    fn default() -> Self {
        Self {
            page_size: ImagePdfPageSize::A4,
            margin_points: 0.0,
        }
    }
}

/// Build a PDF with one page per image path. Inputs are read and registered one
/// at a time; the builder retains the encoded image data required for output.
pub fn images_to_pdf_from_paths(paths: &[PathBuf], options: ImageToPdfOptions) -> Result<Vec<u8>> {
    if paths.is_empty() {
        return Err(OxideError::MalformedPdf(
            "image-to-pdf: at least one image is required".to_string(),
        ));
    }
    let mut builder = PdfBuilder::new();
    for path in paths {
        let bytes = fs::read(path)?;
        add_image_page(
            &mut builder,
            &bytes,
            path.extension().and_then(|s| s.to_str()),
            options,
        )?;
    }
    builder.to_bytes()
}

/// Build a PDF with one page per in-memory image.
pub fn images_to_pdf_from_bytes(
    images: &[(&[u8], Option<&str>)],
    options: ImageToPdfOptions,
) -> Result<Vec<u8>> {
    if images.is_empty() {
        return Err(OxideError::MalformedPdf(
            "image-to-pdf: at least one image is required".to_string(),
        ));
    }
    let mut builder = PdfBuilder::new();
    for (bytes, hint) in images {
        add_image_page(&mut builder, bytes, *hint, options)?;
    }
    builder.to_bytes()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StampPosition {
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
    Tile,
}

impl StampPosition {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "center" => Some(Self::Center),
            "top-left" | "top_left" => Some(Self::TopLeft),
            "top-center" | "top_center" => Some(Self::TopCenter),
            "top-right" | "top_right" => Some(Self::TopRight),
            "bottom-left" | "bottom_left" => Some(Self::BottomLeft),
            "bottom-center" | "bottom_center" => Some(Self::BottomCenter),
            "bottom-right" | "bottom_right" => Some(Self::BottomRight),
            "tile" | "tiled" => Some(Self::Tile),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Default for RgbColor {
    fn default() -> Self {
        Self {
            r: 0.55,
            g: 0.55,
            b: 0.55,
        }
    }
}

impl RgbColor {
    pub fn to_color(self) -> Result<Color> {
        if !self.r.is_finite()
            || !self.g.is_finite()
            || !self.b.is_finite()
            || !(0.0..=1.0).contains(&self.r)
            || !(0.0..=1.0).contains(&self.g)
            || !(0.0..=1.0).contains(&self.b)
        {
            return Err(OxideError::MalformedPdf(
                "color components must be finite values in 0.0..=1.0".to_string(),
            ));
        }
        Ok(Color::device_rgb(self.r, self.g, self.b))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextWatermarkOptions {
    pub pages: Vec<usize>,
    pub position: StampPosition,
    pub opacity: f64,
    pub rotation_degrees: f64,
    pub font_size: f64,
    pub color: RgbColor,
}

impl Default for TextWatermarkOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            position: StampPosition::Center,
            opacity: 0.28,
            rotation_degrees: 45.0,
            font_size: 64.0,
            color: RgbColor::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageWatermarkOptions {
    pub pages: Vec<usize>,
    pub position: StampPosition,
    pub opacity: f64,
    pub scale: f64,
}

impl Default for ImageWatermarkOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            position: StampPosition::Center,
            opacity: 0.3,
            scale: 0.5,
        }
    }
}

/// Add a text watermark as an overlay content stream.
pub fn watermark_text_pdf(
    input: Vec<u8>,
    text: &str,
    options: TextWatermarkOptions,
) -> Result<Vec<u8>> {
    if text.trim().is_empty() {
        return Err(OxideError::MalformedPdf(
            "watermark: text must not be empty".to_string(),
        ));
    }
    validate_opacity(options.opacity, "watermark")?;
    validate_positive(options.font_size, "watermark: font size")?;
    validate_finite(options.rotation_degrees, "watermark: rotation")?;

    let mut editor = PdfEditor::open_bytes(input)?;
    let total = editor.document().get_pages()?.len();
    let pages = normalize_pages(total, &options.pages)?;
    let style = EditTextStyle::new(options.font_size)
        .fill(options.color.to_color()?)
        .opacity(options.opacity)
        .rotation_degrees(options.rotation_degrees);
    if options.position == StampPosition::Center {
        editor.add_watermark_text(
            text,
            WatermarkOptions {
                pages: Some(pages),
                style,
                layer: OverlayLayer::Overlay,
            },
        )?;
    } else {
        draw_positioned_text(&mut editor, &pages, text, style, options.position)?;
    }
    editor.save_to_bytes(EditMode::FullRewrite)
}

/// Add an image watermark as overlay content streams.
pub fn watermark_image_pdf(
    input: Vec<u8>,
    image_bytes: &[u8],
    extension_hint: Option<&str>,
    options: ImageWatermarkOptions,
) -> Result<Vec<u8>> {
    if image_bytes.is_empty() {
        return Err(OxideError::MalformedPdf(
            "watermark: image bytes must not be empty".to_string(),
        ));
    }
    validate_opacity(options.opacity, "watermark")?;
    validate_positive(options.scale, "watermark: image scale")?;
    if options.scale > 10.0 {
        return Err(OxideError::MalformedPdf(
            "watermark: image scale must be <= 10".to_string(),
        ));
    }
    let mut editor = PdfEditor::open_bytes(input)?;
    let total = editor.document().get_pages()?.len();
    let pages = normalize_pages(total, &options.pages)?;
    let image = image_payload(image_bytes, extension_hint)?;
    let all_pages = editor.document().get_pages()?;
    for page_number in pages {
        let page = &all_pages[page_number - 1];
        let page_width = page.media_box[2] - page.media_box[0];
        let page_height = page.media_box[3] - page.media_box[1];
        let max_w = page_width * options.scale;
        let max_h = page_height * options.scale;
        let (w, h) = fit_rect(image.width as f64, image.height as f64, max_w, max_h);
        let (x, y) = position_rect(page.media_box, w, h, options.position, 36.0);
        let rect = ImageRect::new(x, y, w, h);
        let stamp = ImageStampOptions {
            opacity: options.opacity,
            layer: OverlayLayer::Overlay,
        };
        match &image.kind {
            ImagePayloadKind::Jpeg(bytes) => {
                editor.stamp_jpeg_image(page_number, bytes.clone(), rect, stamp)?;
            }
            ImagePayloadKind::Rgba(pixels) => {
                editor.stamp_rgba_image(
                    page_number,
                    image.width,
                    image.height,
                    pixels.clone(),
                    rect,
                    stamp,
                )?;
            }
        }
    }
    editor.save_to_bytes(EditMode::FullRewrite)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PageNumberOptions {
    pub pages: Vec<usize>,
    pub position: StampPosition,
    pub format: String,
    pub start: isize,
    pub font_size: f64,
    pub color: RgbColor,
}

impl Default for PageNumberOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            position: StampPosition::BottomCenter,
            format: "Page {n} of {total}".to_string(),
            start: 1,
            font_size: 10.0,
            color: RgbColor {
                r: 0.2,
                g: 0.2,
                b: 0.2,
            },
        }
    }
}

pub fn add_page_numbers_pdf(input: Vec<u8>, options: PageNumberOptions) -> Result<Vec<u8>> {
    validate_positive(options.font_size, "page-numbers: font size")?;
    validate_page_number_format(&options.format)?;
    let mut editor = PdfEditor::open_bytes(input)?;
    let total = editor.document().get_pages()?.len();
    let pages = normalize_pages(total, &options.pages)?;
    let style = EditTextStyle::new(options.font_size).fill(options.color.to_color()?);
    let all_pages = editor.document().get_pages()?;
    for page_number in pages {
        let page = &all_pages[page_number - 1];
        let n = options.start + page_number as isize - 1;
        let text = options
            .format
            .replace("{n}", &n.to_string())
            .replace("{page}", &n.to_string())
            .replace("{total}", &total.to_string());
        let text_width = approximate_text_width(&text, options.font_size);
        let (x, y) = position_text(
            page.media_box,
            text_width,
            options.font_size,
            options.position,
            36.0,
        );
        editor.draw_text(
            page_number,
            text,
            x,
            y,
            style.clone(),
            OverlayLayer::Overlay,
        )?;
    }
    editor.save_to_bytes(EditMode::FullRewrite)
}

/// Convenience wrapper for the existing writer's ordered page-copy operation.
pub fn organize_pdf(engine: &ContentEngine, order: &[usize]) -> Result<Vec<u8>> {
    if order.is_empty() {
        return Err(OxideError::MalformedPdf(
            "organize: page order must not be empty".to_string(),
        ));
    }
    build_subset(engine.document(), order)
}

/// Organize one primary document and optionally insert pages from another
/// document at a 1-based output position.
pub fn organize_pdf_with_insert(
    primary: &ContentEngine,
    order: &[usize],
    inserted: Option<(&ContentEngine, Vec<usize>, usize)>,
) -> Result<Vec<u8>> {
    let base = if order.is_empty() {
        (1..=primary.page_count()?).collect::<Vec<_>>()
    } else {
        order.to_vec()
    };
    if let Some((other, insert_pages, at)) = inserted {
        if insert_pages.is_empty() {
            return Err(OxideError::MalformedPdf(
                "organize: inserted page list must not be empty".to_string(),
            ));
        }
        let split = at.saturating_sub(1).min(base.len());
        let before = base[..split].to_vec();
        let after = base[split..].to_vec();
        let mut inputs = Vec::new();
        if !before.is_empty() {
            inputs.push((primary.document(), before));
        }
        inputs.push((other.document(), insert_pages));
        if !after.is_empty() {
            inputs.push((primary.document(), after));
        }
        build_merged(&inputs)
    } else {
        organize_pdf(primary, &base)
    }
}

/// Write an unencrypted normalized copy of a password-opened document.
pub fn decrypt_pdf(engine: &ContentEngine) -> Result<Vec<u8>> {
    write_document_roundtrip(engine.document().reader())
}

/// Thin wrappers used by bindings to avoid reaching into low-level modules.
pub fn merge_pdf_documents(inputs: &[(&crate::PdfDocument, Vec<usize>)]) -> Result<Vec<u8>> {
    build_merged(inputs)
}

pub fn encrypt_pdf(engine: &ContentEngine, params: &EncryptParams) -> Result<Vec<u8>> {
    crate::structural::encrypt(engine, params)
}

pub fn rotate_pdf(
    engine: &ContentEngine,
    pages: &[usize],
    rotation: crate::Rotation,
) -> Result<Vec<u8>> {
    crate::structural::rotate_pages(engine, pages, rotation)
}

pub fn repair_pdf(bytes: Vec<u8>, password: &[u8]) -> Result<Vec<u8>> {
    crate::structural::repair(bytes, password)
}

pub fn optimize_pdf(engine: &ContentEngine) -> Result<(Vec<u8>, crate::OptimizeReport)> {
    crate::structural::optimize(engine)
}

pub fn linearize_pdf(engine: &ContentEngine) -> Result<Vec<u8>> {
    crate::structural::linearize::linearize(engine)
}

pub fn fonts_json(engine: &ContentEngine) -> Result<Vec<FontInfo>> {
    engine.list_fonts()
}

pub fn attachments_json(engine: &ContentEngine) -> Result<Vec<Attachment>> {
    engine.list_attachments()
}

pub fn signatures_json(engine: &ContentEngine) -> Result<Vec<SignatureReport>> {
    engine.verify_signatures()
}

pub fn html_string(engine: &ContentEngine, pages: &[usize]) -> Result<String> {
    let selected = if pages.is_empty() {
        (1..=engine.page_count()?).collect::<Vec<_>>()
    } else {
        pages.to_vec()
    };
    crate::HtmlExporter::export(engine, &selected, &crate::HtmlOptions::default())
}

fn normalize_pages(total: usize, pages: &[usize]) -> Result<Vec<usize>> {
    if total == 0 {
        return Err(OxideError::MalformedPdf(
            "document has no pages".to_string(),
        ));
    }
    if pages.is_empty() {
        return Ok((1..=total).collect());
    }
    for &page in pages {
        if page == 0 || page > total {
            return Err(OxideError::MalformedPdf(format!(
                "page {page} is out of range 1..={total}"
            )));
        }
    }
    Ok(pages.to_vec())
}

fn add_image_page(
    builder: &mut PdfBuilder,
    bytes: &[u8],
    extension_hint: Option<&str>,
    options: ImageToPdfOptions,
) -> Result<()> {
    let payload = image_payload(bytes, extension_hint)?;
    let page_size = options.page_size.resolve(payload.width, payload.height)?;
    let handle = match payload.kind {
        ImagePayloadKind::Jpeg(bytes) => builder.add_jpeg_image(bytes)?,
        ImagePayloadKind::Rgba(pixels) => {
            builder.add_rgba_image(payload.width, payload.height, pixels)?
        }
    };
    let margin = options.margin_points.max(0.0);
    let max_w = (page_size.width - margin * 2.0).max(1.0);
    let max_h = (page_size.height - margin * 2.0).max(1.0);
    let (w, h) = fit_rect(payload.width as f64, payload.height as f64, max_w, max_h);
    let x = margin + (max_w - w) / 2.0;
    let y = margin + (max_h - h) / 2.0;
    builder.add_page(page_size).draw_image(handle, x, y, w, h);
    Ok(())
}

#[derive(Debug, Clone)]
struct ImagePayload {
    width: u32,
    height: u32,
    kind: ImagePayloadKind,
}

#[derive(Debug, Clone)]
enum ImagePayloadKind {
    Jpeg(Vec<u8>),
    Rgba(Vec<u8>),
}

fn image_payload(bytes: &[u8], extension_hint: Option<&str>) -> Result<ImagePayload> {
    let lower = extension_hint.unwrap_or("").to_ascii_lowercase();
    let is_jpeg = bytes.starts_with(&[0xFF, 0xD8]) || matches!(lower.as_str(), "jpg" | "jpeg");
    let is_png = bytes.starts_with(b"\x89PNG\r\n\x1a\n") || lower == "png";
    if is_jpeg {
        let (pixels, width, height, channels) = ImageDecoder::decode_jpeg_with_info(bytes)?;
        enforce_decode_cap(width, height)?;
        if channels == 4 {
            let rgb = crate::images::decoder::ColorSpaceConverter::cmyk_to_rgb(&pixels);
            let rgba = rgb_to_rgba(width, height, &rgb, 3)?;
            return Ok(ImagePayload {
                width,
                height,
                kind: ImagePayloadKind::Rgba(rgba),
            });
        }
        return Ok(ImagePayload {
            width,
            height,
            kind: ImagePayloadKind::Jpeg(bytes.to_vec()),
        });
    }
    if is_png {
        let raw = decode_png_rgba(bytes)?;
        enforce_decode_cap(raw.width, raw.height)?;
        return Ok(ImagePayload {
            width: raw.width,
            height: raw.height,
            kind: ImagePayloadKind::Rgba(raw.pixels),
        });
    }
    Err(OxideError::UnsupportedFeature(
        "image-to-pdf/watermark: supported image formats are JPEG and PNG".to_string(),
    ))
}

fn decode_png_rgba(bytes: &[u8]) -> Result<RawImage> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|err| OxideError::MalformedPdf(format!("PNG decode failed: {err}")))?;
    enforce_decode_cap(reader.info().width, reader.info().height)?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|err| OxideError::MalformedPdf(format!("PNG decode failed: {err}")))?;
    let samples = &buf[..info.buffer_size()];
    let pixels = match info.color_type {
        png::ColorType::Grayscale => gray_to_rgba(info.width, info.height, samples),
        png::ColorType::GrayscaleAlpha => gray_alpha_to_rgba(info.width, info.height, samples),
        png::ColorType::Rgb => rgb_to_rgba(info.width, info.height, samples, 3)?,
        png::ColorType::Rgba => samples.to_vec(),
        png::ColorType::Indexed => {
            return Err(OxideError::UnsupportedFeature(
                "indexed PNG did not expand to samples".to_string(),
            ))
        }
    };
    Ok(RawImage {
        width: info.width,
        height: info.height,
        channels: 4,
        bits_per_sample: 8,
        pixels,
    })
}

fn enforce_decode_cap(width: u32, height: u32) -> Result<()> {
    let pixels = u64::from(width) * u64::from(height);
    let cap = crate::engine::max_decode_pixels();
    if pixels > cap {
        return Err(OxideError::ResourceLimit(format!(
            "image has {pixels} pixels, exceeding decode cap {cap}"
        )));
    }
    Ok(())
}

fn fit_rect(source_w: f64, source_h: f64, max_w: f64, max_h: f64) -> (f64, f64) {
    let scale = (max_w / source_w).min(max_h / source_h).min(1.0);
    ((source_w * scale).max(1.0), (source_h * scale).max(1.0))
}

fn draw_positioned_text(
    editor: &mut PdfEditor,
    pages: &[usize],
    text: &str,
    style: EditTextStyle,
    position: StampPosition,
) -> Result<()> {
    let all_pages = editor.document().get_pages()?;
    for &page_number in pages {
        let page = &all_pages[page_number - 1];
        if position == StampPosition::Tile {
            let width = page.media_box[2] - page.media_box[0];
            let height = page.media_box[3] - page.media_box[1];
            let step_x = (approximate_text_width(text, style.font_size) * 1.7).max(160.0);
            let step_y = (style.font_size * 4.0).max(120.0);
            let mut y = page.media_box[1] + 72.0;
            while y < page.media_box[1] + height {
                let mut x = page.media_box[0] + 36.0;
                while x < page.media_box[0] + width {
                    editor.draw_text(
                        page_number,
                        text,
                        x,
                        y,
                        style.clone(),
                        OverlayLayer::Overlay,
                    )?;
                    x += step_x;
                }
                y += step_y;
            }
            continue;
        }
        let text_width = approximate_text_width(text, style.font_size);
        let (x, y) = position_text(page.media_box, text_width, style.font_size, position, 36.0);
        editor.draw_text(
            page_number,
            text,
            x,
            y,
            style.clone(),
            OverlayLayer::Overlay,
        )?;
    }
    Ok(())
}

fn position_text(
    media_box: [f64; 4],
    text_width: f64,
    _font_size: f64,
    position: StampPosition,
    margin: f64,
) -> (f64, f64) {
    let width = media_box[2] - media_box[0];
    let height = media_box[3] - media_box[1];
    match position {
        StampPosition::Center | StampPosition::Tile => (
            media_box[0] + (width - text_width) / 2.0,
            media_box[1] + height / 2.0,
        ),
        StampPosition::TopLeft => (media_box[0] + margin, media_box[3] - margin),
        StampPosition::TopCenter => (
            media_box[0] + (width - text_width) / 2.0,
            media_box[3] - margin,
        ),
        StampPosition::TopRight => (media_box[2] - margin - text_width, media_box[3] - margin),
        StampPosition::BottomLeft => (media_box[0] + margin, media_box[1] + margin),
        StampPosition::BottomCenter => (
            media_box[0] + (width - text_width) / 2.0,
            media_box[1] + margin,
        ),
        StampPosition::BottomRight => (media_box[2] - margin - text_width, media_box[1] + margin),
    }
}

fn position_rect(
    media_box: [f64; 4],
    rect_width: f64,
    rect_height: f64,
    position: StampPosition,
    margin: f64,
) -> (f64, f64) {
    let width = media_box[2] - media_box[0];
    let height = media_box[3] - media_box[1];
    match position {
        StampPosition::Center | StampPosition::Tile => (
            media_box[0] + (width - rect_width) / 2.0,
            media_box[1] + (height - rect_height) / 2.0,
        ),
        StampPosition::TopLeft => (media_box[0] + margin, media_box[3] - margin - rect_height),
        StampPosition::TopCenter => (
            media_box[0] + (width - rect_width) / 2.0,
            media_box[3] - margin - rect_height,
        ),
        StampPosition::TopRight => (
            media_box[2] - margin - rect_width,
            media_box[3] - margin - rect_height,
        ),
        StampPosition::BottomLeft => (media_box[0] + margin, media_box[1] + margin),
        StampPosition::BottomCenter => (
            media_box[0] + (width - rect_width) / 2.0,
            media_box[1] + margin,
        ),
        StampPosition::BottomRight => (media_box[2] - margin - rect_width, media_box[1] + margin),
    }
}

fn approximate_text_width(text: &str, font_size: f64) -> f64 {
    text.chars().count() as f64 * font_size * 0.5
}

fn validate_opacity(value: f64, op: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(OxideError::MalformedPdf(format!(
            "{op}: opacity must be between 0 and 1"
        )));
    }
    Ok(())
}

fn validate_positive(value: f64, field: &str) -> Result<()> {
    if !value.is_finite() || value <= 0.0 {
        return Err(OxideError::MalformedPdf(format!(
            "{field} must be a positive finite value"
        )));
    }
    Ok(())
}

fn validate_finite(value: f64, field: &str) -> Result<()> {
    if !value.is_finite() {
        return Err(OxideError::MalformedPdf(format!("{field} must be finite")));
    }
    Ok(())
}

fn validate_page_number_format(format: &str) -> Result<()> {
    if format.trim().is_empty() {
        return Err(OxideError::MalformedPdf(
            "page-numbers: format must not be empty".to_string(),
        ));
    }
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let end = after.find('}').ok_or_else(|| {
            OxideError::MalformedPdf("page-numbers: unclosed format token".to_string())
        })?;
        let token = &after[..end];
        if !matches!(token, "n" | "page" | "total") {
            return Err(OxideError::MalformedPdf(format!(
                "page-numbers: unsupported format token {{{token}}}"
            )));
        }
        rest = &after[end + 1..];
    }
    if rest.contains('}') {
        return Err(OxideError::MalformedPdf(
            "page-numbers: unmatched closing brace".to_string(),
        ));
    }
    Ok(())
}

fn gray_to_rgba(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for &g in samples {
        out.extend_from_slice(&[g, g, g, 255]);
    }
    out
}

fn gray_alpha_to_rgba(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for chunk in samples.chunks_exact(2) {
        out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
    }
    out
}

fn rgb_to_rgba(width: u32, height: u32, samples: &[u8], channels: u8) -> Result<Vec<u8>> {
    if channels != 3 {
        return Err(OxideError::UnsupportedFeature(format!(
            "unsupported RGB conversion channel count {channels}"
        )));
    }
    let mut out = Vec::with_capacity(width as usize * height as usize * 4);
    for chunk in samples.chunks_exact(3) {
        out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentEngine, TextStyle};

    fn tiny_pdf() -> Vec<u8> {
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("Phase 3", 72.0, 720.0, &TextStyle::default())
            .unwrap();
        builder.to_bytes().unwrap()
    }

    fn tiny_png() -> Vec<u8> {
        let image = RawImage {
            width: 2,
            height: 1,
            channels: 4,
            bits_per_sample: 8,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        ImageEncoder::encode_png(&image).unwrap()
    }

    #[test]
    fn render_page_image_exports_jpeg_dimensions() {
        let engine = ContentEngine::open_bytes(tiny_pdf()).unwrap();
        let (bytes, width, height) =
            render_page_image(&engine, 1, 72, RasterImageFormat::Jpeg, 80).unwrap();
        assert!(bytes.starts_with(&[0xFF, 0xD8]));
        assert_eq!((width, height), (612, 792));
    }

    #[test]
    fn images_to_pdf_wraps_each_image_as_page() {
        let png = tiny_png();
        let bytes =
            images_to_pdf_from_bytes(&[(&png, Some("png"))], ImageToPdfOptions::default()).unwrap();
        let engine = ContentEngine::open_bytes(bytes).unwrap();
        assert_eq!(engine.page_count().unwrap(), 1);
    }

    #[test]
    fn watermark_and_page_numbers_preserve_original_text() {
        let watermarked =
            watermark_text_pdf(tiny_pdf(), "DRAFT", TextWatermarkOptions::default()).unwrap();
        let numbered = add_page_numbers_pdf(watermarked, PageNumberOptions::default()).unwrap();
        let engine = ContentEngine::open_bytes(numbered).unwrap();
        let text = engine.get_page_text(1).unwrap();
        assert!(text.contains("Phase 3"));
        assert!(text.contains("DRAFT"));
        assert!(text.contains("Page 1 of 1"));
    }

    #[test]
    fn organize_reorders_and_duplicates_pages() {
        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("One", 72.0, 720.0, &TextStyle::default())
            .unwrap();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("Two", 72.0, 720.0, &TextStyle::default())
            .unwrap();
        let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
        let organized = organize_pdf(&engine, &[2, 1, 2]).unwrap();
        let engine = ContentEngine::open_bytes(organized).unwrap();
        assert_eq!(engine.page_count().unwrap(), 3);
        assert!(engine.get_page_text(1).unwrap().contains("Two"));
        assert!(engine.get_page_text(2).unwrap().contains("One"));
    }

    #[test]
    fn invalid_page_number_format_errors_cleanly() {
        let err = add_page_numbers_pdf(
            tiny_pdf(),
            PageNumberOptions {
                format: "Page {bogus}".to_string(),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.to_string().contains("unsupported format token"));
    }
}
