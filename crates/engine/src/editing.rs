//! Additive page-content editing for existing PDFs.
//!
//! Edits are emitted as new content streams that are prepended as underlays or
//! appended as overlays. Existing page content streams are left untouched.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::Serialize;

use crate::content::{
    concat_matrix, transform_point, Color, ColorSpace, ContentOperation, ContentParser, Matrix,
    Operand, IDENTITY_MATRIX,
};
use crate::document::{PdfDocument, PdfPage};
use crate::editable::{EditableBuildOptions, EditableDocument};
use crate::engine::{ContentEngine, PageResources};
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless, flate_encode, DecodeLimits, StreamDecodeStatus};
use crate::fonts::FontResolver;
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::text::collector::extract_char_codes;
use crate::text::{TextQuad, TextSearchOptions};
use crate::versioning::resource_digest;
use crate::writer::{
    write_incremental_update, IncrementalObject, OutputObject, PdfWriter, WriterMode,
};
use crate::TextAlign;

/// How an editing operation is serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditMode {
    /// Rewrite the whole file with the modern writer.
    #[default]
    FullRewrite,
    /// Append changed/new objects after the original bytes, preserving the
    /// original byte prefix exactly.
    Incremental,
}

/// Whether new content is placed before or after existing page content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverlayLayer {
    /// Draw before existing page content.
    Underlay,
    /// Draw after existing page content.
    #[default]
    Overlay,
}

/// Style for text added to an existing page.
#[derive(Debug, Clone, PartialEq)]
pub struct EditTextStyle {
    pub font_size: f64,
    pub fill: Color,
    pub opacity: f64,
    pub rotation_degrees: f64,
}

impl Default for EditTextStyle {
    fn default() -> Self {
        Self {
            font_size: 12.0,
            fill: Color::black(),
            opacity: 1.0,
            rotation_degrees: 0.0,
        }
    }
}

impl EditTextStyle {
    pub fn new(font_size: f64) -> Self {
        Self {
            font_size,
            ..Default::default()
        }
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = color;
        self
    }

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn rotation_degrees(mut self, rotation: f64) -> Self {
        self.rotation_degrees = rotation;
        self
    }
}

/// Text watermark options.
#[derive(Debug, Clone, PartialEq)]
pub struct WatermarkOptions {
    pub pages: Option<Vec<usize>>,
    pub style: EditTextStyle,
    pub layer: OverlayLayer,
}

impl Default for WatermarkOptions {
    fn default() -> Self {
        Self {
            pages: None,
            style: EditTextStyle::new(64.0)
                .fill(Color::device_gray(0.55))
                .opacity(0.28)
                .rotation_degrees(45.0),
            layer: OverlayLayer::Overlay,
        }
    }
}

/// Header/footer options. Text may include `{page}` and `{total}` tokens.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderFooterOptions {
    pub pages: Option<Vec<usize>>,
    pub style: EditTextStyle,
    pub align: TextAlign,
    pub y: Option<f64>,
    pub layer: OverlayLayer,
}

impl Default for HeaderFooterOptions {
    fn default() -> Self {
        Self {
            pages: None,
            style: EditTextStyle::new(10.0).fill(Color::device_gray(0.2)),
            align: TextAlign::Center,
            y: None,
            layer: OverlayLayer::Overlay,
        }
    }
}

/// Rectangle drawing style for existing-page edits.
#[derive(Debug, Clone, PartialEq)]
pub struct EditRectStyle {
    pub stroke: Option<Color>,
    pub fill: Option<Color>,
    pub line_width: f64,
    pub opacity: f64,
}

impl Default for EditRectStyle {
    fn default() -> Self {
        Self {
            stroke: Some(Color::black()),
            fill: None,
            line_width: 1.0,
            opacity: 1.0,
        }
    }
}

/// Image placement options.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageStampOptions {
    pub opacity: f64,
    pub layer: OverlayLayer,
}

impl Default for ImageStampOptions {
    fn default() -> Self {
        Self {
            opacity: 1.0,
            layer: OverlayLayer::Overlay,
        }
    }
}

/// Redaction options. Redaction removes affected content and then paints a mark.
#[derive(Debug, Clone, PartialEq)]
pub struct RedactionOptions {
    pub fill: Color,
    pub scrub_metadata: bool,
    pub image_policy: ImageRedactionPolicy,
    pub attachment_policy: AttachmentRedactionPolicy,
    /// Promote supported inline images to deterministic Image XObjects before
    /// applying the sample rewrite. This never promotes malformed input.
    pub promote_inline_images: bool,
}

