//! The editor's operation list: the only way an edit reaches a document.
//!
//! The browser records every edit as one of these operations and previews it
//! by applying it with this crate compiled to WASM; the server replays the
//! same list with this crate compiled natively. [`OperationList::validate`]
//! is the shared admission rule: anything outside these bounds is refused
//! before a byte of the PDF is touched.

use serde::{Deserialize, Serialize};
use wellfriend_pdf_edit::{
    ImagePlacementSelector, ImageSelector, PdfMatrix, VectorPathMutation, VectorPathSelector,
};

use crate::error::EditorError;

pub const OPERATION_LIST_VERSION: u32 = 1;
pub const MAX_OPERATIONS: usize = 500;
pub const MAX_TEXT_CHARS: usize = 20_000;
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_IMAGE_BYTES: usize = 24 * 1024 * 1024;
pub const MAX_URL_CHARS: usize = 2_048;
/// PDF implementation limit for user-space coordinates (ISO 32000 Annex C).
pub const MAX_COORDINATE: f64 = 14_400.0;
pub const MAX_PAGE: u32 = 100_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationList {
    pub version: u32,
    pub operations: Vec<EditorOperation>,
}

/// An RGB colour, each component in `0..=1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rgb {
    pub r: f64,
    pub g: f64,
    pub b: f64,
}

impl Rgb {
    pub const BLACK: Rgb = Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };
    pub const WHITE: Rgb = Rgb {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };
}

/// A user-space rectangle: lower-left corner plus size, in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Inline image bytes (base64) for added or replacement images.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImageData {
    pub mime_type: String,
    pub base64: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeKind {
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase", deny_unknown_fields)]
pub enum EditorOperation {
    /// Replace `[start, end)` of the page's layout text in place, keeping the
    /// source font, size, colour and position (the engine's style-preserving
    /// reflow transaction). `expected` must equal the current text of the
    /// range. `fallbackFont` names a library font used only when the
    /// embedded (usually subset) font cannot draw the new text.
    #[serde(rename_all = "camelCase")]
    ReplaceText {
        page: u32,
        start: usize,
        end: usize,
        expected: String,
        text: String,
        #[serde(default)]
        fallback_font: Option<String>,
    },
    /// Set an image placement's matrix (move, resize, rotate).
    #[serde(rename_all = "camelCase")]
    TransformImage {
        selector: ImagePlacementSelector,
        matrix: PdfMatrix,
    },
    /// Remove one image placement from its content stream.
    #[serde(rename_all = "camelCase")]
    DeleteImage { selector: ImagePlacementSelector },
    /// Replace an image XObject's samples with a JPEG.
    #[serde(rename_all = "camelCase")]
    ReplaceImage {
        selector: ImageSelector,
        image: ImageData,
    },
    /// Recolour, transform or delete one vector path group.
    #[serde(rename_all = "camelCase")]
    EditVector {
        selector: VectorPathSelector,
        mutation: VectorPathMutation,
    },
    /// Add a text box with an embedded library font. `rect` is the box; text
    /// wraps at its width and starts at its top edge.
    #[serde(rename_all = "camelCase")]
    AddText {
        page: u32,
        rect: Rect,
        text: String,
        font: String,
        size: f64,
        color: Rgb,
        #[serde(default)]
        align: Align,
        #[serde(default = "default_line_height")]
        line_height: f64,
    },
    #[serde(rename_all = "camelCase")]
    AddImage {
        page: u32,
        rect: Rect,
        image: ImageData,
    },
    #[serde(rename_all = "camelCase")]
    AddShape {
        page: u32,
        shape: ShapeKind,
        rect: Rect,
        #[serde(default)]
        stroke: Option<Rgb>,
        #[serde(default)]
        fill: Option<Rgb>,
        #[serde(default = "default_line_width")]
        line_width: f64,
    },
    #[serde(rename_all = "camelCase")]
    AddLine {
        page: u32,
        from: [f64; 2],
        to: [f64; 2],
        color: Rgb,
        #[serde(default = "default_line_width")]
        line_width: f64,
    },
    /// An opaque box painted over content (Foxit's "whiteout"). This covers;
    /// use `Redact` to remove.
    #[serde(rename_all = "camelCase")]
    AddWhiteout {
        page: u32,
        rect: Rect,
        #[serde(default = "white")]
        color: Rgb,
    },
    /// A link annotation to a URL (`http`, `https`, `mailto`) or a page.
    #[serde(rename_all = "camelCase")]
    AddLink {
        page: u32,
        rect: Rect,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        target_page: Option<u32>,
    },
    /// True redaction: text, image and path content under `rect` is removed
    /// from the content streams (full rewrite), then the area is filled.
    #[serde(rename_all = "camelCase")]
    Redact {
        page: u32,
        rect: Rect,
        #[serde(default = "black")]
        fill: Rgb,
    },
}

fn default_line_height() -> f64 {
    1.2
}
fn default_line_width() -> f64 {
    1.0
}
fn white() -> Rgb {
    Rgb::WHITE
}
fn black() -> Rgb {
    Rgb::BLACK
}

impl EditorOperation {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReplaceText { .. } => "replaceText",
            Self::TransformImage { .. } => "transformImage",
            Self::DeleteImage { .. } => "deleteImage",
            Self::ReplaceImage { .. } => "replaceImage",
            Self::EditVector { .. } => "editVector",
            Self::AddText { .. } => "addText",
            Self::AddImage { .. } => "addImage",
            Self::AddShape { .. } => "addShape",
            Self::AddLine { .. } => "addLine",
            Self::AddWhiteout { .. } => "addWhiteout",
            Self::AddLink { .. } => "addLink",
            Self::Redact { .. } => "redact",
        }
    }

    /// The 1-based page the operation edits.
    pub fn page(&self) -> u32 {
        match self {
            Self::ReplaceText { page, .. }
            | Self::AddText { page, .. }
            | Self::AddImage { page, .. }
            | Self::AddShape { page, .. }
            | Self::AddLine { page, .. }
            | Self::AddWhiteout { page, .. }
            | Self::AddLink { page, .. }
            | Self::Redact { page, .. } => *page,
            Self::TransformImage { selector, .. } | Self::DeleteImage { selector } => {
                selector.page_number
            }
            Self::ReplaceImage { .. } => 0,
            Self::EditVector { selector, .. } => selector.page_number,
        }
    }
}

