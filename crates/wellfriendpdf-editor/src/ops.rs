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

/// A paint colour, each component in `0..=1`: device RGB `{r, g, b}`,
/// device CMYK `{c, m, y, k}`, or a spot colour `{spot, tint, c, m, y, k}`
/// written as a `/Separation` colour space whose CMYK values are the
/// alternate (full-tint) appearance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Paint {
    Spot {
        spot: String,
        #[serde(default = "full_tint")]
        tint: f64,
        c: f64,
        m: f64,
        y: f64,
        k: f64,
    },
    Cmyk {
        c: f64,
        m: f64,
        y: f64,
        k: f64,
    },
    Rgb {
        r: f64,
        g: f64,
        b: f64,
    },
}

/// The historical name: every colour field accepts any [`Paint`].
pub type Rgb = Paint;

fn full_tint() -> f64 {
    1.0
}

impl Paint {
    pub const BLACK: Paint = Paint::Rgb {
        r: 0.0,
        g: 0.0,
        b: 0.0,
    };
    pub const WHITE: Paint = Paint::Rgb {
        r: 1.0,
        g: 1.0,
        b: 1.0,
    };

    pub fn rgb(r: f64, g: f64, b: f64) -> Self {
        Paint::Rgb { r, g, b }
    }

    pub fn components(&self) -> Vec<f64> {
        match self {
            Paint::Rgb { r, g, b } => vec![*r, *g, *b],
            Paint::Cmyk { c, m, y, k } => vec![*c, *m, *y, *k],
            Paint::Spot { tint, c, m, y, k, .. } => vec![*tint, *c, *m, *y, *k],
        }
    }

    /// The colour the engine draws with: RGB, CMYK, or for a spot colour
    /// its alternate CMYK scaled by the tint (rewritten to the separation
    /// afterwards, see `overlay::merge`).
    pub fn engine(&self) -> wellfriendpdf_engine::Color {
        match self {
            Paint::Rgb { r, g, b } => wellfriendpdf_engine::Color::device_rgb(*r, *g, *b),
            Paint::Cmyk { c, m, y, k } => wellfriendpdf_engine::Color::device_cmyk(*c, *m, *y, *k),
            Paint::Spot { tint, c, m, y, k, .. } => {
                wellfriendpdf_engine::Color::device_cmyk(c * tint, m * tint, y * tint, k * tint)
            }
        }
    }

    /// `(name, alternate CMYK, tint)` of a spot colour.
    pub fn spot(&self) -> Option<SpotColor> {
        match self {
            Paint::Spot { spot, tint, c, m, y, k } => Some(SpotColor {
                name: spot.clone(),
                alternate: [*c, *m, *y, *k],
                tint: *tint,
            }),
            _ => None,
        }
    }

    /// An approximate RGB for previews of CMYK and spot colours.
    pub fn approximate_rgb(&self) -> [f64; 3] {
        match self {
            Paint::Rgb { r, g, b } => [*r, *g, *b],
            Paint::Cmyk { c, m, y, k } => [(1.0 - c) * (1.0 - k), (1.0 - m) * (1.0 - k), (1.0 - y) * (1.0 - k)],
            Paint::Spot { tint, c, m, y, k, .. } => [
                (1.0 - c * tint) * (1.0 - k * tint),
                (1.0 - m * tint) * (1.0 - k * tint),
                (1.0 - y * tint) * (1.0 - k * tint),
            ],
        }
    }
}

/// A named spot colour used by an operation.
#[derive(Debug, Clone, PartialEq)]
pub struct SpotColor {
    pub name: String,
    pub alternate: [f64; 4],
    pub tint: f64,
}

/// A page annotation kind the editor adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnnotationKind {
    Highlight,
    Underline,
    Strikeout,
    Note,
    Ink,
    Stamp,
}

/// A normalised crop of an image placement (fractions of the image).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Crop {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
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