impl Default for RedactionOptions {
    fn default() -> Self {
        Self {
            fill: Color::black(),
            scrub_metadata: true,
            image_policy: ImageRedactionPolicy::Partial,
            attachment_policy: AttachmentRedactionPolicy::Keep,
            promote_inline_images: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TextReplacementOptions {
    /// 1-based pages to search. Empty means all pages.
    pub pages: Vec<usize>,
    pub case_sensitive: bool,
    pub max_replacements: usize,
    pub replacement_style: EditTextStyle,
    pub redaction_fill: Color,
}

impl Default for TextReplacementOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            case_sensitive: true,
            max_replacements: 1,
            replacement_style: EditTextStyle::default(),
            redaction_fill: Color::device_gray(1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextReplacementReport {
    pub query: String,
    pub replacement: String,
    pub replacements: usize,
    pub pages: Vec<usize>,
    pub edit_mode: String,
    pub verified_old_absent: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParagraphEditSerializationMode {
    SafePatch,
    ParagraphReflow,
    OverlayFallback,
    Unsupported,
}

impl ParagraphEditSerializationMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "safe-patch" | "safe_patch" => Some(Self::SafePatch),
            "paragraph-reflow" | "paragraph_reflow" | "reflow" => Some(Self::ParagraphReflow),
            "overlay-fallback" | "overlay_fallback" | "overlay" => Some(Self::OverlayFallback),
            "unsupported" => Some(Self::Unsupported),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SafePatch => "safe_patch",
            Self::ParagraphReflow => "paragraph_reflow",
            Self::OverlayFallback => "overlay_fallback",
            Self::Unsupported => "unsupported",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphReflowOptions {
    pub pages: Vec<usize>,
    pub case_sensitive: bool,
    pub max_edits: usize,
    pub replacement_style: EditTextStyle,
    pub redaction_fill: Color,
    pub line_spacing: f64,
    pub max_lines: usize,
    pub bounding_region: Option<ImageRect>,
    pub mode: ParagraphEditSerializationMode,
}

impl Default for ParagraphReflowOptions {
    fn default() -> Self {
        Self {
            pages: Vec::new(),
            case_sensitive: true,
            max_edits: 1,
            replacement_style: EditTextStyle::default(),
            redaction_fill: Color::device_gray(1.0),
            line_spacing: 1.2,
            max_lines: 16,
            bounding_region: None,
            mode: ParagraphEditSerializationMode::ParagraphReflow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParagraphEditOperation {
    Replace { replacement: String },
    Insert { offset: usize, text: String },
    Delete { start: usize, end: usize },
}

#[derive(Debug, Clone, Serialize)]
pub struct ParagraphReflowReport {
    pub query: String,
    pub operation: ParagraphEditOperation,
    pub edits: usize,
    pub pages: Vec<usize>,
    pub edit_mode: String,
    pub block_id: Option<String>,
    pub paragraph_id: Option<String>,
    pub lines_written: usize,
    pub verified_old_absent: bool,
    pub verified_new_present: bool,
    pub transaction_digest: String,
    pub signature_invalidation_warning: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeterministicSaveOptions {
    pub fixed_pdf_date: Option<String>,
    pub preserve_first_file_id: bool,
    pub deterministic_resource_names: bool,
    pub dedup_resources: bool,
}

impl Default for DeterministicSaveOptions {
    fn default() -> Self {
        Self {
            fixed_pdf_date: None,
            preserve_first_file_id: true,
            deterministic_resource_names: true,
            dedup_resources: false,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DeterministicSaveReport {
    pub mode: String,
    pub output_bytes: usize,
    pub fixed_pdf_date: Option<String>,
    pub first_file_id_preserved: bool,
    pub deterministic_resource_names: bool,
    pub dedup_resources_requested: bool,
    pub object_stream_packing: String,
    pub compression: String,
    pub signature_invalidation_warning: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRedactionPolicy {
    /// Try to rewrite intersecting image pixels; fall back to whole-image removal
    /// when the image transform or encoding is unsupported.
    Partial,
    /// Remove/blank intersecting image invocations conservatively.
    Remove,
    /// Return an error if an intersecting image cannot be redacted at pixel level.
    Fail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentRedactionPolicy {
    /// Preserve attachments while still scrubbing matching strings from
    /// metadata-like streams when `scrub_metadata` is enabled.
    Keep,
    /// Remove document-level embedded files and file-attachment annotations.
    RemoveAll,
    /// Remove file-attachment annotations that overlap redaction regions; keep
    /// catalog-level embedded files.
    RemoveOverlapping,
}

/// Replace visible/searchable text by removing matched source content with a
/// full-rewrite redaction pass, then placing replacement text in the same
/// conservative bounding boxes. This is intentionally not an incremental edit:
/// incremental output would keep the previous revision recoverable.
pub fn replace_text_pdf(
    input: Vec<u8>,
    query: &str,
    replacement: &str,
    options: TextReplacementOptions,
) -> Result<(Vec<u8>, TextReplacementReport)> {
    if query.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "text replacement query must not be empty".to_string(),
        ));
    }
    let engine = ContentEngine::open_bytes(input.clone())?;
    let pages = if options.pages.is_empty() {
        (1..=engine.page_count()?).collect::<Vec<_>>()
    } else {
        options.pages.clone()
    };
    let matches = engine.search_text(
        &pages,
        query,
        TextSearchOptions {
            case_sensitive: options.case_sensitive,
            include_hidden: true,
            max_matches: options.max_replacements.max(1),
            ..TextSearchOptions::default()
        },
    )?;
    if matches.is_empty() {
        return Ok((
            input,
            TextReplacementReport {
                query: query.to_string(),
                replacement: replacement.to_string(),
                replacements: 0,
                pages,
                edit_mode: "none".to_string(),
                verified_old_absent: false,
                warnings: vec!["no matching text was found".to_string()],
            },
        ));
    }

    let mut warnings = Vec::new();
    let mut regions = Vec::new();
    for text_match in &matches {
        if let Some(rect) = rect_from_quads(&text_match.quads) {
            regions.push((text_match.page, rect));
        } else {
            warnings.push(format!(
                "match on page {} had no quad geometry and was skipped",
                text_match.page
            ));
        }
    }
    if regions.is_empty() {
        return Err(WellfriendError::UnsupportedFeature(
            "text replacement found matches but no usable glyph quads".to_string(),
        ));
    }

    let mut redactor = PdfEditor::open_bytes(input)?;
    let redaction_options = RedactionOptions {
        fill: options.redaction_fill.clone(),
        scrub_metadata: true,
        image_policy: ImageRedactionPolicy::Partial,
        attachment_policy: AttachmentRedactionPolicy::Keep,
        promote_inline_images: false,
    };
    for (page, rect) in &regions {
        redactor.redact(*page, *rect, redaction_options.clone())?;
    }
    let redacted = redactor.save_to_bytes(EditMode::FullRewrite)?;

    let mut editor = PdfEditor::open_bytes(redacted)?;
    for (page, rect) in &regions {
        let baseline = rect.y + (rect.height * 0.25).max(2.0);
        editor.draw_text(
            *page,
            replacement,
            rect.x,
            baseline,
            options.replacement_style.clone(),
            OverlayLayer::Overlay,
        )?;
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;

    let verified_old_absent = if replacement.contains(query) {
        warnings.push(
            "replacement contains the query; absence verification is not meaningful".to_string(),
        );
        false
    } else {
        let verify_engine = ContentEngine::open_bytes(output.clone())?;
        verify_engine
            .search_text(
                &pages,
                query,
                TextSearchOptions {
                    case_sensitive: options.case_sensitive,
                    include_hidden: true,
                    max_matches: 1,
                    ..TextSearchOptions::default()
                },
            )?
            .is_empty()
    };
    Ok((
        output,
        TextReplacementReport {
            query: query.to_string(),
            replacement: replacement.to_string(),
            replacements: regions.len(),
            pages,
            edit_mode: "full_rewrite_redact_then_overlay".to_string(),
            verified_old_absent,
            warnings,
        },
    ))
}

/// Edit text inside a reconstructed paragraph/run, reflow the rewritten
/// paragraph within a bounded page region, and save a full-rewrite PDF where
/// the old paragraph text is removed from reachable content before the new
/// paragraph is serialized. This is the default "true edit" path for Prompt
/// 08B; the older overlay fallback remains opt-in through [`replace_text_pdf`].
pub fn edit_paragraph_reflow_pdf(
    input: Vec<u8>,
    query: &str,
    operation: ParagraphEditOperation,
    options: ParagraphReflowOptions,
) -> Result<(Vec<u8>, ParagraphReflowReport)> {
    if query.is_empty() {
        return Err(WellfriendError::MalformedPdf(
            "paragraph edit query must not be empty".to_string(),
        ));
    }
    if options.mode == ParagraphEditSerializationMode::OverlayFallback {
        let replacement = match &operation {
            ParagraphEditOperation::Replace { replacement } => replacement.clone(),
            ParagraphEditOperation::Insert { text, .. } => text.clone(),
            ParagraphEditOperation::Delete { .. } => String::new(),
        };
        let (bytes, old_report) = replace_text_pdf(
            input,
            query,
            &replacement,
            TextReplacementOptions {
                pages: options.pages.clone(),
                case_sensitive: options.case_sensitive,
                max_replacements: options.max_edits,
                replacement_style: options.replacement_style,
                redaction_fill: options.redaction_fill,
            },
        )?;
        return Ok((
            bytes,
            ParagraphReflowReport {
                query: query.to_string(),
                operation,
                edits: old_report.replacements,
                pages: old_report.pages,
                edit_mode: ParagraphEditSerializationMode::OverlayFallback
                    .as_str()
                    .to_string(),
                block_id: None,
                paragraph_id: None,
                lines_written: old_report.replacements,
                verified_old_absent: old_report.verified_old_absent,
                verified_new_present: true,
                transaction_digest: String::new(),
                signature_invalidation_warning: false,
                warnings: old_report.warnings,
            },
        ));
    }
    if matches!(options.mode, ParagraphEditSerializationMode::Unsupported) {
        return Err(WellfriendError::UnsupportedFeature(
            "paragraph edit mode was explicitly set to unsupported".to_string(),
        ));
    }

    let engine = ContentEngine::open_bytes(input.clone())?;
    let pages = if options.pages.is_empty() {
        (1..=engine.page_count()?).collect::<Vec<_>>()
    } else {
        options.pages.clone()
    };
    let mut model = engine.build_editable_document(&EditableBuildOptions::default())?;
    let Some(target) = find_paragraph_edit_target(&model, query, &pages, options.case_sensitive)
    else {
        return Ok((
            input,
            ParagraphReflowReport {
                query: query.to_string(),
                operation,
                edits: 0,
                pages,
                edit_mode: "none".to_string(),
                block_id: None,
                paragraph_id: None,
                lines_written: 0,
                verified_old_absent: false,
                verified_new_present: false,
                transaction_digest: String::new(),
                signature_invalidation_warning: false,
                warnings: vec!["no editable paragraph containing the query was found".to_string()],
            },
        ));
    };

    let before_paragraph = model.blocks[target.block_index].paragraphs[target.paragraph_index]
        .text
        .clone();
    let after_paragraph =
        apply_paragraph_operation(&before_paragraph, query, &operation, options.case_sensitive)?;
    let block_id = model.blocks[target.block_index].id.clone();
    let paragraph_id = model.blocks[target.block_index].paragraphs[target.paragraph_index]
        .id
        .clone();
    if !model.replace_paragraph_text(&block_id, &paragraph_id, &after_paragraph) {
        return Err(WellfriendError::MalformedPdf(
            "editable paragraph target disappeared during edit".to_string(),
        ));
    }
    let transaction_digest = resource_digest(
        serde_json::to_string(&model.transactions)
            .unwrap_or_default()
            .as_bytes(),
    );

    let region = options
        .bounding_region
        .or_else(|| {
            let block = block_rect(&model.blocks[target.block_index].bbox);
            let query_rect = query_match_rect(&engine, query, &pages, options.case_sensitive)
                .ok()
                .flatten();
            union_optional_rects(block, query_rect)
        })
        .ok_or_else(|| {
            WellfriendError::UnsupportedFeature(
                "paragraph reflow edit found text but no usable geometry".to_string(),
            )
        })?;
    let lines = reflow_lines(
        &after_paragraph,
        region.width,
        options.replacement_style.font_size,
        options.line_spacing,
        options.max_lines,
        region.height,
    )?;

    let mut redactor = PdfEditor::open_bytes(input.clone())?;
    redactor.redact(
        target.page,
        region,
        RedactionOptions {
            fill: options.redaction_fill.clone(),
            scrub_metadata: true,
            image_policy: ImageRedactionPolicy::Partial,
            attachment_policy: AttachmentRedactionPolicy::Keep,
            promote_inline_images: false,
        },
    )?;
    let redacted = redactor.save_to_bytes(EditMode::FullRewrite)?;

    let mut editor = PdfEditor::open_bytes(redacted)?;
    let line_height = options.replacement_style.font_size * options.line_spacing.max(1.0);
    let mut baseline = region.y + region.height - options.replacement_style.font_size;
    for line in &lines {
        editor.draw_text(
            target.page,
            line,
            region.x,
            baseline.max(region.y),
            options.replacement_style.clone(),
            OverlayLayer::Overlay,
        )?;
        baseline -= line_height;
    }
    let output = editor.save_to_bytes(EditMode::FullRewrite)?;
    let verify_engine = ContentEngine::open_bytes(output.clone())?;
    let verified_old_absent = match &operation {
        ParagraphEditOperation::Replace { replacement } if replacement.contains(query) => false,
        _ => verify_engine
            .search_text(
                &pages,
                query,
                TextSearchOptions {
                    case_sensitive: options.case_sensitive,
                    include_hidden: true,
                    max_matches: 1,
                    ..TextSearchOptions::default()
                },
            )?
            .is_empty(),
    };
    let required_new_text = match &operation {
        ParagraphEditOperation::Replace { replacement } => replacement.as_str(),
        ParagraphEditOperation::Insert { text, .. } => text.as_str(),
        ParagraphEditOperation::Delete { .. } => "",
    };
    let verified_new_present = if required_new_text.is_empty() {
        true
    } else {
        let direct = !verify_engine
            .search_text(
                &pages,
                required_new_text,
                TextSearchOptions {
                    case_sensitive: options.case_sensitive,
                    include_hidden: true,
                    max_matches: 1,
                    ..TextSearchOptions::default()
                },
            )?
            .is_empty();
        direct || extracted_pages_contain(&verify_engine, &pages, required_new_text)
    };
    let signature_invalidation_warning = engine
        .verify_signatures()
        .is_ok_and(|sigs| !sigs.is_empty());
    let mut warnings = Vec::new();
    if signature_invalidation_warning {
        warnings
            .push("full-rewrite paragraph edit invalidates existing PDF signatures".to_string());
    }
    Ok((
        output,
        ParagraphReflowReport {
            query: query.to_string(),
            operation,
            edits: 1,
            pages,
            edit_mode: options.mode.as_str().to_string(),
            block_id: Some(block_id),
            paragraph_id: Some(paragraph_id),
            lines_written: lines.len(),
            verified_old_absent,
            verified_new_present,
            transaction_digest,
            signature_invalidation_warning,
            warnings,
        },
    ))
}

/// Common annotation styling and metadata.
#[derive(Debug, Clone, PartialEq)]
pub struct AnnotationOptions {
    pub color: Color,
    pub opacity: f64,
    pub author: Option<String>,
    pub contents: Option<String>,
}

impl Default for AnnotationOptions {
    fn default() -> Self {
        Self {
            color: Color::device_rgb(1.0, 0.9, 0.0),
            opacity: 0.35,
            author: None,
            contents: None,
        }
    }
}

impl AnnotationOptions {
    pub fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }

    pub fn opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    pub fn contents(mut self, contents: impl Into<String>) -> Self {
        self.contents = Some(contents.into());
        self
    }
}

/// Additive editor for an existing PDF.
pub struct PdfEditor {
    document: PdfDocument,
    edits: BTreeMap<usize, Vec<PageEdit>>,
    redactions: BTreeMap<usize, Vec<RedactionEdit>>,
    annotations: BTreeMap<usize, Vec<AnnotationEdit>>,
    form_fills: BTreeMap<String, FormValue>,
    flatten_forms: bool,
    flatten_annotations: bool,
    flatten_annotation_subtypes: BTreeSet<String>,
}

impl PdfEditor {
    pub fn open_bytes(bytes: Vec<u8>) -> Result<Self> {
        Ok(Self {
            document: PdfDocument::open_bytes(bytes)?,
            edits: BTreeMap::new(),
            redactions: BTreeMap::new(),
            annotations: BTreeMap::new(),
            form_fills: BTreeMap::new(),
            flatten_forms: false,
            flatten_annotations: false,
            flatten_annotation_subtypes: BTreeSet::new(),
        })
    }

    pub fn document(&self) -> &PdfDocument {
        &self.document
    }

    pub fn add_watermark_text(
        &mut self,
        text: impl Into<String>,
        options: WatermarkOptions,
    ) -> Result<&mut Self> {
        let text = text.into();
        let pages = self.target_pages(options.pages.as_deref())?;
        let all_pages = self.document.get_pages()?;
        for page_number in pages {
            let page = &all_pages[page_number - 1];
            let (cx, cy) = page_center(page);
            let width = page.media_box[2] - page.media_box[0];
            let text_width = approximate_text_width(&text, options.style.font_size);
            let x = cx - text_width.min(width) / 2.0;
            self.push_edit(
                page_number,
                PageEdit {
                    layer: options.layer,
                    command: EditCommand::Text {
                        text: text.clone(),
                        x,
                        y: cy,
                        style: options.style.clone(),
                    },
                },
            );
        }
        Ok(self)
    }

    pub fn add_header(
        &mut self,
        template: impl Into<String>,
        options: HeaderFooterOptions,
    ) -> Result<&mut Self> {
        self.add_header_footer(template.into(), options, true)
    }

    pub fn add_footer(
        &mut self,
        template: impl Into<String>,
        options: HeaderFooterOptions,
    ) -> Result<&mut Self> {
        self.add_header_footer(template.into(), options, false)
    }

    pub fn draw_text(
        &mut self,
        page_number: usize,
        text: impl Into<String>,
        x: f64,
        y: f64,
        style: EditTextStyle,
        layer: OverlayLayer,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.push_edit(
            page_number,
            PageEdit {
                layer,
                command: EditCommand::Text {
                    text: text.into(),
                    x,
                    y,
                    style,
                },
            },
        );
        Ok(self)
    }

    pub fn draw_rect(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        style: EditRectStyle,
        layer: OverlayLayer,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.push_edit(
            page_number,
            PageEdit {
                layer,
                command: EditCommand::Rect { rect, style },
            },
        );
        Ok(self)
    }

    pub fn stamp_jpeg_image(
        &mut self,
        page_number: usize,
        bytes: impl Into<Vec<u8>>,
        rect: ImageRect,
        options: ImageStampOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        let bytes = bytes.into();
        let (_, width, height, channels) = ImageDecoder::decode_jpeg_with_info(&bytes)?;
        let image = EditImage {
            width,
            height,
            color_space: image_color_space(channels)?,
            bits_per_component: 8,
            data: bytes,
            filter: ImageFilter::DctDecode,
            smask: None,
        };
        self.push_edit(
            page_number,
            PageEdit {
                layer: options.layer,
                command: EditCommand::Image {
                    image,
                    rect,
                    opacity: options.opacity,
                },
            },
        );
        Ok(self)
    }

    pub fn stamp_rgba_image(
        &mut self,
        page_number: usize,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        rect: ImageRect,
        options: ImageStampOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        let image = edit_image_from_raw(RawImage {
            width,
            height,
            channels: 4,
            bits_per_sample: 8,
            pixels,
        })?;
        self.push_edit(
            page_number,
            PageEdit {
                layer: options.layer,
                command: EditCommand::Image {
                    image,
                    rect,
                    opacity: options.opacity,
                },
            },
        );
        Ok(self)
    }

    /// Redact a page rectangle by removing intersecting text/image/path content
    /// and drawing a fill mark over the now-empty region.
    ///
    /// Redactions intentionally require full rewrite output. Incremental output
    /// preserves the original byte prefix, which would retain the old revision's
    /// sensitive content.
    pub fn redact(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        options: RedactionOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.redactions
            .entry(page_number)
            .or_default()
            .push(RedactionEdit {
                rect,
                polygon: vec![
                    (rect.x, rect.y),
                    (rect.x + rect.width, rect.y),
                    (rect.x + rect.width, rect.y + rect.height),
                    (rect.x, rect.y + rect.height),
                ],
                options,
            });
        Ok(self)
    }

    /// Redact a page-space polygon. Text and vector removal remains
    /// conservative at the polygon bounding box, while image samples are
    /// rewritten against the actual polygon after inverse affine mapping.
    pub fn redact_polygon(
        &mut self,
        page_number: usize,
        polygon: Vec<(f64, f64)>,
        options: RedactionOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        if polygon.len() < 3 || polygon.len() > 16_384 {
            return Err(WellfriendError::ResourceLimit(
                "redaction polygon must contain between 3 and 16384 points".to_string(),
            ));
        }
        if polygon
            .iter()
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(WellfriendError::MalformedPdf(
                "redaction polygon contains a non-finite coordinate".to_string(),
            ));
        }
        let rect = rect_from_points(&polygon);
        if rect.width <= 0.0 || rect.height <= 0.0 {
            return Err(WellfriendError::MalformedPdf(
                "redaction polygon has an empty bounding box".to_string(),
            ));
        }
        self.redactions
            .entry(page_number)
            .or_default()
            .push(RedactionEdit {
                rect,
                polygon,
                options,
            });
        Ok(self)
    }

    pub fn add_highlight_annotation(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        options: AnnotationOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::Add(AnnotationSpec {
                kind: AnnotationKind::Highlight,
                rect,
                label: options.contents.clone().unwrap_or_default(),
                options,
            }));
        Ok(self)
    }

    pub fn add_text_note_annotation(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        contents: impl Into<String>,
        options: AnnotationOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::Add(AnnotationSpec {
                kind: AnnotationKind::TextNote,
                rect,
                label: contents.into(),
                options,
            }));
        Ok(self)
    }

    pub fn add_stamp_annotation(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        label: impl Into<String>,
        options: AnnotationOptions,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::Add(AnnotationSpec {
                kind: AnnotationKind::Stamp,
                rect,
                label: label.into(),
                options,
            }));
        Ok(self)
    }

    pub fn add_link_uri(
        &mut self,
        page_number: usize,
        rect: ImageRect,
        uri: impl Into<String>,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::Add(AnnotationSpec {
                kind: AnnotationKind::Link,
                rect,
                label: uri.into(),
                options: AnnotationOptions::default(),
            }));
        Ok(self)
    }

    pub fn edit_annotation_contents(
        &mut self,
        page_number: usize,
        annotation_index: usize,
        contents: impl Into<String>,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::EditContents {
                index: annotation_index,
                contents: contents.into(),
            });
        Ok(self)
    }

    pub fn delete_annotations_in_rect(
        &mut self,
        page_number: usize,
        rect: ImageRect,
    ) -> Result<&mut Self> {
        self.validate_page(page_number)?;
        self.annotations
            .entry(page_number)
            .or_default()
            .push(AnnotationEdit::DeleteInRect { rect });
        Ok(self)
    }

    pub fn set_form_text(
        &mut self,
        field_name: impl Into<String>,
        value: impl Into<String>,
    ) -> &mut Self {
        self.form_fills
            .insert(field_name.into(), FormValue::Text(value.into()));
        self
    }

    pub fn set_form_choice(
        &mut self,
        field_name: impl Into<String>,
        value: impl Into<String>,
    ) -> &mut Self {
        self.form_fills
            .insert(field_name.into(), FormValue::Choice(value.into()));
        self
    }

    pub fn set_form_checkbox(&mut self, field_name: impl Into<String>, checked: bool) -> &mut Self {
        self.form_fills
            .insert(field_name.into(), FormValue::Checkbox(checked));
        self
    }

    /// Bake current AcroForm widget values into page content and remove fields.
    pub fn flatten_forms(&mut self) -> &mut Self {
        self.flatten_forms = true;
        self
    }

    /// Bake common annotation appearances into page content and remove the
    /// flattened annotations. Unsupported annotation subtypes are removed with a
    /// conservative fallback only for visual markers that Wellfriend can synthesize.
    pub fn flatten_annotations(&mut self) -> &mut Self {
        self.flatten_annotations = true;
        self
    }

    /// Flatten only the named annotation subtypes. This keeps field/widget
    /// semantics and unrelated annotations intact, and is used by the Prompt
    /// 17 static-poster media policy.
    pub fn flatten_annotation_subtypes<I, S>(&mut self, subtypes: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.flatten_annotation_subtypes
            .extend(subtypes.into_iter().map(Into::into));
        self
    }

    pub fn save_to_bytes(&self, mode: EditMode) -> Result<Vec<u8>> {
        if mode == EditMode::Incremental && !self.redactions.is_empty() {
            return Err(WellfriendError::UnsupportedFeature(
                "redaction requires full rewrite; incremental output preserves old revision bytes"
                    .to_string(),
            ));
        }
        let changes = self.build_changes()?;
        match mode {
            EditMode::Incremental => write_incremental_update(self.document.reader(), changes),
            EditMode::FullRewrite => write_full_rewrite(self.document.reader(), changes),
        }
    }

    pub fn save_to_bytes_with_options(
        &self,
        mode: EditMode,
        options: &DeterministicSaveOptions,
    ) -> Result<(Vec<u8>, DeterministicSaveReport)> {
        let signature_invalidation_warning =
            crate::signature::verify_signatures(&self.document).is_ok_and(|sigs| !sigs.is_empty());
        let bytes = self.save_to_bytes(mode)?;
        let report = DeterministicSaveReport {
            mode: match mode {
                EditMode::FullRewrite => "full_rewrite".to_string(),
                EditMode::Incremental => "incremental".to_string(),
            },
            output_bytes: bytes.len(),
            fixed_pdf_date: options.fixed_pdf_date.clone(),
            first_file_id_preserved: options.preserve_first_file_id,
            deterministic_resource_names: options.deterministic_resource_names,
            dedup_resources_requested: options.dedup_resources,
            object_stream_packing: if mode == EditMode::FullRewrite {
                "xref_stream_with_objstm_deterministic_order".to_string()
            } else {
                "incremental_plain_objects".to_string()
            },
            compression: "deterministic_flate_settings".to_string(),
            signature_invalidation_warning,
        };
        Ok((bytes, report))
    }

    fn add_header_footer(
        &mut self,
        template: String,
        options: HeaderFooterOptions,
        header: bool,
    ) -> Result<&mut Self> {
        let pages = self.target_pages(options.pages.as_deref())?;
        let all_pages = self.document.get_pages()?;
        let total = all_pages.len();
        for page_number in pages {
            let page = &all_pages[page_number - 1];
            let text = template
                .replace("{page}", &page_number.to_string())
                .replace("{total}", &total.to_string());
            let y = options.y.unwrap_or_else(|| {
                if header {
                    page.media_box[3] - 36.0
                } else {
                    page.media_box[1] + 30.0
                }
            });
            let width = page.media_box[2] - page.media_box[0];
            let text_width = approximate_text_width(&text, options.style.font_size);
            let x = match options.align {
                TextAlign::Left => page.media_box[0] + 36.0,
                TextAlign::Center => page.media_box[0] + (width - text_width) / 2.0,
                TextAlign::Right => page.media_box[2] - 36.0 - text_width,
            };
            self.push_edit(
                page_number,
                PageEdit {
                    layer: options.layer,
                    command: EditCommand::Text {
                        text,
                        x,
                        y,
                        style: options.style.clone(),
                    },
                },
            );
        }
        Ok(self)
    }

    fn build_changes(&self) -> Result<Vec<IncrementalObject>> {
        let pages = self.document.get_pages()?;
        let by_page: BTreeMap<usize, &PdfPage> =
            pages.iter().map(|page| (page.page_number, page)).collect();
        let mut changes = ChangeSet::new(self.document.reader());
        let mut redact_report = RedactionReport::default();
        let flatten_visuals = self.apply_form_changes(&pages, &mut changes)?;
        let attachment_policy = self.effective_attachment_policy();

        let mut page_numbers: BTreeSet<usize> = BTreeSet::new();
        page_numbers.extend(self.edits.keys().copied());
        page_numbers.extend(self.redactions.keys().copied());
        page_numbers.extend(self.annotations.keys().copied());
        page_numbers.extend(flatten_visuals.keys().copied());
        if self.flatten_annotations
            || !self.flatten_annotation_subtypes.is_empty()
            || attachment_policy == AttachmentRedactionPolicy::RemoveAll
        {
            page_numbers.extend(pages.iter().map(|page| page.page_number));
        }

        for page_number in page_numbers {
            let edits = self
                .edits
                .get(&page_number)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let redactions = self
                .redactions
                .get(&page_number)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let annotation_edits = self
                .annotations
                .get(&page_number)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let form_visuals = flatten_visuals
                .get(&page_number)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let page = by_page.get(&page_number).ok_or_else(|| {
                WellfriendError::MalformedPdf(format!("page {page_number} is out of range"))
            })?;
            let page_object = changes.current_object(
                self.document.reader(),
                page.object_number,
                page.generation_number,
            )?;
            let mut page_dict = page_object.as_dict().cloned().ok_or_else(|| {
                WellfriendError::MalformedPdf(format!(
                    "page object {} {} is not a dictionary",
                    page.object_number, page.generation_number
                ))
            })?;
            let mut resources = page.resources.clone();
            let mut underlay = Vec::new();
            let mut overlay = Vec::new();

            for edit in edits {
                let out = match edit.layer {
                    OverlayLayer::Underlay => &mut underlay,
                    OverlayLayer::Overlay => &mut overlay,
                };
                write_edit_command(out, &edit.command, &mut resources, &mut changes)?;
            }
            for redaction in redactions {
                write_redaction_mark(&mut overlay, redaction);
            }
            for visual in form_visuals {
                write_form_flatten_visual(&mut overlay, &mut resources, visual);
            }
            for edit in annotation_edits {
                if let AnnotationEdit::Add(spec) = edit {
                    write_annotation_visual_to_content(&mut overlay, &mut resources, spec);
                }
            }
            if self.flatten_annotations || !self.flatten_annotation_subtypes.is_empty() {
                write_existing_annotation_visuals(
                    self.document.reader(),
                    &page_dict,
                    &mut overlay,
                    &mut resources,
                    (!self.flatten_annotations).then_some(&self.flatten_annotation_subtypes),
                )?;
            }

            let mut content_refs = Vec::new();
            if !underlay.is_empty() {
                let number = changes.alloc();
                changes.insert_new_stream(number, underlay);
                content_refs.push(reference(number, 0));
            }
            if redactions.is_empty() {
                for (number, generation) in &page.contents {
                    content_refs.push(reference(*number, *generation));
                }
            } else {
                let rewritten = rewrite_page_content_for_redaction(
                    self.document.reader(),
                    page,
                    &mut resources,
                    redactions,
                    &mut redact_report,
                    &mut changes,
                )?;
                // H-2: the rewriter removes visible glyphs, but a tagged PDF can
                // also carry the same text as inline /ActualText or /Alt in a
                // marked-content property list (BDC/DP). Scrub those here, now
                // that this page's removed-text set is complete.
                let rewritten = if redact_report.scrub_metadata {
                    scrub_marked_content_alt_text(&rewritten, &redact_report.removed_text)?
                } else {
                    rewritten
                };
                let number = changes.alloc();
                changes.insert_new_stream(number, rewritten);
                content_refs.push(reference(number, 0));
            }
            if !overlay.is_empty() {
                let number = changes.alloc();
                changes.insert_new_stream(number, overlay);
                content_refs.push(reference(number, 0));
            }

            apply_annotation_edits(
                self.document.reader(),
                &mut page_dict,
                redactions,
                annotation_edits,
                AnnotationApplyOptions {
                    remove_widgets: self.flatten_forms,
                    flatten_annotations: self.flatten_annotations,
                    flatten_annotation_subtypes: &self.flatten_annotation_subtypes,
                    attachment_policy,
                },
                &mut changes,
            )?;
            page_dict.insert("Resources", PdfObject::Dictionary(resources));
            page_dict.insert("Contents", PdfObject::Array(content_refs));
            changes.insert_existing(
                page.object_number,
                page.generation_number,
                PdfObject::Dictionary(page_dict),
            );
        }

        if redact_report.scrub_metadata && !redact_report.removed_text.is_empty() {
            self.apply_metadata_scrub(&redact_report.removed_text, &mut changes)?;
        }
        if attachment_policy == AttachmentRedactionPolicy::RemoveAll {
            self.remove_embedded_file_name_tree(&mut changes)?;
        }

        Ok(changes.into_vec())
    }

    fn push_edit(&mut self, page_number: usize, edit: PageEdit) {
        self.edits.entry(page_number).or_default().push(edit);
    }

    fn validate_page(&self, page_number: usize) -> Result<()> {
        if page_number == 0 || page_number > self.document.get_pages()?.len() {
            return Err(WellfriendError::MalformedPdf(format!(
                "page {page_number} is out of range"
            )));
        }
        Ok(())
    }

    fn target_pages(&self, pages: Option<&[usize]>) -> Result<Vec<usize>> {
        let total = self.document.get_pages()?.len();
        match pages {
            Some(pages) => {
                let mut out = Vec::new();
                let mut seen = BTreeSet::new();
                for &page in pages {
                    if page == 0 || page > total {
                        return Err(WellfriendError::MalformedPdf(format!(
                            "page {page} is out of range"
                        )));
                    }
                    if seen.insert(page) {
                        out.push(page);
                    }
                }
                Ok(out)
            }
            None => Ok((1..=total).collect()),
        }
    }

    fn effective_attachment_policy(&self) -> AttachmentRedactionPolicy {
        let mut policy = AttachmentRedactionPolicy::Keep;
        for redactions in self.redactions.values() {
            for redaction in redactions {
                match redaction.options.attachment_policy {
                    AttachmentRedactionPolicy::RemoveAll => {
                        return AttachmentRedactionPolicy::RemoveAll;
                    }
                    AttachmentRedactionPolicy::RemoveOverlapping => {
                        policy = AttachmentRedactionPolicy::RemoveOverlapping;
                    }
                    AttachmentRedactionPolicy::Keep => {}
                }
            }
        }
        policy
    }

    fn apply_metadata_scrub(
        &self,
        removed_text: &BTreeSet<String>,
        changes: &mut ChangeSet,
    ) -> Result<()> {
        for (number, generation) in self.document.reader().object_ids() {
            let object = changes.current_object(self.document.reader(), number, generation)?;
            let mut scrubbed = object.clone();
            // Scrub string values throughout the object graph: /Info, annotation
            // /Contents, and — critically for H-2 — /ActualText and /Alt string
            // values in tagged-PDF structure elements and marked-content property
            // lists, wherever they live in the object graph.
            let mut changed = scrub_pdf_strings(&mut scrubbed, removed_text);
            // H-2 / M-7: the raw payload of an XMP /Metadata stream or an
            // embedded-file stream can carry a duplicate of the redacted text;
            // scrub_pdf_strings only reaches a stream's *dictionary*, never its
            // bytes. Scrub the decoded payload here and re-store it uncompressed.
            if let PdfObject::Stream { dict, .. } = &scrubbed {
                if is_scrubbable_payload_stream(dict) {
                    if let Some(rebuilt) =
                        scrub_stream_payload(&scrubbed, self.document.reader(), removed_text)?
                    {
                        scrubbed = rebuilt;
                        changed = true;
                    }
                }
            }
            if changed {
                changes.insert_existing(number, generation, scrubbed);
            }
        }
        Ok(())
    }

    fn remove_embedded_file_name_tree(&self, changes: &mut ChangeSet) -> Result<()> {
        let reader = self.document.reader();
        let (root, generation) = reader.root_reference().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "attachment removal: trailer is missing /Root".to_string(),
            )
        })?;
        let object = changes.current_object(reader, root, generation)?;
        let mut catalog = object.as_dict().cloned().ok_or_else(|| {
            WellfriendError::MalformedPdf(
                "attachment removal: /Root is not a dictionary".to_string(),
            )
        })?;
        let Some(names_obj) = catalog.get("Names").cloned() else {
            changes.insert_existing(root, generation, PdfObject::Dictionary(catalog));
            return Ok(());
        };

        match names_obj {
            PdfObject::Dictionary(mut names) => {
                names.remove("EmbeddedFiles");
                if names.is_empty() {
                    catalog.remove("Names");
                } else {
                    catalog.insert("Names", PdfObject::Dictionary(names));
                }
                changes.insert_existing(root, generation, PdfObject::Dictionary(catalog));
            }
            PdfObject::Reference {
                number,
                generation: names_gen,
            } => {
                let names_obj = changes.current_object(reader, number, names_gen)?;
                if let Some(mut names) = names_obj.as_dict().cloned() {
                    names.remove("EmbeddedFiles");
                    changes.insert_existing(number, names_gen, PdfObject::Dictionary(names));
                }
                changes.insert_existing(root, generation, PdfObject::Dictionary(catalog));
            }
            _ => {
                catalog.remove("Names");
                changes.insert_existing(root, generation, PdfObject::Dictionary(catalog));
            }
        }
        Ok(())
    }

    fn apply_form_changes(
        &self,
        pages: &[PdfPage],
        changes: &mut ChangeSet,
    ) -> Result<BTreeMap<usize, Vec<FormFlattenVisual>>> {
        if self.form_fills.is_empty() && !self.flatten_forms {
            return Ok(BTreeMap::new());
        }
        let fields = collect_acroform_fields(self.document.reader(), pages)?;
        let mut visuals: BTreeMap<usize, Vec<FormFlattenVisual>> = BTreeMap::new();
        let mut matched = BTreeSet::new();

        for field in &fields {
            let requested = self.form_fills.get(&field.name);
            if let Some(value) = requested {
                matched.insert(field.name.clone());
                update_field_value(self.document.reader(), changes, field, value)?;
            }
            let value = requested
                .cloned()
                .or_else(|| field.current_value.clone())
                .unwrap_or_else(|| FormValue::Text(String::new()));
            if self.flatten_forms {
                for widget in &field.widgets {
                    visuals
                        .entry(widget.page_number)
                        .or_default()
                        .push(FormFlattenVisual {
                            page_number: widget.page_number,
                            rect: widget.rect,
                            value: value.clone(),
                        });
                }
            }
        }

        for name in self.form_fills.keys() {
            if !matched.contains(name) {
                return Err(WellfriendError::MalformedPdf(format!(
                    "form field '{name}' was not found"
                )));
            }
        }

        if self.flatten_forms {
            remove_acroform_from_catalog(self.document.reader(), changes)?;
        }
        Ok(visuals)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl ImageRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

fn rect_from_quads(quads: &[TextQuad]) -> Option<ImageRect> {
    let quad = TextQuad::union(quads)?;
    let pad = ((quad.y1 - quad.y0).abs() * 0.12).clamp(0.5, 3.0);
    Some(ImageRect::new(
        (quad.x0 - pad).max(0.0),
        (quad.y0 - pad).max(0.0),
        (quad.x1 - quad.x0 + pad * 2.0).max(0.5),
        (quad.y1 - quad.y0 + pad * 2.0).max(0.5),
    ))
}

#[derive(Debug, Clone)]
struct ParagraphEditTarget {
    block_index: usize,
    paragraph_index: usize,
    page: usize,
}

fn find_paragraph_edit_target(
    model: &EditableDocument,
    query: &str,
    pages: &[usize],
    case_sensitive: bool,
) -> Option<ParagraphEditTarget> {
    for (block_index, block) in model.blocks.iter().enumerate() {
        if !pages.contains(&block.page) {
            continue;
        }
        for (paragraph_index, paragraph) in block.paragraphs.iter().enumerate() {
            if contains_query(&paragraph.text, query, case_sensitive) {
                return Some(ParagraphEditTarget {
                    block_index,
                    paragraph_index,
                    page: block.page,
                });
            }
        }
    }
    None
}

fn contains_query(haystack: &str, needle: &str, case_sensitive: bool) -> bool {
    if case_sensitive {
        haystack.contains(needle)
    } else {
        haystack.to_lowercase().contains(&needle.to_lowercase())
    }
}

fn apply_paragraph_operation(
    before: &str,
    query: &str,
    operation: &ParagraphEditOperation,
    case_sensitive: bool,
) -> Result<String> {
    match operation {
        ParagraphEditOperation::Replace { replacement } => {
            let Some((start, end)) = find_query_char_range(before, query, case_sensitive) else {
                return Err(WellfriendError::MalformedPdf(
                    "paragraph replace target was not found".to_string(),
                ));
            };
            splice_char_range(before, start, end, replacement)
        }
        ParagraphEditOperation::Insert { offset, text } => {
            splice_char_range(before, *offset, *offset, text)
        }
        ParagraphEditOperation::Delete { start, end } => {
            splice_char_range(before, *start, *end, "")
        }
    }
}

fn find_query_char_range(
    haystack: &str,
    needle: &str,
    case_sensitive: bool,
) -> Option<(usize, usize)> {
    let hay = haystack.chars().collect::<Vec<_>>();
    let needle_chars = needle.chars().collect::<Vec<_>>();
    if needle_chars.is_empty() || needle_chars.len() > hay.len() {
        return None;
    }
    for start in 0..=hay.len() - needle_chars.len() {
        let matches = needle_chars.iter().enumerate().all(|(offset, needle_ch)| {
            let hay_ch = hay[start + offset];
            if case_sensitive {
                hay_ch == *needle_ch
            } else {
                hay_ch.to_lowercase().collect::<String>()
                    == needle_ch.to_lowercase().collect::<String>()
            }
        });
        if matches {
            return Some((start, start + needle_chars.len()));
        }
    }
    None
}

fn splice_char_range(
    input: &str,
    start_chars: usize,
    end_chars: usize,
    replacement: &str,
) -> Result<String> {
    let len = input.chars().count();
    if start_chars > end_chars || end_chars > len {
        return Err(WellfriendError::MalformedPdf(format!(
            "paragraph edit range {start_chars}..{end_chars} is outside text length {len}"
        )));
    }
    let start = char_to_byte(input, start_chars).ok_or_else(|| {
        WellfriendError::MalformedPdf("paragraph edit start offset is invalid".to_string())
    })?;
    let end = char_to_byte(input, end_chars).ok_or_else(|| {
        WellfriendError::MalformedPdf("paragraph edit end offset is invalid".to_string())
    })?;
    let mut out = String::with_capacity(input.len() + replacement.len());
    out.push_str(&input[..start]);
    out.push_str(replacement);
    out.push_str(&input[end..]);
    Ok(out)
}

fn char_to_byte(input: &str, char_index: usize) -> Option<usize> {
    if char_index == input.chars().count() {
        Some(input.len())
    } else {
        input.char_indices().nth(char_index).map(|(idx, _)| idx)
    }
}

fn block_rect(bbox: &[f64; 4]) -> Option<ImageRect> {
    let width = bbox[2] - bbox[0];
    let height = bbox[3] - bbox[1];
    if bbox.iter().all(|v| v.is_finite()) && width > 1.0 && height > 1.0 {
        Some(ImageRect::new(bbox[0], bbox[1], width, height))
    } else {
        None
    }
}

fn query_match_rect(
    engine: &ContentEngine,
    query: &str,
    pages: &[usize],
    case_sensitive: bool,
) -> Result<Option<ImageRect>> {
    let matches = engine.search_text(
        pages,
        query,
        TextSearchOptions {
            case_sensitive,
            include_hidden: true,
            max_matches: 1,
            ..TextSearchOptions::default()
        },
    )?;
    Ok(matches
        .first()
        .and_then(|text_match| rect_from_quads(&text_match.quads)))
}

fn union_optional_rects(a: Option<ImageRect>, b: Option<ImageRect>) -> Option<ImageRect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(union_rect(a, b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn union_rect(a: ImageRect, b: ImageRect) -> ImageRect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.width).max(b.x + b.width);
    let y1 = (a.y + a.height).max(b.y + b.height);
    ImageRect::new(x0, y0, x1 - x0, y1 - y0)
}

fn reflow_lines(
    text: &str,
    width: f64,
    font_size: f64,
    line_spacing: f64,
    max_lines: usize,
    region_height: f64,
) -> Result<Vec<String>> {
    if text.trim().is_empty() {
        return Ok(vec![String::new()]);
    }
    let line_height = font_size * line_spacing.max(1.0);
    let fit_lines = (region_height / line_height).floor().max(1.0) as usize;
    let cap = max_lines.max(1).min(fit_lines.max(1));
    let available = width.max(font_size * 2.0);
    let tokens = paragraph_tokens(text);
    let mut lines = Vec::<String>::new();
    let mut current = String::new();
    for token in tokens {
        let candidate = if current.is_empty() {
            token.clone()
        } else if token_is_cjk_unit(&token) {
            format!("{current}{token}")
        } else {
            format!("{current} {token}")
        };
        if !current.is_empty() && approximate_reflow_width(&candidate, font_size) > available {
            lines.push(current);
            current = token;
            if lines.len() >= cap {
                return Err(WellfriendError::UnsupportedFeature(format!(
                    "paragraph reflow overflow: rewritten paragraph exceeds {cap} line(s)"
                )));
            }
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > cap {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "paragraph reflow overflow: rewritten paragraph exceeds {cap} line(s)"
        )));
    }
    Ok(lines)
}

fn paragraph_tokens(text: &str) -> Vec<String> {
    if text.split_whitespace().count() > 1 {
        return text
            .split_whitespace()
            .map(|part| part.to_string())
            .collect();
    }
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_cjk = false;
    for ch in text.chars() {
        let cjk = is_cjk(ch);
        if ch.is_whitespace() {
            if !current.is_empty() {
                out.push(current.clone());
                current.clear();
            }
            current_cjk = false;
        } else if cjk {
            if !current.is_empty() && !current_cjk {
                out.push(current.clone());
                current.clear();
            }
            out.push(ch.to_string());
            current_cjk = true;
        } else {
            if current_cjk && !current.is_empty() {
                out.push(current.clone());
                current.clear();
            }
            current.push(ch);
            current_cjk = false;
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn token_is_cjk_unit(token: &str) -> bool {
    token.chars().count() == 1 && token.chars().next().is_some_and(is_cjk)
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
            | 0xF900..=0xFAFF
    )
}

fn approximate_reflow_width(text: &str, font_size: f64) -> f64 {
    text.chars()
        .map(|ch| {
            if ch.is_whitespace() {
                font_size * 0.30
            } else if is_cjk(ch) {
                font_size
            } else if matches!(ch, 'i' | 'l' | 'I' | '.' | ',' | ';' | ':' | '!' | '|') {
                font_size * 0.28
            } else if matches!(ch, 'm' | 'w' | 'M' | 'W') {
                font_size * 0.78
            } else {
                font_size * 0.52
            }
        })
        .sum()
}

fn extracted_pages_contain(engine: &ContentEngine, pages: &[usize], needle: &str) -> bool {
    let needle = normalize_search_text(needle);
    if needle.is_empty() {
        return true;
    }
    let mut out = String::new();
    for page in pages {
        if let Ok(text) = engine.get_page_text(*page) {
            out.push_str(&text);
            out.push(' ');
        }
    }
    normalize_search_text(&out).contains(&needle)
}

fn normalize_search_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone)]
struct PageEdit {
    layer: OverlayLayer,
    command: EditCommand,
}

#[derive(Debug, Clone)]
struct RedactionEdit {
    rect: ImageRect,
    polygon: Vec<(f64, f64)>,
    options: RedactionOptions,
}

#[derive(Debug, Clone)]
enum AnnotationEdit {
    Add(AnnotationSpec),
    EditContents { index: usize, contents: String },
    DeleteInRect { rect: ImageRect },
}

#[derive(Debug, Clone)]
struct AnnotationSpec {
    kind: AnnotationKind,
    rect: ImageRect,
    label: String,
    options: AnnotationOptions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnotationKind {
    Highlight,
    TextNote,
    Stamp,
    Link,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum FormValue {
    Text(String),
    Choice(String),
    Checkbox(bool),
}

#[derive(Debug, Clone)]
struct FormFlattenVisual {
    page_number: usize,
    rect: ImageRect,
    value: FormValue,
}

#[derive(Debug, Clone)]
enum EditCommand {
    Text {
        text: String,
        x: f64,
        y: f64,
        style: EditTextStyle,
    },
    Rect {
        rect: ImageRect,
        style: EditRectStyle,
    },
    Image {
        image: EditImage,
        rect: ImageRect,
        opacity: f64,
    },
}

#[derive(Debug, Clone)]
struct EditImage {
    width: u32,
    height: u32,
    color_space: &'static str,
    bits_per_component: u8,
    data: Vec<u8>,
    filter: ImageFilter,
    smask: Option<EditSoftMask>,
}

#[derive(Debug, Clone)]
struct EditSoftMask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageFilter {
    DctDecode,
    FlateDecode,
}

impl ImageFilter {
    fn pdf_name(self) -> &'static str {
        match self {
            Self::DctDecode => "DCTDecode",
            Self::FlateDecode => "FlateDecode",
        }
    }
}

struct ChangeSet {
    next: u32,
    objects: BTreeMap<(u32, u16), PdfObject>,
}

impl ChangeSet {
    fn new(reader: &PdfReader) -> Self {
        Self {
            next: next_free_object_number(reader),
            objects: BTreeMap::new(),
        }
    }

    fn alloc(&mut self) -> u32 {
        let number = self.next;
        self.next += 1;
        number
    }

    fn insert_existing(&mut self, number: u32, generation: u16, object: PdfObject) {
        self.objects.insert((number, generation), object);
    }

    fn insert_new(&mut self, number: u32, object: PdfObject) {
        self.insert_existing(number, 0, object);
    }

    fn insert_new_stream(&mut self, number: u32, raw: Vec<u8>) {
        self.insert_new(
            number,
            PdfObject::Stream {
                dict: PdfDictionary::empty(),
                raw,
            },
        );
    }

    fn current_object(
        &self,
        reader: &PdfReader,
        number: u32,
        generation: u16,
    ) -> Result<PdfObject> {
        self.objects
            .get(&(number, generation))
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| reader.get_object(number, generation))
    }

    fn into_vec(self) -> Vec<IncrementalObject> {
        self.objects
            .into_iter()
            .map(|((number, generation), object)| IncrementalObject {
                number,
                generation,
                object,
            })
            .collect()
    }
}

#[derive(Default)]
struct RedactionReport {
    removed_text: BTreeSet<String>,
    scrub_metadata: bool,
}

#[derive(Clone)]
struct RedactionState {
    ctm: Matrix,
    stack: Vec<Matrix>,
    text_matrix: Matrix,
    text_line_matrix: Matrix,
    font_size: f64,
    /// Resource name of the currently selected font (the `Tf` operand), used to
    /// look up real glyph metrics. `None` until a font is selected.
    font_name: Option<String>,
    char_spacing: f64,
    word_spacing: f64,
    /// Horizontal scaling factor (`Tz` / 100), default 1.0.
    h_scale: f64,
    /// Text leading (`TL`); 0.0 means "unset" and `T*` falls back to 1.2em.
    leading: f64,
    /// Text rise (`Ts`).
    rise: f64,
}

impl Default for RedactionState {
    fn default() -> Self {
        Self {
            ctm: IDENTITY_MATRIX,
            stack: Vec::new(),
            text_matrix: IDENTITY_MATRIX,
            text_line_matrix: IDENTITY_MATRIX,
            font_size: 12.0,
            font_name: None,
            char_spacing: 0.0,
            word_spacing: 0.0,
            h_scale: 1.0,
            leading: 0.0,
            rise: 0.0,
        }
    }
}

impl RedactionState {
    fn apply(&mut self, op: &ContentOperation, resolvers: &HashMap<String, FontResolver>) {
        match op.operator.as_str() {
            "q" => self.stack.push(self.ctm),
            "Q" => {
                if let Some(ctm) = self.stack.pop() {
                    self.ctm = ctm;
                }
            }
            "cm" => {
                let m = [
                    op.number(0).unwrap_or(1.0),
                    op.number(1).unwrap_or(0.0),
                    op.number(2).unwrap_or(0.0),
                    op.number(3).unwrap_or(1.0),
                    op.number(4).unwrap_or(0.0),
                    op.number(5).unwrap_or(0.0),
                ];
                self.ctm = concat_matrix(&m, &self.ctm);
            }
            "BT" => {
                self.text_matrix = IDENTITY_MATRIX;
                self.text_line_matrix = IDENTITY_MATRIX;
            }
            "Tf" => {
                self.font_name = op.name(0).map(|name| name.to_string());
                if let Some(size) = op.number(1) {
                    self.font_size = size.abs().max(1.0);
                }
            }
            "Tc" => {
                if let Some(v) = op.number(0) {
                    self.char_spacing = v;
                }
            }
            "Tw" => {
                if let Some(v) = op.number(0) {
                    self.word_spacing = v;
                }
            }
            "Tz" => {
                if let Some(v) = op.number(0) {
                    let scale = v / 100.0;
                    self.h_scale = if scale > 0.0 { scale } else { 1.0 };
                }
            }
            "TL" => {
                if let Some(v) = op.number(0) {
                    self.leading = v;
                }
            }
            "Ts" => {
                if let Some(v) = op.number(0) {
                    self.rise = v;
                }
            }
            "Td" | "TD" => {
                let tx = op.number(0).unwrap_or(0.0);
                let ty = op.number(1).unwrap_or(0.0);
                if op.operator == "TD" {
                    self.leading = -ty;
                }
                self.text_line_matrix[4] += tx;
                self.text_line_matrix[5] += ty;
                self.text_matrix = self.text_line_matrix;
            }
            "Tm" => {
                self.text_matrix = [
                    op.number(0).unwrap_or(1.0),
                    op.number(1).unwrap_or(0.0),
                    op.number(2).unwrap_or(0.0),
                    op.number(3).unwrap_or(1.0),
                    op.number(4).unwrap_or(0.0),
                    op.number(5).unwrap_or(0.0),
                ];
                self.text_line_matrix = self.text_matrix;
            }
            "T*" => {
                self.text_line_matrix[5] -= self.line_leading();
                self.text_matrix = self.text_line_matrix;
            }
            "Tj" => {
                if let Some(bytes) = op.string_bytes(0) {
                    let advance = self.string_advance(bytes, self.current_resolver(resolvers));
                    self.advance_pen(advance);
                }
            }
            "'" => {
                self.apply(&ContentOperation::new("T*", Vec::new()), resolvers);
                if let Some(bytes) = op.string_bytes(0) {
                    let advance = self.string_advance(bytes, self.current_resolver(resolvers));
                    self.advance_pen(advance);
                }
            }
            "\"" => {
                if let Some(aw) = op.number(0) {
                    self.word_spacing = aw;
                }
                if let Some(ac) = op.number(1) {
                    self.char_spacing = ac;
                }
                self.apply(&ContentOperation::new("T*", Vec::new()), resolvers);
                if let Some(bytes) = op.string_bytes(2) {
                    let advance = self.string_advance(bytes, self.current_resolver(resolvers));
                    self.advance_pen(advance);
                }
            }
            "TJ" => {
                if let Some(items) = op.operand(0).and_then(Operand::as_array) {
                    let resolver = self.current_resolver(resolvers);
                    let mut deltas = Vec::with_capacity(items.len());
                    for item in items {
                        deltas.push(match item {
                            Operand::String(bytes) => self.string_advance(bytes, resolver),
                            Operand::Integer(n) => self.tj_adjust(-(*n as f64)),
                            Operand::Real(n) => self.tj_adjust(-*n),
                            _ => 0.0,
                        });
                    }
                    for delta in deltas {
                        self.advance_pen(delta);
                    }
                }
            }
            _ => {}
        }
    }

    fn current_resolver<'a>(
        &self,
        resolvers: &'a HashMap<String, FontResolver>,
    ) -> Option<&'a FontResolver> {
        self.font_name
            .as_deref()
            .and_then(|name| resolvers.get(name))
    }

    fn line_leading(&self) -> f64 {
        if self.leading.abs() > f64::EPSILON {
            self.leading
        } else {
            self.font_size * 1.2
        }
    }

    /// Advance of one glyph in text space, from its real width (per-mille of em)
    /// plus character/word spacing, scaled by horizontal scaling.
    fn glyph_advance(&self, width_units: f64, is_space: bool) -> f64 {
        (width_units / 1000.0 * self.font_size
            + self.char_spacing
            + if is_space { self.word_spacing } else { 0.0 })
            * self.h_scale
    }

    /// Total text-space advance of a show string, using real font metrics when a
    /// resolver is available and a conservative per-em estimate otherwise.
    fn string_advance(&self, bytes: &[u8], resolver: Option<&FontResolver>) -> f64 {
        let code_size = resolver.map(FontResolver::code_size).unwrap_or(1).max(1);
        extract_char_codes(bytes, code_size)
            .into_iter()
            .map(|code| match resolver {
                Some(r) => self.glyph_advance(r.glyph_width(code), r.is_space_code(code)),
                None => self.glyph_advance(FALLBACK_GLYPH_WIDTH, code == 0x20),
            })
            .sum()
    }

    /// Text-space displacement of a `TJ` numeric adjustment (thousandths of em).
    fn tj_adjust(&self, value_units: f64) -> f64 {
        value_units / 1000.0 * self.font_size * self.h_scale
    }

    fn advance_pen(&mut self, dx: f64) {
        self.text_matrix[4] += dx;
    }

    fn unit_rect(&self) -> ImageRect {
        let (x1, y1) = transform_point(&self.ctm, 0.0, 0.0);
        let (x2, y2) = transform_point(&self.ctm, 1.0, 0.0);
        let (x3, y3) = transform_point(&self.ctm, 0.0, 1.0);
        let (x4, y4) = transform_point(&self.ctm, 1.0, 1.0);
        rect_from_points(&[(x1, y1), (x2, y2), (x3, y3), (x4, y4)])
    }
}

#[derive(Default)]
struct PendingPath {
    operations: Vec<ContentOperation>,
    bbox: Option<ImageRect>,
}

impl PendingPath {
    fn push(&mut self, op: ContentOperation, state: &RedactionState) {
        self.expand_from_operation(&op, state);
        self.operations.push(op);
    }

    fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    fn intersects(&self, redactions: &[RedactionEdit]) -> bool {
        self.bbox
            .as_ref()
            .map(|bbox| {
                redactions
                    .iter()
                    .any(|redaction| rects_intersect(*bbox, redaction.rect))
            })
            .unwrap_or(false)
    }

    fn flush_to(&mut self, out: &mut Vec<u8>) {
        for op in self.operations.drain(..) {
            serialize_content_operation(&op, out);
        }
        self.bbox = None;
    }

    fn clear(&mut self) {
        self.operations.clear();
        self.bbox = None;
    }

    fn expand_from_operation(&mut self, op: &ContentOperation, state: &RedactionState) {
        let points: Vec<(f64, f64)> = match op.operator.as_str() {
            "re" => {
                let x = op.number(0).unwrap_or(0.0);
                let y = op.number(1).unwrap_or(0.0);
                let w = op.number(2).unwrap_or(0.0);
                let h = op.number(3).unwrap_or(0.0);
                vec![(x, y), (x + w, y), (x, y + h), (x + w, y + h)]
            }
            "m" | "l" => vec![(op.number(0).unwrap_or(0.0), op.number(1).unwrap_or(0.0))],
            "c" => vec![
                (op.number(0).unwrap_or(0.0), op.number(1).unwrap_or(0.0)),
                (op.number(2).unwrap_or(0.0), op.number(3).unwrap_or(0.0)),
                (op.number(4).unwrap_or(0.0), op.number(5).unwrap_or(0.0)),
            ],
            "v" | "y" => vec![
                (op.number(0).unwrap_or(0.0), op.number(1).unwrap_or(0.0)),
                (op.number(2).unwrap_or(0.0), op.number(3).unwrap_or(0.0)),
            ],
            _ => Vec::new(),
        };
        for (x, y) in points {
            let (tx, ty) = transform_point(&state.ctm, x, y);
            self.include_point(tx, ty);
        }
    }