fn invalid(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_OPERATION_INVALID", message)
}

fn finite(value: f64, what: &str) -> Result<(), EditorError> {
    if value.is_finite() && value.abs() <= MAX_COORDINATE {
        Ok(())
    } else {
        Err(invalid(format!(
            "{what} must be a finite coordinate within ±{MAX_COORDINATE}"
        )))
    }
}

fn page(page: u32) -> Result<(), EditorError> {
    if (1..=MAX_PAGE).contains(&page) {
        Ok(())
    } else {
        Err(invalid("page must be a 1-based page number"))
    }
}

fn rect(rect: &Rect) -> Result<(), EditorError> {
    finite(rect.x, "rect.x")?;
    finite(rect.y, "rect.y")?;
    finite(rect.width, "rect.width")?;
    finite(rect.height, "rect.height")?;
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return Err(invalid("rect width and height must be positive"));
    }
    Ok(())
}

fn color(color: &Rgb) -> Result<(), EditorError> {
    for component in [color.r, color.g, color.b] {
        if !component.is_finite() || !(0.0..=1.0).contains(&component) {
            return Err(invalid("colour components must be within 0..1"));
        }
    }
    Ok(())
}

fn text(value: &str, allow_empty: bool) -> Result<(), EditorError> {
    if value.chars().count() > MAX_TEXT_CHARS {
        return Err(invalid(format!(
            "text is limited to {MAX_TEXT_CHARS} characters"
        )));
    }
    if !allow_empty && value.trim().is_empty() {
        return Err(invalid("text must not be empty"));
    }
    if value
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(invalid("text must not contain control characters"));
    }
    Ok(())
}