/// What a text operation does when its text does not fit the room it has
/// (the page's bottom margin, the text below it in its column, or an added
/// text box). No policy drops words: every applied text operation is
/// checked to keep all of its text on the page (`crate::overflow`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TextOverflow {
    /// The operation fails with `EDITOR_TEXT_OVERFLOW` and the document is
    /// unchanged.
    #[default]
    Refuse,
    /// The font size (and with it the line spacing) is reduced until every
    /// word fits.
    Shrink,
    /// The text keeps its style and runs past its box or the bottom margin,
    /// down to the page edge, never over other text.
    Continue,
    /// The words that do not fit continue at the top of the same column on
    /// the next page, which moves its column text down to make room.
    NextPage,
}

impl TextOverflow {
    pub fn is_refuse(&self) -> bool {
        *self == TextOverflow::Refuse
    }
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
        /// When the edited paragraph no longer fits on the page.
        #[serde(default, skip_serializing_if = "TextOverflow::is_refuse")]
        overflow: TextOverflow,
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
        /// When the text does not fit in `rect`.
        #[serde(default, skip_serializing_if = "TextOverflow::is_refuse")]
        overflow: TextOverflow,
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
    /// Restyle existing text `[start, end)`: set it in the library face
    /// `font` (family, bold, italic) and optionally change size, colour,
    /// alignment and line spacing. The text is re-flowed inside its own block.
    #[serde(rename_all = "camelCase")]
    RestyleText {
        page: u32,
        start: usize,
        end: usize,
        expected: String,
        font: String,
        #[serde(default)]
        size: Option<f64>,
        #[serde(default)]
        color: Option<Rgb>,
        #[serde(default)]
        align: Option<Align>,
        #[serde(default)]
        line_height: Option<f64>,
        /// When the restyled paragraph no longer fits on the page.
        #[serde(default, skip_serializing_if = "TextOverflow::is_refuse")]
        overflow: TextOverflow,
    },
    /// Crop an image placement to the kept rectangle `crop` in image space
    /// (0 to 1 from the image's lower-left corner; `right > left` and
    /// `top > bottom`), not insets.
    #[serde(rename_all = "camelCase")]
    CropImage {
        selector: ImagePlacementSelector,
        crop: Crop,
    },
    /// Set an image placement's constant opacity.
    #[serde(rename_all = "camelCase")]
    SetImageOpacity {
        selector: ImagePlacementSelector,
        opacity: f64,
    },
    /// Set the page's display rotation (0, 90, 180 or 270).
    #[serde(rename_all = "camelCase")]
    RotatePage { page: u32, rotation: i32 },
    #[serde(rename_all = "camelCase")]
    DeletePage { page: u32 },
    /// Move `page` so it becomes page `to`.
    #[serde(rename_all = "camelCase")]
    MovePage { page: u32, to: u32 },
    /// Insert a blank page after `after` (0 = first) of the given size.
    #[serde(rename_all = "camelCase")]
    InsertBlankPage { after: u32, width: f64, height: f64 },
    /// Insert pages of another PDF (all pages when `pages` is empty).
    #[serde(rename_all = "camelCase")]
    InsertPages {
        after: u32,
        pdf: String,
        #[serde(default)]
        pages: Vec<u32>,
    },
    /// Add a markup annotation. Highlight, underline and strike-out mark
    /// `quads` (each `[x0, y0, x1, y1]`); a note sits at `rect`; ink draws
    /// `ink` polylines; a stamp shows `label`. Each has an appearance stream.
    #[serde(rename_all = "camelCase")]
    AddAnnotation {
        page: u32,
        kind: AnnotationKind,
        rect: Rect,
        #[serde(default)]
        quads: Vec<[f64; 4]>,
        #[serde(default)]
        ink: Vec<Vec<[f64; 2]>>,
        #[serde(default)]
        contents: Option<String>,
        #[serde(default)]
        label: Option<String>,
        color: Rgb,
        #[serde(default)]
        author: Option<String>,
    },
    /// Fill an AcroForm field by its fully qualified name; appearances are
    /// regenerated. Checkboxes take "true"/"false".
    #[serde(rename_all = "camelCase")]
    SetFormField {
        name: String,
        value: String,
        #[serde(default)]
        kind: FieldValueKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FieldValueKind {
    #[default]
    Text,
    Choice,
    Checkbox,
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
            Self::RestyleText { .. } => "restyleText",
            Self::CropImage { .. } => "cropImage",
            Self::SetImageOpacity { .. } => "setImageOpacity",
            Self::RotatePage { .. } => "rotatePage",
            Self::DeletePage { .. } => "deletePage",
            Self::MovePage { .. } => "movePage",
            Self::InsertBlankPage { .. } => "insertBlankPage",
            Self::InsertPages { .. } => "insertPages",
            Self::AddAnnotation { .. } => "addAnnotation",
            Self::SetFormField { .. } => "setFormField",
        }
    }