    fn include_point(&mut self, x: f64, y: f64) {
        self.bbox = Some(match self.bbox {
            Some(rect) => ImageRect {
                x: rect.x.min(x),
                y: rect.y.min(y),
                width: (rect.x + rect.width).max(x) - rect.x.min(x),
                height: (rect.y + rect.height).max(y) - rect.y.min(y),
            },
            None => ImageRect::new(x, y, 0.0, 0.0),
        });
    }
}

fn rewrite_page_content_for_redaction(
    reader: &PdfReader,
    page: &PdfPage,
    resources: &mut PdfDictionary,
    redactions: &[RedactionEdit],
    report: &mut RedactionReport,
    changes: &mut ChangeSet,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let resolvers = build_font_resolvers(resources, reader);
    let mut state = RedactionState::default();
    let mut pending_path = PendingPath::default();
    let mut retained_xobjects = BTreeSet::new();
    report.scrub_metadata |= redactions
        .iter()
        .any(|redaction| redaction.options.scrub_metadata);

    for (number, generation) in &page.contents {
        let object = reader.get_object(*number, *generation)?;
        let decoded = decode_stream_lossless(&object, reader)?;
        let operations = ContentParser::parse(&decoded.data)?;
        let mut operation_index = 0usize;
        while operation_index < operations.len() {
            if operations[operation_index].operator == "BI" {
                let end = operations[operation_index..]
                    .iter()
                    .position(|operation| operation.operator == "EI")
                    .map(|offset| operation_index + offset)
                    .unwrap_or(operations.len().saturating_sub(1));
                let image_rect = state.unit_rect();
                let overlapping: Vec<&RedactionEdit> = redactions
                    .iter()
                    .filter(|redaction| rects_intersect(image_rect, redaction.rect))
                    .collect();
                if overlapping.is_empty() {
                    serialize_inline_image_group(&operations[operation_index..=end], &mut out)?;
                } else {
                    let policy = overlapping
                        .iter()
                        .map(|redaction| redaction.options.image_policy)
                        .find(|policy| *policy == ImageRedactionPolicy::Fail)
                        .unwrap_or_else(|| {
                            overlapping
                                .iter()
                                .map(|redaction| redaction.options.image_policy)
                                .find(|policy| *policy == ImageRedactionPolicy::Remove)
                                .unwrap_or(ImageRedactionPolicy::Partial)
                        });
                    if policy != ImageRedactionPolicy::Remove {
                        let promote = overlapping
                            .iter()
                            .any(|redaction| redaction.options.promote_inline_images);
                        let (rewritten, promoted_name) = rewrite_inline_image_group(
                            reader,
                            &operations[operation_index..=end],
                            state.ctm,
                            &overlapping,
                            resources,
                            changes,
                            promote,
                            &mut out,
                        )?;
                        if let Some(name) = promoted_name {
                            retained_xobjects.insert(name);
                        }
                        if !rewritten && policy == ImageRedactionPolicy::Fail {
                            return Err(WellfriendError::UnsupportedFeature(
                                "inline image redaction failed closed: the filter, color space, bit depth, transform, or sample layout has no bounded deterministic rewrite"
                                    .to_string(),
                            ));
                        }
                    }
                }
                // Unsupported Partial and explicit Remove omit the complete
                // BI/ID/data/EI invocation. No visual overlay is accepted as
                // secure redaction.
                operation_index = end.saturating_add(1);
                continue;
            }
            let op = operations[operation_index].clone();
            operation_index += 1;
            if is_path_construction(&op) {
                pending_path.push(op, &state);
                continue;
            }
            if is_path_paint(&op) {
                if pending_path.intersects(redactions) {
                    pending_path.clear();
                } else {
                    pending_path.flush_to(&mut out);
                    serialize_content_operation(&op, &mut out);
                }
                state.apply(&op, &resolvers);
                continue;
            }
            if !pending_path.is_empty() {
                pending_path.flush_to(&mut out);
            }
            match op.operator.as_str() {
                "Tj" => {
                    let resolver = state.current_resolver(&resolvers);
                    if let Some(rewritten) =
                        redact_text_show(&op, &state, resolver, redactions, report)
                    {
                        serialize_content_operation(&rewritten, &mut out);
                    }
                    state.apply(&op, &resolvers);
                }
                "TJ" => {
                    let resolver = state.current_resolver(&resolvers);
                    if let Some(rewritten) =
                        redact_text_array(&op, &state, resolver, redactions, report)
                    {
                        serialize_content_operation(&rewritten, &mut out);
                    }
                    state.apply(&op, &resolvers);
                }
                "'" | "\"" => {
                    // ' and " move to the next line *before* showing; test the
                    // glyphs at that post-move position. On intersection (or any
                    // uncertainty) the whole operator is dropped — fail closed.
                    if line_show_intersects(&op, &state, &resolvers, redactions) {
                        collect_text_from_operation(&op, report);
                    } else {
                        serialize_content_operation(&op, &mut out);
                    }
                    state.apply(&op, &resolvers);
                }
                "Do" => {
                    let xobject_name = op.name(0).map(str::to_string);
                    let xobject = op
                        .name(0)
                        .and_then(|name| xobject_reference(resources, reader, name))
                        .and_then(|(number, generation)| {
                            reader
                                .get_object(number, generation)
                                .ok()
                                .map(|object| (number, generation, object))
                        });
                    let image_rect = xobject
                        .as_ref()
                        .and_then(|(_, _, object)| object_dictionary(object))
                        .and_then(|dict| {
                            (dict.get_name("Subtype") == Some("Form"))
                                .then(|| form_invocation_rect(&state.ctm, dict))
                                .flatten()
                        })
                        .unwrap_or_else(|| state.unit_rect());
                    let intersects = redactions
                        .iter()
                        .any(|redaction| rects_intersect(image_rect, redaction.rect));
                    if intersects {
                        let policy = redactions
                            .iter()
                            .filter(|redaction| rects_intersect(image_rect, redaction.rect))
                            .map(|redaction| redaction.options.image_policy)
                            .find(|policy| *policy == ImageRedactionPolicy::Fail)
                            .unwrap_or_else(|| {
                                redactions
                                    .iter()
                                    .filter(|redaction| rects_intersect(image_rect, redaction.rect))
                                    .map(|redaction| redaction.options.image_policy)
                                    .find(|policy| *policy == ImageRedactionPolicy::Remove)
                                    .unwrap_or(ImageRedactionPolicy::Partial)
                            });
                        let mut handled = false;
                        if policy != ImageRedactionPolicy::Remove {
                            if let Some((obj, gen, object)) = xobject.as_ref() {
                                let is_image = object_dictionary(object)
                                    .is_some_and(|dict| dict.get_name("Subtype") == Some("Image"));
                                if is_image {
                                    match redacted_image_xobject(
                                        reader, *obj, *gen, state.ctm, redactions, changes,
                                    ) {
                                        Ok(Some(redacted)) => {
                                            let new_number = changes.alloc();
                                            changes.insert_new(new_number, redacted);
                                            let new_name =
                                                add_redacted_xobject(resources, new_number);
                                            let mut rewritten = op.clone();
                                            if let Some(first) = rewritten.operands.first_mut() {
                                                *first = Operand::Name(new_name);
                                            }
                                            serialize_content_operation(&rewritten, &mut out);
                                            retained_xobjects.insert(
                                                rewritten.name(0).unwrap_or_default().to_string(),
                                            );
                                            handled = true;
                                        }
                                        Ok(None) => {}
                                        Err(err) if policy == ImageRedactionPolicy::Fail => {
                                            return Err(err);
                                        }
                                        Err(_) => {}
                                    }
                                }
                            }
                        }
                        if !handled && policy == ImageRedactionPolicy::Fail {
                            return Err(WellfriendError::UnsupportedFeature(
                                "partial image redaction could not prove a secure sample-space rewrite; use remove policy for conservative invocation removal"
                                    .to_string(),
                            ));
                        }
                        // Secure fallback: omit only this invocation. The original
                        // shared resource remains available to unaffected uses.
                    } else {
                        serialize_content_operation(&op, &mut out);
                        if let Some(name) = xobject_name {
                            retained_xobjects.insert(name);
                        }
                    }
                    state.apply(&op, &resolvers);
                }
                _ => {
                    serialize_content_operation(&op, &mut out);
                    state.apply(&op, &resolvers);
                }
            }
        }
        if !pending_path.is_empty() {
            pending_path.flush_to(&mut out);
        }
        let xobjects = resources
            .get("XObject")
            .and_then(|object| reader.resolve(object.clone()).ok())
            .and_then(|object| object.as_dict().cloned())
            .unwrap_or_else(PdfDictionary::empty);
        let xobjects = PdfDictionary::new(
            xobjects
                .entries()
                .filter(|(name, _)| retained_xobjects.contains(*name))
                .map(|(name, object)| (name.clone(), object.clone()))
                .collect(),
        );
        if xobjects.is_empty() {
            resources.remove("XObject");
        } else {
            resources.insert("XObject", PdfObject::Dictionary(xobjects));
        }
        out.push(b'\n');
    }
    Ok(out)
}

/// Conservative width (per-mille of em) assumed for a glyph whose font metrics
/// could not be resolved. Only used to keep the pen roughly positioned; the
/// removal decision for unresolved fonts is made fail-closed at string scope.
const FALLBACK_GLYPH_WIDTH: f64 = 500.0;

fn build_font_resolvers(
    resources: &PdfDictionary,
    reader: &PdfReader,
) -> HashMap<String, FontResolver> {
    PageResources::from_dict(resources, reader)
        .fonts
        .iter()
        .map(|(name, font_dict)| (name.clone(), FontResolver::new(font_dict, reader)))
        .collect()
}

/// Device-space rectangle occupied by a single glyph whose pen position (text
/// space) is `pen_x` and whose box width is `box_w`. The vertical band is the
/// font ascent/descent envelope (intentionally generous so a covered glyph is
/// never judged outside the redaction box).
fn glyph_rect_at(state: &RedactionState, pen_x: f64, box_w: f64) -> ImageRect {
    let y0 = state.text_matrix[5] + state.rise - state.font_size * 0.25;
    let y1 = state.text_matrix[5] + state.rise + state.font_size * 0.90;
    let width = box_w.max(state.font_size * 0.05);
    let (ax, ay) = transform_point(&state.ctm, pen_x, y0);
    let (bx, by) = transform_point(&state.ctm, pen_x + width, y1);
    rect_from_corners(ax, ay, bx, by)
}

fn record_removed_text(bytes: &[u8], report: &mut RedactionReport) {
    let text = decode_pdf_text_string(bytes);
    if !text.trim().is_empty() {
        report.removed_text.insert(text);
    }
}

/// A `TJ` numeric operand (thousandths of em, before font/scale) that advances
/// the pen forward by `adv` text-space units, preserving following positions.
fn advance_number(adv: f64, state: &RedactionState) -> Operand {
    let denom = state.font_size * state.h_scale;
    let units = if denom.abs() > f64::EPSILON {
        adv / denom * 1000.0
    } else {
        0.0
    };
    Operand::Integer(-(units.round() as i64))
}

fn advance_only(adv: f64, state: &RedactionState) -> Option<Vec<Operand>> {
    (adv.abs() > f64::EPSILON).then(|| vec![advance_number(adv, state)])
}

/// Fail-closed test for a string whose font is unresolved: assume each byte may
/// be up to a full em wide and a generous vertical band, so we never under-cover
/// an unknown font.
fn failclosed_string_intersects(
    bytes: &[u8],
    state: &RedactionState,
    redactions: &[RedactionEdit],
) -> bool {
    let span = (bytes.len().max(1) as f64) * state.font_size * state.h_scale;
    let y0 = state.text_matrix[5] + state.rise - state.font_size * 0.5;
    let y1 = state.text_matrix[5] + state.rise + state.font_size;
    let (ax, ay) = transform_point(&state.ctm, state.text_matrix[4], y0);
    let (bx, by) = transform_point(&state.ctm, state.text_matrix[4] + span, y1);
    let region = rect_from_corners(ax, ay, bx, by);
    redactions
        .iter()
        .any(|redaction| rects_intersect(region, redaction.rect))
}

/// True if any glyph of `bytes`, positioned with real metrics from `state`,
/// intersects a redaction. Falls back to the fail-closed whole-string test when
/// no font resolver is available.
fn string_glyphs_intersect(
    bytes: &[u8],
    state: &RedactionState,
    resolver: Option<&FontResolver>,
    redactions: &[RedactionEdit],
) -> bool {
    let Some(resolver) = resolver else {
        return failclosed_string_intersects(bytes, state, redactions);
    };
    let code_size = resolver.code_size().max(1);
    let mut pen = state.text_matrix[4];
    for code in extract_char_codes(bytes, code_size) {
        let width_units = resolver.glyph_width(code);
        let box_w = width_units / 1000.0 * state.font_size * state.h_scale;
        let rect = glyph_rect_at(state, pen, box_w);
        if redactions
            .iter()
            .any(|redaction| rects_intersect(rect, redaction.rect))
        {
            return true;
        }
        pen += state.glyph_advance(width_units, resolver.is_space_code(code));
    }
    false
}

/// Intersection test for the `'` and `"` operators, which advance to the next
/// line before showing. `"` also sets word/char spacing from its first operands.
fn line_show_intersects(
    op: &ContentOperation,
    state: &RedactionState,
    resolvers: &HashMap<String, FontResolver>,
    redactions: &[RedactionEdit],
) -> bool {
    let mut probe = state.clone();
    let bytes = if op.operator == "\"" {
        if let Some(aw) = op.number(0) {
            probe.word_spacing = aw;
        }
        if let Some(ac) = op.number(1) {
            probe.char_spacing = ac;
        }
        op.string_bytes(2)
    } else {
        op.string_bytes(0)
    };
    probe.text_line_matrix[5] -= probe.line_leading();
    probe.text_matrix = probe.text_line_matrix;
    let resolver = probe.current_resolver(resolvers);
    bytes
        .map(|bytes| string_glyphs_intersect(bytes, &probe, resolver, redactions))
        .unwrap_or(false)
}

fn redact_text_show(
    op: &ContentOperation,
    state: &RedactionState,
    resolver: Option<&FontResolver>,
    redactions: &[RedactionEdit],
    report: &mut RedactionReport,
) -> Option<ContentOperation> {
    let bytes = op.string_bytes(0)?;
    let rewritten = redact_string_bytes(bytes, state, resolver, redactions, report);
    rewritten.map(|operands| ContentOperation::new("TJ", vec![Operand::Array(operands)]))
}

fn redact_text_array(
    op: &ContentOperation,
    state: &RedactionState,
    resolver: Option<&FontResolver>,
    redactions: &[RedactionEdit],
    report: &mut RedactionReport,
) -> Option<ContentOperation> {
    let items = op.operand(0).and_then(Operand::as_array)?;
    let mut local = state.clone();
    let mut out = Vec::new();
    for item in items {
        match item {
            Operand::String(bytes) => {
                if let Some(mut replacement) =
                    redact_string_bytes(bytes, &local, resolver, redactions, report)
                {
                    out.append(&mut replacement);
                }
                let advance = local.string_advance(bytes, resolver);
                local.advance_pen(advance);
            }
            Operand::Integer(n) => {
                out.push(Operand::Integer(*n));
                let delta = local.tj_adjust(-(*n as f64));
                local.advance_pen(delta);
            }
            Operand::Real(n) => {
                out.push(Operand::Real(*n));
                let delta = local.tj_adjust(-*n);
                local.advance_pen(delta);
            }
            other => out.push(other.clone()),
        }
    }
    (!out.is_empty()).then(|| ContentOperation::new("TJ", vec![Operand::Array(out)]))
}

/// Rewrite a show string so that every glyph intersecting a redaction is
/// removed from the content stream (not merely covered), preserving the
/// positions of surviving glyphs via numeric `TJ` adjustments.
///
/// With real font metrics this is precise per glyph. Without a resolver it is
/// fail-closed: if the string's generously-bounded run touches any redaction the
/// entire string is dropped, so an unknown-font glyph can never survive under a
/// mark.
fn redact_string_bytes(
    bytes: &[u8],
    state: &RedactionState,
    resolver: Option<&FontResolver>,
    redactions: &[RedactionEdit],
    report: &mut RedactionReport,
) -> Option<Vec<Operand>> {
    let Some(resolver) = resolver else {
        if failclosed_string_intersects(bytes, state, redactions) {
            record_removed_text(bytes, report);
            return advance_only(state.string_advance(bytes, None), state);
        }
        return Some(vec![Operand::String(bytes.to_vec())]);
    };

    let code_size = resolver.code_size().max(1) as usize;
    let codes = extract_char_codes(bytes, resolver.code_size().max(1));
    let mut out: Vec<Operand> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut removed: Vec<u8> = Vec::new();
    let mut pending_adv = 0.0_f64;
    let mut pen = state.text_matrix[4];

    for (index, code) in codes.into_iter().enumerate() {
        let start = index * code_size;
        let end = (start + code_size).min(bytes.len());
        let glyph_bytes = &bytes[start..end];
        let width_units = resolver.glyph_width(code);
        let is_space = resolver.is_space_code(code);
        let box_w = width_units / 1000.0 * state.font_size * state.h_scale;
        let advance = state.glyph_advance(width_units, is_space);
        let rect = glyph_rect_at(state, pen, box_w);
        let intersects = redactions
            .iter()
            .any(|redaction| rects_intersect(rect, redaction.rect));
        if intersects {
            if !current.is_empty() {
                out.push(Operand::String(std::mem::take(&mut current)));
            }
            removed.extend_from_slice(glyph_bytes);
            pending_adv += advance;
        } else {
            if pending_adv.abs() > f64::EPSILON {
                out.push(advance_number(pending_adv, state));
                pending_adv = 0.0;
            }
            current.extend_from_slice(glyph_bytes);
        }
        pen += advance;
    }
    if !current.is_empty() {
        out.push(Operand::String(current));
    }
    if pending_adv.abs() > f64::EPSILON {
        out.push(advance_number(pending_adv, state));
    }
    if !removed.is_empty() {
        record_removed_text(&removed, report);
    }
    (!out.is_empty()).then_some(out)
}

fn collect_text_from_operation(op: &ContentOperation, report: &mut RedactionReport) {
    let bytes = match op.operator.as_str() {
        "'" => op.string_bytes(0),
        "\"" => op.string_bytes(2),
        _ => None,
    };
    if let Some(bytes) = bytes {
        let text = decode_pdf_text_string(bytes);
        if !text.trim().is_empty() {
            report.removed_text.insert(text);
        }
    }
}

fn is_path_construction(op: &ContentOperation) -> bool {
    matches!(
        op.operator.as_str(),
        "m" | "l" | "c" | "v" | "y" | "h" | "re"
    )
}

fn is_path_paint(op: &ContentOperation) -> bool {
    matches!(
        op.operator.as_str(),
        "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n"
    )
}

fn write_redaction_mark(out: &mut Vec<u8>, redaction: &RedactionEdit) {
    let Some(first) = redaction.polygon.first() else {
        return;
    };
    out.extend_from_slice(b"q\n");
    write_fill_color(out, &redaction.options.fill);
    out.extend_from_slice(format!("{} {} m\n", fmt_num(first.0), fmt_num(first.1)).as_bytes());
    for point in redaction.polygon.iter().skip(1) {
        out.extend_from_slice(format!("{} {} l\n", fmt_num(point.0), fmt_num(point.1)).as_bytes());
    }
    out.extend_from_slice(b"h f\nQ\n");
}

fn write_form_flatten_visual(
    out: &mut Vec<u8>,
    resources: &mut PdfDictionary,
    visual: &FormFlattenVisual,
) {
    let _ = visual.page_number;
    match &visual.value {
        FormValue::Text(text) | FormValue::Choice(text) => {
            let font = ensure_standard_font(resources);
            let style = EditTextStyle::new((visual.rect.height * 0.45).clamp(8.0, 14.0))
                .fill(Color::black());
            write_text(
                out,
                &font,
                None,
                text,
                visual.rect.x + 3.0,
                visual.rect.y + visual.rect.height * 0.35,
                &style,
            );
        }
        FormValue::Checkbox(checked) => {
            let style = EditRectStyle {
                stroke: Some(Color::black()),
                fill: Some(Color::device_gray(1.0)),
                line_width: 1.0,
                opacity: 1.0,
            };
            write_rect(out, None, visual.rect, &style);
            if *checked {
                out.extend_from_slice(
                    format!(
                        "q 0 0 0 RG 2 w {} {} m {} {} l {} {} l S Q\n",
                        fmt_num(visual.rect.x + 3.0),
                        fmt_num(visual.rect.y + visual.rect.height * 0.5),
                        fmt_num(visual.rect.x + visual.rect.width * 0.4),
                        fmt_num(visual.rect.y + 3.0),
                        fmt_num(visual.rect.x + visual.rect.width - 3.0),
                        fmt_num(visual.rect.y + visual.rect.height - 3.0)
                    )
                    .as_bytes(),
                );
            }
        }
    }
}

fn serialize_content_operation(op: &ContentOperation, out: &mut Vec<u8>) {
    for operand in &op.operands {
        serialize_content_operand(operand, out);
        out.push(b' ');
    }
    out.extend_from_slice(op.operator.as_bytes());
    out.push(b'\n');
}

fn serialize_inline_image_group(operations: &[ContentOperation], out: &mut Vec<u8>) -> Result<()> {
    let id = operations
        .iter()
        .find(|operation| operation.operator == "ID")
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("inline image has no ID operator".to_string())
        })?;
    let data = operations
        .iter()
        .find(|operation| operation.operator == "inline_image_data")
        .and_then(|operation| operation.string_bytes(0))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("inline image has no captured data".to_string())
        })?;
    out.extend_from_slice(b"BI\n");
    for pair in id.operands.chunks(2) {
        if pair.len() != 2 {
            return Err(WellfriendError::MalformedPdf(
                "inline image parameter list is not key/value paired".to_string(),
            ));
        }
        serialize_content_operand(&pair[0], out);
        out.push(b' ');
        serialize_content_operand(&pair[1], out);
        out.push(b'\n');
    }
    out.extend_from_slice(b"ID\n");
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nEI\n");
    Ok(())
}

/// Securely rewrite a bounded inline image into a deterministic Flate image.
///
/// The content parser has already expanded abbreviated BI keys and filter
/// names. The tokenizer's stateful inline-image scanner supplies the exact
/// binary payload, so this routine never searches for `EI` inside image data.
/// Unsupported layouts return `Ok(false)` and the caller removes the complete
/// invocation (or fails when strict policy was requested).
#[allow(clippy::too_many_arguments)]
fn rewrite_inline_image_group(
    reader: &PdfReader,
    operations: &[ContentOperation],
    image_ctm: Matrix,
    redactions: &[&RedactionEdit],
    resources: &mut PdfDictionary,
    changes: &mut ChangeSet,
    promote: bool,
    out: &mut Vec<u8>,
) -> Result<(bool, Option<String>)> {
    const MAX_INLINE_PIXELS: u64 = 100_000_000;
    const MAX_INLINE_BYTES: usize = 256 * 1024 * 1024;

    let id = operations
        .iter()
        .find(|operation| operation.operator == "ID")
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("inline image has no ID operator".to_string())
        })?;
    let data = operations
        .iter()
        .find(|operation| operation.operator == "inline_image_data")
        .and_then(|operation| operation.string_bytes(0))
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("inline image has no captured data".to_string())
        })?;
    if data.len() > MAX_INLINE_BYTES || id.operands.len() % 2 != 0 {
        return Ok((false, None));
    }

    let value = |key: &str| -> Option<&Operand> {
        id.operands
            .chunks_exact(2)
            .find(|pair| pair[0].as_name() == Some(key))
            .map(|pair| &pair[1])
    };
    let width = value("Width")
        .and_then(Operand::as_integer)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let height = value("Height")
        .and_then(Operand::as_integer)
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(0);
    let bpc = value("BitsPerComponent")
        .and_then(Operand::as_integer)
        .and_then(|value| u8::try_from(value).ok())
        .unwrap_or(8);
    let image_mask = value("ImageMask")
        .and_then(Operand::as_bool)
        .unwrap_or(false);
    if width == 0
        || height == 0
        || u64::from(width).saturating_mul(u64::from(height)) > MAX_INLINE_PIXELS
        || !matches!(bpc, 1 | 2 | 4 | 8)
        || (image_mask && bpc != 1)
    {
        return Ok((false, None));
    }

    let color_space = if image_mask {
        "DeviceGray"
    } else {
        value("ColorSpace")
            .and_then(Operand::as_name)
            .unwrap_or("DeviceGray")
    };
    if !matches!(color_space, "DeviceGray" | "DeviceRGB" | "DeviceCMYK") {
        return Ok((false, None));
    }
    let filters: Vec<&str> = match value("Filter") {
        None => Vec::new(),
        Some(Operand::Name(name)) => vec![name.as_str()],
        Some(Operand::Array(items)) => {
            let names = items
                .iter()
                .filter_map(Operand::as_name)
                .collect::<Vec<_>>();
            if names.len() != items.len() {
                return Ok((false, None));
            }
            names
        }
        _ => return Ok((false, None)),
    };
    let decode_params = match inline_decode_params(value("DecodeParms"), filters.len()) {
        Ok(params) => params,
        Err(_) => return Ok((false, None)),
    };
    let input_channels = match color_space {
        "DeviceGray" => 1,
        "DeviceRGB" => 3,
        "DeviceCMYK" => 4,
        _ => unreachable!(),
    };
    if !inline_predictor_layout_is_safe(
        &decode_params,
        width,
        input_channels,
        bpc,
        MAX_INLINE_BYTES,
    ) {
        return Ok((false, None));
    }

    let mut raw = match ImageDecoder::decode_inline_with_param_array(
        data,
        width,
        height,
        bpc,
        color_space,
        &filters,
        &decode_params,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_INLINE_BYTES as u64,
            ..DecodeLimits::default()
        },
    ) {
        Ok(raw) => raw,
        Err(_) => return Ok((false, None)),
    };
    if raw.bits_per_sample != 8
        || !matches!(raw.channels, 1 | 3 | 4)
        || !raw.is_valid()
        || raw.byte_count() > MAX_INLINE_BYTES
    {
        return Ok((false, None));
    }
    let inverse = match invert_affine(&image_ctm) {
        Some(inverse) => inverse,
        None => return Ok((false, None)),
    };
    let channels = raw.channels as usize;
    let mut changed = false;
    for redaction in redactions {
        let sample_polygon = redaction
            .polygon
            .iter()
            .map(|(x, y)| transform_point(&inverse, *x, *y))
            .map(|(u, v)| (u * raw.width as f64, (1.0 - v) * raw.height as f64))
            .collect::<Vec<_>>();
        if sample_polygon.is_empty()
            || sample_polygon
                .iter()
                .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Ok((false, None));
        }
        let bounds = rect_from_points(&sample_polygon);
        let x0 = bounds.x.floor().max(0.0) as usize;
        let y0 = bounds.y.floor().max(0.0) as usize;
        let x1 = (bounds.x + bounds.width).ceil().min(raw.width as f64) as usize;
        let y1 = (bounds.y + bounds.height).ceil().min(raw.height as f64) as usize;
        let fill = fill_for_channels(&redaction.options.fill, raw.channels);
        let mask_transparent = if inline_mask_paints_ones(value("Decode")) {
            0
        } else {
            255
        };
        for y in y0..y1 {
            for x in x0..x1 {
                if !polygon_intersects_pixel(&sample_polygon, x as f64, y as f64) {
                    continue;
                }
                let offset = (y * raw.width as usize + x) * channels;
                if image_mask {
                    raw.pixels[offset] = mask_transparent;
                } else {
                    raw.pixels[offset..offset + channels].copy_from_slice(&fill[..channels]);
                }
                changed = true;
            }
        }
    }
    if !changed {
        return Ok((false, None));
    }

    let output_bpc = if image_mask { 1 } else { 8 };
    let unpacked = if image_mask {
        pack_subbyte_rows(&raw.pixels, raw.width, raw.height, 1)
    } else {
        raw.pixels.clone()
    };
    let predictor = if image_mask {
        None
    } else {
        output_predictor(
            &decode_params,
            raw.width,
            raw.channels,
            output_bpc,
            &unpacked,
        )
    };
    let encoded_samples = predictor
        .as_ref()
        .map(|(_, bytes)| bytes.as_slice())
        .unwrap_or(&unpacked);
    let encoded = flate_encode(encoded_samples, 9);
    let mut image_dict = PdfDictionary::empty();
    image_dict.insert("Width", PdfObject::Integer(i64::from(raw.width)));
    image_dict.insert("Height", PdfObject::Integer(i64::from(raw.height)));
    if image_mask {
        image_dict.insert("ImageMask", PdfObject::Boolean(true));
    } else {
        let color_space = match raw.channels {
            1 => "DeviceGray",
            3 => "DeviceRGB",
            4 => "DeviceCMYK",
            _ => return Ok((false, None)),
        };
        image_dict.insert("ColorSpace", PdfObject::Name(color_space.to_string()));
    }
    image_dict.insert(
        "BitsPerComponent",
        PdfObject::Integer(i64::from(output_bpc)),
    );
    image_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    if let Some(decode) = value("Decode").and_then(operand_to_pdf_object_for_inline) {
        image_dict.insert("Decode", decode);
    }
    if let Some((params, _)) = predictor {
        image_dict.insert("DecodeParms", PdfObject::Dictionary(params));
    }

    if promote {
        image_dict.insert("Type", PdfObject::Name("XObject".to_string()));
        image_dict.insert("Subtype", PdfObject::Name("Image".to_string()));
        let number = changes.alloc();
        changes.insert_new(
            number,
            PdfObject::Stream {
                dict: image_dict,
                raw: encoded,
            },
        );
        let name = add_promoted_inline_xobject(resources, number);
        serialize_content_operation(
            &ContentOperation::new("Do", vec![Operand::Name(name.clone())]),
            out,
        );
        let _ = reader;
        Ok((true, Some(name)))
    } else {
        out.extend_from_slice(b"BI\n");
        for (key, value) in image_dict.entries() {
            out.push(b'/');
            out.extend_from_slice(key.as_bytes());
            out.push(b' ');
            let Some(value) = pdf_object_to_inline_operand(value) else {
                return Ok((false, None));
            };
            serialize_content_operand(&value, out);
            out.push(b'\n');
        }
        out.extend_from_slice(b"ID\n");
        out.extend_from_slice(&encoded);
        out.extend_from_slice(b"\nEI\n");
        let _ = reader;
        Ok((true, None))
    }
}

fn inline_decode_params(
    operand: Option<&Operand>,
    filter_count: usize,
) -> Result<Vec<Option<PdfDictionary>>> {
    let Some(operand) = operand else {
        return Ok(vec![None; filter_count]);
    };
    match operand {
        Operand::Dictionary(entries) => {
            let mut out = vec![None; filter_count];
            if let Some(first) = out.first_mut() {
                *first = Some(inline_operand_dictionary(entries)?);
            } else {
                return Err(WellfriendError::MalformedPdf(
                    "inline DecodeParms present without a Filter".to_string(),
                ));
            }
            Ok(out)
        }
        Operand::Array(items) if items.len() == filter_count => items
            .iter()
            .map(|item| match item {
                Operand::Dictionary(entries) => Ok(Some(inline_operand_dictionary(entries)?)),
                _ => Err(WellfriendError::MalformedPdf(
                    "inline DecodeParms array entries must be dictionaries".to_string(),
                )),
            })
            .collect(),
        Operand::Array(items) => Err(WellfriendError::MalformedPdf(format!(
            "inline DecodeParms count {} does not match Filter count {filter_count}",
            items.len()
        ))),
        _ => Err(WellfriendError::MalformedPdf(
            "inline DecodeParms must be a dictionary or matching array".to_string(),
        )),
    }
}