fn line_width(value: f64) -> Result<(), EditorError> {
    if value.is_finite() && (0.0..=100.0).contains(&value) {
        Ok(())
    } else {
        Err(invalid("line width must be within 0..100"))
    }
}

/// A library font id: lowercase letters, digits and hyphens.
pub fn valid_font_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
}

fn font_id(id: &str) -> Result<(), EditorError> {
    if valid_font_id(id) {
        Ok(())
    } else {
        Err(invalid("font must be a font library id"))
    }
}

fn matrix(m: &PdfMatrix) -> Result<(), EditorError> {
    for (value, what) in [(m.a, "a"), (m.b, "b"), (m.c, "c"), (m.d, "d")] {
        if !value.is_finite() || value.abs() > MAX_COORDINATE {
            return Err(invalid(format!("matrix.{what} is out of range")));
        }
    }
    finite(m.e, "matrix.e")?;
    finite(m.f, "matrix.f")?;
    if (m.a * m.d - m.b * m.c).abs() < 1e-6 {
        return Err(invalid("matrix must be invertible"));
    }
    Ok(())
}

/// Decoded image bytes after the format and size checks.
pub fn decode_image(image: &ImageData) -> Result<Vec<u8>, EditorError> {
    use base64::Engine as _;
    if image.base64.len() > MAX_IMAGE_BYTES / 3 * 4 + 8 {
        return Err(invalid("images are limited to 8 MiB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(image.base64.as_bytes())
        .map_err(|_| invalid("image data is not valid base64"))?;
    if bytes.len() > MAX_IMAGE_BYTES || bytes.is_empty() {
        return Err(invalid("images are limited to 8 MiB"));
    }
    let jpeg = bytes.starts_with(&[0xFF, 0xD8, 0xFF]);
    let png = bytes.starts_with(b"\x89PNG\r\n\x1a\n");
    match image.mime_type.as_str() {
        "image/jpeg" if jpeg => Ok(bytes),
        "image/png" if png => Ok(bytes),
        "image/jpeg" | "image/png" => Err(invalid("image bytes do not match their declared type")),
        _ => Err(invalid("images must be image/jpeg or image/png")),
    }
}

pub fn valid_url(url: &str) -> bool {
    if url.len() > MAX_URL_CHARS || url.chars().any(|c| c.is_control() || c == ' ') {
        return false;
    }
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")) && url.len() > 8
        || lower.starts_with("mailto:") && url.len() > 7
}

impl OperationList {
    pub fn parse(json: &str) -> Result<Self, EditorError> {
        let list: OperationList = serde_json::from_str(json)
            .map_err(|error| invalid(format!("operation list is malformed: {error}")))?;
        list.validate()?;
        Ok(list)
    }

    /// The shared admission rule (browser and server).
    pub fn validate(&self) -> Result<(), EditorError> {
        if self.version != OPERATION_LIST_VERSION {
            return Err(invalid(format!(
                "operation list version {} is not supported",
                self.version
            )));
        }
        if self.operations.len() > MAX_OPERATIONS {
            return Err(invalid(format!(
                "at most {MAX_OPERATIONS} operations per save"
            )));
        }
        let mut image_bytes = 0usize;
        for (index, operation) in self.operations.iter().enumerate() {
            validate_operation(operation, &mut image_bytes).map_err(|error| error.at(index))?;
        }
        Ok(())
    }
}

pub fn validate_operation(
    operation: &EditorOperation,
    image_bytes: &mut usize,
) -> Result<(), EditorError> {
    match operation {
        EditorOperation::ReplaceText {
            page: p,
            start,
            end,
            expected,
            text: t,
            fallback_font,
        } => {
            page(*p)?;
            if start > end {
                return Err(invalid("start must not exceed end"));
            }
            if end - start > MAX_TEXT_CHARS || expected.chars().count() != end - start {
                return Err(invalid("expected must be the exact text of [start, end)"));
            }
            text(t, true)?;
            if start == end && t.is_empty() {
                return Err(invalid(
                    "an empty replacement of an empty range changes nothing",
                ));
            }
            if let Some(font) = fallback_font {
                font_id(font)?;
            }
        }
        EditorOperation::TransformImage {
            selector,
            matrix: m,
        } => {
            page(selector.page_number)?;
            matrix(m)?;
        }
        EditorOperation::DeleteImage { selector } => page(selector.page_number)?,
        EditorOperation::ReplaceImage { image, .. } => {
            let bytes = decode_image(image)?;
            if image.mime_type != "image/jpeg" {
                return Err(invalid("replacement images must be JPEG"));
            }
            *image_bytes += bytes.len();
        }
        EditorOperation::EditVector { selector, mutation } => {
            page(selector.page_number)?;
            match mutation {
                VectorPathMutation::Transform { destination_matrix } => matrix(destination_matrix)?,
                VectorPathMutation::Recolor {
                    destination_fill_color,
                    destination_stroke_color,
                } => {
                    for c in [destination_fill_color, destination_stroke_color]
                        .into_iter()
                        .flatten()
                    {
                        color(&Rgb {
                            r: c.red,
                            g: c.green,
                            b: c.blue,
                        })?;
                    }
                    if destination_fill_color.is_none() && destination_stroke_color.is_none() {
                        return Err(invalid("a recolour needs a fill or stroke colour"));
                    }
                }
                VectorPathMutation::Delete => {}
            }
        }
        EditorOperation::AddText {
            page: p,
            rect: r,
            text: t,
            font,
            size,
            color: c,
            line_height,
            ..
        } => {
            page(*p)?;
            rect(r)?;
            text(t, false)?;
            font_id(font)?;
            color(c)?;
            if !size.is_finite() || !(1.0..=500.0).contains(size) {
                return Err(invalid("font size must be within 1..500 pt"));
            }
            if !line_height.is_finite() || !(0.5..=5.0).contains(line_height) {
                return Err(invalid("line height must be within 0.5..5"));
            }
        }
        EditorOperation::AddImage {
            page: p,
            rect: r,
            image,
        } => {
            page(*p)?;
            rect(r)?;
            *image_bytes += decode_image(image)?.len();
        }
        EditorOperation::AddShape {
            page: p,
            rect: r,
            stroke,
            fill,
            line_width: w,
            ..
        } => {
            page(*p)?;
            rect(r)?;
            line_width(*w)?;
            if stroke.is_none() && fill.is_none() {
                return Err(invalid("a shape needs a stroke or a fill"));
            }
            for c in [stroke, fill].into_iter().flatten() {
                color(c)?;
            }
        }
        EditorOperation::AddLine {
            page: p,
            from,
            to,
            color: c,
            line_width: w,
        } => {
            page(*p)?;
            for v in from.iter().chain(to.iter()) {
                finite(*v, "line point")?;
            }
            color(c)?;
            line_width(*w)?;
            if *w <= 0.0 {
                return Err(invalid("a line needs a positive width"));
            }
        }
        EditorOperation::AddWhiteout {
            page: p,
            rect: r,
            color: c,
        } => {
            page(*p)?;
            rect(r)?;
            color(c)?;
        }
        EditorOperation::AddLink {
            page: p,
            rect: r,
            url,
            target_page,
        } => {
            page(*p)?;
            rect(r)?;
            match (url, target_page) {
                (Some(url), None) if valid_url(url) => {}
                (Some(_), None) => return Err(invalid("links must be http, https or mailto URLs")),
                (None, Some(target)) => page(*target)?,
                _ => return Err(invalid("a link needs exactly one of url or targetPage")),
            }
        }
        EditorOperation::Redact {
            page: p,
            rect: r,
            fill,
        } => {
            page(*p)?;
            rect(r)?;
            color(fill)?;
        }
    }
    if *image_bytes > MAX_TOTAL_IMAGE_BYTES {
        return Err(invalid("images in one save are limited to 24 MiB"));
    }
    Ok(())
}