    /// Whether the operation changes the page sequence (page numbers after
    /// it refer to the new sequence).
    pub fn restructures_pages(&self) -> bool {
        matches!(
            self,
            Self::DeletePage { .. } | Self::MovePage { .. } | Self::InsertBlankPage { .. } | Self::InsertPages { .. }
        )
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
            | Self::Redact { page, .. }
            | Self::RestyleText { page, .. }
            | Self::RotatePage { page, .. }
            | Self::DeletePage { page }
            | Self::MovePage { page, .. }
            | Self::AddAnnotation { page, .. } => *page,
            Self::InsertBlankPage { after, .. } | Self::InsertPages { after, .. } => after + 1,
            Self::TransformImage { selector, .. }
            | Self::DeleteImage { selector }
            | Self::CropImage { selector, .. }
            | Self::SetImageOpacity { selector, .. } => selector.page_number,
            Self::ReplaceImage { .. } | Self::SetFormField { .. } => 0,
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
    for component in color.components() {
        if !component.is_finite() || !(0.0..=1.0).contains(&component) {
            return Err(invalid("colour components must be within 0..1"));
        }
    }
    if let Paint::Spot { spot, .. } = color {
        let valid = !spot.is_empty()
            && spot.len() <= 64
            && spot.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_');
        if !valid {
            return Err(invalid("spot colour names are 1-64 letters, digits, spaces, hyphens or underscores"));
        }
    }
    Ok(())
}

fn rgb_only(color: &Rgb) -> Result<(), EditorError> {
    match color {
        Paint::Rgb { .. } => Ok(()),
        _ => Err(invalid("this colour must be RGB")),
    }
}

fn point(value: [f64; 2]) -> Result<(), EditorError> {
    finite(value[0], "point.x")?;
    finite(value[1], "point.y")
}

pub const MAX_INSERTED_PDF_BYTES: usize = 24 * 1024 * 1024;

/// Decoded bytes of an inserted PDF.
pub fn decode_pdf(base64_pdf: &str) -> Result<Vec<u8>, EditorError> {
    use base64::Engine as _;
    if base64_pdf.len() > MAX_INSERTED_PDF_BYTES / 3 * 4 + 8 {
        return Err(invalid("inserted PDFs are limited to 24 MiB"));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_pdf.as_bytes())
        .map_err(|_| invalid("the inserted PDF is not valid base64"))?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(invalid("the inserted file is not a PDF"));
    }
    Ok(bytes)
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
            ..
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
            *image_bytes += decode_image(image)?.len();
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
                        color(&Paint::rgb(c.red, c.green, c.blue))?;
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
            rgb_only(fill)?;
        }
        EditorOperation::RestyleText {
            page: p,
            start,
            end,
            expected,
            font,
            size,
            color: c,
            line_height,
            ..
        } => {
            page(*p)?;
            if start >= end || end - start > MAX_TEXT_CHARS || expected.chars().count() != end - start {
                return Err(invalid("expected must be the exact, non-empty text of [start, end)"));
            }
            font_id(font)?;
            if let Some(size) = size {
                if !size.is_finite() || !(1.0..=500.0).contains(size) {
                    return Err(invalid("font size must be within 1..500 pt"));
                }
            }
            if let Some(c) = c {
                color(c)?;
            }
            if let Some(line_height) = line_height {
                if !line_height.is_finite() || !(0.5..=5.0).contains(line_height) {
                    return Err(invalid("line height must be within 0.5..5"));
                }
            }
        }
        EditorOperation::CropImage { selector, crop } => {
            page(selector.page_number)?;
            let sides = [crop.left, crop.bottom, crop.right, crop.top];
            if sides.iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                || crop.left >= crop.right
                || crop.bottom >= crop.top
            {
                return Err(invalid("a crop keeps a non-empty part of the image (fractions 0..1)"));
            }
        }
        EditorOperation::SetImageOpacity { selector, opacity } => {
            page(selector.page_number)?;
            if !opacity.is_finite() || !(0.0..=1.0).contains(opacity) {
                return Err(invalid("opacity must be within 0..1"));
            }
        }
        EditorOperation::RotatePage { page: p, rotation } => {
            page(*p)?;
            if ![0, 90, 180, 270].contains(rotation) {
                return Err(invalid("rotation must be 0, 90, 180 or 270"));
            }
        }
        EditorOperation::DeletePage { page: p } => page(*p)?,
        EditorOperation::MovePage { page: p, to } => {
            page(*p)?;
            page(*to)?;
        }
        EditorOperation::InsertBlankPage { after, width, height } => {
            if *after > MAX_PAGE {
                return Err(invalid("after must be a page number or 0"));
            }
            for v in [width, height] {
                if !v.is_finite() || !(36.0..=MAX_COORDINATE).contains(v) {
                    return Err(invalid("page sizes are 36..14400 pt"));
                }
            }
        }
        EditorOperation::InsertPages { after, pdf, pages } => {
            if *after > MAX_PAGE {
                return Err(invalid("after must be a page number or 0"));
            }
            for p in pages {
                page(*p)?;
            }
            *image_bytes += decode_pdf(pdf)?.len();
        }
        EditorOperation::AddAnnotation {
            page: p,
            kind,
            rect: r,
            quads,
            ink,
            contents,
            label,
            color: c,
            author,
        } => {
            page(*p)?;
            rect(r)?;
            color(c)?;
            rgb_only(c)?;
            for value in [contents, label, author].into_iter().flatten() {
                text(value, true)?;
                if value.chars().count() > 4_000 {
                    return Err(invalid("annotation text is limited to 4000 characters"));
                }
            }
            if quads.len() > 500 || ink.len() > 200 || ink.iter().map(Vec::len).sum::<usize>() > 20_000 {
                return Err(invalid("annotation geometry is too large"));
            }
            for quad in quads {
                for v in quad {
                    finite(*v, "quad")?;
                }
            }
            for stroke in ink {
                for p in stroke {
                    point(*p)?;
                }
            }
            match kind {
                AnnotationKind::Highlight | AnnotationKind::Underline | AnnotationKind::Strikeout if quads.is_empty() => {
                    return Err(invalid("text markup needs at least one quad"));
                }
                AnnotationKind::Ink if ink.iter().all(|stroke| stroke.len() < 2) => {
                    return Err(invalid("ink needs a stroke of two or more points"));
                }
                AnnotationKind::Stamp if label.as_deref().is_none_or(|l| l.trim().is_empty()) => {
                    return Err(invalid("a stamp needs a label"));
                }
                _ => {}
            }
        }
        EditorOperation::SetFormField { name, value, .. } => {
            if name.is_empty() || name.len() > 512 || name.chars().any(char::is_control) {
                return Err(invalid("field names are 1-512 printable characters"));
            }
            text(value, true)?;
        }
    }
    if *image_bytes > MAX_TOTAL_IMAGE_BYTES {
        return Err(invalid("images in one save are limited to 24 MiB"));
    }
    Ok(())
}