fn inline_operand_dictionary(entries: &[(String, Operand)]) -> Result<PdfDictionary> {
    let mut dict = PdfDictionary::empty();
    for (key, value) in entries {
        let value = operand_to_pdf_object_for_inline(value).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "inline dictionary /{key} contains an unsupported value"
            ))
        })?;
        dict.insert(key, value);
    }
    Ok(dict)
}

fn operand_to_pdf_object_for_inline(operand: &Operand) -> Option<PdfObject> {
    match operand {
        Operand::Integer(value) => Some(PdfObject::Integer(*value)),
        Operand::Real(value) => Some(PdfObject::Real(*value)),
        Operand::Boolean(value) => Some(PdfObject::Boolean(*value)),
        Operand::Name(value) => Some(PdfObject::Name(value.clone())),
        Operand::String(value) => Some(PdfObject::String(value.clone())),
        Operand::Array(items) => Some(PdfObject::Array(
            items
                .iter()
                .map(operand_to_pdf_object_for_inline)
                .collect::<Option<Vec<_>>>()?,
        )),
        Operand::Dictionary(entries) => Some(PdfObject::Dictionary(
            inline_operand_dictionary(entries).ok()?,
        )),
    }
}

fn pdf_object_to_inline_operand(object: &PdfObject) -> Option<Operand> {
    match object {
        PdfObject::Integer(value) => Some(Operand::Integer(*value)),
        PdfObject::Real(value) => Some(Operand::Real(*value)),
        PdfObject::Boolean(value) => Some(Operand::Boolean(*value)),
        PdfObject::Name(value) => Some(Operand::Name(value.clone())),
        PdfObject::String(value) => Some(Operand::String(value.clone())),
        PdfObject::Array(items) => Some(Operand::Array(
            items
                .iter()
                .map(pdf_object_to_inline_operand)
                .collect::<Option<Vec<_>>>()?,
        )),
        PdfObject::Dictionary(dict) => Some(Operand::Dictionary(
            dict.entries()
                .map(|(key, value)| Some((key.clone(), pdf_object_to_inline_operand(value)?)))
                .collect::<Option<Vec<_>>>()?,
        )),
        _ => None,
    }
}

fn inline_predictor_layout_is_safe(
    params: &[Option<PdfDictionary>],
    width: u32,
    colors: u8,
    bpc: u8,
    byte_cap: usize,
) -> bool {
    params.iter().flatten().all(|params| {
        let predictor = params.get_integer("Predictor").unwrap_or(1);
        let columns = params.get_integer("Columns").unwrap_or(i64::from(width));
        let param_colors = params.get_integer("Colors").unwrap_or(i64::from(colors));
        let param_bpc = params
            .get_integer("BitsPerComponent")
            .unwrap_or(i64::from(bpc));
        let row_bits = u64::try_from(columns)
            .ok()
            .and_then(|columns| columns.checked_mul(u64::try_from(param_colors).ok()?))
            .and_then(|samples| samples.checked_mul(u64::try_from(param_bpc).ok()?));
        matches!(predictor, 1 | 2 | 10..=15)
            && columns == i64::from(width)
            && param_colors == i64::from(colors)
            && param_bpc == i64::from(bpc)
            && row_bits
                .and_then(|bits| usize::try_from(bits.div_ceil(8)).ok())
                .is_some_and(|bytes| bytes > 0 && bytes <= byte_cap)
    })
}

fn output_predictor(
    params: &[Option<PdfDictionary>],
    width: u32,
    colors: u8,
    bpc: u8,
    samples: &[u8],
) -> Option<(PdfDictionary, Vec<u8>)> {
    let predictor = params
        .iter()
        .flatten()
        .find_map(|params| params.get_integer("Predictor"))
        .filter(|value| *value != 1)?;
    if bpc != 8 {
        return None;
    }
    let row_len = usize::try_from(width).ok()?.checked_mul(colors as usize)?;
    if row_len == 0 || !samples.len().is_multiple_of(row_len) {
        return None;
    }
    let encoded = match predictor {
        2 => {
            let bytes_per_pixel = colors as usize;
            let mut encoded = samples.to_vec();
            for row in encoded.chunks_exact_mut(row_len) {
                for index in (bytes_per_pixel..row.len()).rev() {
                    row[index] = row[index].wrapping_sub(row[index - bytes_per_pixel]);
                }
            }
            encoded
        }
        10..=15 => {
            let mut encoded = Vec::with_capacity(samples.len() + samples.len() / row_len);
            for row in samples.chunks_exact(row_len) {
                encoded.push(0);
                encoded.extend_from_slice(row);
            }
            encoded
        }
        _ => return None,
    };
    let mut output = PdfDictionary::empty();
    output.insert("Predictor", PdfObject::Integer(predictor));
    output.insert("Colors", PdfObject::Integer(i64::from(colors)));
    output.insert("BitsPerComponent", PdfObject::Integer(i64::from(bpc)));
    output.insert("Columns", PdfObject::Integer(i64::from(width)));
    Some((output, encoded))
}

fn inline_mask_paints_ones(decode: Option<&Operand>) -> bool {
    decode
        .and_then(Operand::as_array)
        .and_then(|items| Some((items.first()?.as_number()?, items.get(1)?.as_number()?)))
        .map(|(zero, one)| one >= zero)
        .unwrap_or(true)
}

fn pack_subbyte_rows(samples: &[u8], width: u32, height: u32, bpc: u8) -> Vec<u8> {
    let samples_per_row = width as usize;
    let row_bytes = samples_per_row.saturating_mul(bpc as usize).div_ceil(8);
    let max = (1u16 << bpc) - 1;
    let mut packed = vec![0u8; row_bytes.saturating_mul(height as usize)];
    for y in 0..height as usize {
        for x in 0..samples_per_row {
            let sample = samples
                .get(y.saturating_mul(samples_per_row).saturating_add(x))
                .copied()
                .unwrap_or(0);
            let value = ((u16::from(sample) * max + 127) / 255) as u8;
            let bit = x.saturating_mul(bpc as usize);
            let shift = 8usize - bpc as usize - (bit % 8);
            packed[y * row_bytes + bit / 8] |= value << shift;
        }
    }
    packed
}

fn add_promoted_inline_xobject(resources: &mut PdfDictionary, number: u32) -> String {
    let mut xobjects = dict_resource(resources, "XObject");
    let name = next_resource_name(&xobjects, "OxP18Inline");
    xobjects.insert(&name, reference(number, 0));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    name
}

fn serialize_content_operand(operand: &Operand, out: &mut Vec<u8>) {
    match operand {
        Operand::Integer(value) => out.extend_from_slice(value.to_string().as_bytes()),
        Operand::Real(value) => out.extend_from_slice(fmt_num(*value).as_bytes()),
        Operand::Boolean(value) => {
            out.extend_from_slice(if *value { b"true" } else { b"false" });
        }
        Operand::Name(value) => {
            out.push(b'/');
            out.extend_from_slice(value.as_bytes());
        }
        Operand::String(bytes) => {
            out.push(b'<');
            out.extend_from_slice(hex_string(bytes).as_bytes());
            out.push(b'>');
        }
        Operand::Array(items) => {
            out.push(b'[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(b' ');
                }
                serialize_content_operand(item, out);
            }
            out.push(b']');
        }
        Operand::Dictionary(entries) => {
            out.extend_from_slice(b"<<");
            for (key, value) in entries {
                out.push(b'/');
                out.extend_from_slice(key.as_bytes());
                out.push(b' ');
                serialize_content_operand(value, out);
                out.push(b' ');
            }
            out.extend_from_slice(b">>");
        }
    }
}

fn xobject_reference(
    resources: &PdfDictionary,
    reader: &PdfReader,
    name: &str,
) -> Option<(u32, u16)> {
    let xobjects = resources.get("XObject")?;
    let resolved = reader.resolve(xobjects.clone()).ok()?;
    let dict = resolved.as_dict()?;
    dict.get(name).and_then(PdfObject::as_reference)
}

fn object_dictionary(object: &PdfObject) -> Option<&PdfDictionary> {
    match object {
        PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => Some(dict),
        _ => None,
    }
}

fn add_redacted_xobject(resources: &mut PdfDictionary, number: u32) -> String {
    let mut xobjects = dict_resource(resources, "XObject");
    let name = next_resource_name(&xobjects, "OxP17RedactIm");
    xobjects.insert(&name, reference(number, 0));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    name
}

fn form_invocation_rect(ctm: &Matrix, dict: &PdfDictionary) -> Option<ImageRect> {
    let bbox = dict.get("BBox")?.as_array()?;
    let values: Vec<f64> = bbox.iter().filter_map(PdfObject::as_number).collect();
    if values.len() != 4 {
        return None;
    }
    let form_matrix = dict
        .get("Matrix")
        .and_then(PdfObject::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(PdfObject::as_number)
                .collect::<Vec<_>>()
        })
        .filter(|items| items.len() == 6)
        .map(|items| [items[0], items[1], items[2], items[3], items[4], items[5]])
        .unwrap_or(IDENTITY_MATRIX);
    let matrix = concat_matrix(&form_matrix, ctm);
    let points = [
        transform_point(&matrix, values[0], values[1]),
        transform_point(&matrix, values[2], values[1]),
        transform_point(&matrix, values[2], values[3]),
        transform_point(&matrix, values[0], values[3]),
    ];
    Some(rect_from_points(&points))
}

fn redacted_image_xobject(
    reader: &PdfReader,
    number: u32,
    generation: u16,
    image_ctm: Matrix,
    redactions: &[RedactionEdit],
    changes: &mut ChangeSet,
) -> Result<Option<PdfObject>> {
    let obj = reader.get_object(number, generation)?;
    let PdfObject::Stream {
        dict: image_dict, ..
    } = &obj
    else {
        return Ok(None);
    };
    if image_dict.get_name("Subtype") != Some("Image") {
        return Ok(None);
    }
    let width = image_dict
        .get_integer("Width")
        .or_else(|| image_dict.get_integer("W"))
        .unwrap_or(0)
        .max(0) as u32;
    let height = image_dict
        .get_integer("Height")
        .or_else(|| image_dict.get_integer("H"))
        .unwrap_or(0)
        .max(0) as u32;
    if width == 0 || height == 0 {
        return Ok(None);
    }
    let inverse = invert_affine(&image_ctm).ok_or_else(|| {
        WellfriendError::UnsupportedFeature(
            "partial image redaction rejected a singular or non-finite image transform".to_string(),
        )
    })?;
    let image_mask = image_dict
        .get_bool("ImageMask")
        .or_else(|| image_dict.get_bool("IM"))
        .unwrap_or(false);
    let bpc = image_dict
        .get_integer("BitsPerComponent")
        .or_else(|| image_dict.get_integer("BPC"))
        .unwrap_or(if image_mask { 1 } else { 8 })
        .clamp(0, 16) as u8;
    let layout = secure_image_sample_layout(image_dict, reader, image_mask)?;
    if !matches!(bpc, 1 | 2 | 4 | 8)
        || (image_mask && bpc != 1)
        || (!matches!(layout, SecureImageLayout::Indexed { .. }) && !image_mask && bpc != 8)
    {
        return Ok(None);
    }
    let decoded = decode_stream_lossless(&obj, reader)?;
    if !matches!(decoded.status, StreamDecodeStatus::Complete) {
        return Ok(None);
    }
    let channels = layout.channels();
    let expected_samples = usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(height as usize))
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| WellfriendError::ResourceLimit("image sample count overflow".to_string()))?;
    let mut samples = unpack_samples_exact(&decoded.data, width, height, channels, bpc)?;
    if samples.len() != expected_samples {
        return Err(WellfriendError::MalformedPdf(
            "decoded image sample length does not match dimensions".to_string(),
        ));
    }
    let mut changed = false;
    for redaction in redactions {
        let image_rect = transformed_unit_rect(&image_ctm);
        if !rects_intersect(image_rect, redaction.rect) {
            continue;
        }
        let sample_polygon: Vec<(f64, f64)> = redaction
            .polygon
            .iter()
            .map(|(x, y)| transform_point(&inverse, *x, *y))
            .map(|(u, v)| (u * width as f64, (1.0 - v) * height as f64))
            .collect();
        if sample_polygon
            .iter()
            .any(|(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(WellfriendError::UnsupportedFeature(
                "partial image redaction produced non-finite sample coordinates".to_string(),
            ));
        }
        let sample_bounds = rect_from_points(&sample_polygon);
        let x0 = sample_bounds.x.floor().max(0.0) as usize;
        let y0 = sample_bounds.y.floor().max(0.0) as usize;
        let x1 = (sample_bounds.x + sample_bounds.width)
            .ceil()
            .min(width as f64) as usize;
        let y1 = (sample_bounds.y + sample_bounds.height)
            .ceil()
            .min(height as f64) as usize;
        let replacement =
            secure_replacement_samples(&layout, image_dict, reader, &redaction.options.fill, bpc)?;
        for y in y0..y1 {
            for x in x0..x1 {
                if !polygon_intersects_pixel(&sample_polygon, x as f64, y as f64) {
                    continue;
                }
                let offset = (y * width as usize + x) * channels;
                samples[offset..offset + channels].copy_from_slice(&replacement[..channels]);
                changed = true;
            }
        }
    }
    if !changed {
        return Ok(None);
    }
    let packed = pack_samples_exact(&samples, width, height, channels, bpc)?;
    let mut output_dict = image_dict.clone();
    output_dict.remove("F");
    output_dict.remove("Filter");
    output_dict.remove("DP");
    output_dict.remove("DecodeParms");
    output_dict.remove("Length");
    output_dict.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    output_dict.remove("Mask");
    output_dict.remove("SMask");

    if let Some(mask_ref) = image_dict.get("Mask").and_then(PdfObject::as_reference) {
        let mask = redacted_associated_mask(reader, mask_ref, image_ctm, redactions, false)?;
        let mask_number = changes.alloc();
        changes.insert_new(mask_number, mask);
        output_dict.insert("Mask", reference(mask_number, 0));
    } else if let Some(color_key) = image_dict.get("Mask") {
        output_dict.insert("Mask", color_key.clone());
    }
    if let Some(mask_ref) = image_dict.get("SMask").and_then(PdfObject::as_reference) {
        let mask = redacted_associated_mask(reader, mask_ref, image_ctm, redactions, true)?;
        let mask_number = changes.alloc();
        changes.insert_new(mask_number, mask);
        output_dict.insert("SMask", reference(mask_number, 0));
    }
    Ok(Some(PdfObject::Stream {
        dict: output_dict,
        raw: flate_encode(&packed, 9),
    }))
}

#[derive(Debug, Clone, Copy)]
enum SecureImageLayout {
    Stencil { paints_ones: bool },
    Device { channels: usize },
    Indexed { hival: u8, base_channels: usize },
    IccBased { channels: usize },
}

impl SecureImageLayout {
    fn channels(self) -> usize {
        match self {
            Self::Stencil { .. } | Self::Indexed { .. } => 1,
            Self::Device { channels } | Self::IccBased { channels } => channels,
        }
    }
}

fn secure_image_sample_layout(
    dict: &PdfDictionary,
    reader: &PdfReader,
    image_mask: bool,
) -> Result<SecureImageLayout> {
    if image_mask {
        return Ok(SecureImageLayout::Stencil {
            paints_ones: pdf_decode_paints_ones(dict.get("Decode")),
        });
    }
    let color_space = dict.get("ColorSpace").or_else(|| dict.get("CS"));
    match color_space {
        Some(PdfObject::Name(name)) => match name.as_str() {
            "DeviceGray" | "G" => Ok(SecureImageLayout::Device { channels: 1 }),
            "DeviceRGB" | "RGB" => Ok(SecureImageLayout::Device { channels: 3 }),
            "DeviceCMYK" | "CMYK" => Ok(SecureImageLayout::Device { channels: 4 }),
            _ => Err(WellfriendError::UnsupportedFeature(format!(
                "secure image rewrite does not resolve named color space /{name}"
            ))),
        },
        Some(PdfObject::Array(items)) => match items.first().and_then(PdfObject::as_name) {
            Some("Indexed" | "I") if items.len() >= 4 => {
                let hival = items[2]
                    .as_integer()
                    .and_then(|value| u8::try_from(value).ok())
                    .ok_or_else(|| {
                        WellfriendError::MalformedPdf(
                            "Indexed /hival is outside 0..255".to_string(),
                        )
                    })?;
                let base_channels = match items[1].as_name() {
                    Some("DeviceGray" | "G") => 1,
                    Some("DeviceRGB" | "RGB") => 3,
                    Some("DeviceCMYK" | "CMYK") => 4,
                    _ => {
                        return Err(WellfriendError::UnsupportedFeature(
                            "secure Indexed rewrite supports DeviceGray/RGB/CMYK bases".to_string(),
                        ))
                    }
                };
                Ok(SecureImageLayout::Indexed {
                    hival,
                    base_channels,
                })
            }
            Some("ICCBased") if items.len() >= 2 => {
                let profile = reader.resolve(items[1].clone())?;
                let channels = object_dictionary(&profile)
                    .and_then(|dict| dict.get_integer("N"))
                    .and_then(|value| usize::try_from(value).ok())
                    .filter(|value| matches!(value, 1 | 3 | 4))
                    .ok_or_else(|| {
                        WellfriendError::UnsupportedFeature(
                            "ICCBased secure rewrite requires profile /N 1, 3, or 4".to_string(),
                        )
                    })?;
                Ok(SecureImageLayout::IccBased { channels })
            }
            Some(other) => Err(WellfriendError::UnsupportedFeature(format!(
                "secure image rewrite does not support array color space {other}"
            ))),
            None => Err(WellfriendError::MalformedPdf(
                "image ColorSpace array has no family name".to_string(),
            )),
        },
        _ => Err(WellfriendError::MalformedPdf(
            "non-stencil image has no supported ColorSpace".to_string(),
        )),
    }
}

fn unpack_samples_exact(
    bytes: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    bpc: u8,
) -> Result<Vec<u8>> {
    if !matches!(bpc, 1 | 2 | 4 | 8) || channels == 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "packed sample rewrite supports 1, 2, 4, or 8 bits".to_string(),
        ));
    }
    let samples_per_row = (width as usize)
        .checked_mul(channels)
        .ok_or_else(|| WellfriendError::ResourceLimit("packed row sample overflow".to_string()))?;
    let row_bytes = samples_per_row
        .checked_mul(bpc as usize)
        .map(|bits| bits.div_ceil(8))
        .ok_or_else(|| WellfriendError::ResourceLimit("packed row byte overflow".to_string()))?;
    let expected = row_bytes
        .checked_mul(height as usize)
        .ok_or_else(|| WellfriendError::ResourceLimit("packed image byte overflow".to_string()))?;
    if bytes.len() != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "packed image decoded length {} does not equal row-padded length {expected}",
            bytes.len()
        )));
    }
    let mask = ((1u16 << bpc) - 1) as u8;
    let mut out = Vec::with_capacity(samples_per_row.saturating_mul(height as usize));
    for row in bytes.chunks_exact(row_bytes) {
        for sample in 0..samples_per_row {
            let bit = sample * bpc as usize;
            let shift = 8usize - bpc as usize - (bit % 8);
            out.push((row[bit / 8] >> shift) & mask);
        }
    }
    Ok(out)
}

fn pack_samples_exact(
    samples: &[u8],
    width: u32,
    height: u32,
    channels: usize,
    bpc: u8,
) -> Result<Vec<u8>> {
    let samples_per_row = (width as usize)
        .checked_mul(channels)
        .ok_or_else(|| WellfriendError::ResourceLimit("packed row sample overflow".to_string()))?;
    let expected_samples = samples_per_row
        .checked_mul(height as usize)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit("packed image sample overflow".to_string())
        })?;
    if samples.len() != expected_samples {
        return Err(WellfriendError::MalformedPdf(
            "packed rewrite sample buffer length mismatch".to_string(),
        ));
    }
    let row_bytes = samples_per_row
        .checked_mul(bpc as usize)
        .map(|bits| bits.div_ceil(8))
        .ok_or_else(|| WellfriendError::ResourceLimit("packed row byte overflow".to_string()))?;
    let mask = ((1u16 << bpc) - 1) as u8;
    let mut out = vec![0u8; row_bytes.saturating_mul(height as usize)];
    for y in 0..height as usize {
        for x in 0..samples_per_row {
            let bit = x * bpc as usize;
            let shift = 8usize - bpc as usize - (bit % 8);
            out[y * row_bytes + bit / 8] |= (samples[y * samples_per_row + x] & mask) << shift;
        }
    }
    Ok(out)
}

fn secure_replacement_samples(
    layout: &SecureImageLayout,
    dict: &PdfDictionary,
    reader: &PdfReader,
    fill: &Color,
    bpc: u8,
) -> Result<Vec<u8>> {
    let max = ((1u16 << bpc) - 1) as u8;
    match *layout {
        SecureImageLayout::Stencil { paints_ones } => Ok(vec![if paints_ones { 0 } else { max }]),
        SecureImageLayout::Device { channels } | SecureImageLayout::IccBased { channels } => {
            let bytes = fill_for_channels(fill, channels as u8);
            Ok((0..channels)
                .map(|channel| encode_sample_for_decode(dict, channel, bytes[channel], max))
                .collect())
        }
        SecureImageLayout::Indexed {
            hival,
            base_channels,
        } => {
            let lookup = indexed_lookup_bytes(dict, reader)?;
            let target = fill_for_channels(fill, base_channels as u8);
            let mut best = (u64::MAX, 0u8);
            for index in 0..=hival {
                let start = index as usize * base_channels;
                let end = start + base_channels;
                let color = lookup.get(start..end).ok_or_else(|| {
                    WellfriendError::MalformedPdf("Indexed lookup table is too short".to_string())
                })?;
                let distance = color
                    .iter()
                    .zip(target.iter())
                    .map(|(actual, target)| {
                        let delta = i64::from(*actual) - i64::from(*target);
                        (delta * delta) as u64
                    })
                    .sum();
                if distance < best.0 {
                    best = (distance, index);
                }
            }
            Ok(vec![encode_index_for_decode(dict, best.1, max)])
        }
    }
}

fn encode_sample_for_decode(dict: &PdfDictionary, channel: usize, desired: u8, max: u8) -> u8 {
    let Some(values) = dict.get("Decode").and_then(PdfObject::as_array) else {
        return ((u16::from(desired) * u16::from(max) + 127) / 255) as u8;
    };
    let Some(low) = values.get(channel * 2).and_then(PdfObject::as_number) else {
        return ((u16::from(desired) * u16::from(max) + 127) / 255) as u8;
    };
    let Some(high) = values.get(channel * 2 + 1).and_then(PdfObject::as_number) else {
        return ((u16::from(desired) * u16::from(max) + 127) / 255) as u8;
    };
    if !low.is_finite() || !high.is_finite() || (high - low).abs() <= f64::EPSILON {
        return 0;
    }
    let desired = f64::from(desired) / 255.0;
    (((desired - low) / (high - low)).clamp(0.0, 1.0) * f64::from(max)).round() as u8
}

fn encode_index_for_decode(dict: &PdfDictionary, desired: u8, max: u8) -> u8 {
    let Some(values) = dict.get("Decode").and_then(PdfObject::as_array) else {
        return desired.min(max);
    };
    let low = values.first().and_then(PdfObject::as_number).unwrap_or(0.0);
    let high = values
        .get(1)
        .and_then(PdfObject::as_number)
        .unwrap_or(f64::from(max));
    if !low.is_finite() || !high.is_finite() || (high - low).abs() <= f64::EPSILON {
        return 0;
    }
    (((f64::from(desired) - low) / (high - low)).clamp(0.0, 1.0) * f64::from(max)).round() as u8
}

fn indexed_lookup_bytes(dict: &PdfDictionary, reader: &PdfReader) -> Result<Vec<u8>> {
    let items = dict
        .get("ColorSpace")
        .or_else(|| dict.get("CS"))
        .and_then(PdfObject::as_array)
        .ok_or_else(|| {
            WellfriendError::MalformedPdf("Indexed ColorSpace is not an array".to_string())
        })?;
    match items.get(3) {
        Some(PdfObject::String(bytes)) => Ok(bytes.clone()),
        Some(value) => match reader.resolve(value.clone())? {
            PdfObject::String(bytes) => Ok(bytes),
            stream @ PdfObject::Stream { .. } => {
                let decoded = decode_stream_lossless(&stream, reader)?;
                if !matches!(decoded.status, StreamDecodeStatus::Complete) {
                    return Err(WellfriendError::UnsupportedFeature(
                        "Indexed lookup uses an unsupported image codec".to_string(),
                    ));
                }
                Ok(decoded.data)
            }
            _ => Err(WellfriendError::MalformedPdf(
                "Indexed lookup is not a string or stream".to_string(),
            )),
        },
        None => Err(WellfriendError::MalformedPdf(
            "Indexed ColorSpace has no lookup table".to_string(),
        )),
    }
}

fn pdf_decode_paints_ones(decode: Option<&PdfObject>) -> bool {
    decode
        .and_then(PdfObject::as_array)
        .and_then(|items| Some((items.first()?.as_number()?, items.get(1)?.as_number()?)))
        .map(|(zero, one)| one >= zero)
        .unwrap_or(true)
}

fn redacted_associated_mask(
    reader: &PdfReader,
    mask_ref: (u32, u16),
    image_ctm: Matrix,
    redactions: &[RedactionEdit],
    soft_mask: bool,
) -> Result<PdfObject> {
    let object = reader.get_object(mask_ref.0, mask_ref.1)?;
    let PdfObject::Stream { dict, .. } = &object else {
        return Err(WellfriendError::MalformedPdf(
            "associated image mask is not a stream".to_string(),
        ));
    };
    let width = dict.get_integer("Width").unwrap_or(0).max(0) as u32;
    let height = dict.get_integer("Height").unwrap_or(0).max(0) as u32;
    let stencil = dict.get_bool("ImageMask").unwrap_or(false);
    let bpc = dict
        .get_integer("BitsPerComponent")
        .unwrap_or(if stencil { 1 } else { 8 }) as u8;
    if width == 0 || height == 0 || !matches!(bpc, 1 | 8) {
        return Err(WellfriendError::UnsupportedFeature(
            "associated mask rewrite supports bounded 1-bit stencils and 8-bit gray masks"
                .to_string(),
        ));
    }
    let decoded = decode_stream_lossless(&object, reader)?;
    if !matches!(decoded.status, StreamDecodeStatus::Complete) {
        return Err(WellfriendError::UnsupportedFeature(
            "associated mask codec has no safe lossless decoder".to_string(),
        ));
    }
    let mut samples = unpack_samples_exact(&decoded.data, width, height, 1, bpc)?;
    let inverse = invert_affine(&image_ctm).ok_or_else(|| {
        WellfriendError::UnsupportedFeature("associated mask transform is singular".to_string())
    })?;
    let max = ((1u16 << bpc) - 1) as u8;
    let transparent = if stencil && !soft_mask && !pdf_decode_paints_ones(dict.get("Decode")) {
        max
    } else {
        0
    };
    for redaction in redactions {
        let polygon = redaction
            .polygon
            .iter()
            .map(|(x, y)| transform_point(&inverse, *x, *y))
            .map(|(u, v)| (u * width as f64, (1.0 - v) * height as f64))
            .collect::<Vec<_>>();
        let bounds = rect_from_points(&polygon);
        let x0 = bounds.x.floor().max(0.0) as usize;
        let y0 = bounds.y.floor().max(0.0) as usize;
        let x1 = (bounds.x + bounds.width).ceil().min(width as f64) as usize;
        let y1 = (bounds.y + bounds.height).ceil().min(height as f64) as usize;
        for y in y0..y1 {
            for x in x0..x1 {
                if polygon_intersects_pixel(&polygon, x as f64, y as f64) {
                    samples[y * width as usize + x] = transparent;
                }
            }
        }
    }
    let mut output = dict.clone();
    output.remove("F");
    output.remove("Filter");
    output.remove("DP");
    output.remove("DecodeParms");
    output.remove("Length");
    output.insert("Filter", PdfObject::Name("FlateDecode".to_string()));
    Ok(PdfObject::Stream {
        dict: output,
        raw: flate_encode(&pack_samples_exact(&samples, width, height, 1, bpc)?, 9),
    })
}

fn fill_for_channels(color: &Color, channels: u8) -> [u8; 4] {
    let c = |idx: usize| -> u8 {
        (color
            .components
            .get(idx)
            .copied()
            .unwrap_or(0.0)
            .clamp(0.0, 1.0)
            * 255.0)
            .round() as u8
    };
    match channels {
        1 => [c(0), 0, 0, 0],
        4 => match color.space {
            ColorSpace::DeviceCMYK => [c(0), c(1), c(2), c(3)],
            ColorSpace::DeviceGray => {
                let k = 255u8.saturating_sub(c(0));
                [0, 0, 0, k]
            }
            _ => {
                let r = c(0) as f64 / 255.0;
                let g = c(1) as f64 / 255.0;
                let b = c(2) as f64 / 255.0;
                let k = 1.0 - r.max(g).max(b);
                if k >= 1.0 - f64::EPSILON {
                    [0, 0, 0, 255]
                } else {
                    [
                        (((1.0 - r - k) / (1.0 - k)) * 255.0).round() as u8,
                        (((1.0 - g - k) / (1.0 - k)) * 255.0).round() as u8,
                        (((1.0 - b - k) / (1.0 - k)) * 255.0).round() as u8,
                        (k * 255.0).round() as u8,
                    ]
                }
            }
        },
        _ => match color.space {
            ColorSpace::DeviceGray => {
                let g = c(0);
                [g, g, g, 0]
            }
            _ => [c(0), c(1), c(2), 0],
        },
    }
}

fn invert_affine(matrix: &Matrix) -> Option<Matrix> {
    if matrix.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let [a, b, c, d, e, f] = *matrix;
    let determinant = a * d - b * c;
    if !determinant.is_finite() || determinant.abs() <= 1e-12 {
        return None;
    }
    let inverse = [
        d / determinant,
        -b / determinant,
        -c / determinant,
        a / determinant,
        (c * f - d * e) / determinant,
        (b * e - a * f) / determinant,
    ];
    inverse
        .iter()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

fn transformed_unit_rect(matrix: &Matrix) -> ImageRect {
    let points = [
        transform_point(matrix, 0.0, 0.0),
        transform_point(matrix, 1.0, 0.0),
        transform_point(matrix, 1.0, 1.0),
        transform_point(matrix, 0.0, 1.0),
    ];
    rect_from_points(&points)
}

fn polygon_intersects_pixel(polygon: &[(f64, f64)], x: f64, y: f64) -> bool {
    let clipped = clip_polygon_axis(polygon, 0, x, true);
    let clipped = clip_polygon_axis(&clipped, 0, x + 1.0, false);
    let clipped = clip_polygon_axis(&clipped, 1, y, true);
    let clipped = clip_polygon_axis(&clipped, 1, y + 1.0, false);
    polygon_area(&clipped) > 1.0e-12
}

fn clip_polygon_axis(
    polygon: &[(f64, f64)],
    axis: usize,
    bound: f64,
    keep_greater: bool,
) -> Vec<(f64, f64)> {
    let Some(mut previous) = polygon.last().copied() else {
        return Vec::new();
    };
    let mut output = Vec::with_capacity(polygon.len() + 4);
    let coordinate = |point: (f64, f64)| if axis == 0 { point.0 } else { point.1 };
    let inside = |point: (f64, f64)| {
        if keep_greater {
            coordinate(point) >= bound
        } else {
            coordinate(point) <= bound
        }
    };
    for &current in polygon {
        let previous_inside = inside(previous);
        let current_inside = inside(current);
        if previous_inside != current_inside {
            let delta = coordinate(current) - coordinate(previous);
            if delta.abs() > f64::EPSILON {
                let t = ((bound - coordinate(previous)) / delta).clamp(0.0, 1.0);
                output.push((
                    previous.0 + (current.0 - previous.0) * t,
                    previous.1 + (current.1 - previous.1) * t,
                ));
            }
        }
        if current_inside {
            output.push(current);
        }
        previous = current;
    }
    output
}

fn polygon_area(polygon: &[(f64, f64)]) -> f64 {
    if polygon.len() < 3 {
        return 0.0;
    }
    polygon
        .iter()
        .copied()
        .zip(polygon.iter().copied().cycle().skip(1))
        .take(polygon.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        .abs()
        * 0.5
}

fn write_existing_annotation_visuals(
    reader: &PdfReader,
    page_dict: &PdfDictionary,
    out: &mut Vec<u8>,
    resources: &mut PdfDictionary,
    subtype_filter: Option<&BTreeSet<String>>,
) -> Result<()> {
    for annot_ref in resolve_annotation_refs(reader, page_dict.get("Annots"))? {
        let annot = reader.get_and_resolve(annot_ref.0, annot_ref.1)?;
        let Some(dict) = annot.as_dict() else {
            continue;
        };
        if subtype_filter.is_some_and(|filter| {
            !dict
                .get_name("Subtype")
                .is_some_and(|subtype| filter.contains(subtype))
        }) {
            continue;
        }
        write_existing_annotation_visual(reader, dict, out, resources);
    }
    Ok(())
}

fn write_existing_annotation_visual(
    reader: &PdfReader,
    dict: &PdfDictionary,
    out: &mut Vec<u8>,
    resources: &mut PdfDictionary,
) {
    let subtype = dict.get_name("Subtype").unwrap_or("Unknown");
    if subtype == "Widget" || subtype == "Link" || subtype == "Popup" {
        return;
    }
    let rect = rect_from_dict(dict, reader).unwrap_or_else(|| ImageRect::new(0.0, 0.0, 0.0, 0.0));
    let color = color_from_annotation(dict, reader);
    let opacity = dict
        .get("CA")
        .and_then(PdfObject::as_number)
        .or_else(|| dict.get("ca").and_then(PdfObject::as_number))
        .unwrap_or(0.85)
        .clamp(0.0, 1.0);
    match subtype {
        "Highlight" => {
            let gs = ensure_extgstate(resources, opacity.min(0.45));
            let quads = quad_rects(dict, reader);
            if quads.is_empty() {
                write_rect(
                    out,
                    Some(&gs),
                    rect,
                    &EditRectStyle {
                        stroke: None,
                        fill: Some(color),
                        line_width: 0.0,
                        opacity,
                    },
                );
            } else {
                for quad in quads {
                    write_rect(
                        out,
                        Some(&gs),
                        quad,
                        &EditRectStyle {
                            stroke: None,
                            fill: Some(color.clone()),
                            line_width: 0.0,
                            opacity,
                        },
                    );
                }
            }
        }
        "Underline" | "StrikeOut" | "Squiggly" => {
            for quad in nonempty_or_rect(quad_rects(dict, reader), rect) {
                let y = match subtype {
                    "StrikeOut" => quad.y + quad.height * 0.5,
                    _ => quad.y + quad.height * 0.12,
                };
                if subtype == "Squiggly" {
                    write_squiggly(out, quad.x, y, quad.x + quad.width, 2.0, &color);
                } else {
                    write_line_segment(out, quad.x, y, quad.x + quad.width, y, 1.0, &color);
                }
            }
        }
        "FreeText" => {
            write_rect(
                out,
                None,
                rect,
                &EditRectStyle {
                    stroke: Some(color.clone()),
                    fill: None,
                    line_width: 1.0,
                    opacity: 1.0,
                },
            );
            if let Some(contents) = dict.get("Contents").and_then(pdf_string_or_name) {
                let font = ensure_standard_font(resources);
                let style =
                    EditTextStyle::new((rect.height * 0.35).clamp(8.0, 14.0)).fill(Color::black());
                write_text(
                    out,
                    &font,
                    None,
                    &contents,
                    rect.x + 3.0,
                    rect.y + rect.height * 0.45,
                    &style,
                );
            }
        }
        "Ink" => {
            if let Some(paths) = ink_paths(dict, reader) {
                for points in paths {
                    write_polyline(out, &points, false, 1.5, &color);
                }
            }
        }
        "Line" => {
            if let Some(values) = number_array_from_key(dict, reader, "L") {
                if values.len() >= 4 {
                    write_line_segment(
                        out, values[0], values[1], values[2], values[3], 1.2, &color,
                    );
                }
            }
        }
        "Square" => {
            write_rect(
                out,
                None,
                rect,
                &EditRectStyle {
                    stroke: Some(color),
                    fill: None,
                    line_width: 1.0,
                    opacity: 1.0,
                },
            );
        }
        "Circle" => write_ellipse(out, rect, 1.0, &color),
        "Polygon" | "PolyLine" => {
            if let Some(points) = vertices(dict, reader) {
                write_polyline(out, &points, subtype == "Polygon", 1.0, &color);
            }
        }
        "Stamp" => {
            let label = dict
                .get_name("Name")
                .map(str::to_string)
                .or_else(|| dict.get("Contents").and_then(pdf_string_or_name))
                .unwrap_or_else(|| "STAMP".to_string());
            let spec = AnnotationSpec {
                kind: AnnotationKind::Stamp,
                rect,
                label,
                options: AnnotationOptions::default().color(color),
            };
            write_annotation_visual_to_content(out, resources, &spec);
        }
        "Text" | "FileAttachment" => {
            write_rect(
                out,
                None,
                ImageRect::new(rect.x, rect.y, rect.width.max(14.0), rect.height.max(14.0)),
                &EditRectStyle {
                    stroke: Some(Color::black()),
                    fill: Some(color),
                    line_width: 1.0,
                    opacity: 1.0,
                },
            );
        }
        _ => write_static_annotation_appearance(reader, dict, rect, out, resources),
    }
}

fn write_static_annotation_appearance(
    reader: &PdfReader,
    dict: &PdfDictionary,
    rect: ImageRect,
    out: &mut Vec<u8>,
    resources: &mut PdfDictionary,
) {
    let Some((appearance_ref, form)) = selected_normal_appearance_reference(reader, dict) else {
        return;
    };
    let Some(bbox) = form.get("BBox").and_then(PdfObject::as_array) else {
        return;
    };
    let values: Vec<f64> = bbox.iter().filter_map(PdfObject::as_number).collect();
    if values.len() != 4 {
        return;
    }
    let bw = values[2] - values[0];
    let bh = values[3] - values[1];
    if !bw.is_finite()
        || !bh.is_finite()
        || bw.abs() <= f64::EPSILON
        || bh.abs() <= f64::EPSILON
        || rect.width <= 0.0
        || rect.height <= 0.0
    {
        return;
    }
    let name = format!("OxP17Poster{}_{}", appearance_ref.0, appearance_ref.1);
    let mut xobjects = dict_resource(resources, "XObject");
    xobjects.insert(&name, reference(appearance_ref.0, appearance_ref.1));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    let sx = rect.width / bw;
    let sy = rect.height / bh;
    let tx = rect.x - values[0] * sx;
    let ty = rect.y - values[1] * sy;
    out.extend_from_slice(
        format!(
            "q {} 0 0 {} {} {} cm /{} Do Q\n",
            fmt_num(sx),
            fmt_num(sy),
            fmt_num(tx),
            fmt_num(ty),
            name
        )
        .as_bytes(),
    );
}

fn selected_normal_appearance_reference(
    reader: &PdfReader,
    dict: &PdfDictionary,
) -> Option<((u32, u16), PdfDictionary)> {
    let ap = reader.resolve(dict.get("AP")?.clone()).ok()?;
    let normal = ap.as_dict()?.get("N")?.clone();
    if let Some(reference) = normal.as_reference() {
        let object = reader.get_and_resolve(reference.0, reference.1).ok()?;
        let form = object_dictionary(&object)?.clone();
        return (form.get_name("Subtype") == Some("Form")).then_some((reference, form));
    }
    let states = reader.resolve(normal).ok()?;
    let states = states.as_dict()?;
    let state = dict.get_name("AS").unwrap_or("Off");
    let selected = states
        .get(state)
        .or_else(|| states.get("Off"))
        .or_else(|| states.entries().next().map(|(_, value)| value))?;
    let reference = selected.as_reference()?;
    let object = reader.get_and_resolve(reference.0, reference.1).ok()?;
    let form = object_dictionary(&object)?.clone();
    (form.get_name("Subtype") == Some("Form")).then_some((reference, form))
}

fn color_from_annotation(dict: &PdfDictionary, reader: &PdfReader) -> Color {
    match number_array_from_key(dict, reader, "C").as_deref() {
        Some([gray]) => Color::device_gray(*gray),
        Some([red, green, blue]) => Color::device_rgb(*red, *green, *blue),
        Some([cyan, magenta, yellow, black]) => {
            Color::device_cmyk(*cyan, *magenta, *yellow, *black)
        }
        _ => Color::device_rgb(1.0, 0.9, 0.0),
    }
}

fn quad_rects(dict: &PdfDictionary, reader: &PdfReader) -> Vec<ImageRect> {
    number_array_from_key(dict, reader, "QuadPoints")
        .unwrap_or_default()
        .chunks_exact(8)
        .map(|quad| {
            let xs = [quad[0], quad[2], quad[4], quad[6]];
            let ys = [quad[1], quad[3], quad[5], quad[7]];
            let min_x = xs.iter().copied().fold(f64::INFINITY, f64::min);
            let max_x = xs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let min_y = ys.iter().copied().fold(f64::INFINITY, f64::min);
            let max_y = ys.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            ImageRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
        })
        .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
        .collect()
}

fn nonempty_or_rect(rects: Vec<ImageRect>, fallback: ImageRect) -> Vec<ImageRect> {
    if rects.is_empty() && fallback.width > 0.0 && fallback.height > 0.0 {
        vec![fallback]
    } else {
        rects
    }
}

fn number_array_from_key(dict: &PdfDictionary, reader: &PdfReader, key: &str) -> Option<Vec<f64>> {
    let object = reader.resolve(dict.get(key)?.clone()).ok()?;
    let values = object
        .as_array()?
        .iter()
        .filter_map(|item| reader.resolve(item.clone()).ok()?.as_number())
        .collect::<Vec<_>>();
    Some(values)
}

fn write_line_segment(
    out: &mut Vec<u8>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    width: f64,
    color: &Color,
) {
    out.extend_from_slice(b"q\n");
    write_stroke_color(out, color);
    out.extend_from_slice(
        format!(
            "{} w {} {} m {} {} l S\nQ\n",
            fmt_num(width.max(0.1)),
            fmt_num(x1),
            fmt_num(y1),
            fmt_num(x2),
            fmt_num(y2)
        )
        .as_bytes(),
    );
}

fn write_squiggly(out: &mut Vec<u8>, x0: f64, y: f64, x1: f64, amp: f64, color: &Color) {
    if x1 <= x0 {
        return;
    }
    out.extend_from_slice(b"q\n");
    write_stroke_color(out, color);
    out.extend_from_slice(format!("1 w {} {} m\n", fmt_num(x0), fmt_num(y)).as_bytes());
    let step = 4.0;
    let mut x = x0 + step;
    let mut up = true;
    while x < x1 {
        let yy = y + if up { amp } else { -amp };
        out.extend_from_slice(format!("{} {} l\n", fmt_num(x), fmt_num(yy)).as_bytes());
        x += step;
        up = !up;
    }
    out.extend_from_slice(format!("{} {} l S\nQ\n", fmt_num(x1), fmt_num(y)).as_bytes());
}

fn ink_paths(dict: &PdfDictionary, reader: &PdfReader) -> Option<Vec<Vec<(f64, f64)>>> {
    let object = reader.resolve(dict.get("InkList")?.clone()).ok()?;
    let mut paths = Vec::new();
    for path_obj in object.as_array()? {
        let path = reader.resolve(path_obj.clone()).ok()?;
        let mut points = Vec::new();
        for pair in path.as_array()?.chunks_exact(2) {
            let x = reader.resolve(pair[0].clone()).ok()?.as_number()?;
            let y = reader.resolve(pair[1].clone()).ok()?.as_number()?;
            points.push((x, y));
        }
        if points.len() >= 2 {
            paths.push(points);
        }
    }
    Some(paths)
}

fn vertices(dict: &PdfDictionary, reader: &PdfReader) -> Option<Vec<(f64, f64)>> {
    let values = number_array_from_key(dict, reader, "Vertices")?;
    let points = values
        .chunks_exact(2)
        .map(|pair| (pair[0], pair[1]))
        .collect::<Vec<_>>();
    (points.len() >= 2).then_some(points)
}

fn write_polyline(
    out: &mut Vec<u8>,
    points: &[(f64, f64)],
    close: bool,
    width: f64,
    color: &Color,
) {
    let Some((first_x, first_y)) = points.first().copied() else {
        return;
    };
    out.extend_from_slice(b"q\n");
    write_stroke_color(out, color);
    out.extend_from_slice(
        format!(
            "{} w {} {} m\n",
            fmt_num(width.max(0.1)),
            fmt_num(first_x),
            fmt_num(first_y)
        )
        .as_bytes(),
    );
    for (x, y) in points.iter().copied().skip(1) {
        out.extend_from_slice(format!("{} {} l\n", fmt_num(x), fmt_num(y)).as_bytes());
    }
    out.extend_from_slice(if close { b"h S\nQ\n" } else { b"S\nQ\n" });
}

fn write_ellipse(out: &mut Vec<u8>, rect: ImageRect, width: f64, color: &Color) {
    if rect.width <= 0.0 || rect.height <= 0.0 {
        return;
    }
    let k = 0.552_284_749_830_793_6;
    let rx = rect.width / 2.0;
    let ry = rect.height / 2.0;
    let cx = rect.x + rx;
    let cy = rect.y + ry;
    out.extend_from_slice(b"q\n");
    write_stroke_color(out, color);
    out.extend_from_slice(
        format!(
            "{} w {} {} m\n",
            fmt_num(width.max(0.1)),
            fmt_num(cx + rx),
            fmt_num(cy)
        )
        .as_bytes(),
    );
    let commands = [
        (cx + rx, cy + k * ry, cx + k * rx, cy + ry, cx, cy + ry),
        (cx - k * rx, cy + ry, cx - rx, cy + k * ry, cx - rx, cy),
        (cx - rx, cy - k * ry, cx - k * rx, cy - ry, cx, cy - ry),
        (cx + k * rx, cy - ry, cx + rx, cy - k * ry, cx + rx, cy),
    ];
    for (x1, y1, x2, y2, x3, y3) in commands {
        out.extend_from_slice(
            format!(
                "{} {} {} {} {} {} c\n",
                fmt_num(x1),
                fmt_num(y1),
                fmt_num(x2),
                fmt_num(y2),
                fmt_num(x3),
                fmt_num(y3)
            )
            .as_bytes(),
        );
    }
    out.extend_from_slice(b"S\nQ\n");
}

fn apply_annotation_edits(
    reader: &PdfReader,
    page_dict: &mut PdfDictionary,
    redactions: &[RedactionEdit],
    edits: &[AnnotationEdit],
    options: AnnotationApplyOptions<'_>,
    changes: &mut ChangeSet,
) -> Result<()> {
    let mut annots = resolve_annotation_refs(reader, page_dict.get("Annots"))?;
    if !redactions.is_empty()
        || options.remove_widgets
        || options.flatten_annotations
        || options.attachment_policy == AttachmentRedactionPolicy::RemoveAll
    {
        let mut kept = Vec::new();
        for annot_ref in annots {
            let annot = reader.get_and_resolve(annot_ref.0, annot_ref.1)?;
            let Some(dict) = annot.as_dict() else {
                kept.push(annot_ref);
                continue;
            };
            let remove_for_redaction = rect_from_dict(dict, reader)
                .map(|rect| {
                    redactions
                        .iter()
                        .any(|redaction| rects_intersect(rect, redaction.rect))
                })
                .unwrap_or(false);
            let remove_widget =
                options.remove_widgets && dict.get_name("Subtype") == Some("Widget");
            let remove_flattened = dict.get_name("Subtype").is_some_and(|subtype| {
                (options.flatten_annotations && subtype != "Widget")
                    || options.flatten_annotation_subtypes.contains(subtype)
            });
            let remove_attachment = dict.get_name("Subtype") == Some("FileAttachment")
                && match options.attachment_policy {
                    AttachmentRedactionPolicy::RemoveAll => true,
                    AttachmentRedactionPolicy::RemoveOverlapping => remove_for_redaction,
                    AttachmentRedactionPolicy::Keep => false,
                };
            if !remove_for_redaction && !remove_widget && !remove_flattened && !remove_attachment {
                kept.push(annot_ref);
            }
        }
        annots = kept;
    }

    for edit in edits {
        match edit {
            AnnotationEdit::Add(spec) => {
                let appearance = (spec.kind != AnnotationKind::Link)
                    .then(|| annotation_appearance(spec, changes))
                    .transpose()?;
                let annot_number = changes.alloc();
                let annot = annotation_dictionary(spec, appearance);
                changes.insert_new(annot_number, PdfObject::Dictionary(annot));
                annots.push((annot_number, 0));
            }
            AnnotationEdit::EditContents { index, contents } => {
                if let Some((number, generation)) = annots.get(*index).copied() {
                    let object = changes.current_object(reader, number, generation)?;
                    let mut dict = object.as_dict().cloned().ok_or_else(|| {
                        WellfriendError::MalformedPdf(format!(
                            "annotation {number} {generation} is not a dictionary"
                        ))
                    })?;
                    dict.insert("Contents", pdf_text_string(contents));
                    changes.insert_existing(number, generation, PdfObject::Dictionary(dict));
                }
            }
            AnnotationEdit::DeleteInRect { rect } => {
                let mut kept = Vec::new();
                for annot_ref in annots {
                    let annot = changes.current_object(reader, annot_ref.0, annot_ref.1)?;
                    let delete = annot
                        .as_dict()
                        .and_then(|dict| rect_from_dict(dict, reader))
                        .map(|annot_rect| rects_intersect(annot_rect, *rect))
                        .unwrap_or(false);
                    if !delete {
                        kept.push(annot_ref);
                    }
                }
                annots = kept;
            }
        }
    }

    if annots.is_empty() {
        page_dict.remove("Annots");
    } else {
        page_dict.insert(
            "Annots",
            PdfObject::Array(
                annots
                    .into_iter()
                    .map(|(number, generation)| reference(number, generation))
                    .collect(),
            ),
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct AnnotationApplyOptions<'a> {
    remove_widgets: bool,
    flatten_annotations: bool,
    flatten_annotation_subtypes: &'a BTreeSet<String>,
    attachment_policy: AttachmentRedactionPolicy,
}

fn resolve_annotation_refs(
    reader: &PdfReader,
    annots: Option<&PdfObject>,
) -> Result<Vec<(u32, u16)>> {
    let Some(annots) = annots else {
        return Ok(Vec::new());
    };
    let resolved = reader.resolve(annots.clone())?;
    Ok(resolved
        .as_array()
        .map(|items| items.iter().filter_map(PdfObject::as_reference).collect())
        .unwrap_or_default())
}

fn annotation_appearance(spec: &AnnotationSpec, changes: &mut ChangeSet) -> Result<u32> {
    let number = changes.alloc();
    let mut raw = Vec::new();
    let width = spec.rect.width.max(1.0);
    let height = spec.rect.height.max(1.0);
    match spec.kind {
        AnnotationKind::Highlight => {
            write_fill_color(&mut raw, &spec.options.color);
            raw.extend_from_slice(
                format!("0 0 {} {} re f\n", fmt_num(width), fmt_num(height)).as_bytes(),
            );
        }
        AnnotationKind::TextNote => {
            write_fill_color(&mut raw, &spec.options.color);
            raw.extend_from_slice(
                format!("0 0 {} {} re f\n", fmt_num(width), fmt_num(height)).as_bytes(),
            );
            raw.extend_from_slice(b"0 0 0 RG 1 w 0 0 16 16 re S\n");
        }
        AnnotationKind::Stamp => {
            raw.extend_from_slice(b"q 0.9 0.95 1 rg 0 0 0 RG 1 w\n");
            raw.extend_from_slice(
                format!("0 0 {} {} re B\nQ\n", fmt_num(width), fmt_num(height)).as_bytes(),
            );
            let font = "OxAnnF1";
            raw.extend_from_slice(
                format!(
                    "BT /{} {} Tf 0 0 0 rg 4 {} Td <{}> Tj ET\n",
                    font,
                    fmt_num((height * 0.38).clamp(8.0, 16.0)),
                    fmt_num(height * 0.38),
                    hex_string(&encode_win_ansi_lossy(&spec.label))
                )
                .as_bytes(),
            );
        }
        AnnotationKind::Link => {}
    }
    let mut form_dict = form_xobject_dict(width, height);
    if spec.kind == AnnotationKind::Stamp {
        let mut resources = PdfDictionary::empty();
        let mut fonts = PdfDictionary::empty();
        fonts.insert(
            "OxAnnF1",
            PdfObject::Dictionary(dict(&[
                ("Type", PdfObject::Name("Font".to_string())),
                ("Subtype", PdfObject::Name("Type1".to_string())),
                ("BaseFont", PdfObject::Name("Helvetica".to_string())),
                ("Encoding", PdfObject::Name("WinAnsiEncoding".to_string())),
            ])),
        );
        resources.insert("Font", PdfObject::Dictionary(fonts));
        form_dict.insert("Resources", PdfObject::Dictionary(resources));
    }
    changes.insert_new(
        number,
        PdfObject::Stream {
            dict: form_dict,
            raw,
        },
    );
    Ok(number)
}

fn annotation_dictionary(spec: &AnnotationSpec, appearance_number: Option<u32>) -> PdfDictionary {
    let mut annot = PdfDictionary::empty();
    annot.insert("Type", PdfObject::Name("Annot".to_string()));
    annot.insert(
        "Subtype",
        PdfObject::Name(
            match spec.kind {
                AnnotationKind::Highlight => "Highlight",
                AnnotationKind::TextNote => "Text",
                AnnotationKind::Stamp => "Stamp",
                AnnotationKind::Link => "Link",
            }
            .to_string(),
        ),
    );
    annot.insert("Rect", rect_array(spec.rect));
    annot.insert("F", PdfObject::Integer(4));
    if let Some(author) = &spec.options.author {
        annot.insert("T", pdf_text_string(author));
    }
    let contents = if spec.options.contents.is_some() {
        spec.options.contents.as_deref().unwrap_or("")
    } else {
        &spec.label
    };
    if !contents.is_empty() {
        annot.insert("Contents", pdf_text_string(contents));
    }
    if spec.kind != AnnotationKind::Link {
        annot.insert("C", color_array(&spec.options.color));
        annot.insert("CA", pdf_number(spec.options.opacity.clamp(0.0, 1.0)));
        if spec.kind == AnnotationKind::Highlight {
            annot.insert("QuadPoints", highlight_quad_points(spec.rect));
        }
        if let Some(appearance_number) = appearance_number {
            let mut ap = PdfDictionary::empty();
            ap.insert("N", reference(appearance_number, 0));
            annot.insert("AP", PdfObject::Dictionary(ap));
        }
    } else {
        let mut action = PdfDictionary::empty();
        action.insert("S", PdfObject::Name("URI".to_string()));
        action.insert("URI", PdfObject::String(spec.label.as_bytes().to_vec()));
        annot.insert("A", PdfObject::Dictionary(action));
        annot.insert(
            "Border",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
            ]),
        );
    }
    annot
}

fn write_annotation_visual_to_content(
    out: &mut Vec<u8>,
    resources: &mut PdfDictionary,
    spec: &AnnotationSpec,
) {
    match spec.kind {
        AnnotationKind::Highlight => {
            let gs = ensure_extgstate(resources, spec.options.opacity);
            let style = EditRectStyle {
                stroke: None,
                fill: Some(spec.options.color.clone()),
                line_width: 0.0,
                opacity: spec.options.opacity,
            };
            write_rect(out, Some(&gs), spec.rect, &style);
        }
        AnnotationKind::Stamp => {
            let style = EditRectStyle {
                stroke: Some(Color::black()),
                fill: Some(spec.options.color.clone()),
                line_width: 1.0,
                opacity: spec.options.opacity,
            };
            write_rect(out, None, spec.rect, &style);
            let font = ensure_standard_font(resources);
            let text_style = EditTextStyle::new((spec.rect.height * 0.35).clamp(8.0, 18.0));
            write_text(
                out,
                &font,
                None,
                &spec.label,
                spec.rect.x + 4.0,
                spec.rect.y + spec.rect.height * 0.38,
                &text_style,
            );
        }
        AnnotationKind::TextNote => {
            let style = EditRectStyle {
                stroke: Some(Color::black()),
                fill: Some(spec.options.color.clone()),
                line_width: 1.0,
                opacity: spec.options.opacity,
            };
            write_rect(out, None, spec.rect, &style);
        }
        AnnotationKind::Link => {}
    }
}

#[derive(Debug, Clone)]
struct FieldInfo {
    object_ref: (u32, u16),
    name: String,
    dict: PdfDictionary,
    widgets: Vec<WidgetInfo>,
    current_value: Option<FormValue>,
}

#[derive(Debug, Clone)]
struct WidgetInfo {
    object_ref: (u32, u16),
    dict: PdfDictionary,
    rect: ImageRect,
    page_number: usize,
}

fn collect_acroform_fields(reader: &PdfReader, pages: &[PdfPage]) -> Result<Vec<FieldInfo>> {
    let catalog = reader
        .root_reference()
        .and_then(|(n, g)| reader.get_and_resolve(n, g).ok())
        .and_then(|obj| obj.as_dict().cloned())
        .ok_or_else(|| WellfriendError::MalformedPdf("catalog is missing".to_string()))?;
    let Some(acroform_obj) = catalog.get("AcroForm") else {
        return Ok(Vec::new());
    };
    let acroform = reader.resolve(acroform_obj.clone())?;
    let Some(acroform_dict) = acroform.as_dict() else {
        return Ok(Vec::new());
    };
    let Some(fields) = acroform_dict
        .get("Fields")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_array().map(|items| items.to_vec()))
    else {
        return Ok(Vec::new());
    };
    let mut page_annots = BTreeMap::new();
    for page in pages {
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            continue;
        };
        for annot_ref in resolve_annotation_refs(reader, page_dict.get("Annots"))? {
            page_annots.insert(annot_ref, page.page_number);
        }
    }
    let mut out = Vec::new();
    for field in fields {
        walk_field_for_editing(reader, &field, "", &page_annots, 0, &mut out)?;
    }
    Ok(out)
}

fn walk_field_for_editing(
    reader: &PdfReader,
    object: &PdfObject,
    parent_name: &str,
    page_annots: &BTreeMap<(u32, u16), usize>,
    depth: usize,
    out: &mut Vec<FieldInfo>,
) -> Result<()> {
    if depth > 32 {
        return Ok(());
    }
    let Some(object_ref) = object.as_reference() else {
        return Ok(());
    };
    let resolved = reader.get_and_resolve(object_ref.0, object_ref.1)?;
    let Some(dict) = resolved.as_dict().cloned() else {
        return Ok(());
    };
    let name = qualified_field_name(parent_name, dict.get("T"));
    let kids = dict
        .get("Kids")
        .and_then(|obj| reader.resolve(obj.clone()).ok())
        .and_then(|obj| obj.as_array().map(|items| items.to_vec()))
        .unwrap_or_default();
    let child_fields: Vec<PdfObject> = kids
        .iter()
        .filter(|kid| kid_is_editable_field(reader, kid))
        .cloned()
        .collect();
    if !child_fields.is_empty() {
        for kid in child_fields {
            walk_field_for_editing(reader, &kid, &name, page_annots, depth + 1, out)?;
        }
        return Ok(());
    }
    let Some(field_type) = inherited_field_name(reader, &dict, "FT") else {
        return Ok(());
    };
    let mut widgets = Vec::new();
    if dict.get_name("Subtype") == Some("Widget") && dict.get("Rect").is_some() {
        if let Some(widget) = widget_info(reader, object_ref, &dict, page_annots) {
            widgets.push(widget);
        }
    }
    for kid in kids {
        if let Some(kid_ref) = kid.as_reference() {
            let kid_obj = reader.get_and_resolve(kid_ref.0, kid_ref.1)?;
            if let Some(kid_dict) = kid_obj.as_dict() {
                if kid_dict.get_name("Subtype") == Some("Widget") {
                    if let Some(widget) = widget_info(reader, kid_ref, kid_dict, page_annots) {
                        widgets.push(widget);
                    }
                }
            }
        }
    }
    out.push(FieldInfo {
        object_ref,
        name,
        current_value: inherited_field_object(reader, &dict, "V")
            .and_then(|value| form_value_from_object(&field_type, &value)),
        dict,
        widgets,
    });
    Ok(())
}

fn update_field_value(
    reader: &PdfReader,
    changes: &mut ChangeSet,
    field: &FieldInfo,
    value: &FormValue,
) -> Result<()> {
    let mut field_dict = changes
        .current_object(reader, field.object_ref.0, field.object_ref.1)?
        .as_dict()
        .cloned()
        .unwrap_or_else(|| field.dict.clone());
    let value_obj = form_value_pdf_object(value);
    field_dict.insert("V", value_obj.clone());
    if matches!(value, FormValue::Checkbox(_)) {
        let state = checkbox_state(value);
        field_dict.insert("AS", PdfObject::Name(state.to_string()));
    }
    changes.insert_existing(
        field.object_ref.0,
        field.object_ref.1,
        PdfObject::Dictionary(field_dict),
    );

    for widget in &field.widgets {
        let mut widget_dict = changes
            .current_object(reader, widget.object_ref.0, widget.object_ref.1)?
            .as_dict()
            .cloned()
            .unwrap_or_else(|| widget.dict.clone());
        widget_dict.insert("V", value_obj.clone());
        if matches!(value, FormValue::Checkbox(_)) {
            widget_dict.insert("AS", PdfObject::Name(checkbox_state(value).to_string()));
        }
        let ap_number = changes.alloc();
        changes.insert_new(
            ap_number,
            appearance_stream_for_form_value(widget.rect, value),
        );
        let mut ap = PdfDictionary::empty();
        ap.insert("N", reference(ap_number, 0));
        widget_dict.insert("AP", PdfObject::Dictionary(ap));
        changes.insert_existing(
            widget.object_ref.0,
            widget.object_ref.1,
            PdfObject::Dictionary(widget_dict),
        );
    }
    Ok(())
}

fn remove_acroform_from_catalog(reader: &PdfReader, changes: &mut ChangeSet) -> Result<()> {
    let (root, generation) = reader.root_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf("flatten forms: trailer is missing /Root".to_string())
    })?;
    let object = changes.current_object(reader, root, generation)?;
    let mut catalog = object.as_dict().cloned().ok_or_else(|| {
        WellfriendError::MalformedPdf("flatten forms: /Root is not a dictionary".to_string())
    })?;
    catalog.remove("AcroForm");
    changes.insert_existing(root, generation, PdfObject::Dictionary(catalog));
    Ok(())
}

fn widget_info(
    reader: &PdfReader,
    object_ref: (u32, u16),
    dict: &PdfDictionary,
    page_annots: &BTreeMap<(u32, u16), usize>,
) -> Option<WidgetInfo> {
    Some(WidgetInfo {
        object_ref,
        dict: dict.clone(),
        rect: rect_from_dict(dict, reader)?,
        page_number: *page_annots.get(&object_ref).unwrap_or(&1),
    })
}

fn kid_is_editable_field(reader: &PdfReader, object: &PdfObject) -> bool {
    let Ok(resolved) = reader.resolve(object.clone()) else {
        return false;
    };
    let Some(dict) = resolved.as_dict() else {
        return false;
    };
    dict.contains_key("T") || dict.contains_key("FT")
}

fn qualified_field_name(parent: &str, local: Option<&PdfObject>) -> String {
    let local = local.and_then(pdf_string_or_name).unwrap_or_default();
    match (parent.is_empty(), local.is_empty()) {
        (true, true) => String::new(),
        (true, false) => local,
        (false, true) => parent.to_string(),
        (false, false) => format!("{parent}.{local}"),
    }
}

fn inherited_field_name(reader: &PdfReader, dict: &PdfDictionary, key: &str) -> Option<String> {
    inherited_field_object(reader, dict, key).and_then(|obj| obj.as_name().map(str::to_string))
}

fn inherited_field_object(
    reader: &PdfReader,
    dict: &PdfDictionary,
    key: &str,
) -> Option<PdfObject> {
    let mut current = dict.clone();
    for _ in 0..32 {
        if let Some(value) = current.get(key) {
            return reader.resolve(value.clone()).ok();
        }
        let parent = current.get("Parent")?.clone();
        current = reader.resolve(parent).ok()?.as_dict()?.clone();
    }
    None
}

fn form_value_from_object(field_type: &str, value: &PdfObject) -> Option<FormValue> {
    match field_type {
        "Btn" => Some(FormValue::Checkbox(
            value.as_name().map(|name| name != "Off").unwrap_or(false),
        )),
        "Ch" => Some(FormValue::Choice(
            pdf_string_or_name(value).unwrap_or_default(),
        )),
        _ => Some(FormValue::Text(
            pdf_string_or_name(value).unwrap_or_default(),
        )),
    }
}

fn form_value_pdf_object(value: &FormValue) -> PdfObject {
    match value {
        FormValue::Text(text) | FormValue::Choice(text) => pdf_text_string(text),
        FormValue::Checkbox(checked) => {
            PdfObject::Name(if *checked { "Yes" } else { "Off" }.to_string())
        }
    }
}

fn checkbox_state(value: &FormValue) -> &'static str {
    match value {
        FormValue::Checkbox(true) => "Yes",
        _ => "Off",
    }
}

fn appearance_stream_for_form_value(rect: ImageRect, value: &FormValue) -> PdfObject {
    let width = rect.width.max(1.0);
    let height = rect.height.max(1.0);
    let mut raw = Vec::new();
    let mut form_dict = form_xobject_dict(width, height);
    match value {
        FormValue::Text(text) | FormValue::Choice(text) => {
            let mut resources = PdfDictionary::empty();
            let mut fonts = PdfDictionary::empty();
            fonts.insert(
                "OxFormF1",
                PdfObject::Dictionary(dict(&[
                    ("Type", PdfObject::Name("Font".to_string())),
                    ("Subtype", PdfObject::Name("Type1".to_string())),
                    ("BaseFont", PdfObject::Name("Helvetica".to_string())),
                    ("Encoding", PdfObject::Name("WinAnsiEncoding".to_string())),
                ])),
            );
            resources.insert("Font", PdfObject::Dictionary(fonts));
            form_dict.insert("Resources", PdfObject::Dictionary(resources));
            raw.extend_from_slice(
                format!(
                    "q 1 1 1 rg 0 0 {} {} re f 0 0 0 RG 1 w 0 0 {} {} re S Q\n",
                    fmt_num(width),
                    fmt_num(height),
                    fmt_num(width),
                    fmt_num(height)
                )
                .as_bytes(),
            );
            raw.extend_from_slice(
                format!(
                    "BT /OxFormF1 {} Tf 0 0 0 rg 3 {} Td <{}> Tj ET\n",
                    fmt_num((height * 0.45).clamp(8.0, 14.0)),
                    fmt_num(height * 0.35),
                    hex_string(&encode_win_ansi_lossy(text))
                )
                .as_bytes(),
            );
        }
        FormValue::Checkbox(checked) => {
            raw.extend_from_slice(
                format!(
                    "q 1 1 1 rg 0 0 {} {} re f 0 0 0 RG 1 w 0 0 {} {} re S\n",
                    fmt_num(width),
                    fmt_num(height),
                    fmt_num(width),
                    fmt_num(height)
                )
                .as_bytes(),
            );
            if *checked {
                raw.extend_from_slice(
                    format!(
                        "2 w 3 {} m {} 3 l {} {} l S\n",
                        fmt_num(height * 0.5),
                        fmt_num(width * 0.4),
                        fmt_num(width - 3.0),
                        fmt_num(height - 3.0)
                    )
                    .as_bytes(),
                );
            }
            raw.extend_from_slice(b"Q\n");
        }
    }
    PdfObject::Stream {
        dict: form_dict,
        raw,
    }
}

fn write_edit_command(
    out: &mut Vec<u8>,
    command: &EditCommand,
    resources: &mut PdfDictionary,
    changes: &mut ChangeSet,
) -> Result<()> {
    match command {
        EditCommand::Text { text, x, y, style } => {
            let font = ensure_standard_font(resources);
            let gs = ensure_extgstate(resources, style.opacity);
            write_text(out, &font, Some(&gs), text, *x, *y, style);
        }
        EditCommand::Rect { rect, style } => {
            let gs = ensure_extgstate(resources, style.opacity);
            write_rect(out, Some(&gs), *rect, style);
        }
        EditCommand::Image {
            image,
            rect,
            opacity,
        } => {
            let smask_number = if image.smask.is_some() {
                Some(changes.alloc())
            } else {
                None
            };
            let image_number = changes.alloc();
            if let (Some(number), Some(mask)) = (smask_number, image.smask.as_ref()) {
                changes.insert_new(
                    number,
                    PdfObject::Stream {
                        dict: smask_dict(mask),
                        raw: mask.data.clone(),
                    },
                );
            }
            changes.insert_new(
                image_number,
                PdfObject::Stream {
                    dict: image_dict(image, smask_number),
                    raw: image.data.clone(),
                },
            );
            let image_name = add_xobject(resources, image_number);
            let gs = ensure_extgstate(resources, *opacity);
            write_image(out, &image_name, Some(&gs), *rect);
        }
    }
    Ok(())
}

fn write_text(
    out: &mut Vec<u8>,
    font: &str,
    gs: Option<&str>,
    text: &str,
    x: f64,
    y: f64,
    style: &EditTextStyle,
) {
    let rotation = style.rotation_degrees.to_radians();
    let cos = rotation.cos();
    let sin = rotation.sin();
    out.extend_from_slice(b"q\n");
    if let Some(gs) = gs {
        out.extend_from_slice(format!("/{gs} gs\n").as_bytes());
    }
    write_fill_color(out, &style.fill);
    out.extend_from_slice(
        format!(
            "BT /{} {} Tf {} {} {} {} {} {} Tm <{}> Tj ET\nQ\n",
            font,
            fmt_num(style.font_size),
            fmt_num(cos),
            fmt_num(sin),
            fmt_num(-sin),
            fmt_num(cos),
            fmt_num(x),
            fmt_num(y),
            hex_string(&encode_win_ansi_lossy(text))
        )
        .as_bytes(),
    );
}

fn write_rect(out: &mut Vec<u8>, gs: Option<&str>, rect: ImageRect, style: &EditRectStyle) {
    out.extend_from_slice(b"q\n");
    if let Some(gs) = gs {
        out.extend_from_slice(format!("/{gs} gs\n").as_bytes());
    }
    out.extend_from_slice(format!("{} w\n", fmt_num(style.line_width.max(0.0))).as_bytes());
    if let Some(color) = &style.stroke {
        write_stroke_color(out, color);
    }
    if let Some(color) = &style.fill {
        write_fill_color(out, color);
    }
    out.extend_from_slice(
        format!(
            "{} {} {} {} re\n{}\nQ\n",
            fmt_num(rect.x),
            fmt_num(rect.y),
            fmt_num(rect.width),
            fmt_num(rect.height),
            match (style.fill.is_some(), style.stroke.is_some()) {
                (true, true) => "B",
                (true, false) => "f",
                (false, true) => "S",
                (false, false) => "n",
            }
        )
        .as_bytes(),
    );
}

fn write_image(out: &mut Vec<u8>, image_name: &str, gs: Option<&str>, rect: ImageRect) {
    out.extend_from_slice(b"q\n");
    if let Some(gs) = gs {
        out.extend_from_slice(format!("/{gs} gs\n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "{} 0 0 {} {} {} cm\n/{} Do\nQ\n",
            fmt_num(rect.width),
            fmt_num(rect.height),
            fmt_num(rect.x),
            fmt_num(rect.y),
            image_name
        )
        .as_bytes(),
    );
}

fn ensure_standard_font(resources: &mut PdfDictionary) -> String {
    let mut fonts = dict_resource(resources, "Font");
    let name = next_resource_name(&fonts, "OxEdF");
    fonts.insert(
        &name,
        PdfObject::Dictionary(dict(&[
            ("Type", PdfObject::Name("Font".to_string())),
            ("Subtype", PdfObject::Name("Type1".to_string())),
            ("BaseFont", PdfObject::Name("Helvetica".to_string())),
            ("Encoding", PdfObject::Name("WinAnsiEncoding".to_string())),
        ])),
    );
    resources.insert("Font", PdfObject::Dictionary(fonts));
    name
}

fn ensure_extgstate(resources: &mut PdfDictionary, opacity: f64) -> String {
    let mut states = dict_resource(resources, "ExtGState");
    let name = next_resource_name(&states, "OxEdGs");
    let alpha = opacity.clamp(0.0, 1.0);
    states.insert(
        &name,
        PdfObject::Dictionary(dict(&[
            ("Type", PdfObject::Name("ExtGState".to_string())),
            ("ca", pdf_number(alpha)),
            ("CA", pdf_number(alpha)),
        ])),
    );
    resources.insert("ExtGState", PdfObject::Dictionary(states));
    name
}

fn add_xobject(resources: &mut PdfDictionary, number: u32) -> String {
    let mut xobjects = dict_resource(resources, "XObject");
    let name = next_resource_name(&xobjects, "OxEdIm");
    xobjects.insert(&name, reference(number, 0));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    name
}

fn dict_resource(resources: &PdfDictionary, key: &str) -> PdfDictionary {
    resources
        .get(key)
        .and_then(PdfObject::as_dict)
        .cloned()
        .unwrap_or_else(PdfDictionary::empty)
}

fn next_resource_name(dict: &PdfDictionary, prefix: &str) -> String {
    let mut idx = 1usize;
    loop {
        let candidate = format!("{prefix}{idx}");
        if !dict.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

fn write_full_rewrite(reader: &PdfReader, changes: Vec<IncrementalObject>) -> Result<Vec<u8>> {
    if reader.is_encrypted() {
        return Err(WellfriendError::UnsupportedFeature(
            "editing full rewrite does not re-encrypt encrypted inputs".to_string(),
        ));
    }
    let mut changed = BTreeMap::new();
    for object in changes {
        if object.generation != 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "editing full rewrite currently supports generation-0 updates only".to_string(),
            ));
        }
        changed.insert(object.number, object.object);
    }

    let mut objects = BTreeMap::new();
    for (number, generation) in reader.object_ids() {
        if generation != 0 {
            return Err(WellfriendError::UnsupportedFeature(
                "editing full rewrite currently supports generation-0 source objects only"
                    .to_string(),
            ));
        }
        let object = reader.get_object(number, generation)?;
        if is_xref_stream(&object) {
            continue;
        }
        objects.insert(number, changed.remove(&number).unwrap_or(object));
    }
    for (number, object) in changed {
        objects.insert(number, object);
    }

    let (root, root_generation) = reader.root_reference().ok_or_else(|| {
        WellfriendError::MalformedPdf("editing full rewrite: trailer is missing /Root".to_string())
    })?;
    if root_generation != 0 {
        return Err(WellfriendError::UnsupportedFeature(
            "editing full rewrite currently supports generation-0 /Root only".to_string(),
        ));
    }
    let info = match reader.info_reference() {
        Some((number, 0)) => Some(number),
        Some(_) => {
            return Err(WellfriendError::UnsupportedFeature(
                "editing full rewrite currently supports generation-0 /Info only".to_string(),
            ))
        }
        None => None,
    };
    retain_reachable_objects(&mut objects, root, info);

    let outputs = objects
        .into_iter()
        .map(|(number, object)| OutputObject { number, object })
        .collect();
    PdfWriter::new(outputs, root)
        .with_info(info)
        .with_id(reader.first_file_id())
        .with_mode(WriterMode::XrefStreamWithObjStm)
        .write()
}

fn retain_reachable_objects(objects: &mut BTreeMap<u32, PdfObject>, root: u32, info: Option<u32>) {
    let mut stack = vec![root];
    if let Some(info) = info {
        stack.push(info);
    }
    let mut seen = BTreeSet::new();
    while let Some(number) = stack.pop() {
        if !seen.insert(number) {
            continue;
        }
        if let Some(object) = objects.get(&number) {
            collect_references(object, &mut stack);
        }
    }
    objects.retain(|number, _| seen.contains(number));
}

fn collect_references(object: &PdfObject, out: &mut Vec<u32>) {
    match object {
        PdfObject::Reference { number, .. } => out.push(*number),
        PdfObject::Array(items) => {
            for item in items {
                collect_references(item, out);
            }
        }
        PdfObject::Dictionary(dict) => {
            for (_, value) in dict.entries() {
                collect_references(value, out);
            }
        }
        PdfObject::Stream { dict, .. } => {
            for (_, value) in dict.entries() {
                collect_references(value, out);
            }
        }
        PdfObject::Boolean(_)
        | PdfObject::Integer(_)
        | PdfObject::Real(_)
        | PdfObject::String(_)
        | PdfObject::Name(_)
        | PdfObject::Null => {}
    }
}

fn is_xref_stream(object: &PdfObject) -> bool {
    matches!(object, PdfObject::Stream { dict, .. } if dict.get_name("Type") == Some("XRef"))
}

fn next_free_object_number(reader: &PdfReader) -> u32 {
    let max_seen = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .max()
        .unwrap_or(0);
    let trailer_size = reader.size().unwrap_or(0).max(0) as u32;
    max_seen.max(trailer_size.saturating_sub(1)) + 1
}

fn image_color_space(channels: u8) -> Result<&'static str> {
    match channels {
        1 => Ok("DeviceGray"),
        3 => Ok("DeviceRGB"),
        4 => Ok("DeviceCMYK"),
        _ => Err(WellfriendError::UnsupportedFeature(format!(
            "editing: unsupported image channel count {channels}"
        ))),
    }
}

fn edit_image_from_raw(raw: RawImage) -> Result<EditImage> {
    if !raw.is_valid() || raw.bits_per_sample != 8 {
        return Err(WellfriendError::MalformedPdf(
            "editing: image samples must be non-empty 8-bit data".to_string(),
        ));
    }
    let mut samples = Vec::with_capacity(raw.pixel_count() * 3);
    let mut alpha = Vec::with_capacity(raw.pixel_count());
    match raw.channels {
        3 => samples = raw.pixels,
        4 => {
            for px in raw.pixels.chunks_exact(4) {
                samples.extend_from_slice(&px[..3]);
                alpha.push(px[3]);
            }
        }
        other => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "editing: unsupported raw image channel count {other}"
            )))
        }
    }
    let smask = (!alpha.is_empty()).then(|| EditSoftMask {
        width: raw.width,
        height: raw.height,
        data: flate_encode(&alpha, 9),
    });
    Ok(EditImage {
        width: raw.width,
        height: raw.height,
        color_space: "DeviceRGB",
        bits_per_component: 8,
        data: flate_encode(&samples, 9),
        filter: ImageFilter::FlateDecode,
        smask,
    })
}

fn image_dict(image: &EditImage, smask_number: Option<u32>) -> PdfDictionary {
    let mut out = dict(&[
        ("Type", PdfObject::Name("XObject".to_string())),
        ("Subtype", PdfObject::Name("Image".to_string())),
        ("Width", PdfObject::Integer(i64::from(image.width))),
        ("Height", PdfObject::Integer(i64::from(image.height))),
        ("ColorSpace", PdfObject::Name(image.color_space.to_string())),
        (
            "BitsPerComponent",
            PdfObject::Integer(i64::from(image.bits_per_component)),
        ),
        (
            "Filter",
            PdfObject::Name(image.filter.pdf_name().to_string()),
        ),
    ]);
    if let Some(number) = smask_number {
        out.insert("SMask", reference(number, 0));
    }
    out
}

fn smask_dict(mask: &EditSoftMask) -> PdfDictionary {
    dict(&[
        ("Type", PdfObject::Name("XObject".to_string())),
        ("Subtype", PdfObject::Name("Image".to_string())),
        ("Width", PdfObject::Integer(i64::from(mask.width))),
        ("Height", PdfObject::Integer(i64::from(mask.height))),
        ("ColorSpace", PdfObject::Name("DeviceGray".to_string())),
        ("BitsPerComponent", PdfObject::Integer(8)),
        ("Filter", PdfObject::Name("FlateDecode".to_string())),
    ])
}

fn page_center(page: &PdfPage) -> (f64, f64) {
    (
        (page.media_box[0] + page.media_box[2]) / 2.0,
        (page.media_box[1] + page.media_box[3]) / 2.0,
    )
}

fn approximate_text_width(text: &str, font_size: f64) -> f64 {
    text.chars().count() as f64 * font_size * 0.5
}

fn encode_win_ansi_lossy(text: &str) -> Vec<u8> {
    text.chars()
        .map(|ch| {
            if ('\u{20}'..='\u{7e}').contains(&ch) {
                ch as u8
            } else {
                b'?'
            }
        })
        .collect()
}

fn write_stroke_color(out: &mut Vec<u8>, color: &Color) {
    write_color(out, color, false);
}

fn write_fill_color(out: &mut Vec<u8>, color: &Color) {
    write_color(out, color, true);
}

fn write_color(out: &mut Vec<u8>, color: &Color, fill: bool) {
    let op = match (&color.space, fill) {
        (ColorSpace::DeviceGray, false) => "G",
        (ColorSpace::DeviceGray, true) => "g",
        (ColorSpace::DeviceRGB, false) => "RG",
        (ColorSpace::DeviceRGB, true) => "rg",
        (ColorSpace::DeviceCMYK, false) => "K",
        (ColorSpace::DeviceCMYK, true) => "k",
        (ColorSpace::Named(_), false) => "RG",
        (ColorSpace::Named(_), true) => "rg",
    };
    let components = match color.space {
        ColorSpace::Named(_) => vec![0.0, 0.0, 0.0],
        _ => color.components.clone(),
    };
    for (idx, component) in components.iter().enumerate() {
        if idx > 0 {
            out.push(b' ');
        }
        out.extend_from_slice(fmt_num(component.clamp(0.0, 1.0)).as_bytes());
    }
    out.extend_from_slice(format!(" {op}\n").as_bytes());
}

fn rect_from_dict(dict: &PdfDictionary, reader: &PdfReader) -> Option<ImageRect> {
    let rect_obj = dict.get("Rect")?;
    let resolved = reader.resolve(rect_obj.clone()).ok()?;
    let arr = resolved.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let mut vals = [0.0; 4];
    for (idx, item) in arr.iter().enumerate() {
        vals[idx] = reader.resolve(item.clone()).ok()?.as_number()?;
    }
    Some(rect_from_corners(vals[0], vals[1], vals[2], vals[3]))
}

fn rect_array(rect: ImageRect) -> PdfObject {
    PdfObject::Array(vec![
        pdf_number(rect.x),
        pdf_number(rect.y),
        pdf_number(rect.x + rect.width),
        pdf_number(rect.y + rect.height),
    ])
}

fn color_array(color: &Color) -> PdfObject {
    PdfObject::Array(
        color
            .components
            .iter()
            .take(3)
            .map(|component| pdf_number(component.clamp(0.0, 1.0)))
            .collect(),
    )
}

fn highlight_quad_points(rect: ImageRect) -> PdfObject {
    PdfObject::Array(vec![
        pdf_number(rect.x),
        pdf_number(rect.y + rect.height),
        pdf_number(rect.x + rect.width),
        pdf_number(rect.y + rect.height),
        pdf_number(rect.x),
        pdf_number(rect.y),
        pdf_number(rect.x + rect.width),
        pdf_number(rect.y),
    ])
}

fn form_xobject_dict(width: f64, height: f64) -> PdfDictionary {
    dict(&[
        ("Type", PdfObject::Name("XObject".to_string())),
        ("Subtype", PdfObject::Name("Form".to_string())),
        (
            "BBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                pdf_number(width),
                pdf_number(height),
            ]),
        ),
    ])
}

fn pdf_text_string(text: &str) -> PdfObject {
    if text.is_ascii() {
        PdfObject::String(text.as_bytes().to_vec())
    } else {
        let mut bytes = vec![0xFE, 0xFF];
        for code in text.encode_utf16() {
            bytes.push((code >> 8) as u8);
            bytes.push((code & 0xff) as u8);
        }
        PdfObject::String(bytes)
    }
}

fn pdf_string_or_name(object: &PdfObject) -> Option<String> {
    match object {
        PdfObject::String(bytes) => Some(decode_pdf_text_string(bytes)),
        PdfObject::Name(name) => Some(name.clone()),
        _ => None,
    }
}

fn scrub_pdf_strings(object: &mut PdfObject, removed_text: &BTreeSet<String>) -> bool {
    match object {
        PdfObject::String(bytes) => {
            let text = decode_pdf_text_string(bytes);
            let scrubbed = removed_text
                .iter()
                .fold(text.clone(), |acc, secret| acc.replace(secret, ""));
            if scrubbed != text {
                *object = pdf_text_string(&scrubbed);
                true
            } else {
                false
            }
        }
        PdfObject::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= scrub_pdf_strings(item, removed_text);
            }
            changed
        }
        PdfObject::Dictionary(dict) => {
            let keys: Vec<String> = dict.entries().map(|(key, _)| key.clone()).collect();
            let mut changed = false;
            for key in keys {
                if let Some(value) = dict.get(&key).cloned() {
                    let mut value = value;
                    if scrub_pdf_strings(&mut value, removed_text) {
                        dict.insert(key, value);
                        changed = true;
                    }
                }
            }
            changed
        }
        PdfObject::Stream { dict, .. } => {
            let mut wrapper = PdfObject::Dictionary(dict.clone());
            let changed = scrub_pdf_strings(&mut wrapper, removed_text);
            if let PdfObject::Dictionary(scrubbed) = wrapper {
                *dict = scrubbed;
            }
            changed
        }
        _ => false,
    }
}

/// A stream whose raw payload (not just its dictionary) may carry a duplicate of
/// redacted text: the XMP `/Metadata` packet and embedded-file (`/EmbeddedFile`)
/// attachment streams.
fn is_scrubbable_payload_stream(dict: &PdfDictionary) -> bool {
    matches!(dict.get_name("Type"), Some(ty)
        if ty.eq_ignore_ascii_case("Metadata") || ty.eq_ignore_ascii_case("EmbeddedFile"))
}

/// Decode a textual/embedded stream, remove every occurrence of the redacted
/// text from its bytes, and re-store it uncompressed (so the scrub is visible to
/// any reader). Returns `None` if nothing changed.
fn scrub_stream_payload(
    stream: &PdfObject,
    reader: &PdfReader,
    removed_text: &BTreeSet<String>,
) -> Result<Option<PdfObject>> {
    let PdfObject::Stream { dict, .. } = stream else {
        return Ok(None);
    };
    let decoded = decode_stream_lossless(stream, reader)?;
    let Some(scrubbed) = scrub_bytes(&decoded.data, removed_text) else {
        return Ok(None);
    };
    let mut new_dict = dict.clone();
    // Stored decoded: drop the compression filter so the bytes are read verbatim;
    // the writer re-computes /Length.
    new_dict.remove("Filter");
    new_dict.remove("DecodeParms");
    new_dict.remove("DP");
    new_dict.remove("Length");
    Ok(Some(PdfObject::Stream {
        dict: new_dict,
        raw: scrubbed,
    }))
}

/// Remove every occurrence of each redacted string's bytes from `data`. Operates
/// at the byte level so binary embedded-file payloads are not corrupted by lossy
/// text conversion. Returns `None` if nothing matched.
fn scrub_bytes(data: &[u8], removed_text: &BTreeSet<String>) -> Option<Vec<u8>> {
    let mut current = data.to_vec();
    let mut changed = false;
    for secret in removed_text {
        let needle = secret.as_bytes();
        if needle.is_empty() {
            continue;
        }
        while let Some(pos) = current
            .windows(needle.len())
            .position(|window| window == needle)
        {
            current.drain(pos..pos + needle.len());
            changed = true;
        }
    }
    changed.then_some(current)
}

/// Re-parse a (already glyph-redacted) content stream and scrub redacted text
/// from inline marked-content alternate representations (`/ActualText`, `/Alt`)
/// carried in `BDC`/`DP` property lists, which the glyph rewriter passes through
/// verbatim. Dictionaries remain distinct from arrays, and both are traversed
/// recursively so alternate text survives only until scrubbed here.
fn scrub_marked_content_alt_text(
    content: &[u8],
    removed_text: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    if removed_text.is_empty() {
        return Ok(content.to_vec());
    }
    let operations = ContentParser::parse(content)?;
    let mut out = Vec::new();
    for mut op in operations {
        if matches!(op.operator.as_str(), "BDC" | "DP") {
            for operand in &mut op.operands {
                scrub_operand_strings(operand, removed_text);
            }
        }
        serialize_content_operation(&op, &mut out);
    }
    Ok(out)
}

fn scrub_operand_strings(operand: &mut Operand, removed_text: &BTreeSet<String>) {
    match operand {
        Operand::String(bytes) => {
            let text = decode_pdf_text_string(bytes);
            let scrubbed = removed_text
                .iter()
                .fold(text.clone(), |acc, secret| acc.replace(secret, ""));
            if scrubbed != text {
                if let PdfObject::String(new_bytes) = pdf_text_string(&scrubbed) {
                    *bytes = new_bytes;
                }
            }
        }
        Operand::Array(items) => {
            for item in items {
                scrub_operand_strings(item, removed_text);
            }
        }
        Operand::Dictionary(entries) => {
            for (_, value) in entries {
                scrub_operand_strings(value, removed_text);
            }
        }
        _ => {}
    }
}

fn rects_intersect(a: ImageRect, b: ImageRect) -> bool {
    let ax2 = a.x + a.width;
    let ay2 = a.y + a.height;
    let bx2 = b.x + b.width;
    let by2 = b.y + b.height;
    a.x < bx2 && ax2 > b.x && a.y < by2 && ay2 > b.y
}

fn rect_from_corners(x1: f64, y1: f64, x2: f64, y2: f64) -> ImageRect {
    ImageRect {
        x: x1.min(x2),
        y: y1.min(y2),
        width: (x1 - x2).abs(),
        height: (y1 - y2).abs(),
    }
}

fn rect_from_points(points: &[(f64, f64)]) -> ImageRect {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for (x, y) in points {
        min_x = min_x.min(*x);
        min_y = min_y.min(*y);
        max_x = max_x.max(*x);
        max_y = max_y.max(*y);
    }
    if !min_x.is_finite() {
        return ImageRect::new(0.0, 0.0, 0.0, 0.0);
    }
    ImageRect::new(min_x, min_y, max_x - min_x, max_y - min_y)
}

fn reference(number: u32, generation: u16) -> PdfObject {
    PdfObject::Reference { number, generation }
}

fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut out = PdfDictionary::empty();
    for (key, value) in entries {
        out.insert(*key, value.clone());
    }
    out
}

fn pdf_number(value: f64) -> PdfObject {
    if (value - value.round()).abs() < 0.000_000_1 {
        PdfObject::Integer(value.round() as i64)
    } else {
        PdfObject::Real((value * 10_000.0).round() / 10_000.0)
    }
}

fn fmt_num(value: f64) -> String {
    let value = if value.abs() < 0.000_000_1 {
        0.0
    } else {
        (value * 10_000.0).round() / 10_000.0
    };
    if (value - value.round()).abs() < 0.000_000_1 {
        return format!("{}", value.round() as i64);
    }
    let mut s = format!("{value:.4}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'A' + (value - 10)) as char,
    }
}

#[cfg(test)]
mod h2_alt_text_tests {
    use super::*;

    fn removed(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    /// Decoded text of every string operand carried by BDC/DP marked-content ops
    /// in a serialized content stream (operands are hex-encoded on the wire, so
    /// we re-parse and decode rather than scan the raw bytes).
    fn marked_content_strings(content: &[u8]) -> Vec<String> {
        fn collect(operand: &Operand, out: &mut Vec<String>) {
            match operand {
                Operand::String(bytes) => out.push(decode_pdf_text_string(bytes)),
                Operand::Array(items) => items.iter().for_each(|it| collect(it, out)),
                Operand::Dictionary(entries) => {
                    entries.iter().for_each(|(_, value)| collect(value, out))
                }
                _ => {}
            }
        }
        ContentParser::parse(content)
            .unwrap()
            .into_iter()
            .filter(|op| matches!(op.operator.as_str(), "BDC" | "DP"))
            .flat_map(|op| {
                let mut out = Vec::new();
                op.operands.iter().for_each(|o| collect(o, &mut out));
                out
            })
            .collect()
    }

    #[test]
    fn scrubs_inline_actualtext_in_marked_content() {
        // A tagged-PDF span carrying the redacted text as /ActualText (and /Alt)
        // must have it stripped, even though the glyph rewriter passes BDC
        // through verbatim.
        let content =
            b"/Span <</ActualText (SECRET) /Alt (SECRET phrase)>> BDC (x) Tj EMC".to_vec();
        let out = scrub_marked_content_alt_text(&content, &removed(&["SECRET"])).unwrap();
        let alt = marked_content_strings(&out).join("|");
        assert!(
            !alt.contains("SECRET"),
            "inline /ActualText leaked the redacted text: {alt:?}"
        );
        // The marked-content operators themselves are preserved.
        let ops: Vec<String> = ContentParser::parse(&out)
            .unwrap()
            .into_iter()
            .map(|op| op.operator)
            .collect();
        assert!(ops.contains(&"BDC".to_string()) && ops.contains(&"EMC".to_string()));
    }

    #[test]
    fn marked_content_scrub_preserves_non_secret_alt_text() {
        let content = b"/Span <</ActualText (Public)>> BDC (x) Tj EMC".to_vec();
        let out = scrub_marked_content_alt_text(&content, &removed(&["SECRET"])).unwrap();
        let alt = marked_content_strings(&out).join("|");
        assert!(
            alt.contains("Public"),
            "non-secret /ActualText was lost: {alt:?}"
        );
    }

    #[test]
    fn scrub_bytes_removes_secret_from_xmp_like_payload() {
        // XMP packet duplicating a redacted name in dc:creator.
        let xmp = b"<x:xmpmeta><dc:creator>Jane SECRET Doe</dc:creator></x:xmpmeta>";
        let scrubbed = scrub_bytes(xmp, &removed(&["SECRET"])).expect("payload changed");
        assert!(!scrubbed.windows(6).any(|w| w == b"SECRET"));
        // A payload with no secret is left untouched (None).
        assert!(scrub_bytes(b"<x:xmpmeta>clean</x:xmpmeta>", &removed(&["SECRET"])).is_none());
    }

    #[test]
    fn payload_stream_types_are_recognized() {
        let mut meta = PdfDictionary::empty();
        meta.insert("Type", PdfObject::Name("Metadata".to_string()));
        assert!(is_scrubbable_payload_stream(&meta));

        let mut ef = PdfDictionary::empty();
        ef.insert("Type", PdfObject::Name("EmbeddedFile".to_string()));
        assert!(is_scrubbable_payload_stream(&ef));

        let mut page = PdfDictionary::empty();
        page.insert("Type", PdfObject::Name("Page".to_string()));
        assert!(!is_scrubbable_payload_stream(&page));
    }

    #[test]
    fn prompt18b_packed_rows_round_trip_all_supported_depths() {
        for bpc in [1, 2, 4, 8] {
            let max = ((1u16 << bpc) - 1) as u8;
            let samples = vec![0, max, max / 2, max, 0, max / 3];
            let packed = pack_samples_exact(&samples, 3, 1, 2, bpc).unwrap();
            assert_eq!(
                unpack_samples_exact(&packed, 3, 1, 2, bpc).unwrap(),
                samples
            );
        }
    }

    #[test]
    fn prompt18b_tiff_and_png_predictors_reencode_deterministically() {
        let mut tiff = PdfDictionary::empty();
        tiff.insert("Predictor", PdfObject::Integer(2));
        let first = output_predictor(&[Some(tiff)], 2, 1, 8, &[10, 20]).unwrap();
        assert_eq!(first.1, vec![10, 10]);

        let mut png = PdfDictionary::empty();
        png.insert("Predictor", PdfObject::Integer(15));
        let second = output_predictor(&[Some(png)], 2, 1, 8, &[10, 20]).unwrap();
        assert_eq!(second.1, vec![0, 10, 20]);
    }
}
