use jpeg_decoder::{Decoder as JpegDecoder, PixelFormat};
use lopdf::content::{Content, Operation};
use lopdf::{
    Dictionary, Document, LoadOptions, Object, ObjectId, StringFormat, decode_text_string,
    text_string,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{BufWriter, Cursor, Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub mod native_merge;
pub mod native_metadata;
pub mod native_organize;
pub mod native_protect;
pub mod native_rotation;
pub mod full_font;
pub mod native_unlock;
pub mod protocol;
pub mod styled_text;
mod text_show;

pub const MAX_INPUT_BYTES: u64 = 320 * 1024 * 1024;
pub const MAX_DECOMPRESSED_STREAM_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PAGE_COUNT: usize = 1_000;
pub const MAX_CONTENT_OPERATIONS_PER_STREAM: usize = 200_000;
pub const MAX_EDITABLE_TEXT_RUNS: usize = 10_000;
pub const MAX_INSPECTION_TEXT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_EDITABLE_TEXT_RUN_BYTES: usize = 4 * 1024;
pub const MAX_EDITABLE_TEXT_RUN_CHARS: usize = 1_024;
pub const MAX_REFLOW_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_REFLOW_TEXT_CHARS: usize = 16 * 1024;
pub const MAX_REFLOW_LINES: usize = 256;
const REFLOW_COORDINATE_TOLERANCE: f64 = 0.01;
pub const MAX_REPLACEMENT_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
pub const MAX_REPLACEMENT_IMAGE_DECODED_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_EDITABLE_IMAGES: usize = 10_000;
pub const MAX_EDITABLE_IMAGE_PLACEMENTS: usize = 10_000;
pub const MAX_EDITABLE_VECTOR_PATHS: usize = 10_000;
pub const MAX_EDITABLE_LINKS: usize = 10_000;
pub const MAX_EXTERNAL_LINK_BYTES: usize = 2_048;
pub const MAX_EDITABLE_ANNOTATIONS: usize = 10_000;
pub const MAX_ANNOTATION_CONTENT_BYTES: usize = 8 * 1_024;
pub const MAX_ANNOTATION_QUADS: usize = 64;
pub const MAX_ANNOTATION_INK_PATHS: usize = 64;
pub const MAX_ANNOTATION_INK_POINTS: usize = 4_096;
pub const MAX_EDITABLE_BOOKMARKS: usize = 10_000;
pub const MAX_BOOKMARK_TITLE_BYTES: usize = 2_048;
pub const MAX_BOOKMARK_TITLE_CHARS: usize = 512;
pub const MAX_METADATA_TEXT_BYTES: usize = 8 * 1_024;
pub const MAX_METADATA_TEXT_CHARS: usize = 2_048;
pub const MAX_METADATA_KEYWORDS: usize = 128;
pub const MAX_METADATA_KEYWORD_BYTES: usize = 256;
pub const MAX_EDITABLE_FORM_FIELDS: usize = 10_000;
pub const MAX_FORM_FIELD_NAME_BYTES: usize = 1_024;
pub const MAX_FORM_FIELD_LABEL_BYTES: usize = 2_048;
pub const MAX_FORM_FIELD_LABEL_CHARS: usize = 512;
pub const MAX_FORM_FIELD_VALUE_BYTES: usize = 4 * 1_024;
pub const MAX_FORM_FIELD_OPTIONS: usize = 1_024;
pub const MAX_FORM_FIELD_CREATION_WIDGETS: usize = 64;
pub const MAX_FORM_FIELD_OPTION_BYTES: usize = 256;
pub const MAX_PAGE_DIMENSION_POINTS: f64 = 20_000.0;
pub const MAX_PAGE_COORDINATE_POINTS: f64 = 100_000.0;
pub const MAX_PAGE_CLONE_OBJECTS: usize = 50_000;
pub const MAX_PDF_OBJECT_NESTING: usize = 128;

#[derive(Debug, Error)]
pub enum PdfEditError {
    #[error("The input PDF is missing or cannot be read.")]
    InputUnavailable,
    #[error("The input exceeds the native PDF editor's 320 MiB safety ceiling.")]
    InputTooLarge,
    #[error("The input is not a supported, structurally valid PDF.")]
    InvalidPdf,
    #[error(
        "Encrypted PDFs must be unlocked through the canonical unlock workflow before editing."
    )]
    EncryptedPdf,
    #[error("The PDF exceeds the native editor's 1,000-page safety ceiling.")]
    PageLimitExceeded,
    #[error("The requested page does not exist.")]
    PageNotFound,
    #[error(
        "The requested page order must contain every page exactly once and must change the current order."
    )]
    InvalidPageOrder,
    #[error("The PDF page tree cannot be rewritten safely.")]
    PageTreeUnsupported,
    #[error("The selected page no longer matches the inspected page tree.")]
    StalePageMutation,
    #[error(
        "The requested page insertion, removal, duplication, or import is invalid or unchanged."
    )]
    InvalidPageMutation,
    #[error(
        "The page has destinations, forms, annotations, labels, or structural references that cannot yet be rewritten safely."
    )]
    PageDependencyUnsupported,
    #[error("The selected page geometry no longer matches the inspected page.")]
    StalePageGeometry,
    #[error("The requested page rotation or page boxes are invalid, unsafe, or unchanged.")]
    InvalidPageGeometry,
    #[error("The selected content stream is not owned by the requested page.")]
    ContentStreamNotOnPage,
    #[error("The selected content stream is unsupported or exceeds its decompression limit.")]
    ContentStreamUnsupported,
    #[error("The selected content stream contains too many operations.")]
    ContentOperationLimitExceeded,
    #[error("The selected operation is not an exact single-string Tj text run.")]
    TextRunUnsupported,
    #[error("The selected text run no longer matches the expected text or font.")]
    StaleTextRun,
    #[error(
        "The replacement contains glyphs that the existing font encoding cannot represent exactly."
    )]
    GlyphUnavailable,
    #[error("This text cannot be safely reflowed without changing nearby layout.")]
    ReflowUnsafe,
    #[error("The selected image XObject does not exist on an editable page resource.")]
    ImageNotFound,
    #[error(
        "The selected image XObject is masked, a stencil, or otherwise unsupported for safe replacement."
    )]
    ImageUnsupported,
    #[error(
        "The selected image XObject no longer matches its expected object, dimensions, or stream hash."
    )]
    StaleImage,
    #[error(
        "The replacement image is missing, exceeds 25 MiB, or is not a fully decodable JPEG in a supported colour model."
    )]
    ReplacementImageInvalid,
    #[error("The selected image placement does not exist on the requested PDF page.")]
    ImagePlacementNotFound,
    #[error(
        "The selected image placement is not an isolated q/cm/Do/Q operation and cannot be moved safely."
    )]
    ImagePlacementUnsupported,
    #[error("The selected image placement no longer matches the inspected content stream.")]
    StaleImagePlacement,
    #[error("The requested image position is invalid, unsafe, or unchanged.")]
    InvalidImagePlacement,
    #[error("The requested image crop is invalid, unsafe, or unchanged.")]
    InvalidImageCrop,
    #[error("The requested image opacity is invalid, unsafe, or unchanged.")]
    InvalidImageOpacity,
    #[error("The selected vector path does not exist on the requested PDF page.")]
    VectorPathNotFound,
    #[error("The selected vector path is not an isolated, safely recognized path object.")]
    VectorPathUnsupported,
    #[error("The selected vector path no longer matches the inspected content stream.")]
    StaleVectorPath,
    #[error("The requested vector path edit is invalid, unsafe, or unchanged.")]
    InvalidVectorEdit,
    #[error("The requested link annotation does not exist on the selected page.")]
    LinkNotFound,
    #[error("The selected link annotation or page annotation collection is unsupported.")]
    LinkUnsupported,
    #[error("The selected link annotation or page annotation collection has changed.")]
    StaleLink,
    #[error("The requested link edit is invalid, unsafe, or unchanged.")]
    InvalidLinkEdit,
    #[error("The external link URL uses a forbidden or invalid protocol.")]
    InvalidLinkProtocol,
    #[error("The requested native PDF annotation does not exist on the selected page.")]
    AnnotationNotFound,
    #[error("The selected native PDF annotation or page annotation collection is unsupported.")]
    AnnotationUnsupported,
    #[error("The selected native PDF annotation or page annotation collection has changed.")]
    StaleAnnotation,
    #[error("The requested native PDF annotation edit is invalid, unsafe, or unchanged.")]
    InvalidAnnotationEdit,
    #[error("The PDF bookmark outline tree is missing, malformed, cyclic, or unsupported.")]
    BookmarkTreeUnsupported,
    #[error("The requested PDF bookmark does not exist.")]
    BookmarkNotFound,
    #[error("The selected PDF bookmark or outline tree has changed.")]
    StaleBookmark,
    #[error("The requested PDF bookmark edit is invalid, unsafe, or unchanged.")]
    InvalidBookmarkEdit,
    #[error("The PDF document metadata dictionary or XMP stream is malformed or unsupported.")]
    MetadataUnsupported,
    #[error("The selected PDF document metadata has changed since it was inspected.")]
    StaleMetadata,
    #[error("The requested PDF document metadata edit is invalid, unsafe, or unchanged.")]
    InvalidMetadataEdit,
    #[error(
        "Foreign XMP metadata is preserved read-only; only absent or Wellfriend-owned XMP can be synchronised."
    )]
    ForeignXmpReadOnly,
    #[error("The PDF AcroForm field tree is missing, malformed, cyclic, or unsupported.")]
    FormTreeUnsupported,
    #[error("The requested native PDF form field does not exist.")]
    FormFieldNotFound,
    #[error("The selected native PDF form field or AcroForm state has changed.")]
    StaleFormField,
    #[error("The selected native PDF form field type or appearance cannot be edited safely.")]
    FormFieldUnsupported,
    #[error("The requested native PDF form value is invalid, unsafe, or unchanged.")]
    InvalidFormFieldValue,
    #[error(
        "The requested native PDF form field geometry is invalid, outside its page, or unchanged."
    )]
    InvalidFormFieldGeometry,
    #[error("The requested native PDF form field label is invalid, unsafe, or unchanged.")]
    InvalidFormFieldLabel,
    #[error("The requested native PDF dropdown options are invalid, unsafe, or unchanged.")]
    InvalidFormFieldOptions,
    #[error(
        "The selected native PDF form field appearance is already valid or cannot be repaired safely."
    )]
    FormFieldAppearanceNotRepairable,
    #[error(
        "The requested native PDF form field creation is invalid, unsafe, or conflicts with an existing field."
    )]
    InvalidFormFieldCreation,
    #[error("The requested native PDF form field deletion is invalid or unsafe.")]
    InvalidFormFieldDeletion,
    #[error(
        "This edit would invalidate an existing PDF signature and requires explicit confirmation."
    )]
    SignatureConfirmationRequired,
    #[error("Input and output paths must be different.")]
    OutputAliasesInput,
    #[error("The output path already exists.")]
    OutputAlreadyExists,
    #[error("The edited PDF failed structural or text validation.")]
    ValidationFailed,
    #[error("The PDF edit output could not be written.")]
    OutputWriteFailed,
}

impl PdfEditError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InputUnavailable => "PDF_INPUT_UNAVAILABLE",
            Self::InputTooLarge => "PDF_INPUT_TOO_LARGE",
            Self::InvalidPdf => "PDF_INVALID",
            Self::EncryptedPdf => "PDF_ENCRYPTED",
            Self::PageLimitExceeded => "PDF_PAGE_LIMIT_EXCEEDED",
            Self::PageNotFound => "PDF_PAGE_NOT_FOUND",
            Self::InvalidPageOrder => "PDF_PAGE_ORDER_INVALID",
            Self::PageTreeUnsupported => "PDF_PAGE_TREE_UNSUPPORTED",
            Self::StalePageMutation => "PDF_PAGE_MUTATION_STALE",
            Self::InvalidPageMutation => "PDF_PAGE_MUTATION_INVALID",
            Self::PageDependencyUnsupported => "PDF_PAGE_DEPENDENCY_UNSUPPORTED",
            Self::StalePageGeometry => "PDF_PAGE_GEOMETRY_STALE",
            Self::InvalidPageGeometry => "PDF_PAGE_GEOMETRY_INVALID",
            Self::ContentStreamNotOnPage => "PDF_CONTENT_STREAM_NOT_ON_PAGE",
            Self::ContentStreamUnsupported => "PDF_CONTENT_STREAM_UNSUPPORTED",
            Self::ContentOperationLimitExceeded => "PDF_CONTENT_OPERATION_LIMIT_EXCEEDED",
            Self::TextRunUnsupported => "PDF_TEXT_RUN_UNSUPPORTED",
            Self::StaleTextRun => "PDF_TEXT_RUN_STALE",
            Self::GlyphUnavailable => "PDF_FONT_GLYPH_UNAVAILABLE",
            Self::ReflowUnsafe => "REFLOW_UNSAFE",
            Self::ImageNotFound => "PDF_IMAGE_NOT_FOUND",
            Self::ImageUnsupported => "PDF_IMAGE_UNSUPPORTED",
            Self::StaleImage => "PDF_IMAGE_STALE",
            Self::ReplacementImageInvalid => "PDF_REPLACEMENT_IMAGE_INVALID",
            Self::ImagePlacementNotFound => "PDF_IMAGE_PLACEMENT_NOT_FOUND",
            Self::ImagePlacementUnsupported => "PDF_IMAGE_PLACEMENT_UNSUPPORTED",
            Self::StaleImagePlacement => "PDF_IMAGE_PLACEMENT_STALE",
            Self::InvalidImagePlacement => "PDF_IMAGE_PLACEMENT_INVALID",
            Self::InvalidImageCrop => "PDF_IMAGE_CROP_INVALID",
            Self::InvalidImageOpacity => "PDF_IMAGE_OPACITY_INVALID",
            Self::VectorPathNotFound => "PDF_VECTOR_PATH_NOT_FOUND",
            Self::VectorPathUnsupported => "PDF_VECTOR_PATH_UNSUPPORTED",
            Self::StaleVectorPath => "PDF_VECTOR_PATH_STALE",
            Self::InvalidVectorEdit => "PDF_VECTOR_EDIT_INVALID",
            Self::LinkNotFound => "PDF_LINK_NOT_FOUND",
            Self::LinkUnsupported => "PDF_LINK_UNSUPPORTED",
            Self::StaleLink => "PDF_LINK_STALE",
            Self::InvalidLinkEdit => "PDF_LINK_EDIT_INVALID",
            Self::InvalidLinkProtocol => "PDF_LINK_PROTOCOL_INVALID",
            Self::AnnotationNotFound => "PDF_ANNOTATION_NOT_FOUND",
            Self::AnnotationUnsupported => "PDF_ANNOTATION_UNSUPPORTED",
            Self::StaleAnnotation => "PDF_ANNOTATION_STALE",
            Self::InvalidAnnotationEdit => "PDF_ANNOTATION_EDIT_INVALID",
            Self::BookmarkTreeUnsupported => "PDF_BOOKMARK_TREE_UNSUPPORTED",
            Self::BookmarkNotFound => "PDF_BOOKMARK_NOT_FOUND",
            Self::StaleBookmark => "PDF_BOOKMARK_STALE",
            Self::InvalidBookmarkEdit => "PDF_BOOKMARK_EDIT_INVALID",
            Self::MetadataUnsupported => "PDF_METADATA_UNSUPPORTED",
            Self::StaleMetadata => "PDF_METADATA_STALE",
            Self::InvalidMetadataEdit => "PDF_METADATA_EDIT_INVALID",
            Self::ForeignXmpReadOnly => "PDF_METADATA_XMP_FOREIGN_READ_ONLY",
            Self::FormTreeUnsupported => "PDF_FORM_TREE_UNSUPPORTED",
            Self::FormFieldNotFound => "PDF_FORM_FIELD_NOT_FOUND",
            Self::StaleFormField => "PDF_FORM_FIELD_STALE",
            Self::FormFieldUnsupported => "PDF_FORM_FIELD_UNSUPPORTED",
            Self::InvalidFormFieldValue => "PDF_FORM_FIELD_VALUE_INVALID",
            Self::InvalidFormFieldGeometry => "PDF_FORM_FIELD_GEOMETRY_INVALID",
            Self::InvalidFormFieldLabel => "PDF_FORM_FIELD_LABEL_INVALID",
            Self::InvalidFormFieldOptions => "PDF_FORM_FIELD_OPTIONS_INVALID",
            Self::FormFieldAppearanceNotRepairable => "PDF_FORM_FIELD_APPEARANCE_NOT_REPAIRABLE",
            Self::InvalidFormFieldCreation => "PDF_FORM_FIELD_CREATION_INVALID",
            Self::InvalidFormFieldDeletion => "PDF_FORM_FIELD_DELETION_INVALID",
            Self::SignatureConfirmationRequired => "PDF_SIGNATURE_CONFIRMATION_REQUIRED",
            Self::OutputAliasesInput => "PDF_OUTPUT_ALIASES_INPUT",
            Self::OutputAlreadyExists => "PDF_OUTPUT_ALREADY_EXISTS",
            Self::ValidationFailed => "PDF_EDIT_VALIDATION_FAILED",
            Self::OutputWriteFailed => "PDF_OUTPUT_WRITE_FAILED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SignatureEffect {
    None,
    InvalidatesExistingSignatures,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SaveMode {
    FullRewrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfObjectId {
    pub number: u32,
    pub generation: u16,
}

impl From<ObjectId> for PdfObjectId {
    fn from(value: ObjectId) -> Self {
        Self {
            number: value.0,
            generation: value.1,
        }
    }
}

impl From<PdfObjectId> for ObjectId {
    fn from(value: PdfObjectId) -> Self {
        (value.number, value.generation)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextRunSelector {
    pub page_number: u32,
    pub content_stream_object: PdfObjectId,
    pub operation_index: usize,
    pub expected_font_resource: String,
    pub expected_text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMatrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableTextRun {
    pub style_eligible: bool,
    pub selector: TextRunSelector,
    pub operator: String,
    pub font_size: Option<f64>,
    pub text_matrix: Option<TextMatrix>,
    pub bounding_box: Option<PdfBox>,
    pub font_metrics_source: Option<String>,
    pub reflow_eligible: bool,
    pub encoded_byte_length: usize,
    pub exact_round_trip: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfTextAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageSelector {
    pub image_object: PdfObjectId,
    pub expected_stream_sha256: String,
    pub expected_width: u32,
    pub expected_height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageUsage {
    pub page_number: u32,
    pub resource_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMatrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfNormalizedCrop {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfImageOpacity {
    pub value: f64,
    pub resource_name: String,
    pub ext_g_state_object: PdfObjectId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePlacementSelector {
    pub page_number: u32,
    pub content_stream_object: PdfObjectId,
    pub matrix_operation_index: usize,
    pub draw_operation_index: usize,
    pub expected_resource_name: String,
    pub expected_image_object: PdfObjectId,
    pub expected_matrix: PdfMatrix,
    #[serde(default)]
    pub expected_crop: Option<PdfNormalizedCrop>,
    #[serde(default)]
    pub expected_opacity: Option<PdfImageOpacity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableImagePlacement {
    pub selector: ImagePlacementSelector,
    pub matrix: PdfMatrix,
    pub crop: Option<PdfNormalizedCrop>,
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableImage {
    pub selector: ImageSelector,
    pub width: u32,
    pub height: u32,
    pub color_space: Option<String>,
    pub bits_per_component: Option<u32>,
    pub filters: Vec<String>,
    pub usages: Vec<ImageUsage>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfRgbColor {
    pub red: f64,
    pub green: f64,
    pub blue: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorPathSelector {
    pub page_number: u32,
    pub content_stream_object: PdfObjectId,
    pub start_operation_index: usize,
    pub end_operation_index: usize,
    pub expected_operations_sha256: String,
    pub expected_transform: Option<PdfMatrix>,
    pub expected_fill_color: Option<PdfRgbColor>,
    pub expected_stroke_color: Option<PdfRgbColor>,
    pub expected_paint_operator: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableVectorPath {
    pub selector: VectorPathSelector,
    pub transform: PdfMatrix,
    pub bounds: PdfBox,
    pub fill_color: Option<PdfRgbColor>,
    pub stroke_color: Option<PdfRgbColor>,
    pub paint_operator: String,
    pub supports_fill: bool,
    pub supports_stroke: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfLinkTarget {
    ExternalUrl {
        url: String,
    },
    InternalPage {
        #[serde(rename = "pageNumber")]
        page_number: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkSelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub annotation_object: PdfObjectId,
    pub annotation_index: usize,
    pub expected_annotation_sha256: String,
    pub expected_rectangle: PdfBox,
    pub expected_target: PdfLinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkPageSelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub annotations_object: Option<PdfObjectId>,
    pub expected_annotations_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableLink {
    pub selector: LinkSelector,
    pub rectangle: PdfBox,
    pub target: PdfLinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableLinkPage {
    pub selector: LinkPageSelector,
    pub annotation_count: usize,
    pub link_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfPoint {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfQuadPoints {
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub x3: f64,
    pub y3: f64,
    pub x4: f64,
    pub y4: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfAnnotationKind {
    Text,
    Highlight,
    Underline,
    Strikeout,
    Ink,
    Rectangle,
    Arrow,
    Stamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfAnnotationSpec {
    pub kind: PdfAnnotationKind,
    pub rectangle: PdfBox,
    pub contents: String,
    pub color: PdfRgbColor,
    pub opacity: f64,
    #[serde(default)]
    pub quad_points: Vec<PdfQuadPoints>,
    #[serde(default)]
    pub ink_paths: Vec<Vec<PdfPoint>>,
    pub line_start: Option<PdfPoint>,
    pub line_end: Option<PdfPoint>,
    pub stamp_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationSelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub annotation_object: PdfObjectId,
    pub appearance_object: PdfObjectId,
    pub annotation_index: usize,
    pub expected_annotation_sha256: String,
    pub expected_appearance_sha256: String,
    pub expected_spec: PdfAnnotationSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationPageSelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub annotations_object: Option<PdfObjectId>,
    pub expected_annotations_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableAnnotation {
    pub selector: AnnotationSelector,
    pub spec: PdfAnnotationSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableAnnotationPage {
    pub selector: AnnotationPageSelector,
    pub annotation_count: usize,
    pub editable_annotation_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfMetadataLocation {
    Absent,
    TrailerDictionary,
    IndirectObject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfXmpState {
    Absent,
    WellfriendOwned,
    ForeignReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfXmpMode {
    Preserve,
    SyncBasicFields,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMetadataSpec {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfMetadataSelector {
    pub catalog_object: PdfObjectId,
    pub info_location: PdfMetadataLocation,
    pub info_object: Option<PdfObjectId>,
    pub expected_info_sha256: Option<String>,
    pub xmp_state: PdfXmpState,
    pub xmp_object: Option<PdfObjectId>,
    pub expected_xmp_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableMetadata {
    pub selector: PdfMetadataSelector,
    pub values: PdfMetadataSpec,
    pub can_sync_xmp: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkTreeSelector {
    pub catalog_object: PdfObjectId,
    pub outlines_object: Option<PdfObjectId>,
    pub expected_tree_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkSelector {
    pub tree: BookmarkTreeSelector,
    pub outline_object: PdfObjectId,
    pub parent_object: PdfObjectId,
    pub sibling_index: usize,
    pub expected_item_sha256: String,
    pub expected_title: String,
    pub expected_target: PdfLinkTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableBookmark {
    pub selector: BookmarkSelector,
    pub title: String,
    pub target: PdfLinkTarget,
    pub depth: usize,
    pub sibling_index: usize,
    pub child_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableBookmarkTree {
    pub selector: BookmarkTreeSelector,
    pub bookmark_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PdfFormFieldKind {
    Text,
    Checkbox,
    Radio,
    Dropdown,
    Signature,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcroFormSelector {
    pub catalog_object: PdfObjectId,
    pub acro_form_object: PdfObjectId,
    pub expected_form_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormCreationSelector {
    pub catalog_object: PdfObjectId,
    pub acro_form_object: Option<PdfObjectId>,
    pub expected_form_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldSelector {
    pub form: AcroFormSelector,
    pub field_object: PdfObjectId,
    pub expected_field_sha256: String,
    pub expected_name: String,
    pub expected_kind: PdfFormFieldKind,
    pub expected_value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormWidgetSelector {
    pub widget_object: PdfObjectId,
    pub expected_page_number: u32,
    pub expected_page_object: PdfObjectId,
    pub expected_rectangle: PdfBox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableFormWidget {
    pub selector: FormWidgetSelector,
    pub rectangle: PdfBox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditableFormField {
    pub selector: FormFieldSelector,
    pub name: String,
    pub label: Option<String>,
    pub kind: PdfFormFieldKind,
    pub value: Option<String>,
    pub options: Vec<String>,
    pub options_editable: bool,
    pub appearance_valid: bool,
    pub appearance_repairable: bool,
    pub required: bool,
    pub read_only: bool,
    pub value_editable: bool,
    pub page_number: u32,
    pub rectangle: PdfBox,
    pub widget_objects: Vec<PdfObjectId>,
    pub widgets: Vec<EditableFormWidget>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFormFieldWidget {
    pub page: PageIdentitySelector,
    pub rectangle: PdfBox,
    pub export_value: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfBox {
    pub left: f64,
    pub bottom: f64,
    pub right: f64,
    pub top: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGeometrySelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub expected_media_box: PdfBox,
    pub expected_crop_box: PdfBox,
    pub expected_rotation: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageIdentitySelector {
    pub page_number: u32,
    pub page_object: PdfObjectId,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditablePageGeometry {
    pub selector: PageGeometrySelector,
    pub media_box: PdfBox,
    pub crop_box: PdfBox,
    pub rotation: i32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfObjectGraphSummary {
    pub catalogs: usize,
    pub pages: usize,
    pub content_streams: usize,
    pub font_resources: usize,
    pub xobjects: usize,
    pub images: usize,
    pub annotations: usize,
    pub links: usize,
    pub acro_form_fields: usize,
    pub outlines: usize,
    pub names_trees: usize,
    pub metadata_streams: usize,
    pub signature_objects: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSummary {
    pub object: PdfObjectId,
    pub has_byte_range: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PdfInspection {
    pub format_version: String,
    pub input_sha256: String,
    pub input_bytes: u64,
    pub page_count: usize,
    pub object_count: usize,
    pub encrypted: bool,
    pub object_graph: PdfObjectGraphSummary,
    pub signatures: Vec<SignatureSummary>,
    pub editable_text_runs: Vec<EditableTextRun>,
    pub editable_text_runs_truncated: bool,
    pub unsupported_text_show_operations: usize,
    pub editable_images: Vec<EditableImage>,
    pub editable_images_truncated: bool,
    pub unsupported_images: usize,
    pub editable_image_placements: Vec<EditableImagePlacement>,
    pub editable_image_placements_truncated: bool,
    pub unsupported_image_placements: usize,
    pub editable_vector_paths: Vec<EditableVectorPath>,
    pub editable_vector_paths_truncated: bool,
    pub unsupported_vector_path_groups: usize,
    pub editable_links: Vec<EditableLink>,
    pub editable_links_truncated: bool,
    pub editable_link_pages: Vec<EditableLinkPage>,
    pub unsupported_links: usize,
    pub unsupported_link_pages: usize,
    pub editable_annotations: Vec<EditableAnnotation>,
    pub editable_annotations_truncated: bool,
    pub editable_annotation_pages: Vec<EditableAnnotationPage>,
    pub unsupported_annotations: usize,
    pub unsupported_annotation_pages: usize,
    pub editable_metadata: Option<EditableMetadata>,
    pub unsupported_metadata: usize,
    pub editable_bookmark_tree: Option<EditableBookmarkTree>,
    pub editable_bookmarks: Vec<EditableBookmark>,
    pub editable_bookmarks_truncated: bool,
    pub unsupported_bookmarks: usize,
    pub editable_form_fields: Vec<EditableFormField>,
    pub editable_form_fields_truncated: bool,
    pub unsupported_form_fields: usize,
    pub form_creation: Option<FormCreationSelector>,
    pub editable_pages: Vec<EditablePageGeometry>,
    pub unsupported_page_geometries: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceTextRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: TextRunSelector,
    pub replacement_text: String,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflowTextRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: TextRunSelector,
    pub replacement_text: String,
    pub region: PdfBox,
    pub alignment: PdfTextAlignment,
    pub line_spacing: f64,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPagesRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    /// One-based source page numbers in their requested output order.
    pub page_order: Vec<u32>,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceImageRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub replacement_image_path: PathBuf,
    pub selector: ImageSelector,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveImageRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: ImagePlacementSelector,
    pub destination_x: f64,
    pub destination_y: f64,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformImageRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: ImagePlacementSelector,
    pub destination_matrix: PdfMatrix,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropImageRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: ImagePlacementSelector,
    pub destination_crop: PdfNormalizedCrop,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetImageOpacityRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: ImagePlacementSelector,
    pub destination_opacity: f64,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VectorPathMutation {
    Transform {
        destination_matrix: PdfMatrix,
    },
    Recolor {
        destination_fill_color: Option<PdfRgbColor>,
        destination_stroke_color: Option<PdfRgbColor>,
    },
    Delete,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditVectorPathRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: VectorPathSelector,
    pub mutation: VectorPathMutation,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LinkMutation {
    Create {
        page: LinkPageSelector,
        rectangle: PdfBox,
        target: PdfLinkTarget,
    },
    Update {
        selector: LinkSelector,
        rectangle: PdfBox,
        target: PdfLinkTarget,
    },
    Delete {
        selector: LinkSelector,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditLinkRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub mutation: LinkMutation,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationMutation {
    Create {
        page: AnnotationPageSelector,
        spec: PdfAnnotationSpec,
    },
    Update {
        selector: Box<AnnotationSelector>,
        spec: PdfAnnotationSpec,
    },
    Delete {
        selector: Box<AnnotationSelector>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditAnnotationRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub mutation: AnnotationMutation,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditMetadataRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: PdfMetadataSelector,
    pub values: PdfMetadataSpec,
    pub xmp_mode: PdfXmpMode,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "SCREAMING_SNAKE_CASE",
    rename_all_fields = "camelCase"
)]
pub enum BookmarkMutation {
    Create {
        tree: BookmarkTreeSelector,
        parent: Option<BookmarkSelector>,
        sibling_index: usize,
        title: String,
        target: PdfLinkTarget,
    },
    Update {
        selector: BookmarkSelector,
        title: String,
        target: PdfLinkTarget,
    },
    Move {
        selector: BookmarkSelector,
        destination_parent: Option<BookmarkSelector>,
        destination_sibling_index: usize,
    },
    Delete {
        selector: BookmarkSelector,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditBookmarkRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub mutation: BookmarkMutation,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldValueRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    pub value: Option<String>,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldGeometryRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    pub widget: FormWidgetSelector,
    pub rectangle: PdfBox,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldLabelRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    pub label: Option<String>,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldOptionsRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    pub options: Vec<String>,
    pub value: String,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFormFieldAppearanceRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFormFieldRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormCreationSelector,
    pub name: String,
    pub label: Option<String>,
    pub kind: PdfFormFieldKind,
    pub value: Option<String>,
    pub options: Vec<String>,
    pub widgets: Vec<CreateFormFieldWidget>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFormFieldRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: FormFieldSelector,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPageGeometryRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub selector: PageGeometrySelector,
    pub media_box: PdfBox,
    pub crop_box: PdfBox,
    pub rotation: i32,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PageMutation {
    Delete {
        selector: PageIdentitySelector,
    },
    Duplicate {
        selector: PageIdentitySelector,
        insert_before_page_number: u32,
    },
    InsertBlank {
        insert_before_page_number: u32,
        media_box: PdfBox,
    },
    Import {
        import_path: PathBuf,
        source_pages: Vec<PageIdentitySelector>,
        insert_before_page_number: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutatePagesRequest {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub mutation: PageMutation,
    #[serde(default)]
    pub allow_signature_invalidation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_operator_preserved: bool,
    pub replacement_extractable: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl EditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_operator_preserved
            && self.replacement_extractable
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }

    fn failed_checks(&self) -> Vec<&'static str> {
        let checks = [
            ("reparsed", self.reparsed),
            ("page_count_preserved", self.page_count_preserved),
            ("object_count_preserved", self.object_count_preserved),
            (
                "non_target_objects_preserved",
                self.non_target_objects_preserved,
            ),
            ("target_operator_preserved", self.target_operator_preserved),
            ("replacement_extractable", self.replacement_extractable),
            (
                "image_xobject_count_preserved",
                self.image_xobject_count_preserved,
            ),
            ("no_page_rasterization", self.no_page_rasterization),
        ];
        checks
            .into_iter()
            .filter_map(|(name, passed)| (!passed).then_some(name))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceTextResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub font_resource: String,
    pub source_text: String,
    pub replacement_text: String,
    pub source_encoded_bytes: usize,
    pub replacement_encoded_bytes: usize,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: EditValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflowTextValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub source_operator_replaced: bool,
    pub replacement_extractable: bool,
    pub region_confined: bool,
    pub obstacles_clear: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ReflowTextValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.source_operator_replaced
            && self.replacement_extractable
            && self.region_confined
            && self.obstacles_clear
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }

    fn failed_checks(&self) -> Vec<&'static str> {
        let checks = [
            ("reparsed", self.reparsed),
            ("page_count_preserved", self.page_count_preserved),
            ("object_count_preserved", self.object_count_preserved),
            (
                "non_target_objects_preserved",
                self.non_target_objects_preserved,
            ),
            ("source_operator_replaced", self.source_operator_replaced),
            ("replacement_extractable", self.replacement_extractable),
            ("region_confined", self.region_confined),
            ("obstacles_clear", self.obstacles_clear),
            (
                "image_xobject_count_preserved",
                self.image_xobject_count_preserved,
            ),
            ("no_page_rasterization", self.no_page_rasterization),
        ];
        checks
            .into_iter()
            .filter_map(|(name, passed)| (!passed).then_some(name))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflowedTextLine {
    pub text: String,
    pub baseline_x: f64,
    pub baseline_y: f64,
    pub measured_width: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReflowTextResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub font_resource: String,
    pub font_size: f64,
    pub font_metrics_source: String,
    pub source_text: String,
    pub replacement_text: String,
    pub region: PdfBox,
    pub alignment: PdfTextAlignment,
    pub line_spacing: f64,
    pub lines: Vec<ReflowedTextLine>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ReflowTextValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageReorderValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub requested_order_preserved: bool,
    pub page_object_ids_preserved: bool,
    pub page_content_references_preserved: bool,
    pub non_page_tree_objects_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl PageReorderValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.requested_order_preserved
            && self.page_object_ids_preserved
            && self.page_content_references_preserved
            && self.non_page_tree_objects_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReorderPagesResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub source_page_order: Vec<PdfObjectId>,
    pub result_page_order: Vec<PdfObjectId>,
    pub requested_source_page_numbers: Vec<u32>,
    pub changed_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: PageReorderValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageReplacementValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_object_preserved: bool,
    pub target_stream_replaced: bool,
    pub replacement_dimensions_verified: bool,
    pub target_references_preserved: bool,
    pub replacement_jpeg_decodable: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ImageReplacementValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_object_preserved
            && self.target_stream_replaced
            && self.replacement_dimensions_verified
            && self.target_references_preserved
            && self.replacement_jpeg_decodable
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplaceImageResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub image_object: PdfObjectId,
    pub source_stream_sha256: String,
    pub replacement_stream_sha256: String,
    pub source_width: u32,
    pub source_height: u32,
    pub replacement_width: u32,
    pub replacement_height: u32,
    pub replacement_color_space: String,
    pub affected_usages: Vec<ImageUsage>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ImageReplacementValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePlacementValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_content_stream_preserved: bool,
    pub target_draw_operation_preserved: bool,
    pub source_image_object_preserved: bool,
    pub requested_position_preserved: bool,
    pub non_target_operations_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ImagePlacementValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_content_stream_preserved
            && self.target_draw_operation_preserved
            && self.source_image_object_preserved
            && self.requested_position_preserved
            && self.non_target_operations_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveImageResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub image_object: PdfObjectId,
    pub content_stream_object: PdfObjectId,
    pub page_number: u32,
    pub resource_name: String,
    pub source_matrix: PdfMatrix,
    pub result_matrix: PdfMatrix,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ImagePlacementValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageTransformValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_content_stream_preserved: bool,
    pub target_draw_operation_preserved: bool,
    pub source_image_object_preserved: bool,
    pub requested_transform_preserved: bool,
    pub non_target_operations_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ImageTransformValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_content_stream_preserved
            && self.target_draw_operation_preserved
            && self.source_image_object_preserved
            && self.requested_transform_preserved
            && self.non_target_operations_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransformImageResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub image_object: PdfObjectId,
    pub content_stream_object: PdfObjectId,
    pub page_number: u32,
    pub resource_name: String,
    pub source_matrix: PdfMatrix,
    pub result_matrix: PdfMatrix,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ImageTransformValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageCropValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_content_stream_preserved: bool,
    pub target_draw_operation_preserved: bool,
    pub source_image_object_preserved: bool,
    pub requested_crop_preserved: bool,
    pub non_target_operations_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ImageCropValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_content_stream_preserved
            && self.target_draw_operation_preserved
            && self.source_image_object_preserved
            && self.requested_crop_preserved
            && self.non_target_operations_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CropImageResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub image_object: PdfObjectId,
    pub content_stream_object: PdfObjectId,
    pub page_number: u32,
    pub resource_name: String,
    pub matrix: PdfMatrix,
    pub source_crop: Option<PdfNormalizedCrop>,
    pub requested_crop: PdfNormalizedCrop,
    pub result_crop: Option<PdfNormalizedCrop>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ImageCropValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageOpacityValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_target_objects_preserved: bool,
    pub target_content_stream_preserved: bool,
    pub target_draw_operation_preserved: bool,
    pub source_image_object_preserved: bool,
    pub requested_opacity_preserved: bool,
    pub owned_ext_g_state_valid: bool,
    pub non_target_operations_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl ImageOpacityValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_target_objects_preserved
            && self.target_content_stream_preserved
            && self.target_draw_operation_preserved
            && self.source_image_object_preserved
            && self.requested_opacity_preserved
            && self.owned_ext_g_state_valid
            && self.non_target_operations_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetImageOpacityResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub image_object: PdfObjectId,
    pub content_stream_object: PdfObjectId,
    pub page_number: u32,
    pub resource_name: String,
    pub matrix: PdfMatrix,
    pub crop: Option<PdfNormalizedCrop>,
    pub source_opacity: f64,
    pub requested_opacity: f64,
    pub result_opacity: f64,
    pub result_opacity_state: Option<PdfImageOpacity>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: ImageOpacityValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum VectorPathMutationKind {
    Transform,
    Recolor,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VectorPathValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub target_content_stream_preserved: bool,
    pub requested_mutation_preserved: bool,
    pub non_target_operations_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl VectorPathValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_target_objects_preserved
            && self.target_content_stream_preserved
            && self.requested_mutation_preserved
            && self.non_target_operations_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditVectorPathResult {
    pub operation: String,
    pub capability: String,
    pub mutation: VectorPathMutationKind,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub content_stream_object: PdfObjectId,
    pub page_number: u32,
    pub source_bounds: PdfBox,
    pub result_bounds: Option<PdfBox>,
    pub source_transform: PdfMatrix,
    pub result_transform: Option<PdfMatrix>,
    pub source_fill_color: Option<PdfRgbColor>,
    pub result_fill_color: Option<PdfRgbColor>,
    pub source_stroke_color: Option<PdfRgbColor>,
    pub result_stroke_color: Option<PdfRgbColor>,
    pub paint_operator: String,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: VectorPathValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum LinkMutationKind {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkEditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_target_objects_preserved: bool,
    pub annotation_collection_preserved: bool,
    pub requested_mutation_preserved: bool,
    pub link_count_delta_valid: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl LinkEditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_target_objects_preserved
            && self.annotation_collection_preserved
            && self.requested_mutation_preserved
            && self.link_count_delta_valid
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditLinkResult {
    pub operation: String,
    pub capability: String,
    pub mutation: LinkMutationKind,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub page_object: PdfObjectId,
    pub annotation_object: PdfObjectId,
    pub source_rectangle: Option<PdfBox>,
    pub result_rectangle: Option<PdfBox>,
    pub source_target: Option<PdfLinkTarget>,
    pub result_target: Option<PdfLinkTarget>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: LinkEditValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AnnotationMutationKind {
    Create,
    Update,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnnotationEditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_target_objects_preserved: bool,
    pub annotation_collection_preserved: bool,
    pub requested_mutation_preserved: bool,
    pub annotation_count_delta_valid: bool,
    pub appearance_stream_valid: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl AnnotationEditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_target_objects_preserved
            && self.annotation_collection_preserved
            && self.requested_mutation_preserved
            && self.annotation_count_delta_valid
            && self.appearance_stream_valid
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditAnnotationResult {
    pub operation: String,
    pub capability: String,
    pub mutation: AnnotationMutationKind,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub page_object: PdfObjectId,
    pub annotation_object: PdfObjectId,
    pub appearance_object: PdfObjectId,
    pub source_spec: Option<PdfAnnotationSpec>,
    pub result_spec: Option<PdfAnnotationSpec>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: AnnotationEditValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataEditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_metadata_objects_preserved: bool,
    pub requested_values_preserved: bool,
    pub xmp_policy_preserved: bool,
    pub xmp_packet_valid: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl MetadataEditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_metadata_objects_preserved
            && self.requested_values_preserved
            && self.xmp_policy_preserved
            && self.xmp_packet_valid
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditMetadataResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub source_selector: PdfMetadataSelector,
    pub result_selector: PdfMetadataSelector,
    pub source_values: PdfMetadataSpec,
    pub result_values: PdfMetadataSpec,
    pub xmp_mode: PdfXmpMode,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: MetadataEditValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookmarkMutationKind {
    Create,
    Update,
    Move,
    Delete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BookmarkEditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_outline_objects_preserved: bool,
    pub outline_tree_valid: bool,
    pub requested_mutation_preserved: bool,
    pub bookmark_count_delta_valid: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl BookmarkEditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_outline_objects_preserved
            && self.outline_tree_valid
            && self.requested_mutation_preserved
            && self.bookmark_count_delta_valid
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditBookmarkResult {
    pub operation: String,
    pub capability: String,
    pub mutation: BookmarkMutationKind,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub bookmark_object: PdfObjectId,
    pub source_parent_object: Option<PdfObjectId>,
    pub result_parent_object: Option<PdfObjectId>,
    pub source_sibling_index: Option<usize>,
    pub result_sibling_index: Option<usize>,
    pub source_title: Option<String>,
    pub result_title: Option<String>,
    pub source_target: Option<PdfLinkTarget>,
    pub result_target: Option<PdfLinkTarget>,
    pub removed_subtree_count: usize,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: BookmarkEditValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldEditValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_form_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_value_preserved: bool,
    pub appearance_valid: bool,
    pub field_count_preserved: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldEditValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_form_objects_preserved
            && self.form_tree_valid
            && self.requested_value_preserved
            && self.appearance_valid
            && self.field_count_preserved
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldValueResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_kind: PdfFormFieldKind,
    pub source_value: Option<String>,
    pub result_value: Option<String>,
    pub widget_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldEditValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldGeometryValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_form_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_geometry_preserved: bool,
    pub appearance_valid: bool,
    pub field_count_preserved: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldGeometryValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_form_objects_preserved
            && self.form_tree_valid
            && self.requested_geometry_preserved
            && self.appearance_valid
            && self.field_count_preserved
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldGeometryResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_kind: PdfFormFieldKind,
    pub widget_object: PdfObjectId,
    pub page_number: u32,
    pub page_object: PdfObjectId,
    pub source_rectangle: PdfBox,
    pub result_rectangle: PdfBox,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldGeometryValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldLabelValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub non_form_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_label_preserved: bool,
    pub field_identity_preserved: bool,
    pub field_value_preserved: bool,
    pub widget_objects_preserved: bool,
    pub field_count_preserved: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldLabelValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.non_form_objects_preserved
            && self.form_tree_valid
            && self.requested_label_preserved
            && self.field_identity_preserved
            && self.field_value_preserved
            && self.widget_objects_preserved
            && self.field_count_preserved
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldLabelResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_kind: PdfFormFieldKind,
    pub source_label: Option<String>,
    pub result_label: Option<String>,
    pub widget_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldLabelValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldOptionsValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_form_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_options_preserved: bool,
    pub requested_value_preserved: bool,
    pub appearance_valid: bool,
    pub field_identity_preserved: bool,
    pub widget_objects_preserved: bool,
    pub field_count_preserved: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldOptionsValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_form_objects_preserved
            && self.form_tree_valid
            && self.requested_options_preserved
            && self.requested_value_preserved
            && self.appearance_valid
            && self.field_identity_preserved
            && self.widget_objects_preserved
            && self.field_count_preserved
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditFormFieldOptionsResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_kind: PdfFormFieldKind,
    pub source_options: Vec<String>,
    pub result_options: Vec<String>,
    pub source_value: Option<String>,
    pub result_value: String,
    pub widget_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldOptionsValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldAppearanceRepairValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_form_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub source_appearance_invalid: bool,
    pub requested_appearance_repaired: bool,
    pub field_identity_preserved: bool,
    pub field_value_preserved: bool,
    pub widget_objects_preserved: bool,
    pub field_count_preserved: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldAppearanceRepairValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_form_objects_preserved
            && self.form_tree_valid
            && self.source_appearance_invalid
            && self.requested_appearance_repaired
            && self.field_identity_preserved
            && self.field_value_preserved
            && self.widget_objects_preserved
            && self.field_count_preserved
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairFormFieldAppearanceResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_kind: PdfFormFieldKind,
    pub field_value: Option<String>,
    pub widget_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldAppearanceRepairValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldCreationValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_target_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_field_preserved: bool,
    pub requested_widgets_preserved: bool,
    pub appearance_valid: bool,
    pub field_count_incremented: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldCreationValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_target_objects_preserved
            && self.form_tree_valid
            && self.requested_field_preserved
            && self.requested_widgets_preserved
            && self.appearance_valid
            && self.field_count_incremented
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateFormFieldResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_label: Option<String>,
    pub field_kind: PdfFormFieldKind,
    pub result_value: Option<String>,
    pub options: Vec<String>,
    pub widget_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldCreationValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormFieldDeletionValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_delta_valid: bool,
    pub non_target_objects_preserved: bool,
    pub form_tree_valid: bool,
    pub requested_field_removed: bool,
    pub widgets_removed_from_pages: bool,
    pub field_count_decremented: bool,
    pub deleted_objects_removed: bool,
    pub all_object_references_resolve: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl FormFieldDeletionValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_delta_valid
            && self.non_target_objects_preserved
            && self.form_tree_valid
            && self.requested_field_removed
            && self.widgets_removed_from_pages
            && self.field_count_decremented
            && self.deleted_objects_removed
            && self.all_object_references_resolve
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteFormFieldResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub field_object: PdfObjectId,
    pub field_name: String,
    pub field_label: Option<String>,
    pub field_kind: PdfFormFieldKind,
    pub source_value: Option<String>,
    pub widget_objects: Vec<PdfObjectId>,
    pub removed_tree_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: FormFieldDeletionValidation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageGeometryValidation {
    pub reparsed: bool,
    pub page_count_preserved: bool,
    pub object_count_preserved: bool,
    pub target_page_object_preserved: bool,
    pub page_content_references_preserved: bool,
    pub requested_geometry_preserved: bool,
    pub non_target_objects_preserved: bool,
    pub image_xobject_count_preserved: bool,
    pub no_page_rasterization: bool,
}

impl PageGeometryValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.page_count_preserved
            && self.object_count_preserved
            && self.target_page_object_preserved
            && self.page_content_references_preserved
            && self.requested_geometry_preserved
            && self.non_target_objects_preserved
            && self.image_xobject_count_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPageGeometryResult {
    pub operation: String,
    pub capability: String,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub changed_objects: Vec<PdfObjectId>,
    pub page_object: PdfObjectId,
    pub source_media_box: PdfBox,
    pub source_crop_box: PdfBox,
    pub source_rotation: i32,
    pub result_media_box: PdfBox,
    pub result_crop_box: PdfBox,
    pub result_rotation: i32,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: PageGeometryValidation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PageMutationKind {
    Delete,
    Duplicate,
    InsertBlank,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageMutationValidation {
    pub reparsed: bool,
    pub requested_page_count_preserved: bool,
    pub requested_order_preserved: bool,
    pub surviving_page_object_ids_preserved: bool,
    pub inserted_page_objects_valid: bool,
    pub removed_pages_unreachable: bool,
    pub page_content_references_preserved: bool,
    pub all_object_references_resolve: bool,
    pub non_target_objects_preserved: bool,
    pub no_page_rasterization: bool,
}

impl PageMutationValidation {
    pub fn passed(&self) -> bool {
        self.reparsed
            && self.requested_page_count_preserved
            && self.requested_order_preserved
            && self.surviving_page_object_ids_preserved
            && self.inserted_page_objects_valid
            && self.removed_pages_unreachable
            && self.page_content_references_preserved
            && self.all_object_references_resolve
            && self.non_target_objects_preserved
            && self.no_page_rasterization
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MutatePagesResult {
    pub operation: String,
    pub capability: String,
    pub mutation: PageMutationKind,
    pub input_sha256: String,
    pub import_input_sha256: Option<String>,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub source_page_count: usize,
    pub output_page_count: usize,
    pub source_object_count: usize,
    pub output_object_count: usize,
    pub source_page_order: Vec<PdfObjectId>,
    pub result_page_order: Vec<PdfObjectId>,
    pub inserted_page_objects: Vec<PdfObjectId>,
    pub removed_page_objects: Vec<PdfObjectId>,
    pub changed_objects: Vec<PdfObjectId>,
    pub save_mode: SaveMode,
    pub signature_effect: SignatureEffect,
    pub validation: PageMutationValidation,
}

pub fn inspect_pdf(path: &Path) -> Result<PdfInspection, PdfEditError> {
    inspect_pdf_for_page(path, None)
}

pub fn inspect_pdf_for_page(
    path: &Path,
    text_page: Option<u32>,
) -> Result<PdfInspection, PdfEditError> {
    let metadata = checked_metadata(path)?;
    let document = load_checked_pdf(path)?;
    let page_count = document.get_pages().len();
    if text_page.is_some_and(|page| page == 0 || page as usize > page_count) {
        return Err(PdfEditError::PageLimitExceeded);
    }
    let signatures = signature_summaries(&document);
    let (editable_text_runs, editable_text_runs_truncated, unsupported_text_show_operations) =
        inspect_text_runs_for_page(&document, text_page)?;
    let (editable_images, editable_images_truncated, unsupported_images) =
        inspect_editable_images(&document)?;
    let (
        editable_image_placements,
        editable_image_placements_truncated,
        unsupported_image_placements,
    ) = inspect_image_placements(&document)?;
    let (editable_vector_paths, editable_vector_paths_truncated, unsupported_vector_path_groups) =
        inspect_vector_paths(&document)?;
    let (
        editable_links,
        editable_links_truncated,
        editable_link_pages,
        unsupported_links,
        unsupported_link_pages,
    ) = inspect_links(&document)?;
    let (
        editable_annotations,
        editable_annotations_truncated,
        editable_annotation_pages,
        unsupported_annotations,
        unsupported_annotation_pages,
    ) = inspect_annotations(&document)?;
    let (editable_metadata, unsupported_metadata) = inspect_metadata(&document);
    let (
        editable_bookmark_tree,
        editable_bookmarks,
        editable_bookmarks_truncated,
        unsupported_bookmarks,
    ) = inspect_bookmarks(&document)?;
    let (editable_form_fields, editable_form_fields_truncated, unsupported_form_fields) =
        inspect_form_fields(&document)?;
    let form_creation = form_creation_selector(&document);
    let (editable_pages, unsupported_page_geometries) = inspect_page_geometries(&document)?;

    Ok(PdfInspection {
        format_version: document.version.clone(),
        input_sha256: sha256_file(path)?,
        input_bytes: metadata.len(),
        page_count,
        object_count: document.objects.len(),
        encrypted: document.is_encrypted(),
        object_graph: object_graph_summary(&document, signatures.len()),
        signatures,
        editable_text_runs,
        editable_text_runs_truncated,
        unsupported_text_show_operations,
        editable_images,
        editable_images_truncated,
        unsupported_images,
        editable_image_placements,
        editable_image_placements_truncated,
        unsupported_image_placements,
        editable_vector_paths,
        editable_vector_paths_truncated,
        unsupported_vector_path_groups,
        editable_links,
        editable_links_truncated,
        editable_link_pages,
        unsupported_links,
        unsupported_link_pages,
        editable_annotations,
        editable_annotations_truncated,
        editable_annotation_pages,
        unsupported_annotations,
        unsupported_annotation_pages,
        editable_metadata,
        unsupported_metadata,
        editable_bookmark_tree,
        editable_bookmarks,
        editable_bookmarks_truncated,
        unsupported_bookmarks,
        editable_form_fields,
        editable_form_fields_truncated,
        unsupported_form_fields,
        form_creation,
        editable_pages,
        unsupported_page_geometries,
    })
}

pub fn replace_text(request: &ReplaceTextRequest) -> Result<ReplaceTextResult, PdfEditError> {
    if request.selector.expected_text.len() > MAX_EDITABLE_TEXT_RUN_BYTES
        || request.selector.expected_text.chars().count() > MAX_EDITABLE_TEXT_RUN_CHARS
        || request.replacement_text.len() > MAX_EDITABLE_TEXT_RUN_BYTES
        || request.replacement_text.chars().count() > MAX_EDITABLE_TEXT_RUN_CHARS
    {
        return Err(PdfEditError::TextRunUnsupported);
    }
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let source_page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let source_page_id = page_id(&document, request.selector.page_number)?;
    let target_id: ObjectId = request.selector.content_stream_object.into();
    if !document
        .get_page_contents(source_page_id)
        .contains(&target_id)
    {
        return Err(PdfEditError::ContentStreamNotOnPage);
    }

    let (replacement_bytes, source_bytes, font_resource) = replacement_bytes_for_target(
        &document,
        source_page_id,
        target_id,
        &request.selector,
        &request.replacement_text,
    )?;
    // The encoding round-trip proves the codes exist, not that the embedded
    // (usually subset) font program has their glyphs.
    {
        let fonts = document
            .get_page_fonts(source_page_id)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let font = fonts
            .get(font_resource.as_bytes())
            .ok_or(PdfEditError::TextRunUnsupported)?;
        if !full_font::embedded_font_draws(&document, font, &replacement_bytes, &request.replacement_text) {
            return Err(PdfEditError::GlyphUnavailable);
        }
    }

    let target_stream = document
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let plain_content = target_stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let mut content =
        Content::decode(&plain_content).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
        return Err(PdfEditError::ContentOperationLimitExceeded);
    }
    let operation = content
        .operations
        .get_mut(request.selector.operation_index)
        .ok_or(PdfEditError::StaleTextRun)?;
    let source_operator = operation.operator.clone();
    let fonts = document
        .get_page_fonts(source_page_id)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let font = fonts
        .get(font_resource.as_bytes())
        .ok_or(PdfEditError::TextRunUnsupported)?;
    let encoding = font
        .get_font_encoding_with_limit(&document, MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    text_show::replace(operation, &encoding, &request.replacement_text)?;
    let encoded_content = content
        .encode()
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;

    let stream = document
        .get_object_mut(target_id)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    stream.set_plain_content(encoded_content);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let target_operation = target_operation(
        &output,
        page_id(&output, request.selector.page_number)?,
        target_id,
        &request.selector,
    )?;
    let replacement_extractable = decoded_tj_text(
        &output,
        page_id(&output, request.selector.page_number)?,
        &target_operation,
        &font_resource,
    )? == request.replacement_text;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = EditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == source_page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        target_operator_preserved: target_operation.operator == source_operator,
        replacement_extractable,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };

    if !validation.passed() {
        eprintln!(
            "PDF_EDIT_VALIDATION_FAILED failed_checks={}",
            validation.failed_checks().join(",")
        );
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(ReplaceTextResult {
        operation: "REPLACE_TEXT".to_string(),
        capability: "PDF_TEXT_REPLACE".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count: source_page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        font_resource,
        source_text: request.selector.expected_text.clone(),
        replacement_text: request.replacement_text.clone(),
        source_encoded_bytes: source_bytes.len(),
        replacement_encoded_bytes: replacement_bytes.len(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn reflow_text(request: &ReflowTextRequest) -> Result<ReflowTextResult, PdfEditError> {
    if request.replacement_text.len() > MAX_REFLOW_TEXT_BYTES
        || request.replacement_text.chars().count() > MAX_REFLOW_TEXT_CHARS
        || normalized_reflow_text(&request.replacement_text).is_empty()
        || !valid_pdf_box(request.region)
        || !request.line_spacing.is_finite()
    {
        return Err(PdfEditError::ReflowUnsafe);
    }
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let source_page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let source_page_id = page_id(&document, request.selector.page_number)?;
    let target_id: ObjectId = request.selector.content_stream_object.into();
    if !document
        .get_page_contents(source_page_id)
        .contains(&target_id)
    {
        return Err(PdfEditError::ContentStreamNotOnPage);
    }
    let page_geometry = inspect_page_geometries(&document)?
        .0
        .into_iter()
        .find(|page| page.selector.page_number == request.selector.page_number)
        .ok_or(PdfEditError::ReflowUnsafe)?;
    if !pdf_box_contains(&page_geometry.crop_box, &request.region) {
        return Err(PdfEditError::ReflowUnsafe);
    }

    let (_, source_bytes, font_resource) = replacement_bytes_for_target(
        &document,
        source_page_id,
        target_id,
        &request.selector,
        &request.replacement_text,
    )?;
    let target_stream = document
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let plain_content = target_stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let mut content =
        Content::decode(&plain_content).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM
        || !isolated_reflow_operation(&content, request.selector.operation_index)
    {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let matrix_operation_index = request.selector.operation_index - 1;
    let source_matrix = text_matrix(&content.operations[matrix_operation_index].operands)
        .ok_or(PdfEditError::ReflowUnsafe)?;
    if !horizontal_unit_text_matrix(&source_matrix) {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let font_size = content.operations[..request.selector.operation_index]
        .iter()
        .rev()
        .find(|operation| operation.operator == "Tf")
        .and_then(|operation| operation.operands.get(1))
        .and_then(pdf_number)
        .filter(|value| value.is_finite() && (1.0..=512.0).contains(value))
        .ok_or(PdfEditError::ReflowUnsafe)?;
    if request.line_spacing < font_size * 0.8 || request.line_spacing > font_size * 4.0 {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let fonts = document
        .get_page_fonts(source_page_id)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let font = fonts
        .get(font_resource.as_bytes())
        .ok_or(PdfEditError::ReflowUnsafe)?;
    let metrics = simple_font_metrics(&document, font).ok_or(PdfEditError::ReflowUnsafe)?;
    let source_measurement =
        measure_simple_text_run(&document, font, &source_bytes, font_size, &source_matrix)
            .ok_or(PdfEditError::ReflowUnsafe)?;
    if !pdf_box_contains(&request.region, &source_measurement.bounding_box) {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let obstacles_clear = reflow_region_is_clear(&document, &request.selector, &request.region)?;
    if !obstacles_clear {
        return Err(PdfEditError::ReflowUnsafe);
    }

    let region_width = request.region.right - request.region.left;
    let wrapped = wrap_reflow_text(
        &document,
        font,
        &metrics,
        font_size,
        &request.replacement_text,
        region_width,
    )?;
    let ascent_points = metrics.ascent * font_size / 1_000.0;
    let descent_points = metrics.descent * font_size / 1_000.0;
    let first_baseline = request.region.top - ascent_points;
    let last_baseline = first_baseline - request.line_spacing * (wrapped.len() - 1) as f64;
    if first_baseline > request.region.top
        || last_baseline + descent_points < request.region.bottom - 0.000_001
    {
        return Err(PdfEditError::ReflowUnsafe);
    }
    let mut lines = Vec::with_capacity(wrapped.len());
    let mut operations = Vec::with_capacity(wrapped.len() * 2);
    let source_format = match &content.operations[request.selector.operation_index].operands[0] {
        Object::String(_, format) => *format,
        _ => return Err(PdfEditError::ReflowUnsafe),
    };
    for (index, (text, encoded, width)) in wrapped.into_iter().enumerate() {
        let baseline_x = match request.alignment {
            PdfTextAlignment::Left => request.region.left,
            PdfTextAlignment::Center => request.region.left + (region_width - width) / 2.0,
            PdfTextAlignment::Right => request.region.right - width,
        };
        let baseline_y = first_baseline - request.line_spacing * index as f64;
        if baseline_x < request.region.left - 0.000_001
            || baseline_x + width > request.region.right + 0.000_001
            || baseline_y + ascent_points > request.region.top + 0.000_001
            || baseline_y + descent_points < request.region.bottom - 0.000_001
        {
            return Err(PdfEditError::ReflowUnsafe);
        }
        operations.push(Operation::new(
            "Tm",
            vec![
                1.into(),
                0.into(),
                0.into(),
                1.into(),
                pdf_number_object(baseline_x)?,
                pdf_number_object(baseline_y)?,
            ],
        ));
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(encoded, source_format)],
        ));
        lines.push(ReflowedTextLine {
            text,
            baseline_x,
            baseline_y,
            measured_width: width,
        });
    }
    content.operations.splice(
        matrix_operation_index..=request.selector.operation_index,
        operations,
    );
    let encoded_content = content
        .encode()
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let stream = document
        .get_object_mut(target_id)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    stream.set_plain_content(encoded_content);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let output_page_id = page_id(&output, request.selector.page_number)?;
    let output_stream = output
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let output_content = Content::decode(
        &output_stream
            .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
            .map_err(|_| PdfEditError::ValidationFailed)?,
    )
    .map_err(|_| PdfEditError::ValidationFailed)?;
    let mut decoded_lines = Vec::with_capacity(lines.len());
    let mut source_operator_replaced = true;
    let mut region_confined = true;
    for (index, expected) in lines.iter().enumerate() {
        let matrix_index = matrix_operation_index + index * 2;
        let text_index = matrix_index + 1;
        let Some(matrix_operation) = output_content.operations.get(matrix_index) else {
            source_operator_replaced = false;
            region_confined = false;
            break;
        };
        let Some(text_operation) = output_content.operations.get(text_index) else {
            source_operator_replaced = false;
            region_confined = false;
            break;
        };
        let matrix = text_matrix(&matrix_operation.operands);
        source_operator_replaced &=
            matrix_operation.operator == "Tm" && text_operation.operator == "Tj";
        region_confined &= matrix.as_ref().is_some_and(|value| {
            nearly_equal(value.e, expected.baseline_x)
                && nearly_equal(value.f, expected.baseline_y)
                && value.e >= request.region.left - REFLOW_COORDINATE_TOLERANCE
                && value.e + expected.measured_width
                    <= request.region.right + REFLOW_COORDINATE_TOLERANCE
                && value.f + ascent_points <= request.region.top + REFLOW_COORDINATE_TOLERANCE
                && value.f + descent_points >= request.region.bottom - REFLOW_COORDINATE_TOLERANCE
        });
        match decoded_tj_text(&output, output_page_id, text_operation, &font_resource) {
            Ok(value) => decoded_lines.push(value),
            Err(_) => source_operator_replaced = false,
        }
    }
    let terminal_index = output_content
        .operations
        .iter()
        .enumerate()
        .skip(matrix_operation_index + lines.len() * 2)
        .find(|(_, operation)| operation.operator != "T*")
        .map(|(index, _)| index);
    source_operator_replaced &= terminal_index.is_some_and(|index| {
        output_content
            .operations
            .get(index)
            .is_some_and(|operation| operation.operator == "ET")
    });
    let replacement_extractable = normalized_reflow_text(&decoded_lines.join(" "))
        == normalized_reflow_text(&request.replacement_text);
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = ReflowTextValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == source_page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        source_operator_replaced,
        replacement_extractable,
        region_confined,
        obstacles_clear,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        eprintln!(
            "PDF_REFLOW_VALIDATION_FAILED failed_checks={}",
            validation.failed_checks().join(",")
        );
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(ReflowTextResult {
        operation: "REFLOW_TEXT".to_string(),
        capability: "PDF_TEXT_REFLOW".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count: source_page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        font_resource,
        font_size,
        font_metrics_source: metrics.source,
        source_text: request.selector.expected_text.clone(),
        replacement_text: request.replacement_text.clone(),
        region: request.region,
        alignment: request.alignment,
        line_spacing: request.line_spacing,
        lines,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn reorder_pages(request: &ReorderPagesRequest) -> Result<ReorderPagesResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let source_pages = document.get_pages();
    let page_count = source_pages.len();
    let expected_numbers: BTreeSet<u32> = (1..=page_count as u32).collect();
    let requested_numbers: BTreeSet<u32> = request.page_order.iter().copied().collect();
    let identity_order: Vec<u32> = (1..=page_count as u32).collect();
    if request.page_order.len() != page_count
        || requested_numbers != expected_numbers
        || request.page_order == identity_order
    {
        return Err(PdfEditError::InvalidPageOrder);
    }

    let source_page_order: Vec<ObjectId> = source_pages.values().copied().collect();
    let requested_page_order: Vec<ObjectId> = request
        .page_order
        .iter()
        .map(|page_number| {
            source_pages
                .get(page_number)
                .copied()
                .ok_or(PdfEditError::InvalidPageOrder)
        })
        .collect::<Result<_, _>>()?;
    let source_page_contents: BTreeMap<ObjectId, Vec<ObjectId>> = source_page_order
        .iter()
        .map(|page_id| (*page_id, document.get_page_contents(*page_id)))
        .collect();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let root_pages_id = root_pages_id(&document)?;

    // Flattening a nested page tree is safe only after inherited page attributes
    // are materialised on every leaf. Page object IDs and content references stay
    // stable, so destinations, forms, and annotations keep referring to the same
    // actual page objects.
    const INHERITABLE_PAGE_KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
    let mut inherited: BTreeMap<ObjectId, Vec<(Vec<u8>, Object)>> = BTreeMap::new();
    for page_id in &source_page_order {
        let mut values = Vec::new();
        for key in INHERITABLE_PAGE_KEYS {
            if let Some(value) = inherited_page_property(&document, *page_id, key)? {
                values.push((key.to_vec(), value));
            }
        }
        inherited.insert(*page_id, values);
    }

    for page_id in &source_page_order {
        let page = document
            .get_dictionary_mut(*page_id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?;
        for (key, value) in inherited.get(page_id).into_iter().flatten() {
            if !page.has(key) {
                page.set(key.clone(), value.clone());
            }
        }
        page.set("Parent", root_pages_id);
    }
    let root_pages = document
        .get_dictionary_mut(root_pages_id)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    if !root_pages.has_type(b"Pages") {
        return Err(PdfEditError::PageTreeUnsupported);
    }
    root_pages.set(
        "Kids",
        requested_page_order
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    root_pages.set("Count", page_count as i64);

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    let allowed_changed_objects: BTreeSet<ObjectId> = source_page_order
        .iter()
        .copied()
        .chain(std::iter::once(root_pages_id))
        .collect();
    if changed_objects.is_empty()
        || changed_objects
            .iter()
            .any(|object_id| !allowed_changed_objects.contains(object_id))
    {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_pages = output.get_pages();
    let result_page_order: Vec<ObjectId> = output_pages.values().copied().collect();
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let page_content_references_preserved = source_page_contents
        .iter()
        .all(|(page_id, content_ids)| output.get_page_contents(*page_id) == *content_ids);
    let non_page_tree_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_objects.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = PageReorderValidation {
        reparsed: true,
        page_count_preserved: output_pages.len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        requested_order_preserved: result_page_order == requested_page_order,
        page_object_ids_preserved: result_page_order.iter().copied().collect::<BTreeSet<_>>()
            == source_page_order.iter().copied().collect::<BTreeSet<_>>(),
        page_content_references_preserved,
        non_page_tree_objects_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(ReorderPagesResult {
        operation: "REORDER_PAGES".to_string(),
        capability: "PDF_PAGE_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        source_page_order: source_page_order.into_iter().map(Into::into).collect(),
        result_page_order: result_page_order.into_iter().map(Into::into).collect(),
        requested_source_page_numbers: request.page_order.clone(),
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn mutate_pages(request: &MutatePagesRequest) -> Result<MutatePagesResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    if document
        .catalog()
        .is_ok_and(|catalog| catalog.has(b"PageLabels"))
    {
        return Err(PdfEditError::PageDependencyUnsupported);
    }

    let source_pages = document.get_pages();
    let source_page_count = source_pages.len();
    let source_page_order: Vec<ObjectId> = source_pages.values().copied().collect();
    let source_page_contents: BTreeMap<ObjectId, Vec<ObjectId>> = source_page_order
        .iter()
        .map(|page_id| (*page_id, document.get_page_contents(*page_id)))
        .collect();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let root_pages_id = root_pages_id(&document)?;

    let mut result_page_order = source_page_order.clone();
    let mut inserted_page_objects = Vec::new();
    let mut inserted_object_ids = BTreeSet::new();
    let mut removed_page_objects = Vec::new();
    let mut import_input_sha256 = None;
    let mutation_kind;

    match &request.mutation {
        PageMutation::Delete { selector } => {
            mutation_kind = PageMutationKind::Delete;
            if source_page_count <= 1 {
                return Err(PdfEditError::InvalidPageMutation);
            }
            let target = selected_page_id(&source_pages, *selector)?;
            if page_has_external_inbound_reference(&document, target) {
                return Err(PdfEditError::PageDependencyUnsupported);
            }
            result_page_order.retain(|page_id| *page_id != target);
            removed_page_objects.push(target);
        }
        PageMutation::Duplicate {
            selector,
            insert_before_page_number,
        } => {
            mutation_kind = PageMutationKind::Duplicate;
            validate_insert_position(*insert_before_page_number, source_page_count)?;
            if source_page_count >= MAX_PAGE_COUNT {
                return Err(PdfEditError::PageLimitExceeded);
            }
            let source_page = selected_page_id(&source_pages, *selector)?;
            let source_snapshot = document.clone();
            let (new_page, cloned_ids) = clone_page_into_document(
                &source_snapshot,
                source_page,
                &mut document,
                root_pages_id,
            )?;
            if inserted_object_ids.len() + cloned_ids.len() > MAX_PAGE_CLONE_OBJECTS {
                return Err(PdfEditError::PageTreeUnsupported);
            }
            inserted_page_objects.push(new_page);
            inserted_object_ids.extend(cloned_ids);
            result_page_order.insert((*insert_before_page_number - 1) as usize, new_page);
        }
        PageMutation::InsertBlank {
            insert_before_page_number,
            media_box,
        } => {
            mutation_kind = PageMutationKind::InsertBlank;
            validate_insert_position(*insert_before_page_number, source_page_count)?;
            if source_page_count >= MAX_PAGE_COUNT {
                return Err(PdfEditError::PageLimitExceeded);
            }
            validate_page_geometry(*media_box, *media_box, 0)?;
            let content_id = document.add_object(lopdf::Stream::new(Dictionary::new(), Vec::new()));
            let mut page = Dictionary::new();
            page.set("Type", "Page");
            page.set("Parent", root_pages_id);
            page.set("MediaBox", pdf_box_object(*media_box)?);
            page.set("CropBox", pdf_box_object(*media_box)?);
            page.set("Rotate", 0_i64);
            page.set("Resources", Dictionary::new());
            page.set("Contents", content_id);
            let page_id = document.add_object(page);
            inserted_page_objects.push(page_id);
            inserted_object_ids.extend([content_id, page_id]);
            result_page_order.insert((*insert_before_page_number - 1) as usize, page_id);
        }
        PageMutation::Import {
            import_path,
            source_pages: import_selectors,
            insert_before_page_number,
        } => {
            mutation_kind = PageMutationKind::Import;
            validate_insert_position(*insert_before_page_number, source_page_count)?;
            if import_selectors.is_empty()
                || source_page_count
                    .checked_add(import_selectors.len())
                    .is_none_or(|count| count > MAX_PAGE_COUNT)
            {
                return Err(PdfEditError::InvalidPageMutation);
            }
            let input_canonical = request
                .input_path
                .canonicalize()
                .map_err(|_| PdfEditError::InputUnavailable)?;
            let import_canonical = import_path
                .canonicalize()
                .map_err(|_| PdfEditError::InputUnavailable)?;
            if input_canonical == import_canonical {
                return Err(PdfEditError::InvalidPageMutation);
            }
            checked_metadata(import_path)?;
            import_input_sha256 = Some(sha256_file(import_path)?);
            let import_document = load_checked_pdf(import_path)?;
            let import_pages = import_document.get_pages();
            let insertion_start = (*insert_before_page_number - 1) as usize;
            for (insertion_offset, selector) in (insertion_start..).zip(import_selectors.iter()) {
                let source_page = selected_page_id(&import_pages, *selector)?;
                let (new_page, cloned_ids) = clone_page_into_document(
                    &import_document,
                    source_page,
                    &mut document,
                    root_pages_id,
                )?;
                if inserted_object_ids.len() + cloned_ids.len() > MAX_PAGE_CLONE_OBJECTS {
                    return Err(PdfEditError::PageTreeUnsupported);
                }
                inserted_page_objects.push(new_page);
                inserted_object_ids.extend(cloned_ids);
                result_page_order.insert(insertion_offset, new_page);
            }
        }
    }

    rewrite_flat_page_tree(
        &mut document,
        &source_page_order,
        &result_page_order,
        root_pages_id,
    )?;
    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    let allowed_changed_source_objects: BTreeSet<ObjectId> = source_page_order
        .iter()
        .copied()
        .chain(std::iter::once(root_pages_id))
        .collect();
    if changed_objects.is_empty()
        || changed_objects.iter().any(|object_id| {
            source_objects.contains_key(object_id)
                && !allowed_changed_source_objects.contains(object_id)
        })
    {
        return Err(PdfEditError::ValidationFailed);
    }

    let inserted_image_count = inserted_object_ids
        .iter()
        .filter(|object_id| {
            document
                .objects
                .get(object_id)
                .and_then(object_dictionary)
                .is_some_and(|dictionary| {
                    dictionary.has_type(b"XObject")
                        && dictionary
                            .get(b"Subtype")
                            .and_then(Object::as_name)
                            .is_ok_and(|name| name == b"Image")
                })
        })
        .count();

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_pages = output.get_pages();
    let output_page_order: Vec<ObjectId> = output_pages.values().copied().collect();
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let expected_page_count =
        source_page_count + inserted_page_objects.len() - removed_page_objects.len();
    let surviving_source_pages: BTreeSet<ObjectId> = source_page_order
        .iter()
        .copied()
        .filter(|page_id| !removed_page_objects.contains(page_id))
        .collect();
    let output_page_set: BTreeSet<ObjectId> = output_page_order.iter().copied().collect();
    let page_content_references_preserved = surviving_source_pages.iter().all(|page_id| {
        source_page_contents
            .get(page_id)
            .is_some_and(|content_ids| output.get_page_contents(*page_id) == *content_ids)
    });
    let inserted_page_objects_valid = inserted_page_objects.iter().all(|page_id| {
        output_page_set.contains(page_id)
            && output
                .get_dictionary(*page_id)
                .is_ok_and(|page| page.has_type(b"Page"))
            && page_geometry(
                &output,
                output_page_order
                    .iter()
                    .position(|candidate| candidate == page_id)
                    .map(|index| index as u32 + 1)
                    .unwrap_or_default(),
                *page_id,
            )
            .is_ok()
    });
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_source_objects.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = PageMutationValidation {
        reparsed: true,
        requested_page_count_preserved: output_pages.len() == expected_page_count,
        requested_order_preserved: output_page_order == result_page_order,
        surviving_page_object_ids_preserved: surviving_source_pages.is_subset(&output_page_set),
        inserted_page_objects_valid,
        removed_pages_unreachable: removed_page_objects
            .iter()
            .all(|page_id| !output_page_set.contains(page_id)),
        page_content_references_preserved,
        all_object_references_resolve: all_object_references_resolve(&output),
        non_target_objects_preserved,
        no_page_rasterization: output_image_count == source_image_count + inserted_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(MutatePagesResult {
        operation: "MUTATE_PAGES".to_string(),
        capability: "PDF_PAGE_EDIT".to_string(),
        mutation: mutation_kind,
        input_sha256,
        import_input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        source_page_count,
        output_page_count: output_pages.len(),
        source_object_count,
        output_object_count,
        source_page_order: source_page_order.into_iter().map(Into::into).collect(),
        result_page_order: output_page_order.into_iter().map(Into::into).collect(),
        inserted_page_objects: inserted_page_objects.into_iter().map(Into::into).collect(),
        removed_page_objects: removed_page_objects.into_iter().map(Into::into).collect(),
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn edit_page_geometry(
    request: &EditPageGeometryRequest,
) -> Result<EditPageGeometryResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    validate_page_geometry(request.media_box, request.crop_box, request.rotation)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let pages = document.get_pages();
    let page_count = pages.len();
    let target_page_id = pages
        .get(&request.selector.page_number)
        .copied()
        .ok_or(PdfEditError::PageNotFound)?;
    if target_page_id != request.selector.page_object.into() {
        return Err(PdfEditError::StalePageGeometry);
    }
    let source_geometry = page_geometry(&document, request.selector.page_number, target_page_id)?;
    if source_geometry.media_box != request.selector.expected_media_box
        || source_geometry.crop_box != request.selector.expected_crop_box
        || source_geometry.rotation != request.selector.expected_rotation
    {
        return Err(PdfEditError::StalePageGeometry);
    }
    if source_geometry.media_box == request.media_box
        && source_geometry.crop_box == request.crop_box
        && source_geometry.rotation == request.rotation
    {
        return Err(PdfEditError::InvalidPageGeometry);
    }

    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_page_contents = document.get_page_contents(target_page_id);
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let target_page = document
        .get_dictionary_mut(target_page_id)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    target_page.set("MediaBox", pdf_box_object(request.media_box)?);
    target_page.set("CropBox", pdf_box_object(request.crop_box)?);
    target_page.set("Rotate", i64::from(request.rotation));

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_page_id] {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_pages = output.get_pages();
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let result_geometry = page_geometry(
        &output,
        request.selector.page_number,
        output_pages
            .get(&request.selector.page_number)
            .copied()
            .ok_or(PdfEditError::ValidationFailed)?,
    )?;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_page_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = PageGeometryValidation {
        reparsed: true,
        page_count_preserved: output_pages.len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        target_page_object_preserved: output_pages.get(&request.selector.page_number)
            == Some(&target_page_id),
        page_content_references_preserved: output.get_page_contents(target_page_id)
            == source_page_contents,
        requested_geometry_preserved: pdf_boxes_equal(result_geometry.media_box, request.media_box)
            && pdf_boxes_equal(result_geometry.crop_box, request.crop_box)
            && result_geometry.rotation == request.rotation,
        non_target_objects_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(EditPageGeometryResult {
        operation: "EDIT_PAGE_GEOMETRY".to_string(),
        capability: "PDF_PAGE_GEOMETRY_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        page_object: target_page_id.into(),
        source_media_box: source_geometry.media_box,
        source_crop_box: source_geometry.crop_box,
        source_rotation: source_geometry.rotation,
        result_media_box: result_geometry.media_box,
        result_crop_box: result_geometry.crop_box,
        result_rotation: result_geometry.rotation,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn replace_image(request: &ReplaceImageRequest) -> Result<ReplaceImageResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let replacement_metadata = fs::metadata(&request.replacement_image_path)
        .map_err(|_| PdfEditError::ReplacementImageInvalid)?;
    if !replacement_metadata.is_file()
        || replacement_metadata.len() == 0
        || replacement_metadata.len() > MAX_REPLACEMENT_IMAGE_BYTES
    {
        return Err(PdfEditError::ReplacementImageInvalid);
    }
    let replacement_bytes = fs::read(&request.replacement_image_path)
        .map_err(|_| PdfEditError::ReplacementImageInvalid)?;
    let replacement = validate_replacement_jpeg(&replacement_bytes)?;

    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let target_id: ObjectId = request.selector.image_object.into();
    let affected_usages = image_usages(&document)?
        .remove(&target_id)
        .ok_or(PdfEditError::ImageNotFound)?;
    let source_stream = document
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ImageNotFound)?;
    let source_dimensions = supported_image_dimensions(source_stream)?;
    let source_stream_sha256 = sha256_bytes(&source_stream.content);
    if source_stream_sha256 != request.selector.expected_stream_sha256
        || source_dimensions.0 != request.selector.expected_width
        || source_dimensions.1 != request.selector.expected_height
    {
        return Err(PdfEditError::StaleImage);
    }

    let mut replacement_dictionary = source_stream.dict.clone();
    for key in [
        b"Filter".as_slice(),
        b"DecodeParms".as_slice(),
        b"ColorSpace".as_slice(),
        b"BitsPerComponent".as_slice(),
        b"Width".as_slice(),
        b"Height".as_slice(),
        b"Length".as_slice(),
        b"Decode".as_slice(),
        b"ImageMask".as_slice(),
        b"Mask".as_slice(),
        b"SMask".as_slice(),
        b"Matte".as_slice(),
    ] {
        replacement_dictionary.remove(key);
    }
    replacement_dictionary.set("Type", Object::Name(b"XObject".to_vec()));
    replacement_dictionary.set("Subtype", Object::Name(b"Image".to_vec()));
    replacement_dictionary.set("Width", i64::from(replacement.width));
    replacement_dictionary.set("Height", i64::from(replacement.height));
    replacement_dictionary.set(
        "ColorSpace",
        Object::Name(replacement.color_space.as_bytes().to_vec()),
    );
    replacement_dictionary.set("BitsPerComponent", 8_i64);
    replacement_dictionary.set("Filter", Object::Name(b"DCTDecode".to_vec()));
    document.objects.insert(
        target_id,
        Object::Stream(lopdf::Stream::new(
            replacement_dictionary,
            replacement_bytes.clone(),
        )),
    );

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let output_usages = image_usages(&output)?
        .remove(&target_id)
        .ok_or(PdfEditError::ValidationFailed)?;
    let output_stream = output
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let output_dimensions =
        supported_image_dimensions(output_stream).map_err(|_| PdfEditError::ValidationFailed)?;
    let output_replacement = validate_replacement_jpeg(&output_stream.content)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let replacement_stream_sha256 = sha256_bytes(&replacement_bytes);
    let validation = ImageReplacementValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        target_object_preserved: output.objects.contains_key(&target_id),
        target_stream_replaced: sha256_bytes(&output_stream.content) == replacement_stream_sha256,
        replacement_dimensions_verified: output_dimensions
            == (replacement.width, replacement.height),
        target_references_preserved: output_usages == affected_usages,
        replacement_jpeg_decodable: output_replacement == replacement,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(ReplaceImageResult {
        operation: "REPLACE_IMAGE".to_string(),
        capability: "PDF_IMAGE_REPLACE".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        image_object: target_id.into(),
        source_stream_sha256,
        replacement_stream_sha256,
        source_width: source_dimensions.0,
        source_height: source_dimensions.1,
        replacement_width: replacement.width,
        replacement_height: replacement.height,
        replacement_color_space: replacement.color_space.to_string(),
        affected_usages,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

struct ImageMatrixEdit {
    input_sha256: String,
    output_sha256: String,
    input_bytes: u64,
    output_bytes: u64,
    page_count: usize,
    source_object_count: usize,
    output_object_count: usize,
    changed_objects: Vec<ObjectId>,
    image_object: ObjectId,
    content_stream_object: ObjectId,
    page_number: u32,
    resource_name: String,
    source_matrix: PdfMatrix,
    result_matrix: PdfMatrix,
    signature_effect: SignatureEffect,
    validation: ImagePlacementValidation,
}

fn rewrite_image_placement_matrix(
    input_path: &Path,
    output_path: &Path,
    selector: &ImagePlacementSelector,
    destination_matrix: PdfMatrix,
    allow_signature_invalidation: bool,
) -> Result<ImageMatrixEdit, PdfEditError> {
    validate_paths(input_path, output_path)?;
    if !valid_image_matrix(destination_matrix)
        || matrix_equivalent(destination_matrix, selector.expected_matrix)
    {
        return Err(PdfEditError::InvalidImagePlacement);
    }

    let input_metadata = checked_metadata(input_path)?;
    let input_sha256 = sha256_file(input_path)?;
    let mut document = load_checked_pdf(input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let (mut source_content, source_matrix, image_id, _, _) =
        isolated_image_placement(&document, selector)?;
    let target_id: ObjectId = selector.content_stream_object.into();
    let source_page_id = page_id(&document, selector.page_number)?;
    let target_operation = source_content
        .operations
        .get_mut(selector.matrix_operation_index)
        .ok_or(PdfEditError::StaleImagePlacement)?;
    set_matrix(target_operation, destination_matrix)?;
    let encoded_content = source_content
        .encode()
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let stream = document
        .get_object_mut(target_id)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    stream.set_plain_content(encoded_content);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, output_path)?;
    let output_metadata = fs::metadata(output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(output_path)?;
    let output = load_checked_pdf(output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let result_selector = ImagePlacementSelector {
        expected_matrix: destination_matrix,
        ..selector.clone()
    };
    let (result_content, result_matrix, result_image_id, _, _) =
        isolated_image_placement(&output, &result_selector)
            .map_err(|_| PdfEditError::ValidationFailed)?;
    let non_target_operations_preserved = source_content.operations.len()
        == result_content.operations.len()
        && source_content
            .operations
            .iter()
            .zip(result_content.operations.iter())
            .enumerate()
            .all(|(index, (source, result))| {
                index == selector.matrix_operation_index || operations_equal(source, result)
            });
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let source_image_object_preserved = source_objects
        .get(&image_id)
        .zip(output.objects.get(&image_id))
        .is_some_and(|(source, result)| pdf_objects_semantically_equal(source, result));
    let validation = ImagePlacementValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        target_content_stream_preserved: output
            .get_page_contents(page_id(&output, selector.page_number)?)
            .contains(&target_id)
            && document
                .get_page_contents(source_page_id)
                .contains(&target_id),
        target_draw_operation_preserved: result_content
            .operations
            .get(selector.draw_operation_index)
            .zip(source_content.operations.get(selector.draw_operation_index))
            .is_some_and(|(result, source)| operations_equal(result, source)),
        source_image_object_preserved: source_image_object_preserved && result_image_id == image_id,
        requested_position_preserved: matrix_equivalent(result_matrix, destination_matrix),
        non_target_operations_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(ImageMatrixEdit {
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects,
        image_object: image_id,
        content_stream_object: target_id,
        page_number: selector.page_number,
        resource_name: selector.expected_resource_name.clone(),
        source_matrix,
        result_matrix,
        signature_effect,
        validation,
    })
}

pub fn move_image(request: &MoveImageRequest) -> Result<MoveImageResult, PdfEditError> {
    if !valid_pdf_coordinate(request.destination_x) || !valid_pdf_coordinate(request.destination_y)
    {
        return Err(PdfEditError::InvalidImagePlacement);
    }
    let destination_matrix = PdfMatrix {
        e: request.destination_x,
        f: request.destination_y,
        ..request.selector.expected_matrix
    };
    let edit = rewrite_image_placement_matrix(
        &request.input_path,
        &request.output_path,
        &request.selector,
        destination_matrix,
        request.allow_signature_invalidation,
    )?;
    Ok(MoveImageResult {
        operation: "MOVE_IMAGE".to_string(),
        capability: "PDF_OBJECT_MOVE".to_string(),
        input_sha256: edit.input_sha256,
        output_sha256: edit.output_sha256,
        input_bytes: edit.input_bytes,
        output_bytes: edit.output_bytes,
        page_count: edit.page_count,
        source_object_count: edit.source_object_count,
        output_object_count: edit.output_object_count,
        changed_objects: edit.changed_objects.into_iter().map(Into::into).collect(),
        image_object: edit.image_object.into(),
        content_stream_object: edit.content_stream_object.into(),
        page_number: edit.page_number,
        resource_name: edit.resource_name,
        source_matrix: edit.source_matrix,
        result_matrix: edit.result_matrix,
        save_mode: SaveMode::FullRewrite,
        signature_effect: edit.signature_effect,
        validation: edit.validation,
    })
}

pub fn transform_image(
    request: &TransformImageRequest,
) -> Result<TransformImageResult, PdfEditError> {
    let edit = rewrite_image_placement_matrix(
        &request.input_path,
        &request.output_path,
        &request.selector,
        request.destination_matrix,
        request.allow_signature_invalidation,
    )?;
    let validation = ImageTransformValidation {
        reparsed: edit.validation.reparsed,
        page_count_preserved: edit.validation.page_count_preserved,
        object_count_preserved: edit.validation.object_count_preserved,
        non_target_objects_preserved: edit.validation.non_target_objects_preserved,
        target_content_stream_preserved: edit.validation.target_content_stream_preserved,
        target_draw_operation_preserved: edit.validation.target_draw_operation_preserved,
        source_image_object_preserved: edit.validation.source_image_object_preserved,
        requested_transform_preserved: edit.validation.requested_position_preserved,
        non_target_operations_preserved: edit.validation.non_target_operations_preserved,
        image_xobject_count_preserved: edit.validation.image_xobject_count_preserved,
        no_page_rasterization: edit.validation.no_page_rasterization,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(TransformImageResult {
        operation: "TRANSFORM_IMAGE".to_string(),
        capability: "PDF_IMAGE_TRANSFORM".to_string(),
        input_sha256: edit.input_sha256,
        output_sha256: edit.output_sha256,
        input_bytes: edit.input_bytes,
        output_bytes: edit.output_bytes,
        page_count: edit.page_count,
        source_object_count: edit.source_object_count,
        output_object_count: edit.output_object_count,
        changed_objects: edit.changed_objects.into_iter().map(Into::into).collect(),
        image_object: edit.image_object.into(),
        content_stream_object: edit.content_stream_object.into(),
        page_number: edit.page_number,
        resource_name: edit.resource_name,
        source_matrix: edit.source_matrix,
        result_matrix: edit.result_matrix,
        save_mode: SaveMode::FullRewrite,
        signature_effect: edit.signature_effect,
        validation,
    })
}

fn operations_without_placement_crop<'a>(
    operations: &'a [Operation],
    selector: &ImagePlacementSelector,
) -> Vec<&'a Operation> {
    let crop_start = selector.matrix_operation_index + 1;
    let crop_end = crop_start + 3;
    operations
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| {
            (!(selector.expected_crop.is_some() && (crop_start..crop_end).contains(&index)))
                .then_some(operation)
        })
        .collect()
}

fn optional_crop_equivalent(
    left: Option<PdfNormalizedCrop>,
    right: Option<PdfNormalizedCrop>,
) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => crop_equivalent(left, right),
        _ => false,
    }
}

pub fn crop_image(request: &CropImageRequest) -> Result<CropImageResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    if !valid_normalized_crop(request.destination_crop) {
        return Err(PdfEditError::InvalidImageCrop);
    }
    let result_crop = effective_crop(request.destination_crop);
    if optional_crop_equivalent(request.selector.expected_crop, result_crop) {
        return Err(PdfEditError::InvalidImageCrop);
    }

    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let (source_content, matrix, image_id, source_crop, _) =
        isolated_image_placement(&document, &request.selector)?;
    let source_operations = source_content.operations.clone();
    let mut result_content = source_content;
    let crop_start = request.selector.matrix_operation_index + 1;
    match (source_crop, result_crop) {
        (None, Some(crop)) => {
            result_content.operations.splice(
                crop_start..crop_start,
                [
                    crop_operation(crop)?,
                    Operation::new("W", vec![]),
                    Operation::new("n", vec![]),
                ],
            );
        }
        (Some(_), Some(crop)) => {
            result_content.operations[crop_start] = crop_operation(crop)?;
        }
        (Some(_), None) => {
            result_content.operations.drain(crop_start..crop_start + 3);
        }
        (None, None) => return Err(PdfEditError::InvalidImageCrop),
    }

    let target_id: ObjectId = request.selector.content_stream_object.into();
    let source_page_id = page_id(&document, request.selector.page_number)?;
    let encoded_content = result_content
        .encode()
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let stream = document
        .get_object_mut(target_id)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    stream.set_plain_content(encoded_content);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let result_selector = ImagePlacementSelector {
        draw_operation_index: request.selector.matrix_operation_index
            + 1
            + if result_crop.is_some() { 3 } else { 0 }
            + if request.selector.expected_opacity.is_some() {
                1
            } else {
                0
            },
        expected_crop: result_crop,
        ..request.selector.clone()
    };
    let (verified_content, result_matrix, result_image_id, verified_crop, _) =
        isolated_image_placement(&output, &result_selector)
            .map_err(|_| PdfEditError::ValidationFailed)?;
    let source_selector = &request.selector;
    let source_without_crop =
        operations_without_placement_crop(&source_operations, source_selector);
    let result_without_crop =
        operations_without_placement_crop(&verified_content.operations, &result_selector);
    let non_target_operations_preserved = source_without_crop.len() == result_without_crop.len()
        && source_without_crop
            .iter()
            .zip(result_without_crop.iter())
            .all(|(source, result)| operations_equal(source, result));
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let source_image_object_preserved = source_objects
        .get(&image_id)
        .zip(output.objects.get(&image_id))
        .is_some_and(|(source, result)| pdf_objects_semantically_equal(source, result));
    let validation = ImageCropValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        target_content_stream_preserved: output
            .get_page_contents(page_id(&output, request.selector.page_number)?)
            .contains(&target_id)
            && document
                .get_page_contents(source_page_id)
                .contains(&target_id),
        target_draw_operation_preserved: source_operations
            .get(request.selector.draw_operation_index)
            .zip(
                verified_content
                    .operations
                    .get(result_selector.draw_operation_index),
            )
            .is_some_and(|(source, result)| operations_equal(source, result)),
        source_image_object_preserved: source_image_object_preserved && result_image_id == image_id,
        requested_crop_preserved: optional_crop_equivalent(verified_crop, result_crop)
            && matrix_equivalent(result_matrix, matrix),
        non_target_operations_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(CropImageResult {
        operation: "CROP_IMAGE".to_string(),
        capability: "PDF_IMAGE_CROP".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        image_object: image_id.into(),
        content_stream_object: target_id.into(),
        page_number: request.selector.page_number,
        resource_name: request.selector.expected_resource_name.clone(),
        matrix,
        source_crop,
        requested_crop: request.destination_crop,
        result_crop,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn install_owned_image_opacity(
    document: &mut Document,
    page_id: ObjectId,
    value: f64,
) -> Result<PdfImageOpacity, PdfEditError> {
    let mut resources = page_effective_resources_clone(document, page_id)?;
    let mut ext_g_states = ext_g_state_dictionary_clone(document, &resources)?;
    let mut state = Dictionary::new();
    state.set("Type", Object::Name(b"ExtGState".to_vec()));
    state.set("ca", pdf_number_object(value)?);
    state.set("CA", pdf_number_object(value)?);
    let object_id = document.add_object(Object::Dictionary(state));
    let resource_name = format!("WFImgAlpha{}", object_id.0);
    if ext_g_states.has(resource_name.as_bytes()) {
        document.objects.remove(&object_id);
        return Err(PdfEditError::ImagePlacementUnsupported);
    }
    ext_g_states.set(
        resource_name.as_bytes().to_vec(),
        Object::Reference(object_id),
    );
    resources.set("ExtGState", Object::Dictionary(ext_g_states));
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| PdfEditError::ImagePlacementUnsupported)?
        .set("Resources", Object::Dictionary(resources));
    Ok(PdfImageOpacity {
        value,
        resource_name,
        ext_g_state_object: object_id.into(),
    })
}

fn update_owned_image_opacity(
    document: &mut Document,
    page_id: ObjectId,
    expected: &PdfImageOpacity,
    value: f64,
) -> Result<PdfImageOpacity, PdfEditError> {
    let actual = owned_image_opacity(document, page_id, expected.resource_name.as_bytes())
        .ok_or(PdfEditError::StaleImagePlacement)?;
    if actual.ext_g_state_object != expected.ext_g_state_object
        || !opacity_equivalent(actual.value, expected.value)
    {
        return Err(PdfEditError::StaleImagePlacement);
    }
    let object_id: ObjectId = expected.ext_g_state_object.into();
    let state = document
        .get_object_mut(object_id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| PdfEditError::StaleImagePlacement)?;
    state.set("ca", pdf_number_object(value)?);
    state.set("CA", pdf_number_object(value)?);
    Ok(PdfImageOpacity {
        value,
        ..expected.clone()
    })
}

fn remove_owned_image_opacity(
    document: &mut Document,
    page_id: ObjectId,
    expected: &PdfImageOpacity,
) -> Result<(), PdfEditError> {
    let actual = owned_image_opacity(document, page_id, expected.resource_name.as_bytes())
        .ok_or(PdfEditError::StaleImagePlacement)?;
    if actual.ext_g_state_object != expected.ext_g_state_object
        || !opacity_equivalent(actual.value, expected.value)
    {
        return Err(PdfEditError::StaleImagePlacement);
    }
    let mut resources = page_effective_resources_clone(document, page_id)?;
    let mut ext_g_states = ext_g_state_dictionary_clone(document, &resources)?;
    let mapped_object = ext_g_states
        .get(expected.resource_name.as_bytes())
        .and_then(Object::as_reference)
        .map_err(|_| PdfEditError::StaleImagePlacement)?;
    if mapped_object != ObjectId::from(expected.ext_g_state_object) {
        return Err(PdfEditError::StaleImagePlacement);
    }
    ext_g_states.remove(expected.resource_name.as_bytes());
    if ext_g_states.iter().next().is_none() {
        resources.remove(b"ExtGState");
    } else {
        resources.set("ExtGState", Object::Dictionary(ext_g_states));
    }
    document
        .get_object_mut(page_id)
        .and_then(Object::as_dict_mut)
        .map_err(|_| PdfEditError::ImagePlacementUnsupported)?
        .set("Resources", Object::Dictionary(resources));
    document
        .objects
        .remove(&ObjectId::from(expected.ext_g_state_object))
        .ok_or(PdfEditError::StaleImagePlacement)?;
    Ok(())
}

fn operations_without_placement_opacity<'a>(
    operations: &'a [Operation],
    selector: &ImagePlacementSelector,
) -> Vec<&'a Operation> {
    let opacity_index = selector.matrix_operation_index
        + 1
        + if selector.expected_crop.is_some() {
            3
        } else {
            0
        };
    operations
        .iter()
        .enumerate()
        .filter_map(|(index, operation)| {
            (!(selector.expected_opacity.is_some() && index == opacity_index)).then_some(operation)
        })
        .collect()
}

pub fn set_image_opacity(
    request: &SetImageOpacityRequest,
) -> Result<SetImageOpacityResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    if !valid_image_opacity(request.destination_opacity) {
        return Err(PdfEditError::InvalidImageOpacity);
    }
    let result_opacity = effective_opacity(request.destination_opacity);
    let source_opacity_value = request
        .selector
        .expected_opacity
        .as_ref()
        .map_or(1.0, |state| state.value);
    if opacity_equivalent(source_opacity_value, request.destination_opacity) {
        return Err(PdfEditError::InvalidImageOpacity);
    }

    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let (source_content, matrix, image_id, crop, source_opacity_state) =
        isolated_image_placement(&document, &request.selector)?;
    let source_operations = source_content.operations.clone();
    let mut result_content = source_content;
    let page_id = page_id(&document, request.selector.page_number)?;
    let target_id: ObjectId = request.selector.content_stream_object.into();
    let opacity_operation_index = request.selector.matrix_operation_index
        + 1
        + if request.selector.expected_crop.is_some() {
            3
        } else {
            0
        };
    let (result_opacity_state, content_changed) =
        match (source_opacity_state.as_ref(), result_opacity) {
            (None, Some(value)) => {
                let state = install_owned_image_opacity(&mut document, page_id, value)?;
                result_content.operations.insert(
                    opacity_operation_index,
                    Operation::new(
                        "gs",
                        vec![Object::Name(state.resource_name.as_bytes().to_vec())],
                    ),
                );
                (Some(state), true)
            }
            (Some(source), Some(value)) => (
                Some(update_owned_image_opacity(
                    &mut document,
                    page_id,
                    source,
                    value,
                )?),
                false,
            ),
            (Some(source), None) => {
                remove_owned_image_opacity(&mut document, page_id, source)?;
                result_content.operations.remove(opacity_operation_index);
                (None, true)
            }
            (None, None) => return Err(PdfEditError::InvalidImageOpacity),
        };

    if content_changed {
        let encoded_content = result_content
            .encode()
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let stream = document
            .get_object_mut(target_id)
            .and_then(Object::as_stream_mut)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        stream.set_plain_content(encoded_content);
        stream
            .compress()
            .map_err(|_| PdfEditError::OutputWriteFailed)?;
    }

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let result_selector = ImagePlacementSelector {
        draw_operation_index: request.selector.matrix_operation_index
            + 1
            + if crop.is_some() { 3 } else { 0 }
            + if result_opacity_state.is_some() { 1 } else { 0 },
        expected_opacity: result_opacity_state.clone(),
        ..request.selector.clone()
    };
    let (verified_content, result_matrix, result_image_id, verified_crop, verified_opacity) =
        isolated_image_placement(&output, &result_selector)
            .map_err(|_| PdfEditError::ValidationFailed)?;
    let source_without_opacity =
        operations_without_placement_opacity(&source_operations, &request.selector);
    let result_without_opacity =
        operations_without_placement_opacity(&verified_content.operations, &result_selector);
    let non_target_operations_preserved = source_without_opacity.len()
        == result_without_opacity.len()
        && source_without_opacity
            .iter()
            .zip(result_without_opacity.iter())
            .all(|(source, result)| operations_equal(source, result));
    let source_opacity_object = source_opacity_state
        .as_ref()
        .map(|state| ObjectId::from(state.ext_g_state_object));
    let result_opacity_object = result_opacity_state
        .as_ref()
        .map(|state| ObjectId::from(state.ext_g_state_object));
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || *id == page_id
            || Some(*id) == source_opacity_object
            || Some(*id) == result_opacity_object
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let source_image_object_preserved = source_objects
        .get(&image_id)
        .zip(output.objects.get(&image_id))
        .is_some_and(|(source, result)| pdf_objects_semantically_equal(source, result));
    let source_semantic_count = semantic_object_count(&source_objects);
    let output_semantic_count = semantic_object_count(&output.objects);
    let expected_count_delta = match (
        source_opacity_state.is_some(),
        result_opacity_state.is_some(),
    ) {
        (false, true) => 1_i64,
        (true, false) => -1_i64,
        _ => 0_i64,
    };
    let object_count_delta_valid =
        output_semantic_count as i64 == source_semantic_count as i64 + expected_count_delta;
    let owned_ext_g_state_valid = match result_opacity_state.as_ref() {
        Some(expected) => verified_opacity.as_ref().is_some_and(|actual| {
            actual.ext_g_state_object == expected.ext_g_state_object
                && actual.resource_name == expected.resource_name
                && opacity_equivalent(actual.value, expected.value)
        }),
        None => source_opacity_state.as_ref().is_none_or(|source| {
            owned_image_opacity(&output, page_id, source.resource_name.as_bytes()).is_none()
                && !output
                    .objects
                    .contains_key(&ObjectId::from(source.ext_g_state_object))
        }),
    };
    let validation = ImageOpacityValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid,
        non_target_objects_preserved,
        target_content_stream_preserved: output.get_page_contents(page_id).contains(&target_id),
        target_draw_operation_preserved: source_operations
            .get(request.selector.draw_operation_index)
            .zip(
                verified_content
                    .operations
                    .get(result_selector.draw_operation_index),
            )
            .is_some_and(|(source, result)| operations_equal(source, result)),
        source_image_object_preserved: source_image_object_preserved && result_image_id == image_id,
        requested_opacity_preserved: opacity_equivalent(
            verified_opacity.as_ref().map_or(1.0, |state| state.value),
            request.destination_opacity,
        ) && matrix_equivalent(result_matrix, matrix)
            && optional_crop_equivalent(verified_crop, crop),
        owned_ext_g_state_valid,
        non_target_operations_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(SetImageOpacityResult {
        operation: "SET_IMAGE_OPACITY".to_string(),
        capability: "PDF_IMAGE_OPACITY".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        image_object: image_id.into(),
        content_stream_object: target_id.into(),
        page_number: request.selector.page_number,
        resource_name: request.selector.expected_resource_name.clone(),
        matrix,
        crop,
        source_opacity: source_opacity_value,
        requested_opacity: request.destination_opacity,
        result_opacity: request.destination_opacity,
        result_opacity_state,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn identity_matrix() -> PdfMatrix {
    PdfMatrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    }
}

fn valid_rgb_color(color: PdfRgbColor) -> bool {
    [color.red, color.green, color.blue]
        .into_iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
}

fn rgb_equivalent(left: PdfRgbColor, right: PdfRgbColor) -> bool {
    nearly_equal(left.red, right.red)
        && nearly_equal(left.green, right.green)
        && nearly_equal(left.blue, right.blue)
}

fn optional_rgb_equivalent(left: Option<PdfRgbColor>, right: Option<PdfRgbColor>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => rgb_equivalent(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn valid_vector_matrix(value: PdfMatrix) -> bool {
    [value.a, value.b, value.c, value.d, value.e, value.f]
        .into_iter()
        .all(f64::is_finite)
        && (value.a * value.d - value.b * value.c).abs() > 0.000_001
        && value.a.hypot(value.b) <= MAX_PAGE_DIMENSION_POINTS
        && value.c.hypot(value.d) <= MAX_PAGE_DIMENSION_POINTS
        && valid_pdf_coordinate(value.e)
        && valid_pdf_coordinate(value.f)
}

fn vector_matrix_from_operation(operation: &Operation) -> Option<PdfMatrix> {
    if operation.operator != "cm" || operation.operands.len() != 6 {
        return None;
    }
    let matrix = PdfMatrix {
        a: pdf_number(&operation.operands[0])?,
        b: pdf_number(&operation.operands[1])?,
        c: pdf_number(&operation.operands[2])?,
        d: pdf_number(&operation.operands[3])?,
        e: pdf_number(&operation.operands[4])?,
        f: pdf_number(&operation.operands[5])?,
    };
    valid_vector_matrix(matrix).then_some(matrix)
}

fn set_vector_matrix(
    operation: &mut Operation,
    destination: PdfMatrix,
) -> Result<(), PdfEditError> {
    if vector_matrix_from_operation(operation).is_none() || !valid_vector_matrix(destination) {
        return Err(PdfEditError::InvalidVectorEdit);
    }
    for (operand, value) in operation.operands.iter_mut().zip([
        destination.a,
        destination.b,
        destination.c,
        destination.d,
        destination.e,
        destination.f,
    ]) {
        *operand = pdf_number_object(value)?;
    }
    Ok(())
}

fn rgb_operation(operator: &str, color: PdfRgbColor) -> Result<Operation, PdfEditError> {
    if !valid_rgb_color(color) || !matches!(operator, "rg" | "RG") {
        return Err(PdfEditError::InvalidVectorEdit);
    }
    Ok(Operation::new(
        operator,
        vec![
            pdf_number_object(color.red)?,
            pdf_number_object(color.green)?,
            pdf_number_object(color.blue)?,
        ],
    ))
}

fn operation_rgb(operation: &Operation, stroking: bool) -> Option<PdfRgbColor> {
    let expected = if stroking {
        ["G", "RG", "K"]
    } else {
        ["g", "rg", "k"]
    };
    if !expected.contains(&operation.operator.as_str()) {
        return None;
    }
    let values = operation
        .operands
        .iter()
        .map(pdf_number)
        .collect::<Option<Vec<_>>>()?;
    let color = match (operation.operator.as_str(), values.as_slice()) {
        ("g" | "G", [gray]) => PdfRgbColor {
            red: *gray,
            green: *gray,
            blue: *gray,
        },
        ("rg" | "RG", [red, green, blue]) => PdfRgbColor {
            red: *red,
            green: *green,
            blue: *blue,
        },
        ("k" | "K", [cyan, magenta, yellow, black]) => PdfRgbColor {
            red: 1.0 - (cyan + black).min(1.0),
            green: 1.0 - (magenta + black).min(1.0),
            blue: 1.0 - (yellow + black).min(1.0),
        },
        _ => return None,
    };
    valid_rgb_color(color).then_some(color)
}

fn path_paint_capabilities(operator: &str) -> Option<(bool, bool)> {
    match operator {
        "f" | "F" | "f*" => Some((true, false)),
        "S" | "s" => Some((false, true)),
        "B" | "B*" | "b" | "b*" => Some((true, true)),
        _ => None,
    }
}

fn is_path_constructor(operator: &str) -> bool {
    matches!(operator, "m" | "l" | "c" | "v" | "y" | "h" | "re")
}

fn valid_path_operation(operation: &Operation) -> bool {
    let expected_operands = match operation.operator.as_str() {
        "m" | "l" => 2,
        "c" => 6,
        "v" | "y" | "re" => 4,
        "h" => 0,
        _ => return false,
    };
    operation.operands.len() == expected_operands
        && operation
            .operands
            .iter()
            .all(|value| pdf_number(value).is_some_and(valid_pdf_coordinate))
}

fn vector_operations_sha256(operations: &[Operation]) -> Option<String> {
    Content {
        operations: operations.to_vec(),
    }
    .encode()
    .ok()
    .map(|bytes| sha256_bytes(&bytes))
}

fn transformed_point(matrix: PdfMatrix, x: f64, y: f64) -> (f64, f64) {
    (
        matrix.a * x + matrix.c * y + matrix.e,
        matrix.b * x + matrix.d * y + matrix.f,
    )
}

fn vector_bounds(operations: &[Operation], matrix: PdfMatrix) -> Option<PdfBox> {
    let mut points = Vec::new();
    for operation in operations {
        match operation.operator.as_str() {
            "m" | "l" => points.push((
                pdf_number(operation.operands.first()?)?,
                pdf_number(operation.operands.get(1)?)?,
            )),
            "c" => {
                for pair in operation.operands.as_chunks::<2>().0 {
                    points.push((pdf_number(&pair[0])?, pdf_number(&pair[1])?));
                }
            }
            "v" | "y" => {
                for pair in operation.operands.as_chunks::<2>().0 {
                    points.push((pdf_number(&pair[0])?, pdf_number(&pair[1])?));
                }
            }
            "re" => {
                let x = pdf_number(operation.operands.first()?)?;
                let y = pdf_number(operation.operands.get(1)?)?;
                let width = pdf_number(operation.operands.get(2)?)?;
                let height = pdf_number(operation.operands.get(3)?)?;
                points.extend([
                    (x, y),
                    (x + width, y),
                    (x + width, y + height),
                    (x, y + height),
                ]);
            }
            _ => {}
        }
    }
    let mut transformed = points
        .into_iter()
        .map(|(x, y)| transformed_point(matrix, x, y));
    let first = transformed.next()?;
    let mut bounds = PdfBox {
        left: first.0,
        bottom: first.1,
        right: first.0,
        top: first.1,
    };
    for (x, y) in transformed {
        bounds.left = bounds.left.min(x);
        bounds.bottom = bounds.bottom.min(y);
        bounds.right = bounds.right.max(x);
        bounds.top = bounds.top.max(y);
    }
    ([bounds.left, bounds.bottom, bounds.right, bounds.top]
        .into_iter()
        .all(valid_pdf_coordinate)
        && bounds.right > bounds.left
        && bounds.top > bounds.bottom)
        .then_some(bounds)
}

fn recognized_vector_path(
    page_number: u32,
    content_stream_object: ObjectId,
    operations: &[Operation],
    start: usize,
    end: usize,
) -> Option<EditableVectorPath> {
    if end <= start + 2
        || operations.get(start)?.operator != "q"
        || !operations.get(start)?.operands.is_empty()
        || operations.get(end)?.operator != "Q"
        || !operations.get(end)?.operands.is_empty()
    {
        return None;
    }
    let inner = &operations[start + 1..end];
    let mut transform = None;
    let mut fill_color = None;
    let mut stroke_color = None;
    let mut path_started = false;
    let mut path_operations = Vec::new();
    let mut paint_operator = None;
    for (offset, operation) in inner.iter().enumerate() {
        if offset == 0 && operation.operator == "cm" {
            transform = Some(vector_matrix_from_operation(operation)?);
            continue;
        }
        if is_path_constructor(&operation.operator) {
            if paint_operator.is_some() || !valid_path_operation(operation) {
                return None;
            }
            path_started = true;
            path_operations.push(operation.clone());
            continue;
        }
        if let Some((_, _)) = path_paint_capabilities(&operation.operator) {
            if !path_started || paint_operator.is_some() || offset + 1 != inner.len() {
                return None;
            }
            paint_operator = Some(operation.operator.clone());
            continue;
        }
        if path_started {
            return None;
        }
        match operation.operator.as_str() {
            "g" | "rg" | "k" => fill_color = operation_rgb(operation, false),
            "G" | "RG" | "K" => stroke_color = operation_rgb(operation, true),
            "w" if operation.operands.len() == 1
                && operation.operands.first().and_then(pdf_number).is_some() => {}
            _ => return None,
        }
        if matches!(operation.operator.as_str(), "g" | "rg" | "k") && fill_color.is_none()
            || matches!(operation.operator.as_str(), "G" | "RG" | "K") && stroke_color.is_none()
        {
            return None;
        }
    }
    let paint_operator = paint_operator?;
    let (supports_fill, supports_stroke) = path_paint_capabilities(&paint_operator)?;
    let black = PdfRgbColor {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
    };
    let effective_transform = transform.unwrap_or_else(identity_matrix);
    let bounds = vector_bounds(&path_operations, effective_transform)?;
    let operations_sha256 = vector_operations_sha256(&operations[start..=end])?;
    let fill_color = supports_fill.then_some(fill_color.unwrap_or(black));
    let stroke_color = supports_stroke.then_some(stroke_color.unwrap_or(black));
    Some(EditableVectorPath {
        selector: VectorPathSelector {
            page_number,
            content_stream_object: content_stream_object.into(),
            start_operation_index: start,
            end_operation_index: end,
            expected_operations_sha256: operations_sha256,
            expected_transform: transform,
            expected_fill_color: fill_color,
            expected_stroke_color: stroke_color,
            expected_paint_operator: paint_operator.clone(),
        },
        transform: effective_transform,
        bounds,
        fill_color,
        stroke_color,
        paint_operator,
        supports_fill,
        supports_stroke,
    })
}

fn inspect_vector_paths(
    document: &Document,
) -> Result<(Vec<EditableVectorPath>, bool, usize), PdfEditError> {
    let mut editable = Vec::new();
    let mut truncated = false;
    let mut unsupported = 0usize;
    for (page_number, page_id) in document.get_pages() {
        for stream_id in document.get_page_contents(page_id) {
            let stream = match document.get_object(stream_id).and_then(Object::as_stream) {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let bytes = stream
                .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
                .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            let content =
                Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
                return Err(PdfEditError::ContentOperationLimitExceeded);
            }
            let mut index = 0usize;
            while index < content.operations.len() {
                if content.operations[index].operator != "q" {
                    index += 1;
                    continue;
                }
                let mut cursor = index + 1;
                let mut nested = false;
                while cursor < content.operations.len()
                    && content.operations[cursor].operator != "Q"
                {
                    if content.operations[cursor].operator == "q" {
                        nested = true;
                    }
                    cursor += 1;
                }
                if cursor >= content.operations.len() {
                    break;
                }
                let contains_path = content.operations[index + 1..cursor]
                    .iter()
                    .any(|operation| is_path_constructor(&operation.operator));
                let candidate = (!nested)
                    .then(|| {
                        recognized_vector_path(
                            page_number,
                            stream_id,
                            &content.operations,
                            index,
                            cursor,
                        )
                    })
                    .flatten();
                match candidate {
                    Some(path) if editable.len() < MAX_EDITABLE_VECTOR_PATHS => editable.push(path),
                    Some(_) => truncated = true,
                    None if contains_path => unsupported += 1,
                    None => {}
                }
                index = cursor + 1;
            }
        }
    }
    Ok((editable, truncated, unsupported))
}

fn selected_vector_path(
    document: &Document,
    selector: &VectorPathSelector,
) -> Result<(Content, EditableVectorPath), PdfEditError> {
    let page = page_id(document, selector.page_number)?;
    let stream_id: ObjectId = selector.content_stream_object.into();
    if !document.get_page_contents(page).contains(&stream_id) {
        return Err(PdfEditError::VectorPathNotFound);
    }
    let stream = document
        .get_object(stream_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::VectorPathNotFound)?;
    let bytes = stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let content = Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
        return Err(PdfEditError::ContentOperationLimitExceeded);
    }
    let path = recognized_vector_path(
        selector.page_number,
        stream_id,
        &content.operations,
        selector.start_operation_index,
        selector.end_operation_index,
    )
    .ok_or(PdfEditError::VectorPathUnsupported)?;
    if path.selector.expected_operations_sha256 != selector.expected_operations_sha256
        || path.selector.expected_paint_operator != selector.expected_paint_operator
        || path.selector.expected_transform != selector.expected_transform
        || !optional_rgb_equivalent(
            path.selector.expected_fill_color,
            selector.expected_fill_color,
        )
        || !optional_rgb_equivalent(
            path.selector.expected_stroke_color,
            selector.expected_stroke_color,
        )
    {
        return Err(PdfEditError::StaleVectorPath);
    }
    Ok((content, path))
}

fn replace_or_insert_vector_color(
    operations: &mut Vec<Operation>,
    start: usize,
    end: &mut usize,
    stroking: bool,
    color: PdfRgbColor,
) -> Result<(), PdfEditError> {
    let operator = if stroking { "RG" } else { "rg" };
    let color_operation = rgb_operation(operator, color)?;
    let family = if stroking {
        ["G", "RG", "K"]
    } else {
        ["g", "rg", "k"]
    };
    let path_start = (start + 1..*end)
        .find(|index| is_path_constructor(&operations[*index].operator))
        .ok_or(PdfEditError::VectorPathUnsupported)?;
    if let Some(index) = (start + 1..path_start)
        .rev()
        .find(|index| family.contains(&operations[*index].operator.as_str()))
    {
        operations[index] = color_operation;
    } else {
        operations.insert(path_start, color_operation);
        *end += 1;
    }
    Ok(())
}

fn operations_outside_range(
    operations: &[Operation],
    start: usize,
    end: Option<usize>,
) -> Vec<&Operation> {
    match end {
        Some(end) => operations[..start]
            .iter()
            .chain(operations[end + 1..].iter())
            .collect(),
        None => operations.iter().collect(),
    }
}

pub fn edit_vector_path(
    request: &EditVectorPathRequest,
) -> Result<EditVectorPathResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let (source_content, source_path) = selected_vector_path(&document, &request.selector)?;
    let source_operations = source_content.operations.clone();
    let mut result_content = source_content;
    let start = request.selector.start_operation_index;
    let mut result_end = request.selector.end_operation_index;
    let mutation = match &request.mutation {
        VectorPathMutation::Transform { destination_matrix } => {
            if !valid_vector_matrix(*destination_matrix)
                || matrix_equivalent(source_path.transform, *destination_matrix)
            {
                return Err(PdfEditError::InvalidVectorEdit);
            }
            if request.selector.expected_transform.is_some() {
                set_vector_matrix(
                    result_content
                        .operations
                        .get_mut(start + 1)
                        .ok_or(PdfEditError::StaleVectorPath)?,
                    *destination_matrix,
                )?;
            } else {
                result_content.operations.insert(
                    start + 1,
                    Operation::new(
                        "cm",
                        vec![
                            pdf_number_object(destination_matrix.a)?,
                            pdf_number_object(destination_matrix.b)?,
                            pdf_number_object(destination_matrix.c)?,
                            pdf_number_object(destination_matrix.d)?,
                            pdf_number_object(destination_matrix.e)?,
                            pdf_number_object(destination_matrix.f)?,
                        ],
                    ),
                );
                result_end += 1;
            }
            VectorPathMutationKind::Transform
        }
        VectorPathMutation::Recolor {
            destination_fill_color,
            destination_stroke_color,
        } => {
            if destination_fill_color.is_none() && destination_stroke_color.is_none() {
                return Err(PdfEditError::InvalidVectorEdit);
            }
            let mut changed = false;
            if let Some(color) = destination_fill_color {
                if !source_path.supports_fill || !valid_rgb_color(*color) {
                    return Err(PdfEditError::InvalidVectorEdit);
                }
                if !source_path
                    .fill_color
                    .is_some_and(|source| rgb_equivalent(source, *color))
                {
                    replace_or_insert_vector_color(
                        &mut result_content.operations,
                        start,
                        &mut result_end,
                        false,
                        *color,
                    )?;
                    changed = true;
                }
            }
            if let Some(color) = destination_stroke_color {
                if !source_path.supports_stroke || !valid_rgb_color(*color) {
                    return Err(PdfEditError::InvalidVectorEdit);
                }
                if !source_path
                    .stroke_color
                    .is_some_and(|source| rgb_equivalent(source, *color))
                {
                    replace_or_insert_vector_color(
                        &mut result_content.operations,
                        start,
                        &mut result_end,
                        true,
                        *color,
                    )?;
                    changed = true;
                }
            }
            if !changed {
                return Err(PdfEditError::InvalidVectorEdit);
            }
            VectorPathMutationKind::Recolor
        }
        VectorPathMutation::Delete => {
            result_content.operations.drain(start..=result_end);
            VectorPathMutationKind::Delete
        }
    };
    let encoded = result_content
        .encode()
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let target_id: ObjectId = request.selector.content_stream_object.into();
    let stream = document
        .get_object_mut(target_id)
        .and_then(Object::as_stream_mut)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    stream.set_plain_content(encoded);
    stream
        .compress()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects != vec![target_id] {
        return Err(PdfEditError::ValidationFailed);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let output_stream = output
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let output_bytes = output_stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let output_content =
        Content::decode(&output_bytes).map_err(|_| PdfEditError::ValidationFailed)?;
    let result_path = if mutation == VectorPathMutationKind::Delete {
        None
    } else {
        recognized_vector_path(
            request.selector.page_number,
            target_id,
            &output_content.operations,
            start,
            result_end,
        )
    };
    let requested_mutation_preserved = match (&request.mutation, result_path.as_ref()) {
        (VectorPathMutation::Transform { destination_matrix }, Some(path)) => {
            matrix_equivalent(path.transform, *destination_matrix)
        }
        (
            VectorPathMutation::Recolor {
                destination_fill_color,
                destination_stroke_color,
            },
            Some(path),
        ) => {
            destination_fill_color.is_none_or(|color| {
                path.fill_color
                    .is_some_and(|actual| rgb_equivalent(actual, color))
            }) && destination_stroke_color.is_none_or(|color| {
                path.stroke_color
                    .is_some_and(|actual| rgb_equivalent(actual, color))
            })
        }
        (VectorPathMutation::Delete, None) => true,
        _ => false,
    };
    let source_outside = operations_outside_range(
        &source_operations,
        request.selector.start_operation_index,
        Some(request.selector.end_operation_index),
    );
    let result_outside = operations_outside_range(
        &output_content.operations,
        start,
        (mutation != VectorPathMutationKind::Delete).then_some(result_end),
    );
    let non_target_operations_preserved = source_outside.len() == result_outside.len()
        && source_outside
            .iter()
            .zip(result_outside.iter())
            .all(|(source, result)| operations_equal(source, result));
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        *id == target_id
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = VectorPathValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_preserved: semantic_object_count(&output.objects)
            == semantic_object_count(&source_objects),
        non_target_objects_preserved,
        target_content_stream_preserved: output
            .get_page_contents(page_id(&output, request.selector.page_number)?)
            .contains(&target_id),
        requested_mutation_preserved,
        non_target_operations_preserved,
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        eprintln!("PDF_VECTOR_EDIT_VALIDATION_FAILED validation={validation:?}");
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditVectorPathResult {
        operation: "EDIT_VECTOR_PATH".to_string(),
        capability: "PDF_VECTOR_EDIT".to_string(),
        mutation,
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        content_stream_object: target_id.into(),
        page_number: request.selector.page_number,
        source_bounds: source_path.bounds,
        result_bounds: result_path.as_ref().map(|path| path.bounds),
        source_transform: source_path.transform,
        result_transform: result_path.as_ref().map(|path| path.transform),
        source_fill_color: source_path.fill_color,
        result_fill_color: result_path.as_ref().and_then(|path| path.fill_color),
        source_stroke_color: source_path.stroke_color,
        result_stroke_color: result_path.as_ref().and_then(|path| path.stroke_color),
        paint_operator: source_path.paint_operator,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn edit_link(request: &EditLinkRequest) -> Result<EditLinkResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let (
        mutation,
        selected_page_id,
        annotation_id,
        source_rectangle,
        source_target,
        requested_rectangle,
        requested_target,
        source_annotation_ids,
        requested_annotation_ids,
        collection_container,
        allowed_changed_ids,
    ) = match &request.mutation {
        LinkMutation::Create {
            page,
            rectangle,
            target,
        } => {
            if !valid_pdf_box(*rectangle) {
                return Err(PdfEditError::InvalidLinkEdit);
            }
            let target = validated_link_target(&document, target)?;
            let collection = selected_link_page(&document, page)?;
            let target_object = link_target_pdf_object(&document, &target)?;
            let mut annotation = Dictionary::new();
            annotation.set("Type", Object::Name(b"Annot".to_vec()));
            annotation.set("Subtype", Object::Name(b"Link".to_vec()));
            annotation.set("Rect", pdf_box_object(*rectangle)?);
            annotation.set("Border", Object::Array(vec![0.into(), 0.into(), 0.into()]));
            annotation.set("P", collection.page_id);
            set_link_target_object(&mut annotation, target_object);
            let annotation_id = document.add_object(annotation);
            let mut result_ids = collection.annotation_ids.clone();
            result_ids.push(annotation_id);
            set_page_annotation_ids(
                &mut document,
                collection.page_id,
                collection.container,
                &result_ids,
            )?;
            let changed_container_id = collection
                .container
                .object_id()
                .unwrap_or(collection.page_id);
            (
                LinkMutationKind::Create,
                collection.page_id,
                annotation_id,
                None,
                None,
                Some(*rectangle),
                Some(target),
                collection.annotation_ids,
                result_ids,
                collection.container,
                BTreeSet::from([changed_container_id, annotation_id]),
            )
        }
        LinkMutation::Update {
            selector,
            rectangle,
            target,
        } => {
            if !valid_pdf_box(*rectangle) {
                return Err(PdfEditError::InvalidLinkEdit);
            }
            let target = validated_link_target(&document, target)?;
            let (collection, source_link) = selected_link(&document, selector)?;
            if pdf_boxes_equal(source_link.rectangle, *rectangle) && source_link.target == target {
                return Err(PdfEditError::InvalidLinkEdit);
            }
            let target_object = link_target_pdf_object(&document, &target)?;
            let annotation_id: ObjectId = selector.annotation_object.into();
            let annotation = document
                .get_dictionary_mut(annotation_id)
                .map_err(|_| PdfEditError::LinkUnsupported)?;
            annotation.set("Rect", pdf_box_object(*rectangle)?);
            set_link_target_object(annotation, target_object);
            (
                LinkMutationKind::Update,
                collection.page_id,
                annotation_id,
                Some(source_link.rectangle),
                Some(source_link.target),
                Some(*rectangle),
                Some(target),
                collection.annotation_ids.clone(),
                collection.annotation_ids,
                collection.container,
                BTreeSet::from([annotation_id]),
            )
        }
        LinkMutation::Delete { selector } => {
            let (collection, source_link) = selected_link(&document, selector)?;
            let annotation_id: ObjectId = selector.annotation_object.into();
            let collection_object = collection
                .container
                .object_id()
                .unwrap_or(collection.page_id);
            if source_objects.iter().any(|(object_id, object)| {
                *object_id != collection_object
                    && *object_id != annotation_id
                    && object_references_target(object, annotation_id, 0)
            }) {
                return Err(PdfEditError::LinkUnsupported);
            }
            let mut result_ids = collection.annotation_ids.clone();
            result_ids.remove(selector.annotation_index);
            set_page_annotation_ids(
                &mut document,
                collection.page_id,
                collection.container,
                &result_ids,
            )?;
            if document.objects.remove(&annotation_id).is_none() {
                return Err(PdfEditError::LinkNotFound);
            }
            (
                LinkMutationKind::Delete,
                collection.page_id,
                annotation_id,
                Some(source_link.rectangle),
                Some(source_link.target),
                None,
                None,
                collection.annotation_ids,
                result_ids,
                collection.container,
                BTreeSet::from([collection_object, annotation_id]),
            )
        }
    };

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.iter().copied().collect::<BTreeSet<_>>() != allowed_changed_ids {
        return Err(PdfEditError::ValidationFailed);
    }
    save_new_pdf(&mut document, &request.output_path)?;

    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let output_collection = link_annotation_collection(&output, selected_page_id)?;
    let result_link = if mutation == LinkMutationKind::Delete {
        None
    } else {
        editable_link_from_id(
            &output,
            request_page_number(&request.mutation),
            selected_page_id,
            annotation_id,
            output_collection
                .annotation_ids
                .iter()
                .position(|candidate| *candidate == annotation_id)
                .ok_or(PdfEditError::ValidationFailed)?,
        )
        .ok()
    };
    let annotation_collection_preserved = output_collection.container == collection_container
        && output_collection.annotation_ids == requested_annotation_ids
        && source_annotation_ids.iter().all(|source_id| {
            *source_id == annotation_id
                || output.objects.get(source_id).is_some_and(|result| {
                    source_objects
                        .get(source_id)
                        .is_some_and(|source| pdf_objects_semantically_equal(source, result))
                })
        });
    let requested_mutation_preserved = match mutation {
        LinkMutationKind::Create | LinkMutationKind::Update => {
            result_link.as_ref().is_some_and(|link| {
                requested_rectangle
                    .is_some_and(|rectangle| pdf_boxes_equal(link.rectangle, rectangle))
                    && requested_target
                        .as_ref()
                        .is_some_and(|target| &link.target == target)
            })
        }
        LinkMutationKind::Delete => {
            result_link.is_none()
                && !output_collection.annotation_ids.contains(&annotation_id)
                && !output.objects.contains_key(&annotation_id)
        }
    };
    let expected_object_delta = match mutation {
        LinkMutationKind::Create => 1_isize,
        LinkMutationKind::Update => 0,
        LinkMutationKind::Delete => -1,
    };
    let expected_link_delta = expected_object_delta;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_ids.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = LinkEditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: semantic_object_count(&output.objects) as isize
            - semantic_object_count(&source_objects) as isize
            == expected_object_delta,
        non_target_objects_preserved,
        annotation_collection_preserved,
        requested_mutation_preserved,
        link_count_delta_valid: output_graph.links as isize - source_graph.links as isize
            == expected_link_delta,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(EditLinkResult {
        operation: "EDIT_LINK".to_string(),
        capability: "PDF_LINK_EDIT".to_string(),
        mutation,
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        page_object: selected_page_id.into(),
        annotation_object: annotation_id.into(),
        source_rectangle,
        result_rectangle: result_link.as_ref().map(|link| link.rectangle),
        source_target,
        result_target: result_link.map(|link| link.target),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn annotation_page_number(mutation: &AnnotationMutation) -> u32 {
    match mutation {
        AnnotationMutation::Create { page, .. } => page.page_number,
        AnnotationMutation::Update { selector, .. } | AnnotationMutation::Delete { selector } => {
            selector.page_number
        }
    }
}

fn validated_annotation_page(
    document: &Document,
    selector: &AnnotationPageSelector,
) -> Result<LinkAnnotationCollection, PdfEditError> {
    let page_id =
        page_id(document, selector.page_number).map_err(|_| PdfEditError::AnnotationNotFound)?;
    if page_id != ObjectId::from(selector.page_object) {
        return Err(PdfEditError::StaleAnnotation);
    }
    let collection = link_annotation_collection(document, page_id)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if collection.container.object_id().map(Into::into) != selector.annotations_object
        || collection.sha256 != selector.expected_annotations_sha256
    {
        return Err(PdfEditError::StaleAnnotation);
    }
    Ok(collection)
}

fn valid_annotation_point(point: PdfPoint) -> bool {
    point.x.is_finite()
        && point.y.is_finite()
        && point.x.abs() <= MAX_PAGE_COORDINATE_POINTS
        && point.y.abs() <= MAX_PAGE_COORDINATE_POINTS
}

fn point_inside_box(point: PdfPoint, rectangle: PdfBox) -> bool {
    const EPSILON: f64 = 0.001;
    point.x >= rectangle.left - EPSILON
        && point.x <= rectangle.right + EPSILON
        && point.y >= rectangle.bottom - EPSILON
        && point.y <= rectangle.top + EPSILON
}

fn quad_points(quad: PdfQuadPoints) -> [PdfPoint; 4] {
    [
        PdfPoint {
            x: quad.x1,
            y: quad.y1,
        },
        PdfPoint {
            x: quad.x2,
            y: quad.y2,
        },
        PdfPoint {
            x: quad.x3,
            y: quad.y3,
        },
        PdfPoint {
            x: quad.x4,
            y: quad.y4,
        },
    ]
}

fn annotation_contents_valid(value: &str) -> bool {
    value.len() <= MAX_ANNOTATION_CONTENT_BYTES
        && value
            .chars()
            .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t'))
}

fn annotation_color_valid(value: PdfRgbColor) -> bool {
    [value.red, value.green, value.blue]
        .iter()
        .all(|component| component.is_finite() && (0.0..=1.0).contains(component))
}

fn annotation_spec_for_page(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    spec: &PdfAnnotationSpec,
) -> Result<PdfAnnotationSpec, PdfEditError> {
    if !valid_pdf_box(spec.rectangle)
        || !annotation_contents_valid(&spec.contents)
        || !annotation_color_valid(spec.color)
        || !spec.opacity.is_finite()
        || !(0.05..=1.0).contains(&spec.opacity)
    {
        return Err(PdfEditError::InvalidAnnotationEdit);
    }
    let media_box = page_geometry(document, page_number, page_id)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?
        .media_box;
    if spec.rectangle.left < media_box.left
        || spec.rectangle.bottom < media_box.bottom
        || spec.rectangle.right > media_box.right
        || spec.rectangle.top > media_box.top
    {
        return Err(PdfEditError::InvalidAnnotationEdit);
    }
    let quad_valid = !spec.quad_points.is_empty()
        && spec.quad_points.len() <= MAX_ANNOTATION_QUADS
        && spec.quad_points.iter().all(|quad| {
            quad_points(*quad).iter().all(|point| {
                valid_annotation_point(*point) && point_inside_box(*point, spec.rectangle)
            })
        });
    let ink_point_count = spec.ink_paths.iter().map(Vec::len).sum::<usize>();
    let ink_valid = !spec.ink_paths.is_empty()
        && spec.ink_paths.len() <= MAX_ANNOTATION_INK_PATHS
        && ink_point_count <= MAX_ANNOTATION_INK_POINTS
        && spec.ink_paths.iter().all(|path| {
            path.len() >= 2
                && path.iter().all(|point| {
                    valid_annotation_point(*point) && point_inside_box(*point, spec.rectangle)
                })
        });
    let line_valid = spec
        .line_start
        .zip(spec.line_end)
        .is_some_and(|(start, end)| {
            valid_annotation_point(start)
                && valid_annotation_point(end)
                && point_inside_box(start, spec.rectangle)
                && point_inside_box(end, spec.rectangle)
                && ((start.x - end.x).abs() > 0.001 || (start.y - end.y).abs() > 0.001)
        });
    let extras_valid = match spec.kind {
        PdfAnnotationKind::Text | PdfAnnotationKind::Rectangle => {
            spec.quad_points.is_empty()
                && spec.ink_paths.is_empty()
                && spec.line_start.is_none()
                && spec.line_end.is_none()
                && spec.stamp_name.is_none()
        }
        PdfAnnotationKind::Highlight
        | PdfAnnotationKind::Underline
        | PdfAnnotationKind::Strikeout => {
            quad_valid
                && spec.ink_paths.is_empty()
                && spec.line_start.is_none()
                && spec.line_end.is_none()
                && spec.stamp_name.is_none()
        }
        PdfAnnotationKind::Ink => {
            ink_valid
                && spec.quad_points.is_empty()
                && spec.line_start.is_none()
                && spec.line_end.is_none()
                && spec.stamp_name.is_none()
        }
        PdfAnnotationKind::Arrow => {
            line_valid
                && spec.quad_points.is_empty()
                && spec.ink_paths.is_empty()
                && spec.stamp_name.is_none()
        }
        PdfAnnotationKind::Stamp => {
            spec.quad_points.is_empty()
                && spec.ink_paths.is_empty()
                && spec.line_start.is_none()
                && spec.line_end.is_none()
                && spec.stamp_name.as_deref().is_some_and(|name| {
                    matches!(name, "APPROVED" | "DRAFT" | "FINAL" | "CONFIDENTIAL")
                })
        }
    };
    if !extras_valid {
        return Err(PdfEditError::InvalidAnnotationEdit);
    }
    Ok(spec.clone())
}

fn annotation_number_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= 0.001
}

fn annotation_point_equal(left: PdfPoint, right: PdfPoint) -> bool {
    annotation_number_equal(left.x, right.x) && annotation_number_equal(left.y, right.y)
}

fn annotation_quad_equal(left: PdfQuadPoints, right: PdfQuadPoints) -> bool {
    quad_points(left)
        .into_iter()
        .zip(quad_points(right))
        .all(|(left, right)| annotation_point_equal(left, right))
}

fn annotation_specs_equal(left: &PdfAnnotationSpec, right: &PdfAnnotationSpec) -> bool {
    left.kind == right.kind
        && pdf_boxes_equal(left.rectangle, right.rectangle)
        && left.contents == right.contents
        && annotation_number_equal(left.color.red, right.color.red)
        && annotation_number_equal(left.color.green, right.color.green)
        && annotation_number_equal(left.color.blue, right.color.blue)
        && annotation_number_equal(left.opacity, right.opacity)
        && left.quad_points.len() == right.quad_points.len()
        && left
            .quad_points
            .iter()
            .copied()
            .zip(right.quad_points.iter().copied())
            .all(|(left, right)| annotation_quad_equal(left, right))
        && left.ink_paths.len() == right.ink_paths.len()
        && left
            .ink_paths
            .iter()
            .zip(&right.ink_paths)
            .all(|(left, right)| {
                left.len() == right.len()
                    && left
                        .iter()
                        .copied()
                        .zip(right.iter().copied())
                        .all(|(left, right)| annotation_point_equal(left, right))
            })
        && left
            .line_start
            .zip(right.line_start)
            .is_none_or(|(left, right)| annotation_point_equal(left, right))
        && left.line_start.is_some() == right.line_start.is_some()
        && left
            .line_end
            .zip(right.line_end)
            .is_none_or(|(left, right)| annotation_point_equal(left, right))
        && left.line_end.is_some() == right.line_end.is_some()
        && left.stamp_name == right.stamp_name
}

fn annotation_subtype(kind: PdfAnnotationKind) -> &'static [u8] {
    match kind {
        PdfAnnotationKind::Text => b"Text",
        PdfAnnotationKind::Highlight => b"Highlight",
        PdfAnnotationKind::Underline => b"Underline",
        PdfAnnotationKind::Strikeout => b"StrikeOut",
        PdfAnnotationKind::Ink => b"Ink",
        PdfAnnotationKind::Rectangle => b"Square",
        PdfAnnotationKind::Arrow => b"Line",
        PdfAnnotationKind::Stamp => b"Stamp",
    }
}

fn stamp_pdf_name(value: &str) -> Result<&'static [u8], PdfEditError> {
    match value {
        "APPROVED" => Ok(b"Approved"),
        "DRAFT" => Ok(b"Draft"),
        "FINAL" => Ok(b"Final"),
        "CONFIDENTIAL" => Ok(b"Confidential"),
        _ => Err(PdfEditError::InvalidAnnotationEdit),
    }
}

fn stamp_contract_name(value: &[u8]) -> Result<String, PdfEditError> {
    match value {
        b"Approved" => Ok("APPROVED".to_string()),
        b"Draft" => Ok("DRAFT".to_string()),
        b"Final" => Ok("FINAL".to_string()),
        b"Confidential" => Ok("CONFIDENTIAL".to_string()),
        _ => Err(PdfEditError::AnnotationUnsupported),
    }
}

fn annotation_color_object(color: PdfRgbColor) -> Result<Object, PdfEditError> {
    Ok(Object::Array(vec![
        pdf_number_object(color.red)?,
        pdf_number_object(color.green)?,
        pdf_number_object(color.blue)?,
    ]))
}

fn annotation_quad_object(quads: &[PdfQuadPoints]) -> Result<Object, PdfEditError> {
    let mut values = Vec::with_capacity(quads.len() * 8);
    for quad in quads {
        values.extend(
            [
                quad.x1, quad.y1, quad.x2, quad.y2, quad.x3, quad.y3, quad.x4, quad.y4,
            ]
            .into_iter()
            .map(pdf_number_object)
            .collect::<Result<Vec<_>, _>>()?,
        );
    }
    Ok(Object::Array(values))
}

fn annotation_ink_object(paths: &[Vec<PdfPoint>]) -> Result<Object, PdfEditError> {
    Ok(Object::Array(
        paths
            .iter()
            .map(|path| {
                Ok(Object::Array(
                    path.iter()
                        .flat_map(|point| [point.x, point.y])
                        .map(pdf_number_object)
                        .collect::<Result<Vec<_>, _>>()?,
                ))
            })
            .collect::<Result<Vec<_>, PdfEditError>>()?,
    ))
}

fn annotation_line_object(start: PdfPoint, end: PdfPoint) -> Result<Object, PdfEditError> {
    Ok(Object::Array(
        [start.x, start.y, end.x, end.y]
            .into_iter()
            .map(pdf_number_object)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn annotation_dictionary(
    page_id: ObjectId,
    annotation_id: ObjectId,
    appearance_id: ObjectId,
    spec: &PdfAnnotationSpec,
) -> Result<Dictionary, PdfEditError> {
    let mut dictionary = Dictionary::new();
    dictionary.set("Type", Object::Name(b"Annot".to_vec()));
    dictionary.set(
        "Subtype",
        Object::Name(annotation_subtype(spec.kind).to_vec()),
    );
    dictionary.set("Rect", pdf_box_object(spec.rectangle)?);
    dictionary.set("Contents", text_string(&spec.contents));
    dictionary.set("C", annotation_color_object(spec.color)?);
    dictionary.set("CA", pdf_number_object(spec.opacity)?);
    dictionary.set("F", Object::Integer(4));
    dictionary.set("P", page_id);
    dictionary.set(
        "NM",
        text_string(&format!("wf-annotation-{}", annotation_id.0)),
    );
    dictionary.set("WFNativeAnnotation", Object::Boolean(true));
    dictionary.set(
        "AP",
        Object::Dictionary(Dictionary::from_iter([(
            b"N".to_vec(),
            appearance_id.into(),
        )])),
    );
    match spec.kind {
        PdfAnnotationKind::Text => {
            dictionary.set("Name", Object::Name(b"Note".to_vec()));
            dictionary.set("Open", Object::Boolean(false));
        }
        PdfAnnotationKind::Highlight
        | PdfAnnotationKind::Underline
        | PdfAnnotationKind::Strikeout => {
            dictionary.set("QuadPoints", annotation_quad_object(&spec.quad_points)?);
        }
        PdfAnnotationKind::Ink => {
            dictionary.set("InkList", annotation_ink_object(&spec.ink_paths)?);
            dictionary.set(
                "BS",
                Object::Dictionary(Dictionary::from_iter([(
                    b"W".to_vec(),
                    pdf_number_object(1.5)?,
                )])),
            );
        }
        PdfAnnotationKind::Rectangle => {
            dictionary.set("IC", annotation_color_object(spec.color)?);
            dictionary.set(
                "BS",
                Object::Dictionary(Dictionary::from_iter([(
                    b"W".to_vec(),
                    pdf_number_object(1.5)?,
                )])),
            );
        }
        PdfAnnotationKind::Arrow => {
            dictionary.set(
                "L",
                annotation_line_object(spec.line_start.unwrap(), spec.line_end.unwrap())?,
            );
            dictionary.set(
                "LE",
                Object::Array(vec![
                    Object::Name(b"None".to_vec()),
                    Object::Name(b"OpenArrow".to_vec()),
                ]),
            );
            dictionary.set(
                "BS",
                Object::Dictionary(Dictionary::from_iter([(
                    b"W".to_vec(),
                    pdf_number_object(1.5)?,
                )])),
            );
        }
        PdfAnnotationKind::Stamp => {
            dictionary.set(
                "Name",
                Object::Name(stamp_pdf_name(spec.stamp_name.as_deref().unwrap())?.to_vec()),
            );
        }
    }
    Ok(dictionary)
}

fn local_point(point: PdfPoint, rectangle: PdfBox) -> PdfPoint {
    PdfPoint {
        x: point.x - rectangle.left,
        y: point.y - rectangle.bottom,
    }
}

fn annotation_appearance(spec: &PdfAnnotationSpec) -> Result<lopdf::Stream, PdfEditError> {
    let width = spec.rectangle.right - spec.rectangle.left;
    let height = spec.rectangle.top - spec.rectangle.bottom;
    let mut resources = Dictionary::new();
    let mut alpha_state = Dictionary::new();
    alpha_state.set("Type", Object::Name(b"ExtGState".to_vec()));
    alpha_state.set("ca", pdf_number_object(spec.opacity)?);
    alpha_state.set("CA", pdf_number_object(spec.opacity)?);
    let mut ext_gstate = Dictionary::new();
    ext_gstate.set("WFAlpha", Object::Dictionary(alpha_state));
    resources.set("ExtGState", Object::Dictionary(ext_gstate));
    let mut operations = vec![
        Operation::new("q", Vec::new()),
        Operation::new("gs", vec![Object::Name(b"WFAlpha".to_vec())]),
        Operation::new(
            "RG",
            vec![
                pdf_number_object(spec.color.red)?,
                pdf_number_object(spec.color.green)?,
                pdf_number_object(spec.color.blue)?,
            ],
        ),
        Operation::new(
            "rg",
            vec![
                pdf_number_object(spec.color.red)?,
                pdf_number_object(spec.color.green)?,
                pdf_number_object(spec.color.blue)?,
            ],
        ),
        Operation::new("w", vec![pdf_number_object(1.5)?]),
    ];
    match spec.kind {
        PdfAnnotationKind::Text => {
            operations.push(Operation::new(
                "re",
                vec![
                    pdf_number_object(0.75)?,
                    pdf_number_object(0.75)?,
                    pdf_number_object((width - 1.5).max(0.5))?,
                    pdf_number_object((height - 1.5).max(0.5))?,
                ],
            ));
            operations.push(Operation::new("B", Vec::new()));
            for y in [height * 0.7, height * 0.5, height * 0.3] {
                operations.push(Operation::new(
                    "m",
                    vec![pdf_number_object(width * 0.2)?, pdf_number_object(y)?],
                ));
                operations.push(Operation::new(
                    "l",
                    vec![pdf_number_object(width * 0.8)?, pdf_number_object(y)?],
                ));
                operations.push(Operation::new("S", Vec::new()));
            }
        }
        PdfAnnotationKind::Highlight => {
            for quad in &spec.quad_points {
                let points = quad_points(*quad).map(|point| local_point(point, spec.rectangle));
                operations.push(Operation::new(
                    "m",
                    vec![
                        pdf_number_object(points[0].x)?,
                        pdf_number_object(points[0].y)?,
                    ],
                ));
                for point in &points[1..] {
                    operations.push(Operation::new(
                        "l",
                        vec![pdf_number_object(point.x)?, pdf_number_object(point.y)?],
                    ));
                }
                operations.push(Operation::new("h", Vec::new()));
                operations.push(Operation::new("f", Vec::new()));
            }
        }
        PdfAnnotationKind::Underline | PdfAnnotationKind::Strikeout => {
            for quad in &spec.quad_points {
                let points = quad_points(*quad).map(|point| local_point(point, spec.rectangle));
                let left = points
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::INFINITY, f64::min);
                let right = points
                    .iter()
                    .map(|point| point.x)
                    .fold(f64::NEG_INFINITY, f64::max);
                let bottom = points
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::INFINITY, f64::min);
                let top = points
                    .iter()
                    .map(|point| point.y)
                    .fold(f64::NEG_INFINITY, f64::max);
                let y = if spec.kind == PdfAnnotationKind::Underline {
                    bottom + 1.0
                } else {
                    (bottom + top) / 2.0
                };
                operations.push(Operation::new(
                    "m",
                    vec![pdf_number_object(left)?, pdf_number_object(y)?],
                ));
                operations.push(Operation::new(
                    "l",
                    vec![pdf_number_object(right)?, pdf_number_object(y)?],
                ));
                operations.push(Operation::new("S", Vec::new()));
            }
        }
        PdfAnnotationKind::Ink => {
            for path in &spec.ink_paths {
                let first = local_point(path[0], spec.rectangle);
                operations.push(Operation::new(
                    "m",
                    vec![pdf_number_object(first.x)?, pdf_number_object(first.y)?],
                ));
                for point in &path[1..] {
                    let point = local_point(*point, spec.rectangle);
                    operations.push(Operation::new(
                        "l",
                        vec![pdf_number_object(point.x)?, pdf_number_object(point.y)?],
                    ));
                }
                operations.push(Operation::new("S", Vec::new()));
            }
        }
        PdfAnnotationKind::Rectangle | PdfAnnotationKind::Stamp => {
            operations.push(Operation::new(
                "re",
                vec![
                    pdf_number_object(0.75)?,
                    pdf_number_object(0.75)?,
                    pdf_number_object((width - 1.5).max(0.5))?,
                    pdf_number_object((height - 1.5).max(0.5))?,
                ],
            ));
            operations.push(Operation::new("S", Vec::new()));
            if spec.kind == PdfAnnotationKind::Stamp {
                operations.push(Operation::new(
                    "m",
                    vec![
                        pdf_number_object(width * 0.15)?,
                        pdf_number_object(height * 0.35)?,
                    ],
                ));
                operations.push(Operation::new(
                    "l",
                    vec![
                        pdf_number_object(width * 0.35)?,
                        pdf_number_object(height * 0.15)?,
                    ],
                ));
                operations.push(Operation::new(
                    "l",
                    vec![
                        pdf_number_object(width * 0.85)?,
                        pdf_number_object(height * 0.75)?,
                    ],
                ));
                operations.push(Operation::new("S", Vec::new()));
            }
        }
        PdfAnnotationKind::Arrow => {
            let start = local_point(spec.line_start.unwrap(), spec.rectangle);
            let end = local_point(spec.line_end.unwrap(), spec.rectangle);
            operations.push(Operation::new(
                "m",
                vec![pdf_number_object(start.x)?, pdf_number_object(start.y)?],
            ));
            operations.push(Operation::new(
                "l",
                vec![pdf_number_object(end.x)?, pdf_number_object(end.y)?],
            ));
            operations.push(Operation::new("S", Vec::new()));
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let length = (dx * dx + dy * dy).sqrt();
            let ux = dx / length;
            let uy = dy / length;
            let size = 8.0_f64.min(length / 3.0).max(2.0);
            let left = PdfPoint {
                x: end.x - ux * size - uy * size * 0.55,
                y: end.y - uy * size + ux * size * 0.55,
            };
            let right = PdfPoint {
                x: end.x - ux * size + uy * size * 0.55,
                y: end.y - uy * size - ux * size * 0.55,
            };
            operations.push(Operation::new(
                "m",
                vec![pdf_number_object(left.x)?, pdf_number_object(left.y)?],
            ));
            operations.push(Operation::new(
                "l",
                vec![pdf_number_object(end.x)?, pdf_number_object(end.y)?],
            ));
            operations.push(Operation::new(
                "l",
                vec![pdf_number_object(right.x)?, pdf_number_object(right.y)?],
            ));
            operations.push(Operation::new("S", Vec::new()));
        }
    }
    operations.push(Operation::new("Q", Vec::new()));
    let content = Content { operations }
        .encode()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    let mut appearance = lopdf::Stream::new(
        Dictionary::from_iter([
            (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Object::Integer(1)),
            (
                b"BBox".to_vec(),
                Object::Array(vec![
                    pdf_number_object(0.0)?,
                    pdf_number_object(0.0)?,
                    pdf_number_object(width)?,
                    pdf_number_object(height)?,
                ]),
            ),
            (b"Resources".to_vec(), Object::Dictionary(resources)),
            (
                b"WFNativeAnnotationAppearance".to_vec(),
                Object::Boolean(true),
            ),
        ]),
        content,
    );
    appearance
        .compress()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    Ok(appearance)
}

fn appearance_object_from_annotation(annotation: &Dictionary) -> Result<ObjectId, PdfEditError> {
    annotation
        .get(b"AP")
        .and_then(Object::as_dict)
        .and_then(|appearance| appearance.get(b"N"))
        .and_then(Object::as_reference)
        .map_err(|_| PdfEditError::AnnotationUnsupported)
}

fn parse_annotation_color(value: &Object) -> Result<PdfRgbColor, PdfEditError> {
    let values = value
        .as_array()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if values.len() != 3 {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    let color = PdfRgbColor {
        red: pdf_number(&values[0]).ok_or(PdfEditError::AnnotationUnsupported)?,
        green: pdf_number(&values[1]).ok_or(PdfEditError::AnnotationUnsupported)?,
        blue: pdf_number(&values[2]).ok_or(PdfEditError::AnnotationUnsupported)?,
    };
    annotation_color_valid(color)
        .then_some(color)
        .ok_or(PdfEditError::AnnotationUnsupported)
}

fn parse_annotation_quads(value: &Object) -> Result<Vec<PdfQuadPoints>, PdfEditError> {
    let values = value
        .as_array()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if values.is_empty() || values.len() % 8 != 0 || values.len() / 8 > MAX_ANNOTATION_QUADS {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    values
        .as_chunks::<8>()
        .0
        .iter()
        .map(|chunk| {
            let numbers = chunk
                .iter()
                .map(pdf_number)
                .collect::<Option<Vec<_>>>()
                .ok_or(PdfEditError::AnnotationUnsupported)?;
            Ok(PdfQuadPoints {
                x1: numbers[0],
                y1: numbers[1],
                x2: numbers[2],
                y2: numbers[3],
                x3: numbers[4],
                y3: numbers[5],
                x4: numbers[6],
                y4: numbers[7],
            })
        })
        .collect()
}

fn parse_annotation_ink(value: &Object) -> Result<Vec<Vec<PdfPoint>>, PdfEditError> {
    let paths = value
        .as_array()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if paths.is_empty() || paths.len() > MAX_ANNOTATION_INK_PATHS {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    let mut point_count = 0usize;
    let parsed = paths
        .iter()
        .map(|path| {
            let values = path
                .as_array()
                .map_err(|_| PdfEditError::AnnotationUnsupported)?;
            if values.len() < 4 || values.len() % 2 != 0 {
                return Err(PdfEditError::AnnotationUnsupported);
            }
            point_count += values.len() / 2;
            values
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| {
                    Ok(PdfPoint {
                        x: pdf_number(&pair[0]).ok_or(PdfEditError::AnnotationUnsupported)?,
                        y: pdf_number(&pair[1]).ok_or(PdfEditError::AnnotationUnsupported)?,
                    })
                })
                .collect::<Result<Vec<_>, PdfEditError>>()
        })
        .collect::<Result<Vec<_>, PdfEditError>>()?;
    if point_count > MAX_ANNOTATION_INK_POINTS {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    Ok(parsed)
}

fn parse_annotation_line(value: &Object) -> Result<(PdfPoint, PdfPoint), PdfEditError> {
    let values = value
        .as_array()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if values.len() != 4 {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    Ok((
        PdfPoint {
            x: pdf_number(&values[0]).ok_or(PdfEditError::AnnotationUnsupported)?,
            y: pdf_number(&values[1]).ok_or(PdfEditError::AnnotationUnsupported)?,
        },
        PdfPoint {
            x: pdf_number(&values[2]).ok_or(PdfEditError::AnnotationUnsupported)?,
            y: pdf_number(&values[3]).ok_or(PdfEditError::AnnotationUnsupported)?,
        },
    ))
}

fn appearance_stream_valid(
    document: &Document,
    appearance_id: ObjectId,
    rectangle: PdfBox,
) -> bool {
    let Ok(stream) = document
        .get_object(appearance_id)
        .and_then(Object::as_stream)
    else {
        return false;
    };
    if !matches!(
        stream.dict.get(b"WFNativeAnnotationAppearance"),
        Ok(Object::Boolean(true))
    ) || !stream
        .dict
        .get(b"Type")
        .and_then(Object::as_name)
        .is_ok_and(|name| name == b"XObject")
        || !stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|name| name == b"Form")
    {
        return false;
    }
    let expected = PdfBox {
        left: 0.0,
        bottom: 0.0,
        right: rectangle.right - rectangle.left,
        top: rectangle.top - rectangle.bottom,
    };
    let actual = stream
        .dict
        .get(b"BBox")
        .ok()
        .cloned()
        .and_then(|value| pdf_box_from_object(document, value).ok());
    if !actual.is_some_and(|value| pdf_boxes_equal(value, expected)) {
        return false;
    }
    stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .ok()
        .and_then(|bytes| Content::decode(&bytes).ok())
        .is_some_and(|content| {
            !content.operations.is_empty()
                && content.operations.len() <= MAX_CONTENT_OPERATIONS_PER_STREAM
        })
}

fn editable_annotation_from_id(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    annotation_id: ObjectId,
    annotation_index: usize,
) -> Result<EditableAnnotation, PdfEditError> {
    let object = document
        .get_object(annotation_id)
        .map_err(|_| PdfEditError::AnnotationNotFound)?;
    let annotation = object
        .as_dict()
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    if !matches!(
        annotation.get(b"WFNativeAnnotation"),
        Ok(Object::Boolean(true))
    ) || annotation.get(b"P").and_then(Object::as_reference).ok() != Some(page_id)
    {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    let kind = match annotation
        .get(b"Subtype")
        .and_then(Object::as_name)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?
    {
        b"Text" => PdfAnnotationKind::Text,
        b"Highlight" => PdfAnnotationKind::Highlight,
        b"Underline" => PdfAnnotationKind::Underline,
        b"StrikeOut" => PdfAnnotationKind::Strikeout,
        b"Ink" => PdfAnnotationKind::Ink,
        b"Square" => PdfAnnotationKind::Rectangle,
        b"Line" => PdfAnnotationKind::Arrow,
        b"Stamp" => PdfAnnotationKind::Stamp,
        _ => return Err(PdfEditError::AnnotationUnsupported),
    };
    let rectangle = annotation
        .get(b"Rect")
        .cloned()
        .map_err(|_| PdfEditError::AnnotationUnsupported)
        .and_then(|value| {
            pdf_box_from_object(document, value).map_err(|_| PdfEditError::AnnotationUnsupported)
        })?;
    let contents = annotation
        .get(b"Contents")
        .and_then(decode_text_string)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    let color = annotation
        .get(b"C")
        .map_err(|_| PdfEditError::AnnotationUnsupported)
        .and_then(parse_annotation_color)?;
    let opacity = annotation
        .get(b"CA")
        .ok()
        .and_then(pdf_number)
        .ok_or(PdfEditError::AnnotationUnsupported)?;
    let (quad_points, ink_paths, line_start, line_end, stamp_name) = match kind {
        PdfAnnotationKind::Highlight
        | PdfAnnotationKind::Underline
        | PdfAnnotationKind::Strikeout => (
            parse_annotation_quads(
                annotation
                    .get(b"QuadPoints")
                    .map_err(|_| PdfEditError::AnnotationUnsupported)?,
            )?,
            Vec::new(),
            None,
            None,
            None,
        ),
        PdfAnnotationKind::Ink => (
            Vec::new(),
            parse_annotation_ink(
                annotation
                    .get(b"InkList")
                    .map_err(|_| PdfEditError::AnnotationUnsupported)?,
            )?,
            None,
            None,
            None,
        ),
        PdfAnnotationKind::Arrow => {
            let (start, end) = parse_annotation_line(
                annotation
                    .get(b"L")
                    .map_err(|_| PdfEditError::AnnotationUnsupported)?,
            )?;
            (Vec::new(), Vec::new(), Some(start), Some(end), None)
        }
        PdfAnnotationKind::Stamp => (
            Vec::new(),
            Vec::new(),
            None,
            None,
            Some(stamp_contract_name(
                annotation
                    .get(b"Name")
                    .and_then(Object::as_name)
                    .map_err(|_| PdfEditError::AnnotationUnsupported)?,
            )?),
        ),
        PdfAnnotationKind::Text | PdfAnnotationKind::Rectangle => {
            (Vec::new(), Vec::new(), None, None, None)
        }
    };
    let spec = PdfAnnotationSpec {
        kind,
        rectangle,
        contents,
        color,
        opacity,
        quad_points,
        ink_paths,
        line_start,
        line_end,
        stamp_name,
    };
    annotation_spec_for_page(document, page_number, page_id, &spec)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    let appearance_id = appearance_object_from_annotation(annotation)?;
    if !appearance_stream_valid(document, appearance_id, rectangle) {
        return Err(PdfEditError::AnnotationUnsupported);
    }
    let appearance = document
        .get_object(appearance_id)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    Ok(EditableAnnotation {
        selector: AnnotationSelector {
            page_number,
            page_object: page_id.into(),
            annotation_object: annotation_id.into(),
            appearance_object: appearance_id.into(),
            annotation_index,
            expected_annotation_sha256: canonical_object_sha256(object)
                .ok_or(PdfEditError::AnnotationUnsupported)?,
            expected_appearance_sha256: canonical_object_sha256(appearance)
                .ok_or(PdfEditError::AnnotationUnsupported)?,
            expected_spec: spec.clone(),
        },
        spec,
    })
}

fn selected_annotation(
    document: &Document,
    selector: &AnnotationSelector,
) -> Result<(LinkAnnotationCollection, EditableAnnotation), PdfEditError> {
    let page_id =
        page_id(document, selector.page_number).map_err(|_| PdfEditError::AnnotationNotFound)?;
    if page_id != ObjectId::from(selector.page_object) {
        return Err(PdfEditError::StaleAnnotation);
    }
    let collection = link_annotation_collection(document, page_id)
        .map_err(|_| PdfEditError::AnnotationUnsupported)?;
    let annotation_id = collection
        .annotation_ids
        .get(selector.annotation_index)
        .copied()
        .ok_or(PdfEditError::AnnotationNotFound)?;
    if annotation_id != ObjectId::from(selector.annotation_object) {
        return Err(PdfEditError::StaleAnnotation);
    }
    let editable = editable_annotation_from_id(
        document,
        selector.page_number,
        page_id,
        annotation_id,
        selector.annotation_index,
    )?;
    if editable.selector.appearance_object != selector.appearance_object
        || editable.selector.expected_annotation_sha256 != selector.expected_annotation_sha256
        || editable.selector.expected_appearance_sha256 != selector.expected_appearance_sha256
        || !annotation_specs_equal(&editable.spec, &selector.expected_spec)
    {
        return Err(PdfEditError::StaleAnnotation);
    }
    Ok((collection, editable))
}

type AnnotationInspection = (
    Vec<EditableAnnotation>,
    bool,
    Vec<EditableAnnotationPage>,
    usize,
    usize,
);

fn inspect_annotations(document: &Document) -> Result<AnnotationInspection, PdfEditError> {
    let mut editable = Vec::new();
    let mut pages = Vec::new();
    let mut unsupported = 0usize;
    let mut unsupported_pages = 0usize;
    let mut truncated = false;
    for (page_number, page_id) in document.get_pages() {
        let collection = match link_annotation_collection(document, page_id) {
            Ok(collection) => collection,
            Err(_) => {
                unsupported_pages += 1;
                continue;
            }
        };
        let mut editable_on_page = 0usize;
        let mut collection_supported = true;
        for (index, annotation_id) in collection.annotation_ids.iter().copied().enumerate() {
            let annotation = match document.get_dictionary(annotation_id) {
                Ok(annotation) => annotation,
                Err(_) => {
                    collection_supported = false;
                    break;
                }
            };
            if annotation
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| matches!(name, b"Link" | b"Widget"))
            {
                continue;
            }
            match editable_annotation_from_id(document, page_number, page_id, annotation_id, index)
            {
                Ok(annotation) if editable.len() < MAX_EDITABLE_ANNOTATIONS => {
                    editable_on_page += 1;
                    editable.push(annotation);
                }
                Ok(_) => truncated = true,
                Err(PdfEditError::AnnotationUnsupported | PdfEditError::InvalidAnnotationEdit) => {
                    unsupported += 1
                }
                Err(error) => return Err(error),
            }
        }
        if collection_supported {
            pages.push(EditableAnnotationPage {
                selector: AnnotationPageSelector {
                    page_number,
                    page_object: page_id.into(),
                    annotations_object: collection.container.object_id().map(Into::into),
                    expected_annotations_sha256: collection.sha256,
                },
                annotation_count: collection.annotation_ids.len(),
                editable_annotation_count: editable_on_page,
            });
        } else {
            unsupported_pages += 1;
        }
    }
    Ok((editable, truncated, pages, unsupported, unsupported_pages))
}

fn annotation_count(document: &Document) -> Result<usize, PdfEditError> {
    inspect_annotations(document).map(|inspection| inspection.0.len())
}

pub fn edit_annotation(
    request: &EditAnnotationRequest,
) -> Result<EditAnnotationResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_annotation_count = annotation_count(&document)?;
    let source_image_count =
        object_graph_summary(&document, signature_summaries(&document).len()).images;
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let (
        mutation,
        page_id,
        annotation_id,
        appearance_id,
        source_spec,
        result_spec,
        source_annotation_ids,
        requested_annotation_ids,
        collection_container,
        allowed_changed_ids,
    ) = match &request.mutation {
        AnnotationMutation::Create { page, spec } => {
            let collection = validated_annotation_page(&document, page)?;
            let spec =
                annotation_spec_for_page(&document, page.page_number, collection.page_id, spec)?;
            let appearance_id = document.add_object(annotation_appearance(&spec)?);
            let annotation_id = document.new_object_id();
            let annotation =
                annotation_dictionary(collection.page_id, annotation_id, appearance_id, &spec)?;
            document
                .objects
                .insert(annotation_id, Object::Dictionary(annotation));
            let mut result_ids = collection.annotation_ids.clone();
            result_ids.push(annotation_id);
            set_page_annotation_ids(
                &mut document,
                collection.page_id,
                collection.container,
                &result_ids,
            )
            .map_err(|_| PdfEditError::AnnotationUnsupported)?;
            let container_id = collection
                .container
                .object_id()
                .unwrap_or(collection.page_id);
            let result_container = match collection.container {
                AnnotationContainer::Missing => AnnotationContainer::Direct,
                other => other,
            };
            (
                AnnotationMutationKind::Create,
                collection.page_id,
                annotation_id,
                appearance_id,
                None,
                Some(spec),
                collection.annotation_ids,
                result_ids,
                result_container,
                BTreeSet::from([container_id, annotation_id, appearance_id]),
            )
        }
        AnnotationMutation::Update { selector, spec } => {
            let (collection, source) = selected_annotation(&document, selector)?;
            let spec = annotation_spec_for_page(
                &document,
                selector.page_number,
                collection.page_id,
                spec,
            )?;
            if annotation_specs_equal(&source.spec, &spec) {
                return Err(PdfEditError::InvalidAnnotationEdit);
            }
            let annotation_id: ObjectId = selector.annotation_object.into();
            let appearance_id: ObjectId = selector.appearance_object.into();
            document
                .objects
                .insert(appearance_id, Object::Stream(annotation_appearance(&spec)?));
            document.objects.insert(
                annotation_id,
                Object::Dictionary(annotation_dictionary(
                    collection.page_id,
                    annotation_id,
                    appearance_id,
                    &spec,
                )?),
            );
            (
                AnnotationMutationKind::Update,
                collection.page_id,
                annotation_id,
                appearance_id,
                Some(source.spec),
                Some(spec),
                collection.annotation_ids.clone(),
                collection.annotation_ids,
                collection.container,
                BTreeSet::from([annotation_id, appearance_id]),
            )
        }
        AnnotationMutation::Delete { selector } => {
            let (collection, source) = selected_annotation(&document, selector)?;
            let annotation_id: ObjectId = selector.annotation_object.into();
            let appearance_id: ObjectId = selector.appearance_object.into();
            let container_id = collection
                .container
                .object_id()
                .unwrap_or(collection.page_id);
            if source_objects.iter().any(|(object_id, object)| {
                ![container_id, annotation_id, appearance_id].contains(object_id)
                    && (object_references_target(object, annotation_id, 0)
                        || object_references_target(object, appearance_id, 0))
            }) {
                return Err(PdfEditError::AnnotationUnsupported);
            }
            let mut result_ids = collection.annotation_ids.clone();
            result_ids.remove(selector.annotation_index);
            set_page_annotation_ids(
                &mut document,
                collection.page_id,
                collection.container,
                &result_ids,
            )
            .map_err(|_| PdfEditError::AnnotationUnsupported)?;
            if document.objects.remove(&annotation_id).is_none()
                || document.objects.remove(&appearance_id).is_none()
            {
                return Err(PdfEditError::AnnotationNotFound);
            }
            (
                AnnotationMutationKind::Delete,
                collection.page_id,
                annotation_id,
                appearance_id,
                Some(source.spec),
                None,
                collection.annotation_ids,
                result_ids,
                collection.container,
                BTreeSet::from([container_id, annotation_id, appearance_id]),
            )
        }
    };
    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    let changed_object_set = changed_objects.iter().copied().collect::<BTreeSet<_>>();
    let mutation_scope_valid = match mutation {
        AnnotationMutationKind::Update => {
            changed_object_set.contains(&annotation_id)
                && changed_object_set.is_subset(&allowed_changed_ids)
        }
        AnnotationMutationKind::Create | AnnotationMutationKind::Delete => {
            changed_object_set == allowed_changed_ids
        }
    };
    if !mutation_scope_valid {
        eprintln!(
            "PDF_ANNOTATION_EDIT_MUTATION_SCOPE_FAILED changed_objects={changed_objects:?} allowed_changed_ids={allowed_changed_ids:?}"
        );
        return Err(PdfEditError::ValidationFailed);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_collection =
        link_annotation_collection(&output, page_id).map_err(|_| PdfEditError::ValidationFailed)?;
    let result_annotation = if mutation == AnnotationMutationKind::Delete {
        None
    } else {
        editable_annotation_from_id(
            &output,
            annotation_page_number(&request.mutation),
            page_id,
            annotation_id,
            output_collection
                .annotation_ids
                .iter()
                .position(|candidate| *candidate == annotation_id)
                .ok_or(PdfEditError::ValidationFailed)?,
        )
        .ok()
    };
    let annotation_collection_preserved = output_collection.container == collection_container
        && output_collection.annotation_ids == requested_annotation_ids
        && source_annotation_ids.iter().all(|source_id| {
            *source_id == annotation_id
                || output.objects.get(source_id).is_some_and(|result| {
                    source_objects
                        .get(source_id)
                        .is_some_and(|source| pdf_objects_semantically_equal(source, result))
                })
        });
    let requested_mutation_preserved = match mutation {
        AnnotationMutationKind::Create | AnnotationMutationKind::Update => result_annotation
            .as_ref()
            .zip(result_spec.as_ref())
            .is_some_and(|(annotation, requested)| {
                annotation_specs_equal(&annotation.spec, requested)
            }),
        AnnotationMutationKind::Delete => {
            result_annotation.is_none()
                && !output_collection.annotation_ids.contains(&annotation_id)
                && !output.objects.contains_key(&annotation_id)
                && !output.objects.contains_key(&appearance_id)
        }
    };
    let expected_object_delta = match mutation {
        AnnotationMutationKind::Create => 2_isize,
        AnnotationMutationKind::Update => 0,
        AnnotationMutationKind::Delete => -2,
    };
    let expected_annotation_delta = match mutation {
        AnnotationMutationKind::Create => 1_isize,
        AnnotationMutationKind::Update => 0,
        AnnotationMutationKind::Delete => -1,
    };
    let output_annotation_count = annotation_count(&output)?;
    let output_image_count =
        object_graph_summary(&output, signature_summaries(&output).len()).images;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_ids.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = AnnotationEditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_object_count as isize
            == source_object_count as isize + expected_object_delta,
        non_target_objects_preserved,
        annotation_collection_preserved,
        requested_mutation_preserved,
        annotation_count_delta_valid: output_annotation_count as isize
            == source_annotation_count as isize + expected_annotation_delta,
        appearance_stream_valid: mutation == AnnotationMutationKind::Delete
            || appearance_stream_valid(
                &output,
                appearance_id,
                result_spec.as_ref().unwrap().rectangle,
            ),
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_image_count == source_image_count,
        no_page_rasterization: output_image_count == source_image_count,
    };
    if !validation.passed() {
        eprintln!("PDF_ANNOTATION_EDIT_VALIDATION_FAILED validation={validation:?}");
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditAnnotationResult {
        operation: "EDIT_ANNOTATION".to_string(),
        capability: "PDF_ANNOTATION_EDIT".to_string(),
        mutation,
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        page_object: page_id.into(),
        annotation_object: annotation_id.into(),
        appearance_object: appearance_id.into(),
        source_spec,
        result_spec,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn metadata_text(value: &str) -> Result<String, PdfEditError> {
    if value.len() > MAX_METADATA_TEXT_BYTES
        || value.chars().count() > MAX_METADATA_TEXT_CHARS
        || value.chars().any(|character| {
            character == '\0'
                || (character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
        })
    {
        return Err(PdfEditError::InvalidMetadataEdit);
    }
    Ok(value.to_string())
}

fn validate_metadata_spec(value: &PdfMetadataSpec) -> Result<(), PdfEditError> {
    for field in [&value.title, &value.author, &value.subject]
        .into_iter()
        .flatten()
    {
        metadata_text(field)?;
    }
    if value.keywords.len() > MAX_METADATA_KEYWORDS {
        return Err(PdfEditError::InvalidMetadataEdit);
    }
    let mut unique = BTreeSet::new();
    for keyword in &value.keywords {
        if keyword.is_empty()
            || keyword.len() > MAX_METADATA_KEYWORD_BYTES
            || keyword.chars().any(|character| {
                character == '\0'
                    || character == ','
                    || (character.is_control() && !matches!(character, '\t' | '\n' | '\r'))
            })
            || !unique.insert(keyword)
        {
            return Err(PdfEditError::InvalidMetadataEdit);
        }
    }
    Ok(())
}

fn metadata_field(dictionary: &Dictionary, key: &[u8]) -> Result<Option<String>, PdfEditError> {
    match dictionary.get(key) {
        Ok(value) => decode_text_string(value)
            .map(Some)
            .map_err(|_| PdfEditError::MetadataUnsupported),
        Err(_) => Ok(None),
    }
}

fn metadata_spec_from_dictionary(
    dictionary: Option<&Dictionary>,
) -> Result<PdfMetadataSpec, PdfEditError> {
    let Some(dictionary) = dictionary else {
        return Ok(PdfMetadataSpec {
            title: None,
            author: None,
            subject: None,
            keywords: Vec::new(),
        });
    };
    let keywords = metadata_field(dictionary, b"Keywords")?
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let result = PdfMetadataSpec {
        title: metadata_field(dictionary, b"Title")?,
        author: metadata_field(dictionary, b"Author")?,
        subject: metadata_field(dictionary, b"Subject")?,
        keywords,
    };
    validate_metadata_spec(&result).map_err(|_| PdfEditError::MetadataUnsupported)?;
    Ok(result)
}

fn inspect_metadata(document: &Document) -> (Option<EditableMetadata>, usize) {
    let result = (|| -> Result<EditableMetadata, PdfEditError> {
        let catalog_id = document
            .trailer
            .get(b"Root")
            .and_then(Object::as_reference)
            .map_err(|_| PdfEditError::MetadataUnsupported)?;
        let catalog = document
            .get_dictionary(catalog_id)
            .map_err(|_| PdfEditError::MetadataUnsupported)?;
        if !catalog.has_type(b"Catalog") {
            return Err(PdfEditError::MetadataUnsupported);
        }

        let (info_location, info_object, expected_info_sha256, info_dictionary) =
            match document.trailer.get(b"Info") {
                Err(_) => (PdfMetadataLocation::Absent, None, None, None),
                Ok(Object::Dictionary(dictionary)) => (
                    PdfMetadataLocation::TrailerDictionary,
                    None,
                    canonical_object_sha256(&Object::Dictionary(dictionary.clone())),
                    Some(dictionary),
                ),
                Ok(Object::Reference(object_id)) => {
                    let object = document
                        .get_object(*object_id)
                        .map_err(|_| PdfEditError::MetadataUnsupported)?;
                    let dictionary = object
                        .as_dict()
                        .map_err(|_| PdfEditError::MetadataUnsupported)?;
                    (
                        PdfMetadataLocation::IndirectObject,
                        Some((*object_id).into()),
                        canonical_object_sha256(object),
                        Some(dictionary),
                    )
                }
                Ok(_) => return Err(PdfEditError::MetadataUnsupported),
            };
        let values = metadata_spec_from_dictionary(info_dictionary)?;

        let (xmp_state, xmp_object, expected_xmp_sha256) = match catalog.get(b"Metadata") {
            Err(_) => (PdfXmpState::Absent, None, None),
            Ok(Object::Reference(object_id)) => {
                let object = document
                    .get_object(*object_id)
                    .map_err(|_| PdfEditError::MetadataUnsupported)?;
                let stream = object
                    .as_stream()
                    .map_err(|_| PdfEditError::MetadataUnsupported)?;
                let state = if stream
                    .dict
                    .get(b"WFNativeMetadata")
                    .and_then(Object::as_bool)
                    .is_ok_and(|value| value)
                {
                    PdfXmpState::WellfriendOwned
                } else {
                    PdfXmpState::ForeignReadOnly
                };
                (
                    state,
                    Some((*object_id).into()),
                    canonical_object_sha256(object),
                )
            }
            Ok(_) => return Err(PdfEditError::MetadataUnsupported),
        };

        Ok(EditableMetadata {
            selector: PdfMetadataSelector {
                catalog_object: catalog_id.into(),
                info_location,
                info_object,
                expected_info_sha256,
                xmp_state,
                xmp_object,
                expected_xmp_sha256,
            },
            values,
            can_sync_xmp: xmp_state != PdfXmpState::ForeignReadOnly,
        })
    })();
    match result {
        Ok(metadata) => (Some(metadata), 0),
        Err(_) => (None, 1),
    }
}

fn set_metadata_field(dictionary: &mut Dictionary, key: &[u8], value: &Option<String>) {
    if let Some(value) = value {
        dictionary.set(key, text_string(value));
    } else {
        dictionary.remove(key);
    }
}

fn apply_metadata_spec(dictionary: &mut Dictionary, value: &PdfMetadataSpec) {
    set_metadata_field(dictionary, b"Title", &value.title);
    set_metadata_field(dictionary, b"Author", &value.author);
    set_metadata_field(dictionary, b"Subject", &value.subject);
    if value.keywords.is_empty() {
        dictionary.remove(b"Keywords");
    } else {
        dictionary.set("Keywords", text_string(&value.keywords.join(", ")));
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn metadata_xmp_packet(value: &PdfMetadataSpec) -> Vec<u8> {
    let mut fields = String::new();
    if let Some(title) = &value.title {
        fields.push_str(&format!(
            "<dc:title><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:title>",
            xml_escape(title)
        ));
    }
    if let Some(author) = &value.author {
        fields.push_str(&format!(
            "<dc:creator><rdf:Seq><rdf:li>{}</rdf:li></rdf:Seq></dc:creator>",
            xml_escape(author)
        ));
    }
    if let Some(subject) = &value.subject {
        fields.push_str(&format!(
            "<dc:description><rdf:Alt><rdf:li xml:lang=\"x-default\">{}</rdf:li></rdf:Alt></dc:description>",
            xml_escape(subject)
        ));
    }
    if !value.keywords.is_empty() {
        fields.push_str(&format!(
            "<pdf:Keywords>{}</pdf:Keywords>",
            xml_escape(&value.keywords.join(", "))
        ));
    }
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdf=\"http://ns.adobe.com/pdf/1.3/\">{fields}</rdf:Description></rdf:RDF></x:xmpmeta><?xpacket end=\"w\"?>"
    )
    .into_bytes()
}

fn metadata_xmp_stream(value: &PdfMetadataSpec) -> lopdf::Stream {
    let mut dictionary = Dictionary::new();
    dictionary.set("Type", Object::Name(b"Metadata".to_vec()));
    dictionary.set("Subtype", Object::Name(b"XML".to_vec()));
    dictionary.set("WFNativeMetadata", Object::Boolean(true));
    lopdf::Stream::new(dictionary, metadata_xmp_packet(value))
}

fn owned_xmp_matches(document: &Document, object_id: ObjectId, value: &PdfMetadataSpec) -> bool {
    document
        .get_object(object_id)
        .and_then(Object::as_stream)
        .ok()
        .filter(|stream| {
            stream
                .dict
                .get(b"WFNativeMetadata")
                .and_then(Object::as_bool)
                .is_ok_and(|owned| owned)
                && stream
                    .dict
                    .get(b"Type")
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == b"Metadata")
                && stream
                    .dict
                    .get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == b"XML")
        })
        .and_then(|stream| {
            stream
                .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
                .ok()
        })
        .is_some_and(|bytes| bytes == metadata_xmp_packet(value))
}

pub fn edit_metadata(request: &EditMetadataRequest) -> Result<EditMetadataResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    validate_metadata_spec(&request.values)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source = inspect_metadata(&document)
        .0
        .ok_or(PdfEditError::MetadataUnsupported)?;
    if source.selector != request.selector {
        return Err(PdfEditError::StaleMetadata);
    }
    if request.xmp_mode == PdfXmpMode::SyncBasicFields
        && source.selector.xmp_state == PdfXmpState::ForeignReadOnly
    {
        return Err(PdfEditError::ForeignXmpReadOnly);
    }
    if request.values == source.values && request.xmp_mode == PdfXmpMode::Preserve {
        return Err(PdfEditError::InvalidMetadataEdit);
    }

    let mut info = match source.selector.info_location {
        PdfMetadataLocation::Absent => Dictionary::new(),
        PdfMetadataLocation::TrailerDictionary => document
            .trailer
            .get(b"Info")
            .and_then(Object::as_dict)
            .cloned()
            .map_err(|_| PdfEditError::MetadataUnsupported)?,
        PdfMetadataLocation::IndirectObject => document
            .get_dictionary(
                source
                    .selector
                    .info_object
                    .ok_or(PdfEditError::MetadataUnsupported)?
                    .into(),
            )
            .cloned()
            .map_err(|_| PdfEditError::MetadataUnsupported)?,
    };
    apply_metadata_spec(&mut info, &request.values);
    let (result_info_id, added_info_object) = match source.selector.info_location {
        PdfMetadataLocation::IndirectObject => {
            let object_id: ObjectId = source
                .selector
                .info_object
                .ok_or(PdfEditError::MetadataUnsupported)?
                .into();
            document.objects.insert(object_id, Object::Dictionary(info));
            (object_id, false)
        }
        PdfMetadataLocation::Absent | PdfMetadataLocation::TrailerDictionary => {
            let object_id = document.add_object(Object::Dictionary(info));
            document.trailer.set("Info", object_id);
            (object_id, true)
        }
    };

    let mut added_xmp_object = false;
    let mut catalog_changed = false;
    let result_xmp_id = match request.xmp_mode {
        PdfXmpMode::Preserve => source.selector.xmp_object.map(Into::into),
        PdfXmpMode::SyncBasicFields => match source.selector.xmp_state {
            PdfXmpState::Absent => {
                let object_id = document.add_object(metadata_xmp_stream(&request.values));
                let catalog_id: ObjectId = source.selector.catalog_object.into();
                document
                    .get_dictionary_mut(catalog_id)
                    .map_err(|_| PdfEditError::MetadataUnsupported)?
                    .set("Metadata", object_id);
                added_xmp_object = true;
                catalog_changed = true;
                Some(object_id)
            }
            PdfXmpState::WellfriendOwned => {
                let object_id: ObjectId = source
                    .selector
                    .xmp_object
                    .ok_or(PdfEditError::MetadataUnsupported)?
                    .into();
                document.objects.insert(
                    object_id,
                    Object::Stream(metadata_xmp_stream(&request.values)),
                );
                Some(object_id)
            }
            PdfXmpState::ForeignReadOnly => return Err(PdfEditError::ForeignXmpReadOnly),
        },
    };

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidMetadataEdit);
    }
    let mut allowed_changed_ids = BTreeSet::from([result_info_id]);
    if let Some(source_info_id) = source.selector.info_object {
        allowed_changed_ids.insert(source_info_id.into());
    }
    if let Some(xmp_id) = result_xmp_id {
        allowed_changed_ids.insert(xmp_id);
    }
    if let Some(source_xmp_id) = source.selector.xmp_object {
        allowed_changed_ids.insert(source_xmp_id.into());
    }
    if catalog_changed {
        allowed_changed_ids.insert(source.selector.catalog_object.into());
    }
    let changed_set = changed_objects.iter().copied().collect::<BTreeSet<_>>();
    if !changed_set.is_subset(&allowed_changed_ids) {
        return Err(PdfEditError::ValidationFailed);
    }

    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let result = inspect_metadata(&output)
        .0
        .ok_or(PdfEditError::ValidationFailed)?;
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let expected_added_objects = usize::from(added_info_object) + usize::from(added_xmp_object);
    let source_metadata_ids = [source.selector.info_object, source.selector.xmp_object]
        .into_iter()
        .flatten()
        .map(Into::into)
        .collect::<BTreeSet<ObjectId>>();
    let non_metadata_objects_preserved = source_objects.iter().all(|(id, object)| {
        source_metadata_ids.contains(id)
            || (catalog_changed && *id == ObjectId::from(source.selector.catalog_object))
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let xmp_policy_preserved = match request.xmp_mode {
        PdfXmpMode::Preserve => {
            result.selector.xmp_state == source.selector.xmp_state
                && result.selector.xmp_object == source.selector.xmp_object
                && result.selector.expected_xmp_sha256 == source.selector.expected_xmp_sha256
        }
        PdfXmpMode::SyncBasicFields => result.selector.xmp_state == PdfXmpState::WellfriendOwned,
    };
    let xmp_packet_valid = match request.xmp_mode {
        PdfXmpMode::Preserve => true,
        PdfXmpMode::SyncBasicFields => result_xmp_id
            .is_some_and(|object_id| owned_xmp_matches(&output, object_id, &request.values)),
    };
    let validation = MetadataEditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: semantic_object_count(&output.objects)
            == source_semantic_object_count + expected_added_objects,
        non_metadata_objects_preserved,
        requested_values_preserved: result.values == request.values,
        xmp_policy_preserved,
        xmp_packet_valid,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        eprintln!("PDF_METADATA_EDIT_VALIDATION_FAILED validation={validation:?}");
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditMetadataResult {
        operation: "EDIT_METADATA".to_string(),
        capability: "PDF_METADATA_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        source_selector: source.selector,
        result_selector: result.selector,
        source_values: source.values,
        result_values: result.values,
        xmp_mode: request.xmp_mode,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

#[derive(Debug, Clone)]
struct BookmarkNodeState {
    id: ObjectId,
    parent_id: ObjectId,
    title: String,
    target: PdfLinkTarget,
    children: Vec<ObjectId>,
    open: bool,
    depth: usize,
    sibling_index: usize,
}

#[derive(Debug, Clone)]
struct BookmarkTreeState {
    catalog_id: ObjectId,
    root_id: Option<ObjectId>,
    roots: Vec<ObjectId>,
    nodes: BTreeMap<ObjectId, BookmarkNodeState>,
    preorder: Vec<ObjectId>,
    sha256: String,
}

pub fn edit_bookmark(request: &EditBookmarkRequest) -> Result<EditBookmarkResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let source_state = bookmark_tree_state(&document)?;
    let mut state = source_state.clone();
    let source_outline_ids = bookmark_owned_object_ids(&source_state);
    let source_bookmark_count = source_state.nodes.len();
    let mut removed_ids = BTreeSet::new();
    let (
        mutation,
        bookmark_id,
        source_parent_id,
        result_parent_id,
        source_sibling_index,
        result_sibling_index,
        source_title,
        result_title,
        source_target,
        result_target,
        expected_object_delta,
        expected_bookmark_delta,
    ) = match &request.mutation {
        BookmarkMutation::Create {
            tree,
            parent,
            sibling_index,
            title,
            target,
        } => {
            validate_bookmark_tree_selector(&state, tree)?;
            let title = validated_bookmark_title(title)?;
            let target = validated_link_target(&document, target)?;
            let root_was_missing = state.root_id.is_none();
            if root_was_missing {
                let mut root = Dictionary::new();
                root.set("Type", Object::Name(b"Outlines".to_vec()));
                root.set("Count", 0);
                let root_id = document.add_object(root);
                document
                    .get_dictionary_mut(state.catalog_id)
                    .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?
                    .set("Outlines", root_id);
                state.root_id = Some(root_id);
            }
            let root_id = state.root_id.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
            let parent_id = match parent {
                Some(selector) => {
                    validate_bookmark_selector(&document, &state, selector)?;
                    ObjectId::from(selector.outline_object)
                }
                None => root_id,
            };
            let destination = bookmark_children(&state, parent_id)?;
            if *sibling_index > destination.len() {
                return Err(PdfEditError::InvalidBookmarkEdit);
            }
            let mut item = Dictionary::new();
            item.set("Title", text_string(&title));
            item.set("Parent", parent_id);
            set_link_target_object(&mut item, link_target_pdf_object(&document, &target)?);
            let bookmark_id = document.add_object(item);
            let node = BookmarkNodeState {
                id: bookmark_id,
                parent_id,
                title: title.clone(),
                target: target.clone(),
                children: Vec::new(),
                open: true,
                depth: 0,
                sibling_index: *sibling_index,
            };
            state.nodes.insert(bookmark_id, node);
            insert_bookmark_child(&mut state, parent_id, *sibling_index, bookmark_id)?;
            refresh_bookmark_positions(&mut state)?;
            (
                BookmarkMutationKind::Create,
                bookmark_id,
                None,
                bookmark_parent_result(&state, parent_id),
                None,
                Some(*sibling_index),
                None,
                Some(title),
                None,
                Some(target),
                if root_was_missing { 2 } else { 1 },
                1,
            )
        }
        BookmarkMutation::Update {
            selector,
            title,
            target,
        } => {
            validate_bookmark_selector(&document, &state, selector)?;
            let title = validated_bookmark_title(title)?;
            let target = validated_link_target(&document, target)?;
            let bookmark_id = ObjectId::from(selector.outline_object);
            let source = state
                .nodes
                .get(&bookmark_id)
                .cloned()
                .ok_or(PdfEditError::BookmarkNotFound)?;
            if source.title == title && source.target == target {
                return Err(PdfEditError::InvalidBookmarkEdit);
            }
            let target_object = link_target_pdf_object(&document, &target)?;
            let item = document
                .get_dictionary_mut(bookmark_id)
                .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
            item.set("Title", text_string(&title));
            set_link_target_object(item, target_object);
            let node = state
                .nodes
                .get_mut(&bookmark_id)
                .ok_or(PdfEditError::BookmarkNotFound)?;
            node.title = title.clone();
            node.target = target.clone();
            (
                BookmarkMutationKind::Update,
                bookmark_id,
                bookmark_parent_result(&state, source.parent_id),
                bookmark_parent_result(&state, source.parent_id),
                Some(source.sibling_index),
                Some(source.sibling_index),
                Some(source.title),
                Some(title),
                Some(source.target),
                Some(target),
                0,
                0,
            )
        }
        BookmarkMutation::Move {
            selector,
            destination_parent,
            destination_sibling_index,
        } => {
            validate_bookmark_selector(&document, &state, selector)?;
            let bookmark_id = ObjectId::from(selector.outline_object);
            let source = state
                .nodes
                .get(&bookmark_id)
                .cloned()
                .ok_or(PdfEditError::BookmarkNotFound)?;
            let root_id = state.root_id.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
            let destination_parent_id = match destination_parent {
                Some(parent_selector) => {
                    validate_bookmark_selector(&document, &state, parent_selector)?;
                    ObjectId::from(parent_selector.outline_object)
                }
                None => root_id,
            };
            if destination_parent_id == bookmark_id
                || bookmark_descendants(&state, bookmark_id)?.contains(&destination_parent_id)
            {
                return Err(PdfEditError::InvalidBookmarkEdit);
            }
            remove_bookmark_child(&mut state, source.parent_id, bookmark_id)?;
            let destination = bookmark_children(&state, destination_parent_id)?;
            if *destination_sibling_index > destination.len() {
                return Err(PdfEditError::InvalidBookmarkEdit);
            }
            insert_bookmark_child(
                &mut state,
                destination_parent_id,
                *destination_sibling_index,
                bookmark_id,
            )?;
            state
                .nodes
                .get_mut(&bookmark_id)
                .ok_or(PdfEditError::BookmarkNotFound)?
                .parent_id = destination_parent_id;
            refresh_bookmark_positions(&mut state)?;
            let result = state
                .nodes
                .get(&bookmark_id)
                .ok_or(PdfEditError::BookmarkNotFound)?;
            if source.parent_id == result.parent_id && source.sibling_index == result.sibling_index
            {
                return Err(PdfEditError::InvalidBookmarkEdit);
            }
            (
                BookmarkMutationKind::Move,
                bookmark_id,
                bookmark_parent_result(&source_state, source.parent_id),
                bookmark_parent_result(&state, result.parent_id),
                Some(source.sibling_index),
                Some(result.sibling_index),
                Some(source.title.clone()),
                Some(source.title),
                Some(source.target.clone()),
                Some(source.target),
                0,
                0,
            )
        }
        BookmarkMutation::Delete { selector } => {
            validate_bookmark_selector(&document, &state, selector)?;
            let bookmark_id = ObjectId::from(selector.outline_object);
            let source = state
                .nodes
                .get(&bookmark_id)
                .cloned()
                .ok_or(PdfEditError::BookmarkNotFound)?;
            removed_ids.insert(bookmark_id);
            removed_ids.extend(bookmark_descendants(&state, bookmark_id)?);
            if source_objects.iter().any(|(object_id, object)| {
                !source_outline_ids.contains(object_id)
                    && removed_ids
                        .iter()
                        .any(|removed| object_references_target(object, *removed, 0))
            }) {
                return Err(PdfEditError::BookmarkTreeUnsupported);
            }
            remove_bookmark_child(&mut state, source.parent_id, bookmark_id)?;
            for removed in &removed_ids {
                state.nodes.remove(removed);
                state.preorder.retain(|candidate| candidate != removed);
                if document.objects.remove(removed).is_none() {
                    return Err(PdfEditError::BookmarkNotFound);
                }
            }
            refresh_bookmark_positions(&mut state)?;
            (
                BookmarkMutationKind::Delete,
                bookmark_id,
                bookmark_parent_result(&source_state, source.parent_id),
                None,
                Some(source.sibling_index),
                None,
                Some(source.title),
                None,
                Some(source.target),
                None,
                -(removed_ids.len() as isize),
                -(removed_ids.len() as isize),
            )
        }
    };

    rewrite_bookmark_tree(&mut document, &state)?;
    let output_outline_ids = bookmark_owned_object_ids(&state);
    let allowed_changed_ids: BTreeSet<_> = source_outline_ids
        .union(&output_outline_ids)
        .copied()
        .chain(std::iter::once(state.catalog_id))
        .collect();
    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects
        .iter()
        .any(|object_id| !allowed_changed_ids.contains(object_id))
    {
        return Err(PdfEditError::ValidationFailed);
    }
    save_new_pdf(&mut document, &request.output_path)?;

    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let output_state = bookmark_tree_state(&output)?;
    let result_node = output_state.nodes.get(&bookmark_id);
    let requested_mutation_preserved = match mutation {
        BookmarkMutationKind::Delete => {
            result_node.is_none()
                && removed_ids
                    .iter()
                    .all(|removed| !output.objects.contains_key(removed))
        }
        BookmarkMutationKind::Create
        | BookmarkMutationKind::Update
        | BookmarkMutationKind::Move => result_node.is_some_and(|node| {
            result_title
                .as_ref()
                .is_some_and(|title| &node.title == title)
                && result_target
                    .as_ref()
                    .is_some_and(|target| &node.target == target)
                && result_parent_id == bookmark_parent_result(&output_state, node.parent_id)
                && result_sibling_index == Some(node.sibling_index)
        }),
    };
    let non_outline_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_ids.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let validation = BookmarkEditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: semantic_object_count(&output.objects) as isize
            - semantic_object_count(&source_objects) as isize
            == expected_object_delta,
        non_outline_objects_preserved,
        outline_tree_valid: output_state.root_id.is_some(),
        requested_mutation_preserved,
        bookmark_count_delta_valid: output_state.nodes.len() as isize
            - source_bookmark_count as isize
            == expected_bookmark_delta,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }

    Ok(EditBookmarkResult {
        operation: "EDIT_BOOKMARK".to_string(),
        capability: "PDF_BOOKMARK_EDIT".to_string(),
        mutation,
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        bookmark_object: bookmark_id.into(),
        source_parent_object: source_parent_id.map(Into::into),
        result_parent_object: result_parent_id.map(Into::into),
        source_sibling_index,
        result_sibling_index,
        source_title,
        result_title,
        source_target,
        result_target,
        removed_subtree_count: removed_ids.len(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn catalog_object_id(document: &Document) -> Result<ObjectId, PdfEditError> {
    let catalog_id = document
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
    let catalog = document
        .get_dictionary(catalog_id)
        .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
    if !catalog.has_type(b"Catalog") {
        return Err(PdfEditError::BookmarkTreeUnsupported);
    }
    Ok(catalog_id)
}

fn optional_reference(
    dictionary: &Dictionary,
    key: &[u8],
) -> Result<Option<ObjectId>, PdfEditError> {
    match dictionary.get(key) {
        Ok(value) => value
            .as_reference()
            .map(Some)
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported),
        Err(_) => Ok(None),
    }
}

fn bookmark_tree_state(document: &Document) -> Result<BookmarkTreeState, PdfEditError> {
    let catalog_id = catalog_object_id(document)?;
    let catalog = document
        .get_dictionary(catalog_id)
        .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
    let root_id = match catalog.get(b"Outlines") {
        Ok(value) => Some(
            value
                .as_reference()
                .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?,
        ),
        Err(_) => None,
    };
    let mut state = BookmarkTreeState {
        catalog_id,
        root_id,
        roots: Vec::new(),
        nodes: BTreeMap::new(),
        preorder: Vec::new(),
        sha256: String::new(),
    };
    if let Some(root_id) = root_id {
        let root = document
            .get_dictionary(root_id)
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        if root.get(b"Type").is_ok()
            && !root
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Outlines")
        {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        let mut visited = BTreeSet::new();
        state.roots = parse_bookmark_children(
            document,
            root_id,
            root,
            0,
            &mut visited,
            &mut state.nodes,
            &mut state.preorder,
        )?;
    }
    state.sha256 = bookmark_tree_sha256(document, &state)?;
    Ok(state)
}

fn parse_bookmark_children(
    document: &Document,
    parent_id: ObjectId,
    parent: &Dictionary,
    depth: usize,
    visited: &mut BTreeSet<ObjectId>,
    nodes: &mut BTreeMap<ObjectId, BookmarkNodeState>,
    preorder: &mut Vec<ObjectId>,
) -> Result<Vec<ObjectId>, PdfEditError> {
    if depth > MAX_PDF_OBJECT_NESTING {
        return Err(PdfEditError::BookmarkTreeUnsupported);
    }
    let first = optional_reference(parent, b"First")?;
    let last = optional_reference(parent, b"Last")?;
    match (first, last) {
        (None, None) => return Ok(Vec::new()),
        (Some(_), None) | (None, Some(_)) => {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        (Some(_), Some(_)) => {}
    }
    let first = first.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    let last = last.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    let mut result = Vec::new();
    let mut previous = None;
    let mut current = first;
    loop {
        if nodes.len() >= MAX_EDITABLE_BOOKMARKS || !visited.insert(current) {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        let item_object = document
            .get_object(current)
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        let item = item_object
            .as_dict()
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        if optional_reference(item, b"Parent")? != Some(parent_id)
            || optional_reference(item, b"Prev")? != previous
        {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        let next = optional_reference(item, b"Next")?;
        if (current == last && next.is_some()) || (current != last && next.is_none()) {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        let title = item
            .get(b"Title")
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)
            .and_then(|value| {
                decode_text_string(value).map_err(|_| PdfEditError::BookmarkTreeUnsupported)
            })?;
        validated_existing_bookmark_title(&title)?;
        let target = link_target_from_annotation(document, item)
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        let count = match item.get(b"Count") {
            Ok(value) => value
                .as_i64()
                .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?,
            Err(_) => 0,
        };
        let sibling_index = result.len();
        result.push(current);
        preorder.push(current);
        let children =
            parse_bookmark_children(document, current, item, depth + 1, visited, nodes, preorder)?;
        nodes.insert(
            current,
            BookmarkNodeState {
                id: current,
                parent_id,
                title,
                target,
                children,
                open: count >= 0,
                depth,
                sibling_index,
            },
        );
        if current == last {
            break;
        }
        previous = Some(current);
        current = next.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    }
    Ok(result)
}

fn bookmark_tree_sha256(
    document: &Document,
    state: &BookmarkTreeState,
) -> Result<String, PdfEditError> {
    let mut hasher = Sha256::new();
    hasher.update(b"wellfriend-pdf-bookmark-tree-v1\0");
    hasher.update(state.catalog_id.0.to_be_bytes());
    hasher.update(state.catalog_id.1.to_be_bytes());
    match state.root_id {
        Some(root_id) => {
            hasher.update(b"present\0");
            for object_id in std::iter::once(root_id).chain(state.preorder.iter().copied()) {
                hasher.update(object_id.0.to_be_bytes());
                hasher.update(object_id.1.to_be_bytes());
                let object = document
                    .get_object(object_id)
                    .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
                let object_hash =
                    canonical_object_sha256(object).ok_or(PdfEditError::BookmarkTreeUnsupported)?;
                hasher.update(object_hash.as_bytes());
            }
        }
        None => {
            hasher.update(b"missing\0");
            let catalog = document
                .get_object(state.catalog_id)
                .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
            let catalog_hash =
                canonical_object_sha256(catalog).ok_or(PdfEditError::BookmarkTreeUnsupported)?;
            hasher.update(catalog_hash.as_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn bookmark_tree_selector(state: &BookmarkTreeState) -> BookmarkTreeSelector {
    BookmarkTreeSelector {
        catalog_object: state.catalog_id.into(),
        outlines_object: state.root_id.map(Into::into),
        expected_tree_sha256: state.sha256.clone(),
    }
}

fn validate_bookmark_tree_selector(
    state: &BookmarkTreeState,
    selector: &BookmarkTreeSelector,
) -> Result<(), PdfEditError> {
    if state.catalog_id != ObjectId::from(selector.catalog_object)
        || state.root_id != selector.outlines_object.map(Into::into)
        || state.sha256 != selector.expected_tree_sha256
    {
        return Err(PdfEditError::StaleBookmark);
    }
    Ok(())
}

fn validate_bookmark_selector(
    document: &Document,
    state: &BookmarkTreeState,
    selector: &BookmarkSelector,
) -> Result<(), PdfEditError> {
    validate_bookmark_tree_selector(state, &selector.tree)?;
    let object_id = ObjectId::from(selector.outline_object);
    let node = state
        .nodes
        .get(&object_id)
        .ok_or(PdfEditError::BookmarkNotFound)?;
    let object = document
        .get_object(object_id)
        .map_err(|_| PdfEditError::BookmarkNotFound)?;
    let hash = canonical_object_sha256(object).ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    if node.parent_id != ObjectId::from(selector.parent_object)
        || node.sibling_index != selector.sibling_index
        || hash != selector.expected_item_sha256
        || node.title != selector.expected_title
        || node.target != selector.expected_target
    {
        return Err(PdfEditError::StaleBookmark);
    }
    Ok(())
}

fn validated_existing_bookmark_title(value: &str) -> Result<(), PdfEditError> {
    if value.is_empty()
        || value.len() > MAX_BOOKMARK_TITLE_BYTES
        || value.chars().count() > MAX_BOOKMARK_TITLE_CHARS
        || value.chars().any(|character| character == '\0')
    {
        return Err(PdfEditError::BookmarkTreeUnsupported);
    }
    Ok(())
}

fn validated_bookmark_title(value: &str) -> Result<String, PdfEditError> {
    if value.trim().is_empty()
        || value.trim() != value
        || value.len() > MAX_BOOKMARK_TITLE_BYTES
        || value.chars().count() > MAX_BOOKMARK_TITLE_CHARS
        || value.chars().any(char::is_control)
    {
        return Err(PdfEditError::InvalidBookmarkEdit);
    }
    Ok(value.to_string())
}

fn bookmark_owned_object_ids(state: &BookmarkTreeState) -> BTreeSet<ObjectId> {
    state
        .root_id
        .into_iter()
        .chain(state.nodes.keys().copied())
        .collect()
}

fn bookmark_children(
    state: &BookmarkTreeState,
    parent_id: ObjectId,
) -> Result<Vec<ObjectId>, PdfEditError> {
    if state.root_id == Some(parent_id) {
        return Ok(state.roots.clone());
    }
    state
        .nodes
        .get(&parent_id)
        .map(|node| node.children.clone())
        .ok_or(PdfEditError::BookmarkNotFound)
}

fn insert_bookmark_child(
    state: &mut BookmarkTreeState,
    parent_id: ObjectId,
    index: usize,
    child_id: ObjectId,
) -> Result<(), PdfEditError> {
    let children = if state.root_id == Some(parent_id) {
        &mut state.roots
    } else {
        &mut state
            .nodes
            .get_mut(&parent_id)
            .ok_or(PdfEditError::BookmarkNotFound)?
            .children
    };
    if index > children.len() || children.contains(&child_id) {
        return Err(PdfEditError::InvalidBookmarkEdit);
    }
    children.insert(index, child_id);
    Ok(())
}

fn remove_bookmark_child(
    state: &mut BookmarkTreeState,
    parent_id: ObjectId,
    child_id: ObjectId,
) -> Result<(), PdfEditError> {
    let children = if state.root_id == Some(parent_id) {
        &mut state.roots
    } else {
        &mut state
            .nodes
            .get_mut(&parent_id)
            .ok_or(PdfEditError::BookmarkNotFound)?
            .children
    };
    let index = children
        .iter()
        .position(|candidate| *candidate == child_id)
        .ok_or(PdfEditError::BookmarkNotFound)?;
    children.remove(index);
    Ok(())
}

fn bookmark_descendants(
    state: &BookmarkTreeState,
    root_id: ObjectId,
) -> Result<BTreeSet<ObjectId>, PdfEditError> {
    let mut descendants = BTreeSet::new();
    let mut pending = state
        .nodes
        .get(&root_id)
        .ok_or(PdfEditError::BookmarkNotFound)?
        .children
        .clone();
    while let Some(object_id) = pending.pop() {
        if !descendants.insert(object_id) {
            return Err(PdfEditError::BookmarkTreeUnsupported);
        }
        pending.extend(
            state
                .nodes
                .get(&object_id)
                .ok_or(PdfEditError::BookmarkTreeUnsupported)?
                .children
                .iter()
                .copied(),
        );
    }
    Ok(descendants)
}

fn bookmark_parent_result(state: &BookmarkTreeState, parent_id: ObjectId) -> Option<ObjectId> {
    (state.root_id != Some(parent_id)).then_some(parent_id)
}

fn refresh_bookmark_positions(state: &mut BookmarkTreeState) -> Result<(), PdfEditError> {
    fn visit(
        state: &mut BookmarkTreeState,
        children: &[ObjectId],
        parent_id: ObjectId,
        depth: usize,
        preorder: &mut Vec<ObjectId>,
    ) -> Result<(), PdfEditError> {
        for (sibling_index, object_id) in children.iter().copied().enumerate() {
            let child_ids = {
                let node = state
                    .nodes
                    .get_mut(&object_id)
                    .ok_or(PdfEditError::BookmarkTreeUnsupported)?;
                node.parent_id = parent_id;
                node.depth = depth;
                node.sibling_index = sibling_index;
                node.children.clone()
            };
            preorder.push(object_id);
            visit(state, &child_ids, object_id, depth + 1, preorder)?;
        }
        Ok(())
    }
    let root_id = state.root_id.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    let roots = state.roots.clone();
    let mut preorder = Vec::with_capacity(state.nodes.len());
    visit(state, &roots, root_id, 0, &mut preorder)?;
    if preorder.len() != state.nodes.len() {
        return Err(PdfEditError::BookmarkTreeUnsupported);
    }
    state.preorder = preorder;
    Ok(())
}

fn bookmark_descendant_count(
    state: &BookmarkTreeState,
    object_id: ObjectId,
) -> Result<usize, PdfEditError> {
    let node = state
        .nodes
        .get(&object_id)
        .ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    let mut count = node.children.len();
    for child_id in &node.children {
        count += bookmark_descendant_count(state, *child_id)?;
    }
    Ok(count)
}

fn rewrite_bookmark_tree(
    document: &mut Document,
    state: &BookmarkTreeState,
) -> Result<(), PdfEditError> {
    let root_id = state.root_id.ok_or(PdfEditError::BookmarkTreeUnsupported)?;
    set_outline_child_links(
        document,
        root_id,
        &state.roots,
        true,
        state.nodes.len(),
        true,
    )?;
    for object_id in &state.preorder {
        let node = state
            .nodes
            .get(object_id)
            .ok_or(PdfEditError::BookmarkTreeUnsupported)?;
        let descendant_count = bookmark_descendant_count(state, *object_id)?;
        set_outline_child_links(
            document,
            *object_id,
            &node.children,
            false,
            descendant_count,
            node.open,
        )?;
    }
    for (parent_id, children) in std::iter::once((root_id, state.roots.as_slice())).chain(
        state
            .preorder
            .iter()
            .map(|parent_id| (*parent_id, state.nodes[parent_id].children.as_slice())),
    ) {
        for (index, child_id) in children.iter().copied().enumerate() {
            let item = document
                .get_dictionary_mut(child_id)
                .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
            item.set("Parent", parent_id);
            item.remove(b"Prev");
            item.remove(b"Next");
            if let Some(previous) = index.checked_sub(1).and_then(|value| children.get(value)) {
                item.set("Prev", *previous);
            }
            if let Some(next) = children.get(index + 1) {
                item.set("Next", *next);
            }
        }
    }
    Ok(())
}

fn set_outline_child_links(
    document: &mut Document,
    parent_id: ObjectId,
    children: &[ObjectId],
    root: bool,
    descendant_count: usize,
    open: bool,
) -> Result<(), PdfEditError> {
    let parent = document
        .get_dictionary_mut(parent_id)
        .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
    parent.remove(b"First");
    parent.remove(b"Last");
    parent.remove(b"Count");
    if root {
        parent.set("Type", Object::Name(b"Outlines".to_vec()));
    }
    if let (Some(first), Some(last)) = (children.first(), children.last()) {
        parent.set("First", *first);
        parent.set("Last", *last);
    }
    if root || descendant_count > 0 {
        let count =
            i64::try_from(descendant_count).map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        parent.set("Count", if root || open { count } else { -count });
    }
    Ok(())
}

type BookmarkInspection = (
    Option<EditableBookmarkTree>,
    Vec<EditableBookmark>,
    bool,
    usize,
);

fn inspect_bookmarks(document: &Document) -> Result<BookmarkInspection, PdfEditError> {
    let state = match bookmark_tree_state(document) {
        Ok(state) => state,
        Err(PdfEditError::BookmarkTreeUnsupported) => return Ok((None, Vec::new(), false, 1)),
        Err(error) => return Err(error),
    };
    let tree_selector = bookmark_tree_selector(&state);
    let tree = EditableBookmarkTree {
        selector: tree_selector.clone(),
        bookmark_count: state.nodes.len(),
    };
    let mut editable = Vec::with_capacity(state.nodes.len());
    for object_id in &state.preorder {
        let node = state
            .nodes
            .get(object_id)
            .ok_or(PdfEditError::BookmarkTreeUnsupported)?;
        let object = document
            .get_object(*object_id)
            .map_err(|_| PdfEditError::BookmarkTreeUnsupported)?;
        let expected_item_sha256 =
            canonical_object_sha256(object).ok_or(PdfEditError::BookmarkTreeUnsupported)?;
        editable.push(EditableBookmark {
            selector: BookmarkSelector {
                tree: tree_selector.clone(),
                outline_object: node.id.into(),
                parent_object: node.parent_id.into(),
                sibling_index: node.sibling_index,
                expected_item_sha256,
                expected_title: node.title.clone(),
                expected_target: node.target.clone(),
            },
            title: node.title.clone(),
            target: node.target.clone(),
            depth: node.depth,
            sibling_index: node.sibling_index,
            child_count: node.children.len(),
        });
    }
    Ok((Some(tree), editable, false, 0))
}

#[derive(Debug, Clone)]
struct AcroFormState {
    catalog_id: ObjectId,
    acro_form_id: ObjectId,
    fields: Vec<ObjectId>,
    owned_ids: BTreeSet<ObjectId>,
    sha256: String,
}

fn form_catalog_object_id(document: &Document) -> Result<ObjectId, PdfEditError> {
    let catalog_id = document
        .trailer
        .get(b"Root")
        .and_then(Object::as_reference)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    let catalog = document
        .get_dictionary(catalog_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    if !catalog.has_type(b"Catalog") {
        return Err(PdfEditError::FormTreeUnsupported);
    }
    Ok(catalog_id)
}

fn acro_form_state(document: &Document) -> Result<Option<AcroFormState>, PdfEditError> {
    let catalog_id = form_catalog_object_id(document)?;
    let catalog = document
        .get_dictionary(catalog_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    let acro_form_id = match catalog.get(b"AcroForm") {
        Err(_) => return Ok(None),
        Ok(value) => value
            .as_reference()
            .map_err(|_| PdfEditError::FormTreeUnsupported)?,
    };
    let acro_form = document
        .get_dictionary(acro_form_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    let fields_value = acro_form
        .get(b"Fields")
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    let (fields_array_id, fields_array) = match fields_value {
        Object::Array(values) => (None, values),
        Object::Reference(id) => (
            Some(*id),
            document
                .get_object(*id)
                .and_then(Object::as_array)
                .map_err(|_| PdfEditError::FormTreeUnsupported)?,
        ),
        _ => return Err(PdfEditError::FormTreeUnsupported),
    };
    if fields_array.len() > MAX_EDITABLE_FORM_FIELDS {
        return Err(PdfEditError::FormTreeUnsupported);
    }
    let fields = fields_array
        .iter()
        .map(|value| {
            value
                .as_reference()
                .map_err(|_| PdfEditError::FormTreeUnsupported)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut owned_ids = BTreeSet::from([acro_form_id]);
    if let Some(fields_array_id) = fields_array_id {
        owned_ids.insert(fields_array_id);
    }
    let mut visiting = BTreeSet::new();
    for field_id in &fields {
        collect_form_field_objects(document, *field_id, &mut owned_ids, &mut visiting, 0)?;
    }
    let mut hasher = Sha256::new();
    hasher.update(catalog_id.0.to_be_bytes());
    hasher.update(catalog_id.1.to_be_bytes());
    for object_id in &owned_ids {
        hasher.update(object_id.0.to_be_bytes());
        hasher.update(object_id.1.to_be_bytes());
        let object = document
            .get_object(*object_id)
            .map_err(|_| PdfEditError::FormTreeUnsupported)?;
        let digest = canonical_object_sha256(object).ok_or(PdfEditError::FormTreeUnsupported)?;
        hasher.update(digest.as_bytes());
    }
    Ok(Some(AcroFormState {
        catalog_id,
        acro_form_id,
        fields,
        owned_ids,
        sha256: format!("{:x}", hasher.finalize()),
    }))
}

fn form_creation_selector(document: &Document) -> Option<FormCreationSelector> {
    let catalog_id = form_catalog_object_id(document).ok()?;
    match acro_form_state(document) {
        Ok(Some(state)) => Some(FormCreationSelector {
            catalog_object: catalog_id.into(),
            acro_form_object: Some(state.acro_form_id.into()),
            expected_form_sha256: Some(state.sha256),
        }),
        Ok(None) => Some(FormCreationSelector {
            catalog_object: catalog_id.into(),
            acro_form_object: None,
            expected_form_sha256: None,
        }),
        Err(_) => None,
    }
}

fn collect_form_field_objects(
    document: &Document,
    field_id: ObjectId,
    owned_ids: &mut BTreeSet<ObjectId>,
    visiting: &mut BTreeSet<ObjectId>,
    depth: usize,
) -> Result<(), PdfEditError> {
    if depth > MAX_PDF_OBJECT_NESTING || !visiting.insert(field_id) {
        return Err(PdfEditError::FormTreeUnsupported);
    }
    if !owned_ids.insert(field_id) {
        visiting.remove(&field_id);
        return Ok(());
    }
    let field = document
        .get_dictionary(field_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    if let Ok(kids) = field.get(b"Kids") {
        let kids = kids
            .as_array()
            .map_err(|_| PdfEditError::FormTreeUnsupported)?;
        if kids.len() > MAX_EDITABLE_FORM_FIELDS {
            return Err(PdfEditError::FormTreeUnsupported);
        }
        for kid in kids {
            let kid_id = kid
                .as_reference()
                .map_err(|_| PdfEditError::FormTreeUnsupported)?;
            collect_form_field_objects(document, kid_id, owned_ids, visiting, depth + 1)?;
        }
    }
    visiting.remove(&field_id);
    Ok(())
}

fn decoded_form_text(value: &Object) -> Result<String, PdfEditError> {
    match value {
        Object::String(_, _) => {
            decode_text_string(value).map_err(|_| PdfEditError::FormFieldUnsupported)
        }
        Object::Name(value) => {
            String::from_utf8(value.clone()).map_err(|_| PdfEditError::FormFieldUnsupported)
        }
        _ => Err(PdfEditError::FormFieldUnsupported),
    }
}

fn inherited_form_name(parent: &str, dictionary: &Dictionary) -> Result<String, PdfEditError> {
    let own = dictionary
        .get(b"T")
        .ok()
        .map(decoded_form_text)
        .transpose()?;
    let name = match (parent.is_empty(), own) {
        (true, Some(own)) => own,
        (false, Some(own)) => format!("{parent}.{own}"),
        (false, None) => parent.to_string(),
        (true, None) => String::new(),
    };
    if name.len() > MAX_FORM_FIELD_NAME_BYTES || name.chars().any(char::is_control) {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    Ok(name)
}

fn form_field_flags(dictionary: &Dictionary, inherited: u32) -> Result<u32, PdfEditError> {
    match dictionary.get(b"Ff") {
        Ok(value) => value
            .as_i64()
            .ok()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or(PdfEditError::FormFieldUnsupported),
        Err(_) => Ok(inherited),
    }
}

fn form_field_kind(field_type: &[u8], flags: u32) -> Result<PdfFormFieldKind, PdfEditError> {
    match field_type {
        b"Tx" => Ok(PdfFormFieldKind::Text),
        b"Btn" if flags & (1 << 16) != 0 => Err(PdfEditError::FormFieldUnsupported),
        b"Btn" if flags & (1 << 15) != 0 => Ok(PdfFormFieldKind::Radio),
        b"Btn" => Ok(PdfFormFieldKind::Checkbox),
        b"Ch" if flags & (1 << 17) != 0 => Ok(PdfFormFieldKind::Dropdown),
        b"Ch" => Err(PdfEditError::FormFieldUnsupported),
        b"Sig" => Ok(PdfFormFieldKind::Signature),
        _ => Err(PdfEditError::FormFieldUnsupported),
    }
}

fn form_widget_page(
    document: &Document,
    widget_id: ObjectId,
) -> Result<(u32, ObjectId), PdfEditError> {
    if let Ok(page_id) = document
        .get_dictionary(widget_id)
        .and_then(|widget| widget.get(b"P"))
        .and_then(Object::as_reference)
        && let Some((page_number, _)) = document
            .get_pages()
            .into_iter()
            .find(|(_, id)| *id == page_id)
    {
        return Ok((page_number, page_id));
    }
    for (page_number, page_id) in document.get_pages() {
        let collection = link_annotation_collection(document, page_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        if collection.annotation_ids.contains(&widget_id) {
            return Ok((page_number, page_id));
        }
    }
    Err(PdfEditError::FormFieldUnsupported)
}

fn form_field_options(
    document: &Document,
    field: &Dictionary,
    kind: PdfFormFieldKind,
    widgets: &[ObjectId],
) -> Result<Vec<String>, PdfEditError> {
    match kind {
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            let mut values = BTreeSet::new();
            for widget_id in widgets {
                let widget = document
                    .get_dictionary(*widget_id)
                    .map_err(|_| PdfEditError::FormFieldUnsupported)?;
                let normal = widget
                    .get(b"AP")
                    .and_then(Object::as_dict)
                    .and_then(|appearance| appearance.get(b"N"))
                    .and_then(Object::as_dict)
                    .map_err(|_| PdfEditError::FormFieldUnsupported)?;
                for (name, appearance) in normal.iter() {
                    if name.as_slice() != b"Off" && appearance.as_reference().is_ok() {
                        values.insert(
                            String::from_utf8(name.clone())
                                .map_err(|_| PdfEditError::FormFieldUnsupported)?,
                        );
                    }
                }
            }
            if values.len() > MAX_FORM_FIELD_OPTIONS {
                return Err(PdfEditError::FormFieldUnsupported);
            }
            Ok(values.into_iter().collect())
        }
        PdfFormFieldKind::Dropdown => {
            let options = field
                .get(b"Opt")
                .and_then(Object::as_array)
                .map_err(|_| PdfEditError::FormFieldUnsupported)?;
            if options.len() > MAX_FORM_FIELD_OPTIONS {
                return Err(PdfEditError::FormFieldUnsupported);
            }
            let mut values = Vec::with_capacity(options.len());
            let mut unique = BTreeSet::new();
            for option in options {
                let display = match option {
                    Object::Array(pair) if pair.len() == 2 => &pair[0],
                    value => value,
                };
                let value = decoded_form_text(display)?;
                if !unique.insert(value.clone()) {
                    return Err(PdfEditError::FormFieldUnsupported);
                }
                values.push(value);
            }
            Ok(values)
        }
        PdfFormFieldKind::Text | PdfFormFieldKind::Signature => Ok(Vec::new()),
    }
}

fn form_field_options_are_editable(field: &Dictionary, kind: PdfFormFieldKind) -> bool {
    if kind != PdfFormFieldKind::Dropdown {
        return false;
    }
    field
        .get(b"Opt")
        .and_then(Object::as_array)
        .is_ok_and(|options| {
            !options.is_empty()
                && options.len() <= MAX_FORM_FIELD_OPTIONS
                && options
                    .iter()
                    .all(|option| matches!(option, Object::String(_, _)))
        })
}

fn current_form_field_value(field: &Dictionary) -> Result<Option<String>, PdfEditError> {
    field.get(b"V").ok().map(decoded_form_text).transpose()
}

fn form_field_state_sha256(
    document: &Document,
    field_id: ObjectId,
    widgets: &[ObjectId],
) -> Result<String, PdfEditError> {
    let mut hasher = Sha256::new();
    for object_id in std::iter::once(field_id).chain(widgets.iter().copied()) {
        hasher.update(object_id.0.to_be_bytes());
        hasher.update(object_id.1.to_be_bytes());
        let object = document
            .get_object(object_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        let digest = canonical_object_sha256(object).ok_or(PdfEditError::FormFieldUnsupported)?;
        hasher.update(digest.as_bytes());
    }
    Ok(format!("{:x}", hasher.finalize()))
}

struct TerminalFormFieldNode<'a> {
    field_id: ObjectId,
    field: &'a Dictionary,
    name: String,
    field_type: &'a [u8],
    flags: u32,
    widgets: Vec<ObjectId>,
}

fn terminal_form_field(
    document: &Document,
    state: &AcroFormState,
    node: TerminalFormFieldNode<'_>,
) -> Result<EditableFormField, PdfEditError> {
    let TerminalFormFieldNode {
        field_id,
        field,
        name,
        field_type,
        flags,
        widgets,
    } = node;
    if name.is_empty() || widgets.is_empty() || widgets.len() > MAX_EDITABLE_FORM_FIELDS {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    let kind = form_field_kind(field_type, flags)?;
    if matches!(kind, PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown) && widgets.len() != 1 {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    let mut first_page = None;
    let mut first_rectangle = None;
    let mut editable_widgets = Vec::with_capacity(widgets.len());
    for widget_id in &widgets {
        let widget = document
            .get_dictionary(*widget_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        if !widget
            .get(b"Subtype")
            .and_then(Object::as_name)
            .is_ok_and(|name| name == b"Widget")
        {
            return Err(PdfEditError::FormFieldUnsupported);
        }
        let rectangle = widget
            .get(b"Rect")
            .cloned()
            .map_err(|_| PdfEditError::FormFieldUnsupported)
            .and_then(|value| pdf_box_from_object(document, value))?;
        if !valid_pdf_box(rectangle) {
            return Err(PdfEditError::FormFieldUnsupported);
        }
        let (page_number, page_id) = form_widget_page(document, *widget_id)?;
        first_page.get_or_insert(page_number);
        first_rectangle.get_or_insert(rectangle);
        editable_widgets.push(EditableFormWidget {
            selector: FormWidgetSelector {
                widget_object: (*widget_id).into(),
                expected_page_number: page_number,
                expected_page_object: page_id.into(),
                expected_rectangle: rectangle,
            },
            rectangle,
        });
    }
    let options = form_field_options(document, field, kind, &widgets)?;
    let options_editable = form_field_options_are_editable(field, kind);
    let value = current_form_field_value(field)?;
    let read_only = flags & 1 != 0;
    let value_editable = !read_only
        && kind != PdfFormFieldKind::Signature
        && (!matches!(kind, PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio)
            || !options.is_empty());
    let label = field.get(b"TU").ok().map(decoded_form_text).transpose()?;
    let mut result = EditableFormField {
        selector: FormFieldSelector {
            form: AcroFormSelector {
                catalog_object: state.catalog_id.into(),
                acro_form_object: state.acro_form_id.into(),
                expected_form_sha256: state.sha256.clone(),
            },
            field_object: field_id.into(),
            expected_field_sha256: form_field_state_sha256(document, field_id, &widgets)?,
            expected_name: name.clone(),
            expected_kind: kind,
            expected_value: value.clone(),
        },
        name,
        label,
        kind,
        value,
        options,
        options_editable,
        appearance_valid: false,
        appearance_repairable: false,
        required: flags & (1 << 1) != 0,
        read_only,
        value_editable,
        page_number: first_page.ok_or(PdfEditError::FormFieldUnsupported)?,
        rectangle: first_rectangle.ok_or(PdfEditError::FormFieldUnsupported)?,
        widget_objects: widgets.into_iter().map(Into::into).collect(),
        widgets: editable_widgets,
    };
    result.appearance_valid = form_field_appearance_valid(document, &result);
    result.appearance_repairable = form_field_appearance_repairable(document, &result);
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
fn inspect_form_field_node(
    document: &Document,
    state: &AcroFormState,
    field_id: ObjectId,
    parent_name: &str,
    inherited_type: Option<Vec<u8>>,
    inherited_flags: u32,
    visited: &mut BTreeSet<ObjectId>,
    editable: &mut Vec<EditableFormField>,
    unsupported: &mut usize,
    truncated: &mut bool,
    depth: usize,
) -> Result<(), PdfEditError> {
    if depth > MAX_PDF_OBJECT_NESTING || !visited.insert(field_id) {
        return Err(PdfEditError::FormTreeUnsupported);
    }
    let field = document
        .get_dictionary(field_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?;
    let name = match inherited_form_name(parent_name, field) {
        Ok(name) => name,
        Err(PdfEditError::FormFieldUnsupported) => {
            *unsupported += 1;
            visited.remove(&field_id);
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let field_type = field
        .get(b"FT")
        .and_then(Object::as_name)
        .map(ToOwned::to_owned)
        .ok()
        .or(inherited_type);
    let flags = form_field_flags(field, inherited_flags)?;
    let is_widget = field
        .get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|value| value == b"Widget");
    let kids = field
        .get(b"Kids")
        .ok()
        .map(|value| {
            value
                .as_array()
                .map_err(|_| PdfEditError::FormTreeUnsupported)?
                .iter()
                .map(|kid| {
                    kid.as_reference()
                        .map_err(|_| PdfEditError::FormTreeUnsupported)
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let widget_kids = kids
        .iter()
        .copied()
        .filter(|kid_id| {
            document.get_dictionary(*kid_id).is_ok_and(|kid| {
                kid.get(b"Subtype")
                    .and_then(Object::as_name)
                    .is_ok_and(|value| value == b"Widget")
            })
        })
        .collect::<Vec<_>>();
    let result = if is_widget || (!kids.is_empty() && widget_kids.len() == kids.len()) {
        let widgets = if is_widget {
            vec![field_id]
        } else {
            widget_kids
        };
        match field_type.as_deref() {
            Some(field_type) => terminal_form_field(
                document,
                state,
                TerminalFormFieldNode {
                    field_id,
                    field,
                    name,
                    field_type,
                    flags,
                    widgets,
                },
            ),
            None => Err(PdfEditError::FormFieldUnsupported),
        }
        .map(|field| {
            if editable.len() < MAX_EDITABLE_FORM_FIELDS {
                editable.push(field);
            } else {
                *truncated = true;
            }
        })
    } else if kids.is_empty() || !widget_kids.is_empty() {
        Err(PdfEditError::FormFieldUnsupported)
    } else {
        for kid_id in kids {
            inspect_form_field_node(
                document,
                state,
                kid_id,
                &name,
                field_type.clone(),
                flags,
                visited,
                editable,
                unsupported,
                truncated,
                depth + 1,
            )?;
        }
        Ok(())
    };
    if matches!(result, Err(PdfEditError::FormFieldUnsupported)) {
        *unsupported += 1;
    } else {
        result?;
    }
    visited.remove(&field_id);
    Ok(())
}

type FormFieldInspection = (Vec<EditableFormField>, bool, usize);

fn inspect_form_fields(document: &Document) -> Result<FormFieldInspection, PdfEditError> {
    let state = match acro_form_state(document) {
        Ok(Some(state)) => state,
        Ok(None) => return Ok((Vec::new(), false, 0)),
        Err(PdfEditError::FormTreeUnsupported) => return Ok((Vec::new(), false, 1)),
        Err(error) => return Err(error),
    };
    let mut editable = Vec::new();
    let mut unsupported = 0;
    let mut truncated = false;
    let mut visited = BTreeSet::new();
    for field_id in &state.fields {
        inspect_form_field_node(
            document,
            &state,
            *field_id,
            "",
            None,
            0,
            &mut visited,
            &mut editable,
            &mut unsupported,
            &mut truncated,
            0,
        )?;
    }
    Ok((editable, truncated, unsupported))
}

fn validated_form_value(
    field: &EditableFormField,
    requested: &Option<String>,
) -> Result<Option<String>, PdfEditError> {
    if field.read_only || !field.value_editable || field.kind == PdfFormFieldKind::Signature {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    if field.value == *requested {
        return Err(PdfEditError::InvalidFormFieldValue);
    }
    match field.kind {
        PdfFormFieldKind::Text => {
            if let Some(value) = requested
                && (value.len() > MAX_FORM_FIELD_VALUE_BYTES
                    || value.chars().any(|character| character.is_control())
                    || !value.is_ascii())
            {
                return Err(PdfEditError::InvalidFormFieldValue);
            }
        }
        PdfFormFieldKind::Dropdown => {
            let value = requested
                .as_ref()
                .ok_or(PdfEditError::InvalidFormFieldValue)?;
            if !field.options.contains(value) || !value.is_ascii() {
                return Err(PdfEditError::InvalidFormFieldValue);
            }
        }
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            let value = requested
                .as_ref()
                .ok_or(PdfEditError::InvalidFormFieldValue)?;
            if value != "Off" && !field.options.contains(value) {
                return Err(PdfEditError::InvalidFormFieldValue);
            }
        }
        PdfFormFieldKind::Signature => return Err(PdfEditError::FormFieldUnsupported),
    }
    Ok(requested.clone())
}

fn create_form_text_appearance(
    document: &mut Document,
    rectangle: PdfBox,
    value: &str,
) -> Result<ObjectId, PdfEditError> {
    let width = rectangle.right - rectangle.left;
    let height = rectangle.top - rectangle.bottom;
    if width <= 4.0 || height <= 4.0 || !value.is_ascii() {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    let font_size = (height - 6.0).clamp(6.0, 12.0);
    let baseline = ((height - font_size) / 2.0).max(2.0);
    let font_id = document.add_object(Dictionary::from_iter([
        (b"Type".to_vec(), Object::Name(b"Font".to_vec())),
        (b"Subtype".to_vec(), Object::Name(b"Type1".to_vec())),
        (b"BaseFont".to_vec(), Object::Name(b"Helvetica".to_vec())),
        (
            b"Encoding".to_vec(),
            Object::Name(b"WinAnsiEncoding".to_vec()),
        ),
    ]));
    let operations = vec![
        Operation::new("q", Vec::new()),
        Operation::new(
            "rg",
            vec![
                pdf_number_object(1.0)?,
                pdf_number_object(1.0)?,
                pdf_number_object(1.0)?,
            ],
        ),
        Operation::new(
            "re",
            vec![
                pdf_number_object(0.0)?,
                pdf_number_object(0.0)?,
                pdf_number_object(width)?,
                pdf_number_object(height)?,
            ],
        ),
        Operation::new("f", Vec::new()),
        Operation::new("G", vec![pdf_number_object(0.25)?]),
        Operation::new("w", vec![pdf_number_object(0.75)?]),
        Operation::new(
            "re",
            vec![
                pdf_number_object(0.5)?,
                pdf_number_object(0.5)?,
                pdf_number_object(width - 1.0)?,
                pdf_number_object(height - 1.0)?,
            ],
        ),
        Operation::new("S", Vec::new()),
        Operation::new("BT", Vec::new()),
        Operation::new(
            "Tf",
            vec![
                Object::Name(b"Helv".to_vec()),
                pdf_number_object(font_size)?,
            ],
        ),
        Operation::new("g", vec![pdf_number_object(0.0)?]),
        Operation::new(
            "Td",
            vec![pdf_number_object(2.0)?, pdf_number_object(baseline)?],
        ),
        Operation::new(
            "Tj",
            vec![Object::string_literal(value.as_bytes().to_vec())],
        ),
        Operation::new("ET", Vec::new()),
        Operation::new("Q", Vec::new()),
    ];
    let content = Content { operations }
        .encode()
        .map_err(|_| PdfEditError::FormFieldUnsupported)?;
    let mut font_resources = Dictionary::new();
    font_resources.set("Helv", font_id);
    let mut resources = Dictionary::new();
    resources.set("Font", font_resources);
    let mut appearance = lopdf::Stream::new(
        Dictionary::from_iter([
            (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Object::Integer(1)),
            (
                b"BBox".to_vec(),
                Object::Array(vec![
                    pdf_number_object(0.0)?,
                    pdf_number_object(0.0)?,
                    pdf_number_object(width)?,
                    pdf_number_object(height)?,
                ]),
            ),
            (b"Resources".to_vec(), Object::Dictionary(resources)),
        ]),
        content,
    );
    appearance
        .compress()
        .map_err(|_| PdfEditError::FormFieldUnsupported)?;
    Ok(document.add_object(appearance))
}

fn set_text_form_appearance(
    document: &mut Document,
    widget_id: ObjectId,
    value: &str,
) -> Result<(), PdfEditError> {
    let rectangle = document
        .get_dictionary(widget_id)
        .map_err(|_| PdfEditError::FormFieldUnsupported)?
        .get(b"Rect")
        .cloned()
        .map_err(|_| PdfEditError::FormFieldUnsupported)
        .and_then(|value| pdf_box_from_object(document, value))?;
    let mut appearance_dictionary = document
        .get_dictionary(widget_id)
        .ok()
        .and_then(|widget| widget.get(b"AP").ok())
        .and_then(|appearance| dereference_dictionary(document, appearance))
        .cloned()
        .unwrap_or_default();
    let appearance_id = create_form_text_appearance(document, rectangle, value)?;
    appearance_dictionary.set("N", appearance_id);
    let widget = document
        .get_dictionary_mut(widget_id)
        .map_err(|_| PdfEditError::FormFieldUnsupported)?;
    widget.set("AP", appearance_dictionary);
    widget.remove(b"AS");
    Ok(())
}

fn set_button_form_appearance(
    document: &mut Document,
    widgets: &[PdfObjectId],
    value: &str,
) -> Result<(), PdfEditError> {
    let mut matched = 0usize;
    for widget in widgets {
        let widget_id = ObjectId::from(*widget);
        let appearance = document
            .get_dictionary(widget_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?
            .get(b"AP")
            .and_then(Object::as_dict)
            .and_then(|appearance| appearance.get(b"N"))
            .and_then(Object::as_dict)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        let selected = value != "Off"
            && appearance
                .get(value.as_bytes())
                .and_then(Object::as_reference)
                .is_ok();
        if selected {
            matched += 1;
        }
        document
            .get_dictionary_mut(widget_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?
            .set(
                "AS",
                Object::Name(if selected {
                    value.as_bytes().to_vec()
                } else {
                    b"Off".to_vec()
                }),
            );
    }
    if (value == "Off" && matched != 0) || (value != "Off" && matched != 1) {
        return Err(PdfEditError::InvalidFormFieldValue);
    }
    Ok(())
}

fn form_field_appearance_valid(document: &Document, field: &EditableFormField) -> bool {
    match field.kind {
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown => {
            field.widget_objects.iter().all(|widget| {
                document
                    .get_dictionary(ObjectId::from(*widget))
                    .ok()
                    .and_then(|widget| widget.get(b"AP").ok())
                    .and_then(|appearance| dereference_dictionary(document, appearance))
                    .and_then(|appearance| appearance.get(b"N").ok())
                    .and_then(|normal| match normal {
                        Object::Reference(id) => document.get_object(*id).ok(),
                        value => Some(value),
                    })
                    .and_then(|normal| normal.as_stream().ok())
                    .is_some_and(|stream| {
                        stream.dict.has_type(b"XObject")
                            && stream
                                .dict
                                .get(b"Subtype")
                                .and_then(Object::as_name)
                                .is_ok_and(|name| name == b"Form")
                    })
            })
        }
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            field.value.as_ref().is_some_and(|value| {
                field
                    .widget_objects
                    .iter()
                    .filter(|widget| {
                        document
                            .get_dictionary(ObjectId::from(**widget))
                            .is_ok_and(|widget| {
                                widget
                                    .get(b"AS")
                                    .and_then(Object::as_name)
                                    .is_ok_and(|name| name == value.as_bytes())
                            })
                    })
                    .count()
                    == if value == "Off" {
                        field.widget_objects.len()
                    } else {
                        1
                    }
            })
        }
        PdfFormFieldKind::Signature => false,
    }
}

fn form_field_appearance_repairable(document: &Document, field: &EditableFormField) -> bool {
    if field.read_only || field.appearance_valid || field.kind == PdfFormFieldKind::Signature {
        return false;
    }
    match field.kind {
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown => {
            field.value.as_deref().unwrap_or("").is_ascii()
                && field.value.as_deref().unwrap_or("").len() <= MAX_FORM_FIELD_VALUE_BYTES
        }
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            let Some(value) = field.value.as_deref() else {
                return false;
            };
            if value != "Off" && !field.options.iter().any(|option| option == value) {
                return false;
            }
            field.widget_objects.iter().all(|widget| {
                document
                    .get_dictionary(ObjectId::from(*widget))
                    .ok()
                    .and_then(|widget| widget.get(b"AP").ok())
                    .and_then(|appearance| dereference_dictionary(document, appearance))
                    .and_then(|appearance| appearance.get(b"N").ok())
                    .and_then(|normal| dereference_dictionary(document, normal))
                    .is_some_and(|normal| normal.get(b"Off").and_then(Object::as_reference).is_ok())
            })
        }
        PdfFormFieldKind::Signature => false,
    }
}

pub fn edit_form_field_value(
    request: &EditFormFieldValueRequest,
) -> Result<EditFormFieldValueResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|field| field.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }
    let result_value = validated_form_value(&field, &request.value)?;
    let field_id = ObjectId::from(field.selector.field_object);
    {
        let dictionary = document
            .get_dictionary_mut(field_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        match (&field.kind, &result_value) {
            (PdfFormFieldKind::Text, Some(value)) | (PdfFormFieldKind::Dropdown, Some(value)) => {
                dictionary.set("V", text_string(value));
                dictionary.set("DA", Object::string_literal(b"/Helv 10 Tf 0 g".to_vec()));
                dictionary.remove(b"I");
            }
            (PdfFormFieldKind::Text, None) => {
                dictionary.remove(b"V");
            }
            (PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio, Some(value)) => {
                dictionary.set("V", Object::Name(value.as_bytes().to_vec()));
            }
            _ => return Err(PdfEditError::InvalidFormFieldValue),
        }
    }
    match field.kind {
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown => {
            let appearance_value = result_value.as_deref().unwrap_or("");
            for widget in &field.widget_objects {
                set_text_form_appearance(&mut document, ObjectId::from(*widget), appearance_value)?;
            }
        }
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            set_button_form_appearance(
                &mut document,
                &field.widget_objects,
                result_value
                    .as_deref()
                    .ok_or(PdfEditError::InvalidFormFieldValue)?,
            )?;
        }
        PdfFormFieldKind::Signature => return Err(PdfEditError::FormFieldUnsupported),
    }
    document
        .get_dictionary_mut(source_state.acro_form_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?
        .set("NeedAppearances", false);

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidFormFieldValue);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|candidate| candidate.selector.field_object == field.selector.field_object);
    let changed_non_form_objects = source_objects
        .iter()
        .filter_map(|(object_id, object)| {
            (!source_state.owned_ids.contains(object_id)
                && !is_writer_housekeeping_object(object)
                && !output
                    .objects
                    .get(object_id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate)))
            .then_some(*object_id)
        })
        .collect::<Vec<_>>();
    let non_form_objects_preserved = changed_non_form_objects.is_empty();
    let validation = FormFieldEditValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count >= source_semantic_object_count
            && output_semantic_object_count
                <= source_semantic_object_count + field.widget_objects.len() * 2,
        non_form_objects_preserved,
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| state.is_some()),
        requested_value_preserved: output_field
            .is_some_and(|candidate| candidate.value == result_value),
        appearance_valid: output_field
            .is_some_and(|candidate| form_field_appearance_valid(&output, candidate)),
        field_count_preserved: output_fields.len() == source_field_count
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditFormFieldValueResult {
        operation: "EDIT_FORM_FIELD_VALUE".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_kind: field.kind,
        source_value: field.value,
        result_value,
        widget_objects: field.widget_objects,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn validate_form_widget_geometry(
    document: &Document,
    widget: &EditableFormWidget,
    requested: PdfBox,
) -> Result<(), PdfEditError> {
    if !valid_pdf_box(requested) || pdf_boxes_equal(widget.rectangle, requested) {
        return Err(PdfEditError::InvalidFormFieldGeometry);
    }
    let page = page_geometry(
        document,
        widget.selector.expected_page_number,
        ObjectId::from(widget.selector.expected_page_object),
    )
    .map_err(|_| PdfEditError::InvalidFormFieldGeometry)?;
    if requested.left < page.media_box.left
        || requested.bottom < page.media_box.bottom
        || requested.right > page.media_box.right
        || requested.top > page.media_box.top
    {
        return Err(PdfEditError::InvalidFormFieldGeometry);
    }
    Ok(())
}

pub fn edit_form_field_geometry(
    request: &EditFormFieldGeometryRequest,
) -> Result<EditFormFieldGeometryResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|field| field.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }
    let widget = field
        .widgets
        .iter()
        .find(|widget| widget.selector == request.widget)
        .cloned()
        .ok_or(PdfEditError::StaleFormField)?;
    validate_form_widget_geometry(&document, &widget, request.rectangle)?;
    let widget_id = ObjectId::from(widget.selector.widget_object);
    document
        .get_dictionary_mut(widget_id)
        .map_err(|_| PdfEditError::FormFieldUnsupported)?
        .set(
            "Rect",
            pdf_box_object(request.rectangle)
                .map_err(|_| PdfEditError::InvalidFormFieldGeometry)?,
        );
    if matches!(
        field.kind,
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown
    ) {
        set_text_form_appearance(
            &mut document,
            widget_id,
            field.value.as_deref().unwrap_or(""),
        )?;
    }
    document
        .get_dictionary_mut(source_state.acro_form_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?
        .set("NeedAppearances", false);

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidFormFieldGeometry);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|candidate| candidate.selector.field_object == field.selector.field_object);
    let output_widget = output_field.and_then(|candidate| {
        candidate.widgets.iter().find(|candidate_widget| {
            candidate_widget.selector.widget_object == widget.selector.widget_object
        })
    });
    let changed_non_form_objects = source_objects
        .iter()
        .filter_map(|(object_id, object)| {
            (!source_state.owned_ids.contains(object_id)
                && !is_writer_housekeeping_object(object)
                && !output
                    .objects
                    .get(object_id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate)))
            .then_some(*object_id)
        })
        .collect::<Vec<_>>();
    let validation = FormFieldGeometryValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count >= source_semantic_object_count
            && output_semantic_object_count <= source_semantic_object_count + 2,
        non_form_objects_preserved: changed_non_form_objects.is_empty(),
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| state.is_some()),
        requested_geometry_preserved: output_widget.is_some_and(|candidate| {
            candidate.selector.expected_page_number == widget.selector.expected_page_number
                && candidate.selector.expected_page_object == widget.selector.expected_page_object
                && pdf_boxes_equal(candidate.rectangle, request.rectangle)
        }),
        appearance_valid: output_field.is_some_and(|candidate| {
            candidate.kind == PdfFormFieldKind::Signature
                || form_field_appearance_valid(&output, candidate)
        }),
        field_count_preserved: output_fields.len() == source_field_count
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditFormFieldGeometryResult {
        operation: "EDIT_FORM_FIELD_GEOMETRY".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_kind: field.kind,
        widget_object: widget.selector.widget_object,
        page_number: widget.selector.expected_page_number,
        page_object: widget.selector.expected_page_object,
        source_rectangle: widget.rectangle,
        result_rectangle: request.rectangle,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn validated_form_field_label(
    field: &EditableFormField,
    requested: &Option<String>,
) -> Result<Option<String>, PdfEditError> {
    if field.label == *requested {
        return Err(PdfEditError::InvalidFormFieldLabel);
    }
    if let Some(label) = requested
        && (label.trim().is_empty()
            || label.trim() != label
            || label.len() > MAX_FORM_FIELD_LABEL_BYTES
            || label.chars().count() > MAX_FORM_FIELD_LABEL_CHARS
            || label.chars().any(char::is_control))
    {
        return Err(PdfEditError::InvalidFormFieldLabel);
    }
    Ok(requested.clone())
}

pub fn edit_form_field_label(
    request: &EditFormFieldLabelRequest,
) -> Result<EditFormFieldLabelResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|field| field.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }
    let result_label = validated_form_field_label(&field, &request.label)?;
    let field_id = ObjectId::from(field.selector.field_object);
    let dictionary = document
        .get_dictionary_mut(field_id)
        .map_err(|_| PdfEditError::FormFieldUnsupported)?;
    if let Some(label) = &result_label {
        dictionary.set("TU", text_string(label));
    } else {
        dictionary.remove(b"TU");
    }

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidFormFieldLabel);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|candidate| candidate.selector.field_object == field.selector.field_object);
    let changed_non_form_objects = source_objects
        .iter()
        .filter_map(|(object_id, object)| {
            (!source_state.owned_ids.contains(object_id)
                && !is_writer_housekeeping_object(object)
                && !output
                    .objects
                    .get(object_id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate)))
            .then_some(*object_id)
        })
        .collect::<Vec<_>>();
    let widget_objects_preserved = output_field.is_some_and(|candidate| {
        candidate.widget_objects == field.widget_objects
            && candidate.widgets == field.widgets
            && field.widget_objects.iter().all(|widget| {
                let widget_id = ObjectId::from(*widget);
                let source_appearance = source_objects
                    .get(&widget_id)
                    .and_then(|object| object.as_dict().ok())
                    .and_then(|dictionary| dictionary.get(b"AP").ok());
                let output_appearance = output
                    .objects
                    .get(&widget_id)
                    .and_then(|object| object.as_dict().ok())
                    .and_then(|dictionary| dictionary.get(b"AP").ok());
                match (source_appearance, output_appearance) {
                    (None, None) => true,
                    (Some(source), Some(candidate)) => {
                        pdf_objects_semantically_equal(source, candidate)
                    }
                    _ => false,
                }
            })
    });
    let validation = FormFieldLabelValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_preserved: output_semantic_object_count == source_semantic_object_count,
        non_form_objects_preserved: changed_non_form_objects.is_empty(),
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| state.is_some()),
        requested_label_preserved: output_field
            .is_some_and(|candidate| candidate.label == result_label),
        field_identity_preserved: output_field
            .is_some_and(|candidate| candidate.name == field.name && candidate.kind == field.kind),
        field_value_preserved: output_field.is_some_and(|candidate| candidate.value == field.value),
        widget_objects_preserved,
        field_count_preserved: output_fields.len() == source_field_count
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditFormFieldLabelResult {
        operation: "EDIT_FORM_FIELD_LABEL".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_kind: field.kind,
        source_label: field.label,
        result_label,
        widget_objects: field.widget_objects,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn validated_form_field_options(
    field: &EditableFormField,
    requested_options: &[String],
    requested_value: &str,
) -> Result<(), PdfEditError> {
    if field.kind != PdfFormFieldKind::Dropdown || field.read_only || !field.options_editable {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    if requested_options.is_empty()
        || requested_options.len() > MAX_FORM_FIELD_OPTIONS
        || requested_options.iter().any(|option| {
            !validated_created_form_text(option, MAX_FORM_FIELD_OPTION_BYTES) || !option.is_ascii()
        })
        || requested_options.iter().collect::<BTreeSet<_>>().len() != requested_options.len()
        || !requested_options
            .iter()
            .any(|option| option == requested_value)
        || !requested_value.is_ascii()
        || (field.options == requested_options && field.value.as_deref() == Some(requested_value))
    {
        return Err(PdfEditError::InvalidFormFieldOptions);
    }
    Ok(())
}

pub fn edit_form_field_options(
    request: &EditFormFieldOptionsRequest,
) -> Result<EditFormFieldOptionsResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|field| field.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }
    validated_form_field_options(&field, &request.options, &request.value)?;
    let field_id = ObjectId::from(field.selector.field_object);
    {
        let dictionary = document
            .get_dictionary_mut(field_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        dictionary.set(
            "Opt",
            Object::Array(
                request
                    .options
                    .iter()
                    .map(|value| text_string(value))
                    .collect(),
            ),
        );
        dictionary.set("V", text_string(&request.value));
        dictionary.set("DA", Object::string_literal(b"/Helv 10 Tf 0 g".to_vec()));
        dictionary.remove(b"I");
    }
    for widget in &field.widget_objects {
        set_text_form_appearance(&mut document, ObjectId::from(*widget), &request.value)?;
    }
    document
        .get_dictionary_mut(source_state.acro_form_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?
        .set("NeedAppearances", false);

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidFormFieldOptions);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|candidate| candidate.selector.field_object == field.selector.field_object);
    let changed_non_form_objects = source_objects
        .iter()
        .filter_map(|(object_id, object)| {
            (!source_state.owned_ids.contains(object_id)
                && !is_writer_housekeeping_object(object)
                && !output
                    .objects
                    .get(object_id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate)))
            .then_some(*object_id)
        })
        .collect::<Vec<_>>();
    let validation = FormFieldOptionsValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count >= source_semantic_object_count
            && output_semantic_object_count
                <= source_semantic_object_count + field.widget_objects.len() * 2,
        non_form_objects_preserved: changed_non_form_objects.is_empty(),
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| state.is_some()),
        requested_options_preserved: output_field.is_some_and(|candidate| {
            candidate.options == request.options && candidate.options_editable
        }),
        requested_value_preserved: output_field
            .is_some_and(|candidate| candidate.value.as_deref() == Some(request.value.as_str())),
        appearance_valid: output_field
            .is_some_and(|candidate| form_field_appearance_valid(&output, candidate)),
        field_identity_preserved: output_field
            .is_some_and(|candidate| candidate.name == field.name && candidate.kind == field.kind),
        widget_objects_preserved: output_field.is_some_and(|candidate| {
            candidate.widget_objects == field.widget_objects && candidate.widgets == field.widgets
        }),
        field_count_preserved: output_fields.len() == source_field_count
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(EditFormFieldOptionsResult {
        operation: "EDIT_FORM_FIELD_OPTIONS".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_kind: field.kind,
        source_options: field.options,
        result_options: request.options.clone(),
        source_value: field.value,
        result_value: request.value.clone(),
        widget_objects: field.widget_objects,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn repair_form_field_appearance(
    request: &RepairFormFieldAppearanceRequest,
) -> Result<RepairFormFieldAppearanceResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };
    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|field| field.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }
    if !field.appearance_repairable || field.appearance_valid {
        return Err(PdfEditError::FormFieldAppearanceNotRepairable);
    }
    match field.kind {
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown => {
            let value = field.value.as_deref().unwrap_or("");
            for widget in &field.widget_objects {
                set_text_form_appearance(&mut document, ObjectId::from(*widget), value)?;
            }
        }
        PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => {
            set_button_form_appearance(
                &mut document,
                &field.widget_objects,
                field
                    .value
                    .as_deref()
                    .ok_or(PdfEditError::FormFieldAppearanceNotRepairable)?,
            )?;
        }
        PdfFormFieldKind::Signature => {
            return Err(PdfEditError::FormFieldAppearanceNotRepairable);
        }
    }
    document
        .get_dictionary_mut(source_state.acro_form_id)
        .map_err(|_| PdfEditError::FormTreeUnsupported)?
        .set("NeedAppearances", false);

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::FormFieldAppearanceNotRepairable);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|candidate| candidate.selector.field_object == field.selector.field_object);
    let changed_non_form_objects = source_objects
        .iter()
        .filter_map(|(object_id, object)| {
            (!source_state.owned_ids.contains(object_id)
                && !is_writer_housekeeping_object(object)
                && !output
                    .objects
                    .get(object_id)
                    .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate)))
            .then_some(*object_id)
        })
        .collect::<Vec<_>>();
    let validation = FormFieldAppearanceRepairValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count >= source_semantic_object_count
            && output_semantic_object_count
                <= source_semantic_object_count + field.widget_objects.len() * 2,
        non_form_objects_preserved: changed_non_form_objects.is_empty(),
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| state.is_some()),
        source_appearance_invalid: !field.appearance_valid,
        requested_appearance_repaired: output_field
            .is_some_and(|candidate| form_field_appearance_valid(&output, candidate)),
        field_identity_preserved: output_field
            .is_some_and(|candidate| candidate.name == field.name && candidate.kind == field.kind),
        field_value_preserved: output_field.is_some_and(|candidate| {
            candidate.value == field.value && candidate.options == field.options
        }),
        widget_objects_preserved: output_field.is_some_and(|candidate| {
            candidate.widget_objects == field.widget_objects && candidate.widgets == field.widgets
        }),
        field_count_preserved: output_fields.len() == source_field_count
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(RepairFormFieldAppearanceResult {
        operation: "REPAIR_FORM_FIELD_APPEARANCE".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_kind: field.kind,
        field_value: field.value,
        widget_objects: field.widget_objects,
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

fn validated_created_form_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= maximum_bytes
        && !value.chars().any(char::is_control)
}

fn validated_created_export_value(value: &str) -> bool {
    validated_created_form_text(value, MAX_FORM_FIELD_OPTION_BYTES)
        && value != "Off"
        && value.is_ascii()
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || b"#/%()<>[]{}".contains(&byte))
}

fn create_form_button_appearance(
    document: &mut Document,
    rectangle: PdfBox,
    selected: bool,
) -> Result<ObjectId, PdfEditError> {
    let width = rectangle.right - rectangle.left;
    let height = rectangle.top - rectangle.bottom;
    if width <= 4.0 || height <= 4.0 {
        return Err(PdfEditError::InvalidFormFieldCreation);
    }
    let mut operations = vec![
        Operation::new("q", Vec::new()),
        Operation::new("rg", vec![1.into(), 1.into(), 1.into()]),
        Operation::new(
            "re",
            vec![
                0.into(),
                0.into(),
                pdf_number_object(width)?,
                pdf_number_object(height)?,
            ],
        ),
        Operation::new("f", Vec::new()),
        Operation::new("G", vec![pdf_number_object(0.25)?]),
        Operation::new("w", vec![pdf_number_object(0.75)?]),
        Operation::new(
            "re",
            vec![
                pdf_number_object(0.5)?,
                pdf_number_object(0.5)?,
                pdf_number_object(width - 1.0)?,
                pdf_number_object(height - 1.0)?,
            ],
        ),
        Operation::new("S", Vec::new()),
    ];
    if selected {
        operations.extend([
            Operation::new("w", vec![pdf_number_object(1.5)?]),
            Operation::new("m", vec![pdf_number_object(3.0)?, pdf_number_object(3.0)?]),
            Operation::new(
                "l",
                vec![
                    pdf_number_object(width - 3.0)?,
                    pdf_number_object(height - 3.0)?,
                ],
            ),
            Operation::new("S", Vec::new()),
            Operation::new(
                "m",
                vec![pdf_number_object(width - 3.0)?, pdf_number_object(3.0)?],
            ),
            Operation::new(
                "l",
                vec![pdf_number_object(3.0)?, pdf_number_object(height - 3.0)?],
            ),
            Operation::new("S", Vec::new()),
        ]);
    }
    operations.push(Operation::new("Q", Vec::new()));
    let content = Content { operations }
        .encode()
        .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
    let mut appearance = lopdf::Stream::new(
        Dictionary::from_iter([
            (b"Type".to_vec(), Object::Name(b"XObject".to_vec())),
            (b"Subtype".to_vec(), Object::Name(b"Form".to_vec())),
            (b"FormType".to_vec(), Object::Integer(1)),
            (
                b"BBox".to_vec(),
                Object::Array(vec![
                    0.into(),
                    0.into(),
                    pdf_number_object(width)?,
                    pdf_number_object(height)?,
                ]),
            ),
        ]),
        content,
    );
    appearance
        .compress()
        .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
    Ok(document.add_object(appearance))
}

fn form_widget_has_normal_appearance(document: &Document, widget_id: ObjectId) -> bool {
    document
        .get_dictionary(widget_id)
        .ok()
        .and_then(|widget| widget.get(b"AP").ok())
        .and_then(|appearance| dereference_dictionary(document, appearance))
        .and_then(|appearance| appearance.get(b"N").ok())
        .is_some()
}

pub fn create_form_field(
    request: &CreateFormFieldRequest,
) -> Result<CreateFormFieldResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let catalog_id = form_catalog_object_id(&document)?;
    let source_state = acro_form_state(&document)?;
    let selector_matches = request.selector.catalog_object == catalog_id.into()
        && match (
            &source_state,
            request.selector.acro_form_object,
            &request.selector.expected_form_sha256,
        ) {
            (Some(state), Some(object), Some(hash)) => {
                object == state.acro_form_id.into() && hash == &state.sha256
            }
            (None, None, None) => true,
            _ => false,
        };
    if !selector_matches {
        return Err(PdfEditError::StaleFormField);
    }
    let (source_fields, _, source_unsupported) = inspect_form_fields(&document)?;
    if source_unsupported != 0
        || source_fields.len() >= MAX_EDITABLE_FORM_FIELDS
        || !validated_created_form_text(&request.name, MAX_FORM_FIELD_NAME_BYTES)
        || request.name.contains('.')
        || source_fields.iter().any(|field| field.name == request.name)
        || request.widgets.is_empty()
        || request.widgets.len() > MAX_FORM_FIELD_CREATION_WIDGETS
    {
        return Err(PdfEditError::InvalidFormFieldCreation);
    }
    if let Some(label) = &request.label
        && (!validated_created_form_text(label, MAX_FORM_FIELD_LABEL_BYTES)
            || label.chars().count() > MAX_FORM_FIELD_LABEL_CHARS)
    {
        return Err(PdfEditError::InvalidFormFieldCreation);
    }
    if request.options.len() > MAX_FORM_FIELD_OPTIONS
        || request.options.iter().any(|option| {
            !validated_created_form_text(option, MAX_FORM_FIELD_OPTION_BYTES) || !option.is_ascii()
        })
        || request.options.iter().collect::<BTreeSet<_>>().len() != request.options.len()
    {
        return Err(PdfEditError::InvalidFormFieldCreation);
    }

    let (result_value, field_flags) = match request.kind {
        PdfFormFieldKind::Text => {
            if request.widgets.len() != 1
                || !request.options.is_empty()
                || request.widgets[0].export_value.is_some()
                || request.value.as_ref().is_some_and(|value| {
                    value.len() > MAX_FORM_FIELD_VALUE_BYTES
                        || !value.is_ascii()
                        || value.chars().any(char::is_control)
                })
            {
                return Err(PdfEditError::InvalidFormFieldCreation);
            }
            (request.value.clone(), 0u32)
        }
        PdfFormFieldKind::Dropdown => {
            if request.widgets.len() != 1
                || request.options.is_empty()
                || request.widgets[0].export_value.is_some()
                || request
                    .value
                    .as_ref()
                    .is_some_and(|value| !request.options.contains(value))
            {
                return Err(PdfEditError::InvalidFormFieldCreation);
            }
            (request.value.clone(), 1u32 << 17)
        }
        PdfFormFieldKind::Checkbox => {
            if request.widgets.len() != 1
                || request.options.len() != 1
                || request.widgets[0].export_value.as_ref() != request.options.first()
                || !request
                    .options
                    .iter()
                    .all(|value| validated_created_export_value(value))
                || request
                    .value
                    .as_ref()
                    .is_some_and(|value| !request.options.contains(value))
            {
                return Err(PdfEditError::InvalidFormFieldCreation);
            }
            (
                Some(request.value.clone().unwrap_or_else(|| "Off".to_string())),
                0u32,
            )
        }
        PdfFormFieldKind::Radio => {
            let exports = request
                .widgets
                .iter()
                .filter_map(|widget| widget.export_value.as_ref())
                .collect::<Vec<_>>();
            if request.widgets.len() < 2
                || request.options.len() != request.widgets.len()
                || exports.len() != request.widgets.len()
                || exports.iter().copied().collect::<BTreeSet<_>>().len() != exports.len()
                || request.options.iter().collect::<BTreeSet<_>>()
                    != exports.iter().copied().collect::<BTreeSet<_>>()
                || !request
                    .options
                    .iter()
                    .all(|value| validated_created_export_value(value))
                || request
                    .value
                    .as_ref()
                    .is_some_and(|value| !request.options.contains(value))
            {
                return Err(PdfEditError::InvalidFormFieldCreation);
            }
            (
                Some(request.value.clone().unwrap_or_else(|| "Off".to_string())),
                1u32 << 15,
            )
        }
        PdfFormFieldKind::Signature => {
            if request.widgets.len() != 1
                || !request.options.is_empty()
                || request.widgets[0].export_value.is_some()
                || request.value.is_some()
            {
                return Err(PdfEditError::InvalidFormFieldCreation);
            }
            (None, 0u32)
        }
    };

    let pages = document.get_pages();
    let mut page_collections = BTreeMap::new();
    let mut allowed_changed_source_ids = BTreeSet::new();
    if let Some(state) = &source_state {
        allowed_changed_source_ids.insert(state.acro_form_id);
    } else {
        allowed_changed_source_ids.insert(catalog_id);
    }
    for widget in &request.widgets {
        let page_id = selected_page_id(&pages, widget.page)
            .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
        let geometry = page_geometry(&document, widget.page.page_number, page_id)
            .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
        if !valid_pdf_box(widget.rectangle)
            || widget.rectangle.left < geometry.media_box.left
            || widget.rectangle.bottom < geometry.media_box.bottom
            || widget.rectangle.right > geometry.media_box.right
            || widget.rectangle.top > geometry.media_box.top
        {
            return Err(PdfEditError::InvalidFormFieldCreation);
        }
        let collection = link_annotation_collection(&document, page_id)
            .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
        allowed_changed_source_ids.insert(collection.container.object_id().unwrap_or(page_id));
        page_collections.entry(page_id).or_insert(collection);
    }

    let field_id = document.new_object_id();
    document.objects.insert(field_id, Object::Null);
    let mut widget_ids = Vec::with_capacity(request.widgets.len());
    let mut page_widget_ids: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    for widget_request in &request.widgets {
        let page_id = ObjectId::from(widget_request.page.page_object);
        let mut widget = Dictionary::new();
        widget.set("Type", Object::Name(b"Annot".to_vec()));
        widget.set("Subtype", Object::Name(b"Widget".to_vec()));
        widget.set("Parent", field_id);
        widget.set("Rect", pdf_box_object(widget_request.rectangle)?);
        widget.set("P", page_id);
        widget.set("F", 4);
        if matches!(
            request.kind,
            PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio
        ) {
            let export_value = widget_request
                .export_value
                .as_ref()
                .ok_or(PdfEditError::InvalidFormFieldCreation)?;
            let off_appearance =
                create_form_button_appearance(&mut document, widget_request.rectangle, false)?;
            let on_appearance =
                create_form_button_appearance(&mut document, widget_request.rectangle, true)?;
            let mut normal = Dictionary::new();
            normal.set("Off", off_appearance);
            normal.set(export_value.as_bytes(), on_appearance);
            let mut appearances = Dictionary::new();
            appearances.set("N", normal);
            widget.set("AP", appearances);
            widget.set(
                "AS",
                Object::Name(
                    if result_value.as_deref() == Some(export_value.as_str()) {
                        export_value.as_bytes()
                    } else {
                        b"Off"
                    }
                    .to_vec(),
                ),
            );
        }
        let widget_id = document.add_object(widget);
        if matches!(
            request.kind,
            PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown | PdfFormFieldKind::Signature
        ) {
            set_text_form_appearance(
                &mut document,
                widget_id,
                result_value.as_deref().unwrap_or(""),
            )?;
        }
        widget_ids.push(widget_id);
        page_widget_ids.entry(page_id).or_default().push(widget_id);
    }

    let mut field = Dictionary::new();
    field.set(
        "FT",
        Object::Name(
            match request.kind {
                PdfFormFieldKind::Text => b"Tx".as_slice(),
                PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio => b"Btn".as_slice(),
                PdfFormFieldKind::Dropdown => b"Ch".as_slice(),
                PdfFormFieldKind::Signature => b"Sig".as_slice(),
            }
            .to_vec(),
        ),
    );
    field.set("T", text_string(&request.name));
    if let Some(label) = &request.label {
        field.set("TU", text_string(label));
    }
    let flags = field_flags | if request.required { 1u32 << 1 } else { 0 };
    if flags != 0 {
        field.set("Ff", i64::from(flags));
    }
    field.set(
        "Kids",
        Object::Array(widget_ids.iter().copied().map(Into::into).collect()),
    );
    if matches!(
        request.kind,
        PdfFormFieldKind::Text | PdfFormFieldKind::Dropdown
    ) {
        field.set("DA", Object::string_literal(b"/Helv 10 Tf 0 g".to_vec()));
    }
    if request.kind == PdfFormFieldKind::Dropdown {
        field.set(
            "Opt",
            Object::Array(
                request
                    .options
                    .iter()
                    .map(|value| text_string(value))
                    .collect(),
            ),
        );
    }
    if let Some(value) = &result_value {
        if matches!(
            request.kind,
            PdfFormFieldKind::Checkbox | PdfFormFieldKind::Radio
        ) {
            field.set("V", Object::Name(value.as_bytes().to_vec()));
        } else {
            field.set("V", text_string(value));
        }
    }
    document.objects.insert(field_id, Object::Dictionary(field));

    for (page_id, created_widgets) in &page_widget_ids {
        let collection = page_collections
            .get(page_id)
            .ok_or(PdfEditError::InvalidFormFieldCreation)?;
        let mut annotation_ids = collection.annotation_ids.clone();
        annotation_ids.extend(created_widgets.iter().copied());
        set_page_annotation_ids(
            &mut document,
            *page_id,
            collection.container,
            &annotation_ids,
        )
        .map_err(|_| PdfEditError::InvalidFormFieldCreation)?;
    }

    let acro_form_id = if let Some(state) = &source_state {
        let mut fields = state.fields.clone();
        fields.push(field_id);
        let form = document
            .get_dictionary_mut(state.acro_form_id)
            .map_err(|_| PdfEditError::FormTreeUnsupported)?;
        form.set(
            "Fields",
            Object::Array(fields.into_iter().map(Into::into).collect()),
        );
        form.set("NeedAppearances", false);
        if request.kind == PdfFormFieldKind::Signature {
            let flags = form
                .get(b"SigFlags")
                .ok()
                .and_then(|value| value.as_i64().ok())
                .unwrap_or(0)
                | 3;
            form.set("SigFlags", flags);
        }
        state.acro_form_id
    } else {
        let mut form = Dictionary::new();
        form.set("Fields", Object::Array(vec![field_id.into()]));
        form.set("NeedAppearances", false);
        if request.kind == PdfFormFieldKind::Signature {
            form.set("SigFlags", 3);
        }
        let acro_form_id = document.add_object(form);
        document
            .get_dictionary_mut(catalog_id)
            .map_err(|_| PdfEditError::FormTreeUnsupported)?
            .set("AcroForm", acro_form_id);
        acro_form_id
    };

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let output_field = output_fields
        .iter()
        .find(|field| field.name == request.name);
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_source_ids.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let requested_widgets_preserved = output_field.is_some_and(|field| {
        field.widgets.len() == request.widgets.len()
            && field
                .widgets
                .iter()
                .zip(&request.widgets)
                .all(|(actual, requested)| {
                    actual.selector.expected_page_number == requested.page.page_number
                        && actual.selector.expected_page_object == requested.page.page_object
                        && pdf_boxes_equal(actual.rectangle, requested.rectangle)
                })
    });
    let appearance_valid = output_field.is_some_and(|field| {
        if field.kind == PdfFormFieldKind::Signature {
            field
                .widget_objects
                .iter()
                .all(|widget| form_widget_has_normal_appearance(&output, ObjectId::from(*widget)))
        } else {
            form_field_appearance_valid(&output, field)
        }
    });
    let validation = FormFieldCreationValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count > source_semantic_object_count
            && output_semantic_object_count
                <= source_semantic_object_count + request.widgets.len() * 4 + 3,
        non_target_objects_preserved,
        form_tree_valid: acro_form_state(&output)
            .is_ok_and(|state| state.is_some_and(|state| state.acro_form_id == acro_form_id)),
        requested_field_preserved: output_field.is_some_and(|field| {
            field.kind == request.kind
                && field.label == request.label
                && field.value == result_value
                && field.options == request.options
                && field.required == request.required
                && !field.read_only
        }),
        requested_widgets_preserved,
        appearance_valid,
        field_count_incremented: output_fields.len() == source_fields.len() + 1
            && output_unsupported == source_unsupported,
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    let created = output_field.ok_or(PdfEditError::ValidationFailed)?;
    Ok(CreateFormFieldResult {
        operation: "CREATE_FORM_FIELD".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: created.selector.field_object,
        field_name: created.name.clone(),
        field_label: created.label.clone(),
        field_kind: created.kind,
        result_value: created.value.clone(),
        options: created.options.clone(),
        widget_objects: created.widget_objects.clone(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

pub fn delete_form_field(
    request: &DeleteFormFieldRequest,
) -> Result<DeleteFormFieldResult, PdfEditError> {
    validate_paths(&request.input_path, &request.output_path)?;
    let input_metadata = checked_metadata(&request.input_path)?;
    let input_sha256 = sha256_file(&request.input_path)?;
    let mut document = load_checked_pdf(&request.input_path)?;
    let page_count = document.get_pages().len();
    let source_objects = document.objects.clone();
    let source_object_count = source_objects.len();
    let source_semantic_object_count = semantic_object_count(&source_objects);
    let source_graph = object_graph_summary(&document, signature_summaries(&document).len());
    let signatures = signature_summaries(&document);
    let signature_effect = if signatures.is_empty() {
        SignatureEffect::None
    } else {
        if !request.allow_signature_invalidation {
            return Err(PdfEditError::SignatureConfirmationRequired);
        }
        SignatureEffect::InvalidatesExistingSignatures
    };

    let source_state = acro_form_state(&document)?.ok_or(PdfEditError::FormTreeUnsupported)?;
    if request.selector.form.catalog_object != source_state.catalog_id.into()
        || request.selector.form.acro_form_object != source_state.acro_form_id.into()
        || request.selector.form.expected_form_sha256 != source_state.sha256
    {
        return Err(PdfEditError::StaleFormField);
    }
    let (fields, _, source_unsupported) = inspect_form_fields(&document)?;
    if source_unsupported != 0 {
        return Err(PdfEditError::FormFieldUnsupported);
    }
    let source_field_count = fields.len();
    let field = fields
        .into_iter()
        .find(|candidate| candidate.selector.field_object == request.selector.field_object)
        .ok_or(PdfEditError::FormFieldNotFound)?;
    if field.selector != request.selector {
        return Err(PdfEditError::StaleFormField);
    }

    let field_id = ObjectId::from(field.selector.field_object);
    let widget_ids = field
        .widget_objects
        .iter()
        .copied()
        .map(ObjectId::from)
        .collect::<BTreeSet<_>>();
    let mut page_collections = BTreeMap::new();
    let mut allowed_changed_source_ids = BTreeSet::new();
    for widget in &field.widgets {
        let widget_id = ObjectId::from(widget.selector.widget_object);
        let page_id = ObjectId::from(widget.selector.expected_page_object);
        let collection = link_annotation_collection(&document, page_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
        if collection
            .annotation_ids
            .iter()
            .filter(|candidate| **candidate == widget_id)
            .count()
            != 1
        {
            return Err(PdfEditError::InvalidFormFieldDeletion);
        }
        allowed_changed_source_ids.insert(collection.container.object_id().unwrap_or(page_id));
        page_collections.entry(page_id).or_insert(collection);
    }

    let mut removed_tree_ids = BTreeSet::from([field_id]);
    let mut current_id = field_id;
    loop {
        let parent_id = document
            .get_dictionary(current_id)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?
            .get(b"Parent")
            .ok()
            .map(|value| {
                value
                    .as_reference()
                    .map_err(|_| PdfEditError::FormFieldUnsupported)
            })
            .transpose()?;
        if let Some(parent_id) = parent_id {
            if !source_state.owned_ids.contains(&parent_id) || removed_tree_ids.contains(&parent_id)
            {
                return Err(PdfEditError::FormFieldUnsupported);
            }
            let siblings = document
                .get_dictionary(parent_id)
                .and_then(|parent| parent.get(b"Kids"))
                .and_then(Object::as_array)
                .map_err(|_| PdfEditError::FormFieldUnsupported)?
                .iter()
                .map(|value| {
                    value
                        .as_reference()
                        .map_err(|_| PdfEditError::FormFieldUnsupported)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if siblings
                .iter()
                .filter(|candidate| **candidate == current_id)
                .count()
                != 1
            {
                return Err(PdfEditError::FormFieldUnsupported);
            }
            let remaining = siblings
                .into_iter()
                .filter(|candidate| *candidate != current_id)
                .collect::<Vec<_>>();
            if remaining.is_empty() {
                removed_tree_ids.insert(parent_id);
                current_id = parent_id;
                continue;
            }
            document
                .get_dictionary_mut(parent_id)
                .map_err(|_| PdfEditError::FormFieldUnsupported)?
                .set(
                    "Kids",
                    Object::Array(remaining.into_iter().map(Into::into).collect()),
                );
            allowed_changed_source_ids.insert(parent_id);
            break;
        }

        if source_state
            .fields
            .iter()
            .filter(|candidate| **candidate == current_id)
            .count()
            != 1
        {
            return Err(PdfEditError::FormFieldUnsupported);
        }
        let remaining = source_state
            .fields
            .iter()
            .copied()
            .filter(|candidate| *candidate != current_id)
            .collect::<Vec<_>>();
        document
            .get_dictionary_mut(source_state.acro_form_id)
            .map_err(|_| PdfEditError::FormTreeUnsupported)?
            .set(
                "Fields",
                Object::Array(remaining.into_iter().map(Into::into).collect()),
            );
        allowed_changed_source_ids.insert(source_state.acro_form_id);
        break;
    }

    for (page_id, collection) in &page_collections {
        let remaining = collection
            .annotation_ids
            .iter()
            .copied()
            .filter(|candidate| !widget_ids.contains(candidate))
            .collect::<Vec<_>>();
        set_page_annotation_ids(&mut document, *page_id, collection.container, &remaining)
            .map_err(|_| PdfEditError::FormFieldUnsupported)?;
    }

    let mut removed_object_ids = removed_tree_ids.clone();
    removed_object_ids.extend(widget_ids.iter().copied());
    for object_id in &removed_object_ids {
        if document.objects.remove(object_id).is_none() {
            return Err(PdfEditError::InvalidFormFieldDeletion);
        }
    }
    allowed_changed_source_ids.extend(removed_object_ids.iter().copied());

    let changed_objects = changed_object_ids(&source_objects, &document.objects);
    if changed_objects.is_empty() {
        return Err(PdfEditError::InvalidFormFieldDeletion);
    }
    save_new_pdf(&mut document, &request.output_path)?;
    let output_metadata =
        fs::metadata(&request.output_path).map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_sha256 = sha256_file(&request.output_path)?;
    let output = load_checked_pdf(&request.output_path)?;
    let output_object_count = output.objects.len();
    let output_semantic_object_count = semantic_object_count(&output.objects);
    let output_graph = object_graph_summary(&output, signature_summaries(&output).len());
    let (output_fields, _, output_unsupported) = inspect_form_fields(&output)?;
    let non_target_objects_preserved = source_objects.iter().all(|(id, object)| {
        allowed_changed_source_ids.contains(id)
            || is_writer_housekeeping_object(object)
            || output
                .objects
                .get(id)
                .is_some_and(|candidate| pdf_objects_semantically_equal(object, candidate))
    });
    let widgets_removed_from_pages = page_collections.iter().all(|(page_id, source)| {
        let expected = source
            .annotation_ids
            .iter()
            .copied()
            .filter(|candidate| !widget_ids.contains(candidate))
            .collect::<Vec<_>>();
        link_annotation_collection(&output, *page_id)
            .is_ok_and(|candidate| candidate.annotation_ids == expected)
    });
    let validation = FormFieldDeletionValidation {
        reparsed: true,
        page_count_preserved: output.get_pages().len() == page_count,
        object_count_delta_valid: output_semantic_object_count + removed_object_ids.len()
            == source_semantic_object_count,
        non_target_objects_preserved,
        form_tree_valid: acro_form_state(&output).is_ok_and(|state| {
            state.is_some_and(|state| state.acro_form_id == source_state.acro_form_id)
        }),
        requested_field_removed: output_fields
            .iter()
            .all(|candidate| candidate.selector.field_object != field.selector.field_object),
        widgets_removed_from_pages,
        field_count_decremented: output_fields.len() + 1 == source_field_count
            && output_unsupported == source_unsupported,
        deleted_objects_removed: removed_object_ids
            .iter()
            .all(|object_id| !output.objects.contains_key(object_id)),
        all_object_references_resolve: all_object_references_resolve(&output),
        image_xobject_count_preserved: output_graph.images == source_graph.images,
        no_page_rasterization: output_graph.images == source_graph.images,
    };
    if !validation.passed() {
        let _ = fs::remove_file(&request.output_path);
        return Err(PdfEditError::ValidationFailed);
    }
    Ok(DeleteFormFieldResult {
        operation: "DELETE_FORM_FIELD".to_string(),
        capability: "PDF_FORM_EDIT".to_string(),
        input_sha256,
        output_sha256,
        input_bytes: input_metadata.len(),
        output_bytes: output_metadata.len(),
        page_count,
        source_object_count,
        output_object_count,
        changed_objects: changed_objects.into_iter().map(Into::into).collect(),
        field_object: field.selector.field_object,
        field_name: field.name,
        field_label: field.label,
        field_kind: field.kind,
        source_value: field.value,
        widget_objects: field.widget_objects,
        removed_tree_objects: removed_tree_ids.into_iter().map(Into::into).collect(),
        save_mode: SaveMode::FullRewrite,
        signature_effect,
        validation,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AnnotationContainer {
    Missing,
    Direct,
    Indirect(ObjectId),
}

impl AnnotationContainer {
    fn object_id(self) -> Option<ObjectId> {
        match self {
            Self::Indirect(id) => Some(id),
            Self::Missing | Self::Direct => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkAnnotationCollection {
    page_id: ObjectId,
    container: AnnotationContainer,
    annotation_ids: Vec<ObjectId>,
    sha256: String,
}

fn request_page_number(mutation: &LinkMutation) -> u32 {
    match mutation {
        LinkMutation::Create { page, .. } => page.page_number,
        LinkMutation::Update { selector, .. } | LinkMutation::Delete { selector } => {
            selector.page_number
        }
    }
}

fn annotation_collection_sha256(
    container: AnnotationContainer,
    annotation_ids: &[ObjectId],
) -> String {
    let mut hasher = Sha256::new();
    match container {
        AnnotationContainer::Missing => hasher.update(b"MISSING"),
        AnnotationContainer::Direct => hasher.update(b"DIRECT"),
        AnnotationContainer::Indirect((number, generation)) => {
            hasher.update(b"INDIRECT");
            hasher.update(number.to_be_bytes());
            hasher.update(generation.to_be_bytes());
        }
    }
    for (number, generation) in annotation_ids {
        hasher.update(number.to_be_bytes());
        hasher.update(generation.to_be_bytes());
    }
    format!("{:x}", hasher.finalize())
}

fn link_annotation_collection(
    document: &Document,
    page_id: ObjectId,
) -> Result<LinkAnnotationCollection, PdfEditError> {
    let annotations = document
        .get_dictionary(page_id)
        .map_err(|_| PdfEditError::LinkUnsupported)?
        .get(b"Annots")
        .ok()
        .cloned();
    let (container, values) = match annotations {
        None => (AnnotationContainer::Missing, Vec::new()),
        Some(Object::Array(values)) => (AnnotationContainer::Direct, values),
        Some(Object::Reference(id)) => (
            AnnotationContainer::Indirect(id),
            document
                .get_object(id)
                .and_then(Object::as_array)
                .map_err(|_| PdfEditError::LinkUnsupported)?
                .clone(),
        ),
        Some(_) => return Err(PdfEditError::LinkUnsupported),
    };
    if values.len() > MAX_EDITABLE_LINKS {
        return Err(PdfEditError::LinkUnsupported);
    }
    let annotation_ids = values
        .iter()
        .map(|value| {
            value
                .as_reference()
                .map_err(|_| PdfEditError::LinkUnsupported)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let sha256 = annotation_collection_sha256(container, &annotation_ids);
    Ok(LinkAnnotationCollection {
        page_id,
        container,
        annotation_ids,
        sha256,
    })
}

fn selected_link_page(
    document: &Document,
    selector: &LinkPageSelector,
) -> Result<LinkAnnotationCollection, PdfEditError> {
    let selected_page_id = page_id(document, selector.page_number)?;
    if selected_page_id != ObjectId::from(selector.page_object) {
        return Err(PdfEditError::StaleLink);
    }
    let collection = link_annotation_collection(document, selected_page_id)?;
    if collection.container.object_id().map(Into::into) != selector.annotations_object
        || collection.sha256 != selector.expected_annotations_sha256
    {
        return Err(PdfEditError::StaleLink);
    }
    Ok(collection)
}

fn selected_link(
    document: &Document,
    selector: &LinkSelector,
) -> Result<(LinkAnnotationCollection, EditableLink), PdfEditError> {
    let selected_page_id = page_id(document, selector.page_number)?;
    if selected_page_id != ObjectId::from(selector.page_object) {
        return Err(PdfEditError::StaleLink);
    }
    let collection = link_annotation_collection(document, selected_page_id)?;
    let annotation_id = collection
        .annotation_ids
        .get(selector.annotation_index)
        .copied()
        .ok_or(PdfEditError::LinkNotFound)?;
    if annotation_id != ObjectId::from(selector.annotation_object) {
        return Err(PdfEditError::StaleLink);
    }
    let editable = editable_link_from_id(
        document,
        selector.page_number,
        selected_page_id,
        annotation_id,
        selector.annotation_index,
    )?;
    if editable.selector.expected_annotation_sha256 != selector.expected_annotation_sha256
        || !pdf_boxes_equal(editable.rectangle, selector.expected_rectangle)
        || editable.target != selector.expected_target
    {
        return Err(PdfEditError::StaleLink);
    }
    Ok((collection, editable))
}

fn editable_link_from_id(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
    annotation_id: ObjectId,
    annotation_index: usize,
) -> Result<EditableLink, PdfEditError> {
    let object = document
        .get_object(annotation_id)
        .map_err(|_| PdfEditError::LinkNotFound)?;
    let annotation = object
        .as_dict()
        .map_err(|_| PdfEditError::LinkUnsupported)?;
    if !annotation
        .get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|name| name == b"Link")
    {
        return Err(PdfEditError::LinkNotFound);
    }
    let rectangle = annotation
        .get(b"Rect")
        .cloned()
        .map_err(|_| PdfEditError::LinkUnsupported)
        .and_then(|value| pdf_box_from_object(document, value))?;
    if !valid_pdf_box(rectangle) {
        return Err(PdfEditError::LinkUnsupported);
    }
    let target = link_target_from_annotation(document, annotation)?;
    let expected_annotation_sha256 =
        canonical_object_sha256(object).ok_or(PdfEditError::LinkUnsupported)?;
    Ok(EditableLink {
        selector: LinkSelector {
            page_number,
            page_object: page_id.into(),
            annotation_object: annotation_id.into(),
            annotation_index,
            expected_annotation_sha256,
            expected_rectangle: rectangle,
            expected_target: target.clone(),
        },
        rectangle,
        target,
    })
}

fn link_target_from_annotation(
    document: &Document,
    annotation: &Dictionary,
) -> Result<PdfLinkTarget, PdfEditError> {
    if let Ok(destination) = annotation.get(b"Dest") {
        if annotation.get(b"A").is_ok() {
            return Err(PdfEditError::LinkUnsupported);
        }
        return internal_link_target(document, destination);
    }
    let action = annotation
        .get(b"A")
        .and_then(Object::as_dict)
        .map_err(|_| PdfEditError::LinkUnsupported)?;
    match action
        .get(b"S")
        .and_then(Object::as_name)
        .map_err(|_| PdfEditError::LinkUnsupported)?
    {
        b"URI" => {
            let raw = action
                .get(b"URI")
                .and_then(Object::as_str)
                .map_err(|_| PdfEditError::LinkUnsupported)?;
            let url = String::from_utf8(raw.to_vec()).map_err(|_| PdfEditError::LinkUnsupported)?;
            validated_external_url(&url).map(|url| PdfLinkTarget::ExternalUrl { url })
        }
        b"GoTo" => action
            .get(b"D")
            .map_err(|_| PdfEditError::LinkUnsupported)
            .and_then(|destination| internal_link_target(document, destination)),
        _ => Err(PdfEditError::LinkUnsupported),
    }
}

fn internal_link_target(
    document: &Document,
    destination: &Object,
) -> Result<PdfLinkTarget, PdfEditError> {
    let values = destination
        .as_array()
        .map_err(|_| PdfEditError::LinkUnsupported)?;
    if values.len() != 2
        || !values[1]
            .as_name()
            .is_ok_and(|destination_type| destination_type == b"Fit")
    {
        return Err(PdfEditError::LinkUnsupported);
    }
    let target_id = values[0]
        .as_reference()
        .map_err(|_| PdfEditError::LinkUnsupported)?;
    let page_number = document
        .get_pages()
        .into_iter()
        .find_map(|(page_number, page_id)| (page_id == target_id).then_some(page_number))
        .ok_or(PdfEditError::LinkUnsupported)?;
    Ok(PdfLinkTarget::InternalPage { page_number })
}

fn validated_link_target(
    document: &Document,
    target: &PdfLinkTarget,
) -> Result<PdfLinkTarget, PdfEditError> {
    match target {
        PdfLinkTarget::ExternalUrl { url } => {
            validated_external_url(url).map(|url| PdfLinkTarget::ExternalUrl { url })
        }
        PdfLinkTarget::InternalPage { page_number } => {
            page_id(document, *page_number)?;
            Ok(target.clone())
        }
    }
}

fn validated_external_url(value: &str) -> Result<String, PdfEditError> {
    if value.is_empty()
        || value.len() > MAX_EXTERNAL_LINK_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(PdfEditError::InvalidLinkProtocol);
    }
    let parsed = url::Url::parse(value).map_err(|_| PdfEditError::InvalidLinkProtocol)?;
    match parsed.scheme() {
        "http" | "https"
            if parsed.host_str().is_some()
                && parsed.username().is_empty()
                && parsed.password().is_none() => {}
        "mailto" if !parsed.path().is_empty() && parsed.path().contains('@') => {}
        _ => return Err(PdfEditError::InvalidLinkProtocol),
    }
    Ok(parsed.to_string())
}

enum LinkTargetPdfObject {
    Action(Object),
}

fn link_target_pdf_object(
    document: &Document,
    target: &PdfLinkTarget,
) -> Result<LinkTargetPdfObject, PdfEditError> {
    match target {
        PdfLinkTarget::ExternalUrl { url } => {
            let mut action = Dictionary::new();
            action.set("S", Object::Name(b"URI".to_vec()));
            action.set("URI", Object::string_literal(url.as_bytes().to_vec()));
            Ok(LinkTargetPdfObject::Action(Object::Dictionary(action)))
        }
        PdfLinkTarget::InternalPage { page_number } => {
            let mut action = Dictionary::new();
            action.set("S", Object::Name(b"GoTo".to_vec()));
            action.set(
                "D",
                Object::Array(vec![
                    page_id(document, *page_number)?.into(),
                    Object::Name(b"Fit".to_vec()),
                ]),
            );
            Ok(LinkTargetPdfObject::Action(Object::Dictionary(action)))
        }
    }
}

fn set_link_target_object(annotation: &mut Dictionary, target: LinkTargetPdfObject) {
    annotation.remove(b"A");
    annotation.remove(b"Dest");
    match target {
        LinkTargetPdfObject::Action(action) => annotation.set("A", action),
    }
}

fn set_page_annotation_ids(
    document: &mut Document,
    page_id: ObjectId,
    container: AnnotationContainer,
    annotation_ids: &[ObjectId],
) -> Result<(), PdfEditError> {
    let values = Object::Array(annotation_ids.iter().copied().map(Into::into).collect());
    match container {
        AnnotationContainer::Missing | AnnotationContainer::Direct => document
            .get_dictionary_mut(page_id)
            .map_err(|_| PdfEditError::LinkUnsupported)?
            .set("Annots", values),
        AnnotationContainer::Indirect(container_id) => {
            *document
                .get_object_mut(container_id)
                .map_err(|_| PdfEditError::LinkUnsupported)? = values;
        }
    }
    Ok(())
}

type LinkInspection = (Vec<EditableLink>, bool, Vec<EditableLinkPage>, usize, usize);

fn inspect_links(document: &Document) -> Result<LinkInspection, PdfEditError> {
    let mut editable = Vec::new();
    let mut editable_pages = Vec::new();
    let mut unsupported_links = 0;
    let mut unsupported_pages = 0;
    let mut truncated = false;
    for (page_number, page_id) in document.get_pages() {
        let collection = match link_annotation_collection(document, page_id) {
            Ok(collection) => collection,
            Err(PdfEditError::LinkUnsupported) => {
                unsupported_pages += 1;
                continue;
            }
            Err(error) => return Err(error),
        };
        let mut link_count = 0;
        let mut page_supported = true;
        for (index, annotation_id) in collection.annotation_ids.iter().copied().enumerate() {
            let annotation = match document.get_dictionary(annotation_id) {
                Ok(annotation) => annotation,
                Err(_) => {
                    page_supported = false;
                    break;
                }
            };
            if !annotation
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Link")
            {
                continue;
            }
            link_count += 1;
            match editable_link_from_id(document, page_number, page_id, annotation_id, index) {
                Ok(link) if editable.len() < MAX_EDITABLE_LINKS => editable.push(link),
                Ok(_) => truncated = true,
                Err(PdfEditError::LinkUnsupported | PdfEditError::InvalidLinkProtocol) => {
                    unsupported_links += 1;
                }
                Err(error) => return Err(error),
            }
        }
        if page_supported {
            editable_pages.push(EditableLinkPage {
                selector: LinkPageSelector {
                    page_number,
                    page_object: page_id.into(),
                    annotations_object: collection.container.object_id().map(Into::into),
                    expected_annotations_sha256: collection.sha256,
                },
                annotation_count: collection.annotation_ids.len(),
                link_count,
            });
        } else {
            unsupported_pages += 1;
        }
    }
    Ok((
        editable,
        truncated,
        editable_pages,
        unsupported_links,
        unsupported_pages,
    ))
}

fn canonical_object_sha256(object: &Object) -> Option<String> {
    let mut hasher = Sha256::new();
    canonical_object_digest(&mut hasher, object, 0)?;
    Some(format!("{:x}", hasher.finalize()))
}

fn canonical_object_digest(hasher: &mut Sha256, object: &Object, depth: usize) -> Option<()> {
    if depth > MAX_PDF_OBJECT_NESTING {
        return None;
    }
    match object {
        Object::Null => hasher.update([0]),
        Object::Boolean(value) => hasher.update([1, u8::from(*value)]),
        Object::Integer(value) => {
            hasher.update([2]);
            hasher.update(value.to_be_bytes());
        }
        Object::Real(value) => {
            hasher.update([3]);
            hasher.update(value.to_bits().to_be_bytes());
        }
        Object::Name(value) => {
            hasher.update([4]);
            digest_bytes(hasher, value);
        }
        Object::String(value, format) => {
            hasher.update([5]);
            hasher.update([u8::from(*format == StringFormat::Hexadecimal)]);
            digest_bytes(hasher, value);
        }
        Object::Array(values) => {
            hasher.update([6]);
            hasher.update((values.len() as u64).to_be_bytes());
            for value in values {
                canonical_object_digest(hasher, value, depth + 1)?;
            }
        }
        Object::Dictionary(dictionary) => {
            hasher.update([7]);
            digest_dictionary(hasher, dictionary, depth + 1)?;
        }
        Object::Stream(stream) => {
            hasher.update([8]);
            digest_dictionary(hasher, &stream.dict, depth + 1)?;
            digest_bytes(hasher, &stream.content);
        }
        Object::Reference((number, generation)) => {
            hasher.update([9]);
            hasher.update(number.to_be_bytes());
            hasher.update(generation.to_be_bytes());
        }
    }
    Some(())
}

fn digest_dictionary(hasher: &mut Sha256, dictionary: &Dictionary, depth: usize) -> Option<()> {
    let mut entries = dictionary.iter().collect::<Vec<_>>();
    entries.sort_by_key(|(key, _)| (*key).clone());
    hasher.update((entries.len() as u64).to_be_bytes());
    for (key, value) in entries {
        digest_bytes(hasher, key);
        canonical_object_digest(hasher, value, depth + 1)?;
    }
    Some(())
}

fn digest_bytes(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ValidatedJpeg {
    width: u32,
    height: u32,
    color_space: &'static str,
}

fn validate_replacement_jpeg(bytes: &[u8]) -> Result<ValidatedJpeg, PdfEditError> {
    if bytes.is_empty() || bytes.len() as u64 > MAX_REPLACEMENT_IMAGE_BYTES {
        return Err(PdfEditError::ReplacementImageInvalid);
    }
    let mut decoder = JpegDecoder::new(Cursor::new(bytes));
    decoder.set_max_decoding_buffer_size(MAX_REPLACEMENT_IMAGE_DECODED_BYTES);
    decoder
        .read_info()
        .map_err(|_| PdfEditError::ReplacementImageInvalid)?;
    let info = decoder
        .info()
        .ok_or(PdfEditError::ReplacementImageInvalid)?;
    let color_space = match info.pixel_format {
        PixelFormat::L8 => "DeviceGray",
        PixelFormat::RGB24 => "DeviceRGB",
        PixelFormat::L16 | PixelFormat::CMYK32 => {
            return Err(PdfEditError::ReplacementImageInvalid);
        }
    };
    let decoded_size = usize::from(info.width)
        .checked_mul(usize::from(info.height))
        .and_then(|pixels| pixels.checked_mul(info.pixel_format.pixel_bytes()))
        .ok_or(PdfEditError::ReplacementImageInvalid)?;
    if decoded_size == 0 || decoded_size > MAX_REPLACEMENT_IMAGE_DECODED_BYTES {
        return Err(PdfEditError::ReplacementImageInvalid);
    }
    let decoded = decoder
        .decode()
        .map_err(|_| PdfEditError::ReplacementImageInvalid)?;
    if decoded.len() != decoded_size {
        return Err(PdfEditError::ReplacementImageInvalid);
    }
    Ok(ValidatedJpeg {
        width: u32::from(info.width),
        height: u32::from(info.height),
        color_space,
    })
}

fn root_pages_id(document: &Document) -> Result<ObjectId, PdfEditError> {
    let pages_id = document
        .catalog()
        .and_then(|catalog| catalog.get(b"Pages"))
        .and_then(Object::as_reference)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    let pages = document
        .get_dictionary(pages_id)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    if !pages.has_type(b"Pages") {
        return Err(PdfEditError::PageTreeUnsupported);
    }
    Ok(pages_id)
}

fn inherited_page_property(
    document: &Document,
    page_id: ObjectId,
    key: &[u8],
) -> Result<Option<Object>, PdfEditError> {
    let mut current_id = page_id;
    let mut visited = BTreeSet::new();
    for _ in 0..=MAX_PAGE_COUNT + 1 {
        if !visited.insert(current_id) {
            return Err(PdfEditError::PageTreeUnsupported);
        }
        let dictionary = document
            .get_dictionary(current_id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?;
        if let Ok(value) = dictionary.get(key) {
            return Ok(Some(value.clone()));
        }
        current_id = match dictionary.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent_id) => parent_id,
            Err(_) => return Ok(None),
        };
    }
    Err(PdfEditError::PageTreeUnsupported)
}

fn inspect_page_geometries(
    document: &Document,
) -> Result<(Vec<EditablePageGeometry>, usize), PdfEditError> {
    let mut editable = Vec::new();
    let mut unsupported = 0;
    for (page_number, page_id) in document.get_pages() {
        match page_geometry(document, page_number, page_id) {
            Ok(geometry) => editable.push(geometry),
            Err(PdfEditError::InvalidPageGeometry | PdfEditError::PageTreeUnsupported) => {
                unsupported += 1;
            }
            Err(error) => return Err(error),
        }
    }
    Ok((editable, unsupported))
}

fn page_geometry(
    document: &Document,
    page_number: u32,
    page_id: ObjectId,
) -> Result<EditablePageGeometry, PdfEditError> {
    let media_box = inherited_page_property(document, page_id, b"MediaBox")?
        .ok_or(PdfEditError::InvalidPageGeometry)
        .and_then(|value| pdf_box_from_object(document, value))?;
    let crop_box = inherited_page_property(document, page_id, b"CropBox")?
        .map(|value| pdf_box_from_object(document, value))
        .transpose()?
        .unwrap_or(media_box);
    let rotation = inherited_page_property(document, page_id, b"Rotate")?
        .map(|value| page_rotation_from_object(document, value))
        .transpose()?
        .unwrap_or(0);
    validate_page_geometry(media_box, crop_box, rotation)?;
    Ok(EditablePageGeometry {
        selector: PageGeometrySelector {
            page_number,
            page_object: page_id.into(),
            expected_media_box: media_box,
            expected_crop_box: crop_box,
            expected_rotation: rotation,
        },
        media_box,
        crop_box,
        rotation,
    })
}

fn resolve_page_property_object(
    document: &Document,
    mut value: Object,
) -> Result<Object, PdfEditError> {
    let mut visited = BTreeSet::new();
    for _ in 0..16 {
        let Object::Reference(id) = value else {
            return Ok(value);
        };
        if !visited.insert(id) {
            return Err(PdfEditError::PageTreeUnsupported);
        }
        value = document
            .get_object(id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?
            .clone();
    }
    Err(PdfEditError::PageTreeUnsupported)
}

fn pdf_box_from_object(document: &Document, value: Object) -> Result<PdfBox, PdfEditError> {
    let resolved = resolve_page_property_object(document, value)?;
    let values = resolved
        .as_array()
        .map_err(|_| PdfEditError::InvalidPageGeometry)?;
    if values.len() != 4 {
        return Err(PdfEditError::InvalidPageGeometry);
    }
    let parsed = values
        .iter()
        .map(pdf_number)
        .collect::<Option<Vec<_>>>()
        .ok_or(PdfEditError::InvalidPageGeometry)?;
    Ok(PdfBox {
        left: parsed[0],
        bottom: parsed[1],
        right: parsed[2],
        top: parsed[3],
    })
}

fn page_rotation_from_object(document: &Document, value: Object) -> Result<i32, PdfEditError> {
    let resolved = resolve_page_property_object(document, value)?;
    let raw = resolved
        .as_i64()
        .map_err(|_| PdfEditError::InvalidPageGeometry)?;
    if raw % 90 != 0 {
        return Err(PdfEditError::InvalidPageGeometry);
    }
    i32::try_from(raw.rem_euclid(360)).map_err(|_| PdfEditError::InvalidPageGeometry)
}

fn validate_page_geometry(
    media_box: PdfBox,
    crop_box: PdfBox,
    rotation: i32,
) -> Result<(), PdfEditError> {
    if ![0, 90, 180, 270].contains(&rotation)
        || !valid_pdf_box(media_box)
        || !valid_pdf_box(crop_box)
        || crop_box.left < media_box.left
        || crop_box.bottom < media_box.bottom
        || crop_box.right > media_box.right
        || crop_box.top > media_box.top
    {
        return Err(PdfEditError::InvalidPageGeometry);
    }
    Ok(())
}

fn valid_pdf_box(value: PdfBox) -> bool {
    let coordinates = [value.left, value.bottom, value.right, value.top];
    coordinates
        .iter()
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_PAGE_COORDINATE_POINTS)
        && value.right - value.left >= 1.0
        && value.top - value.bottom >= 1.0
        && value.right - value.left <= MAX_PAGE_DIMENSION_POINTS
        && value.top - value.bottom <= MAX_PAGE_DIMENSION_POINTS
}

fn pdf_box_object(value: PdfBox) -> Result<Object, PdfEditError> {
    validate_page_geometry(value, value, 0)?;
    Ok(Object::Array(
        [value.left, value.bottom, value.right, value.top]
            .into_iter()
            .map(pdf_number_object)
            .collect::<Result<Vec<_>, _>>()?,
    ))
}

fn pdf_boxes_equal(left: PdfBox, right: PdfBox) -> bool {
    const EPSILON: f64 = 0.001;
    (left.left - right.left).abs() <= EPSILON
        && (left.bottom - right.bottom).abs() <= EPSILON
        && (left.right - right.right).abs() <= EPSILON
        && (left.top - right.top).abs() <= EPSILON
}

fn selected_page_id(
    pages: &BTreeMap<u32, ObjectId>,
    selector: PageIdentitySelector,
) -> Result<ObjectId, PdfEditError> {
    let selected = pages
        .get(&selector.page_number)
        .copied()
        .ok_or(PdfEditError::PageNotFound)?;
    if selected != ObjectId::from(selector.page_object) {
        return Err(PdfEditError::StalePageMutation);
    }
    Ok(selected)
}

fn validate_insert_position(position: u32, page_count: usize) -> Result<(), PdfEditError> {
    if position == 0 || position as usize > page_count + 1 {
        return Err(PdfEditError::InvalidPageMutation);
    }
    Ok(())
}

fn page_has_external_inbound_reference(document: &Document, page_id: ObjectId) -> bool {
    document.objects.iter().any(|(object_id, object)| {
        if *object_id == page_id || !object_references_target(object, page_id, 0) {
            return false;
        }
        !object_dictionary(object).is_some_and(|dictionary| dictionary.has_type(b"Pages"))
    })
}

fn object_references_target(object: &Object, target: ObjectId, depth: usize) -> bool {
    if depth > MAX_PDF_OBJECT_NESTING {
        return true;
    }
    match object {
        Object::Reference(object_id) => *object_id == target,
        Object::Array(values) => values
            .iter()
            .any(|value| object_references_target(value, target, depth + 1)),
        Object::Dictionary(dictionary) => dictionary
            .iter()
            .any(|(_, value)| object_references_target(value, target, depth + 1)),
        Object::Stream(stream) => stream
            .dict
            .iter()
            .any(|(_, value)| object_references_target(value, target, depth + 1)),
        _ => false,
    }
}

fn rewrite_flat_page_tree(
    document: &mut Document,
    source_page_order: &[ObjectId],
    result_page_order: &[ObjectId],
    root_pages_id: ObjectId,
) -> Result<(), PdfEditError> {
    const INHERITABLE_PAGE_KEYS: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
    let retained: BTreeSet<ObjectId> = result_page_order.iter().copied().collect();
    let mut inherited: BTreeMap<ObjectId, Vec<(Vec<u8>, Object)>> = BTreeMap::new();
    for page_id in source_page_order
        .iter()
        .copied()
        .filter(|page_id| retained.contains(page_id))
    {
        let mut values = Vec::new();
        for key in INHERITABLE_PAGE_KEYS {
            if let Some(value) = inherited_page_property(document, page_id, key)? {
                values.push((key.to_vec(), value));
            }
        }
        inherited.insert(page_id, values);
    }

    for page_id in result_page_order {
        let page = document
            .get_dictionary_mut(*page_id)
            .map_err(|_| PdfEditError::PageTreeUnsupported)?;
        if !page.has_type(b"Page") {
            return Err(PdfEditError::PageTreeUnsupported);
        }
        for (key, value) in inherited.get(page_id).into_iter().flatten() {
            if !page.has(key) {
                page.set(key.clone(), value.clone());
            }
        }
        page.set("Parent", root_pages_id);
    }

    let root_pages = document
        .get_dictionary_mut(root_pages_id)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    if !root_pages.has_type(b"Pages") {
        return Err(PdfEditError::PageTreeUnsupported);
    }
    root_pages.set(
        "Kids",
        result_page_order
            .iter()
            .copied()
            .map(Object::Reference)
            .collect::<Vec<_>>(),
    );
    root_pages.set("Count", result_page_order.len() as i64);
    Ok(())
}

fn clone_page_into_document(
    source: &Document,
    source_page_id: ObjectId,
    destination: &mut Document,
    parent_pages_id: ObjectId,
) -> Result<(ObjectId, Vec<ObjectId>), PdfEditError> {
    let source_page = source
        .get_dictionary(source_page_id)
        .map_err(|_| PdfEditError::PageTreeUnsupported)?;
    if !source_page.has_type(b"Page") {
        return Err(PdfEditError::PageTreeUnsupported);
    }
    const UNSUPPORTED_PAGE_DEPENDENCIES: [&[u8]; 7] = [
        b"AA",
        b"B",
        b"StructParents",
        b"SeparationInfo",
        b"PresSteps",
        b"VP",
        b"Thumb",
    ];
    if UNSUPPORTED_PAGE_DEPENDENCIES
        .iter()
        .any(|key| source_page.has(key))
    {
        return Err(PdfEditError::PageDependencyUnsupported);
    }
    if let Ok(annotations) = source_page.get(b"Annots") {
        let annotations = resolve_page_property_object(source, annotations.clone())?;
        if !annotations
            .as_array()
            .is_ok_and(|annotations| annotations.is_empty())
        {
            return Err(PdfEditError::PageDependencyUnsupported);
        }
    }

    let geometry = page_geometry(source, 1, source_page_id)?;
    let mut materialized_page = source_page.clone();
    materialized_page.remove(b"Parent");
    materialized_page.remove(b"Annots");
    materialized_page.set("MediaBox", pdf_box_object(geometry.media_box)?);
    materialized_page.set("CropBox", pdf_box_object(geometry.crop_box)?);
    materialized_page.set("Rotate", i64::from(geometry.rotation));
    if !materialized_page.has(b"Resources") {
        if let Some(resources) = inherited_page_property(source, source_page_id, b"Resources")? {
            materialized_page.set("Resources", resources);
        } else {
            materialized_page.set("Resources", Dictionary::new());
        }
    }

    let new_page_id = destination.new_object_id();
    let mut mapping = BTreeMap::from([(source_page_id, new_page_id)]);
    let mut allocated = vec![new_page_id];
    destination.set_object(new_page_id, Object::Null);
    let cloned_page = clone_page_object(
        source,
        destination,
        &Object::Dictionary(materialized_page),
        &mut mapping,
        &mut allocated,
        0,
    )?;
    let mut cloned_page = cloned_page
        .as_dict()
        .map_err(|_| PdfEditError::PageTreeUnsupported)?
        .clone();
    cloned_page.set("Type", "Page");
    cloned_page.set("Parent", parent_pages_id);
    destination.set_object(new_page_id, cloned_page);
    Ok((new_page_id, allocated))
}

fn clone_page_object(
    source: &Document,
    destination: &mut Document,
    object: &Object,
    mapping: &mut BTreeMap<ObjectId, ObjectId>,
    allocated: &mut Vec<ObjectId>,
    depth: usize,
) -> Result<Object, PdfEditError> {
    if depth > MAX_PDF_OBJECT_NESTING || allocated.len() > MAX_PAGE_CLONE_OBJECTS {
        return Err(PdfEditError::PageTreeUnsupported);
    }
    match object {
        Object::Reference(source_id) => {
            if let Some(destination_id) = mapping.get(source_id) {
                return Ok(Object::Reference(*destination_id));
            }
            let source_object = source
                .objects
                .get(source_id)
                .ok_or(PdfEditError::PageTreeUnsupported)?;
            if object_dictionary(source_object).is_some_and(|dictionary| {
                [b"Page".as_slice(), b"Pages", b"Catalog", b"Outlines"]
                    .iter()
                    .any(|object_type| dictionary.has_type(object_type))
            }) {
                return Err(PdfEditError::PageDependencyUnsupported);
            }
            if allocated.len() >= MAX_PAGE_CLONE_OBJECTS {
                return Err(PdfEditError::PageTreeUnsupported);
            }
            let destination_id = destination.new_object_id();
            mapping.insert(*source_id, destination_id);
            allocated.push(destination_id);
            destination.set_object(destination_id, Object::Null);
            let cloned = clone_page_object(
                source,
                destination,
                source_object,
                mapping,
                allocated,
                depth + 1,
            )?;
            destination.set_object(destination_id, cloned);
            Ok(Object::Reference(destination_id))
        }
        Object::Array(values) => Ok(Object::Array(
            values
                .iter()
                .map(|value| {
                    clone_page_object(source, destination, value, mapping, allocated, depth + 1)
                })
                .collect::<Result<Vec<_>, _>>()?,
        )),
        Object::Dictionary(dictionary) => Ok(Object::Dictionary(clone_page_dictionary(
            source,
            destination,
            dictionary,
            mapping,
            allocated,
            depth + 1,
        )?)),
        Object::Stream(stream) => {
            let mut cloned = stream.clone();
            cloned.dict = clone_page_dictionary(
                source,
                destination,
                &stream.dict,
                mapping,
                allocated,
                depth + 1,
            )?;
            Ok(Object::Stream(cloned))
        }
        _ => Ok(object.clone()),
    }
}

fn clone_page_dictionary(
    source: &Document,
    destination: &mut Document,
    dictionary: &Dictionary,
    mapping: &mut BTreeMap<ObjectId, ObjectId>,
    allocated: &mut Vec<ObjectId>,
    depth: usize,
) -> Result<Dictionary, PdfEditError> {
    let mut cloned = Dictionary::new();
    for (key, value) in dictionary.iter() {
        cloned.set(
            key.clone(),
            clone_page_object(source, destination, value, mapping, allocated, depth + 1)?,
        );
    }
    Ok(cloned)
}

fn all_object_references_resolve(document: &Document) -> bool {
    object_references_resolve(
        &Object::Dictionary(document.trailer.clone()),
        &document.objects,
        0,
    ) && document
        .objects
        .values()
        .all(|object| object_references_resolve(object, &document.objects, 0))
}

fn object_references_resolve(
    object: &Object,
    objects: &BTreeMap<ObjectId, Object>,
    depth: usize,
) -> bool {
    if depth > MAX_PDF_OBJECT_NESTING {
        return false;
    }
    match object {
        Object::Reference(object_id) => objects.contains_key(object_id),
        Object::Array(values) => values
            .iter()
            .all(|value| object_references_resolve(value, objects, depth + 1)),
        Object::Dictionary(dictionary) => dictionary
            .iter()
            .all(|(_, value)| object_references_resolve(value, objects, depth + 1)),
        Object::Stream(stream) => stream
            .dict
            .iter()
            .all(|(_, value)| object_references_resolve(value, objects, depth + 1)),
        _ => true,
    }
}

fn checked_metadata(path: &Path) -> Result<fs::Metadata, PdfEditError> {
    let metadata = fs::metadata(path).map_err(|_| PdfEditError::InputUnavailable)?;
    if !metadata.is_file() {
        return Err(PdfEditError::InputUnavailable);
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err(PdfEditError::InputTooLarge);
    }
    Ok(metadata)
}

fn load_checked_pdf(path: &Path) -> Result<Document, PdfEditError> {
    let document = Document::load_with_options(
        path,
        LoadOptions::with_max_decompressed_size(MAX_DECOMPRESSED_STREAM_BYTES),
    )
    .map_err(|_| PdfEditError::InvalidPdf)?;
    if document.is_encrypted() {
        return Err(PdfEditError::EncryptedPdf);
    }
    if document.get_pages().len() > MAX_PAGE_COUNT {
        return Err(PdfEditError::PageLimitExceeded);
    }
    Ok(document)
}

fn validate_paths(input: &Path, output: &Path) -> Result<(), PdfEditError> {
    if output.exists() {
        return Err(PdfEditError::OutputAlreadyExists);
    }
    let input = input
        .canonicalize()
        .map_err(|_| PdfEditError::InputUnavailable)?;
    let output_parent = output.parent().ok_or(PdfEditError::OutputWriteFailed)?;
    let output_parent = output_parent
        .canonicalize()
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let output_name = output.file_name().ok_or(PdfEditError::OutputWriteFailed)?;
    if input == output_parent.join(output_name) {
        return Err(PdfEditError::OutputAliasesInput);
    }
    Ok(())
}

fn page_id(document: &Document, page_number: u32) -> Result<ObjectId, PdfEditError> {
    document
        .get_pages()
        .get(&page_number)
        .copied()
        .ok_or(PdfEditError::PageNotFound)
}

#[derive(Debug, Clone)]
struct SimpleFontMetrics {
    first_char: u8,
    widths: Vec<f64>,
    ascent: f64,
    descent: f64,
    source: String,
}

#[derive(Debug, Clone)]
struct TextRunMeasurement {
    bounding_box: PdfBox,
    metrics_source: String,
}

fn simple_font_metrics(document: &Document, font: &Dictionary) -> Option<SimpleFontMetrics> {
    let subtype = font.get(b"Subtype").ok()?.as_name().ok()?;
    if subtype != b"Type1" && subtype != b"TrueType" {
        return None;
    }
    let first_char = u8::try_from(font.get(b"FirstChar").ok()?.as_i64().ok()?).ok()?;
    let widths = font
        .get(b"Widths")
        .ok()?
        .as_array()
        .ok()?
        .iter()
        .map(pdf_number)
        .collect::<Option<Vec<_>>>()?;
    if widths.is_empty()
        || widths
            .iter()
            .any(|width| !width.is_finite() || *width < 0.0)
    {
        return None;
    }
    let descriptor = font
        .get(b"FontDescriptor")
        .ok()
        .and_then(|value| dereference_dictionary(document, value))?;
    let ascent = pdf_number(descriptor.get(b"Ascent").ok()?)?;
    let descent = pdf_number(descriptor.get(b"Descent").ok()?)?;
    if !ascent.is_finite() || !descent.is_finite() || ascent <= 0.0 || descent >= 0.0 {
        return None;
    }
    Some(SimpleFontMetrics {
        first_char,
        widths,
        ascent,
        descent,
        source: "PDF_WIDTHS_FONT_DESCRIPTOR".to_string(),
    })
}

fn simple_text_advance_units(metrics: &SimpleFontMetrics, bytes: &[u8]) -> Option<f64> {
    bytes.iter().try_fold(0.0, |total, byte| {
        let index = usize::from(byte.checked_sub(metrics.first_char)?);
        metrics.widths.get(index).map(|width| total + width)
    })
}

fn horizontal_unit_text_matrix(matrix: &TextMatrix) -> bool {
    nearly_equal(matrix.a, 1.0)
        && nearly_equal(matrix.b, 0.0)
        && nearly_equal(matrix.c, 0.0)
        && nearly_equal(matrix.d, 1.0)
        && valid_pdf_coordinate(matrix.e)
        && valid_pdf_coordinate(matrix.f)
}

fn measure_simple_text_run(
    document: &Document,
    font: &Dictionary,
    bytes: &[u8],
    font_size: f64,
    matrix: &TextMatrix,
) -> Option<TextRunMeasurement> {
    if !font_size.is_finite()
        || !(1.0..=512.0).contains(&font_size)
        || !horizontal_unit_text_matrix(matrix)
    {
        return None;
    }
    let metrics = simple_font_metrics(document, font)?;
    let width = simple_text_advance_units(&metrics, bytes)? * font_size / 1_000.0;
    let bounding_box = PdfBox {
        left: matrix.e,
        bottom: matrix.f + metrics.descent * font_size / 1_000.0,
        right: matrix.e + width,
        top: matrix.f + metrics.ascent * font_size / 1_000.0,
    };
    valid_pdf_box(bounding_box).then_some(TextRunMeasurement {
        bounding_box,
        metrics_source: metrics.source,
    })
}

fn isolated_reflow_operation(content: &Content, operation_index: usize) -> bool {
    if operation_index < 2
        || content
            .operations
            .get(operation_index)
            .map(|value| value.operator.as_str())
            != Some("Tj")
        || content
            .operations
            .get(operation_index.saturating_sub(1))
            .map(|value| value.operator.as_str())
            != Some("Tm")
        || content
            .operations
            .iter()
            .any(|value| value.operator == "cm")
    {
        return false;
    }
    let Some(begin_index) = content.operations[..operation_index]
        .iter()
        .rposition(|value| value.operator == "BT")
    else {
        return false;
    };
    let Some(end_index) = content.operations[operation_index + 1..]
        .iter()
        .position(|value| value.operator == "ET")
        .map(|offset| operation_index + 1 + offset)
    else {
        return false;
    };
    // PDF generators such as pdf-lib commonly place fill-colour and leading
    // state inside an otherwise isolated text object, then emit a terminal T*
    // before ET. Those operators cannot change glyph metrics or paint another
    // text run, so they are safe to preserve. Text spacing, rise, scaling,
    // rendering-mode, and transform operators remain excluded because this
    // engine does not approximate their geometry.
    let prefix_is_geometry_neutral = content.operations[begin_index + 1..operation_index]
        .iter()
        .all(|value| {
            matches!(
                value.operator.as_str(),
                "Tf" | "Tm" | "TL" | "g" | "rg" | "k" | "G" | "RG" | "K"
            )
        });
    let suffix_is_terminal_leading = content.operations[operation_index + 1..end_index]
        .iter()
        .all(|value| value.operator == "T*");
    prefix_is_geometry_neutral && suffix_is_terminal_leading
}

fn pdf_box_contains(outer: &PdfBox, inner: &PdfBox) -> bool {
    outer.left <= inner.left + 0.000_001
        && outer.bottom <= inner.bottom + 0.000_001
        && outer.right + 0.000_001 >= inner.right
        && outer.top + 0.000_001 >= inner.top
}

fn pdf_boxes_intersect(left: &PdfBox, right: &PdfBox) -> bool {
    left.left < right.right - 0.000_001
        && left.right > right.left + 0.000_001
        && left.bottom < right.top - 0.000_001
        && left.top > right.bottom + 0.000_001
}

fn image_placement_bounds(matrix: PdfMatrix) -> Option<PdfBox> {
    vector_bounds(
        &[
            Operation::new("m", vec![0.into(), 0.into()]),
            Operation::new("l", vec![1.into(), 0.into()]),
            Operation::new("l", vec![1.into(), 1.into()]),
            Operation::new("l", vec![0.into(), 1.into()]),
        ],
        matrix,
    )
}

fn reflow_region_is_clear(
    document: &Document,
    selector: &TextRunSelector,
    region: &PdfBox,
) -> Result<bool, PdfEditError> {
    let (text_runs, _, _) = inspect_text_runs(document)?;
    if text_runs.iter().any(|run| {
        run.selector.page_number == selector.page_number
            && run.selector != *selector
            && run
                .bounding_box
                .as_ref()
                .is_some_and(|bounds| pdf_boxes_intersect(bounds, region))
    }) {
        return Ok(false);
    }
    let (image_placements, _, _) = inspect_image_placements(document)?;
    if image_placements.iter().any(|placement| {
        placement.selector.page_number == selector.page_number
            && image_placement_bounds(placement.matrix)
                .is_some_and(|bounds| pdf_boxes_intersect(&bounds, region))
    }) {
        return Ok(false);
    }
    let (vector_paths, _, _) = inspect_vector_paths(document)?;
    if vector_paths.iter().any(|path| {
        path.selector.page_number == selector.page_number
            && pdf_boxes_intersect(&path.bounds, region)
    }) {
        return Ok(false);
    }
    let (fields, _, _) = inspect_form_fields(document)?;
    if fields.iter().any(|field| {
        field.page_number == selector.page_number && pdf_boxes_intersect(&field.rectangle, region)
    }) {
        return Ok(false);
    }
    let (links, _, _, _, _) = inspect_links(document)?;
    if links.iter().any(|link| {
        link.selector.page_number == selector.page_number
            && pdf_boxes_intersect(&link.rectangle, region)
    }) {
        return Ok(false);
    }
    let (annotations, _, _, _, _) = inspect_annotations(document)?;
    Ok(!annotations.iter().any(|annotation| {
        annotation.selector.page_number == selector.page_number
            && pdf_boxes_intersect(&annotation.spec.rectangle, region)
    }))
}

fn encode_simple_text_exact(
    document: &Document,
    font: &Dictionary,
    text: &str,
) -> Result<Vec<u8>, PdfEditError> {
    let encoding = font
        .get_font_encoding_with_limit(document, MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let encoded = Document::encode_text(&encoding, text);
    let decoded =
        Document::decode_text(&encoding, &encoded).map_err(|_| PdfEditError::GlyphUnavailable)?;
    if decoded != text {
        return Err(PdfEditError::GlyphUnavailable);
    }
    Ok(encoded)
}

fn measure_reflow_text(
    document: &Document,
    font: &Dictionary,
    metrics: &SimpleFontMetrics,
    font_size: f64,
    text: &str,
) -> Result<(Vec<u8>, f64), PdfEditError> {
    let encoded = encode_simple_text_exact(document, font, text)?;
    let width = simple_text_advance_units(metrics, &encoded).ok_or(PdfEditError::ReflowUnsafe)?
        * font_size
        / 1_000.0;
    Ok((encoded, width))
}

fn wrap_reflow_text(
    document: &Document,
    font: &Dictionary,
    metrics: &SimpleFontMetrics,
    font_size: f64,
    text: &str,
    max_width: f64,
) -> Result<Vec<(String, Vec<u8>, f64)>, PdfEditError> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let words = paragraph.split_whitespace().collect::<Vec<_>>();
        if words.is_empty() {
            let (encoded, width) = measure_reflow_text(document, font, metrics, font_size, "")?;
            lines.push((String::new(), encoded, width));
            continue;
        }
        let mut current = String::new();
        for word in words {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            let (_, candidate_width) =
                measure_reflow_text(document, font, metrics, font_size, &candidate)?;
            if candidate_width <= max_width + 0.000_001 {
                current = candidate;
                continue;
            }
            if current.is_empty() {
                return Err(PdfEditError::ReflowUnsafe);
            }
            let (encoded, width) =
                measure_reflow_text(document, font, metrics, font_size, &current)?;
            lines.push((current, encoded, width));
            current = word.to_string();
            let (_, word_width) =
                measure_reflow_text(document, font, metrics, font_size, &current)?;
            if word_width > max_width + 0.000_001 {
                return Err(PdfEditError::ReflowUnsafe);
            }
        }
        let (encoded, width) = measure_reflow_text(document, font, metrics, font_size, &current)?;
        lines.push((current, encoded, width));
    }
    if lines.is_empty() || lines.len() > MAX_REFLOW_LINES {
        return Err(PdfEditError::ReflowUnsafe);
    }
    Ok(lines)
}

fn normalized_reflow_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn replacement_bytes_for_target(
    document: &Document,
    page_id: ObjectId,
    target_id: ObjectId,
    selector: &TextRunSelector,
    replacement: &str,
) -> Result<(Vec<u8>, Vec<u8>, String), PdfEditError> {
    let fonts = document
        .get_page_fonts(page_id)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let target_stream = document
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let plain_content = target_stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let content =
        Content::decode(&plain_content).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
        return Err(PdfEditError::ContentOperationLimitExceeded);
    }
    let mut state = text_show::FontState::default();
    // A page's content stream array is one logical stream. Font state and q/Q
    // restoration cannot be reset at each stream boundary.
    for stream_id in document.get_page_contents(page_id) {
        if stream_id == target_id {
            break;
        }
        let stream = document
            .get_object(stream_id)
            .and_then(Object::as_stream)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let bytes = stream
            .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let prior = Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        if prior.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
            return Err(PdfEditError::ContentOperationLimitExceeded);
        }
        for operation in &prior.operations {
            state.apply(operation)?;
        }
    }
    for (index, operation) in content.operations.iter().enumerate() {
        state.apply(operation)?;
        if index != selector.operation_index {
            continue;
        }
        let source_bytes = text_show::bytes(operation)?;
        let font_name = state.name.ok_or(PdfEditError::TextRunUnsupported)?;
        let font_label =
            String::from_utf8(font_name.clone()).map_err(|_| PdfEditError::TextRunUnsupported)?;
        if font_label != selector.expected_font_resource {
            return Err(PdfEditError::StaleTextRun);
        }
        let font = fonts
            .get(&font_name)
            .ok_or(PdfEditError::TextRunUnsupported)?;
        let encoding = font
            .get_font_encoding_with_limit(document, MAX_DECOMPRESSED_STREAM_BYTES)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let decoded_source = Document::decode_text(&encoding, &source_bytes)
            .map_err(|_| PdfEditError::TextRunUnsupported)?;
        if decoded_source != selector.expected_text {
            return Err(PdfEditError::StaleTextRun);
        }
        let replacement_bytes = Document::encode_text(&encoding, replacement);
        let decoded_replacement = Document::decode_text(&encoding, &replacement_bytes)
            .map_err(|_| PdfEditError::GlyphUnavailable)?;
        if decoded_replacement != replacement {
            return Err(PdfEditError::GlyphUnavailable);
        }
        return Ok((replacement_bytes, source_bytes, font_label));
    }
    Err(PdfEditError::StaleTextRun)
}

fn nearly_equal(left: f64, right: f64) -> bool {
    let scale = left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= scale * 0.000_001
}

fn matrix_equivalent(left: PdfMatrix, right: PdfMatrix) -> bool {
    nearly_equal(left.a, right.a)
        && nearly_equal(left.b, right.b)
        && nearly_equal(left.c, right.c)
        && nearly_equal(left.d, right.d)
        && nearly_equal(left.e, right.e)
        && nearly_equal(left.f, right.f)
}

fn valid_pdf_coordinate(value: f64) -> bool {
    value.is_finite() && value.abs() <= MAX_PAGE_COORDINATE_POINTS
}

fn valid_resource_name(value: &[u8]) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_image_matrix(value: PdfMatrix) -> bool {
    let width = value.a.hypot(value.b);
    let height = value.c.hypot(value.d);
    [value.a, value.b, value.c, value.d, value.e, value.f]
        .into_iter()
        .all(f64::is_finite)
        && (1.0..=MAX_PAGE_DIMENSION_POINTS).contains(&width)
        && (1.0..=MAX_PAGE_DIMENSION_POINTS).contains(&height)
        && valid_pdf_coordinate(value.e)
        && valid_pdf_coordinate(value.f)
        && (value.a * value.d - value.b * value.c).abs() > 0.000_001
}

const FULL_IMAGE_CROP: PdfNormalizedCrop = PdfNormalizedCrop {
    left: 0.0,
    bottom: 0.0,
    right: 1.0,
    top: 1.0,
};

fn valid_normalized_crop(value: PdfNormalizedCrop) -> bool {
    [value.left, value.bottom, value.right, value.top]
        .into_iter()
        .all(|coordinate| coordinate.is_finite() && (0.0..=1.0).contains(&coordinate))
        && value.right - value.left >= 0.001
        && value.top - value.bottom >= 0.001
}

fn crop_equivalent(left: PdfNormalizedCrop, right: PdfNormalizedCrop) -> bool {
    nearly_equal(left.left, right.left)
        && nearly_equal(left.bottom, right.bottom)
        && nearly_equal(left.right, right.right)
        && nearly_equal(left.top, right.top)
}

fn effective_crop(value: PdfNormalizedCrop) -> Option<PdfNormalizedCrop> {
    (!crop_equivalent(value, FULL_IMAGE_CROP)).then_some(value)
}

fn valid_image_opacity(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn opacity_equivalent(left: f64, right: f64) -> bool {
    nearly_equal(left, right)
}

fn effective_opacity(value: f64) -> Option<f64> {
    (!opacity_equivalent(value, 1.0)).then_some(value)
}

fn crop_from_operation(operation: &Operation) -> Option<PdfNormalizedCrop> {
    if operation.operator != "re" || operation.operands.len() != 4 {
        return None;
    }
    let left = pdf_number(&operation.operands[0])?;
    let bottom = pdf_number(&operation.operands[1])?;
    let width = pdf_number(&operation.operands[2])?;
    let height = pdf_number(&operation.operands[3])?;
    let crop = PdfNormalizedCrop {
        left,
        bottom,
        right: left + width,
        top: bottom + height,
    };
    valid_normalized_crop(crop).then_some(crop)
}

fn crop_operation(value: PdfNormalizedCrop) -> Result<Operation, PdfEditError> {
    if !valid_normalized_crop(value) || effective_crop(value).is_none() {
        return Err(PdfEditError::InvalidImageCrop);
    }
    Ok(Operation::new(
        "re",
        vec![
            pdf_number_object(value.left)?,
            pdf_number_object(value.bottom)?,
            pdf_number_object(value.right - value.left)?,
            pdf_number_object(value.top - value.bottom)?,
        ],
    ))
}

fn matrix_from_operation(operation: &Operation) -> Option<PdfMatrix> {
    if operation.operator != "cm" || operation.operands.len() != 6 {
        return None;
    }
    let matrix = PdfMatrix {
        a: pdf_number(&operation.operands[0])?,
        b: pdf_number(&operation.operands[1])?,
        c: pdf_number(&operation.operands[2])?,
        d: pdf_number(&operation.operands[3])?,
        e: pdf_number(&operation.operands[4])?,
        f: pdf_number(&operation.operands[5])?,
    };
    valid_image_matrix(matrix).then_some(matrix)
}

fn pdf_number_object(value: f64) -> Result<Object, PdfEditError> {
    if !valid_pdf_coordinate(value) {
        return Err(PdfEditError::InvalidImagePlacement);
    }
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return Ok(Object::Integer(value as i64));
    }
    let real = value as f32;
    if !real.is_finite() {
        return Err(PdfEditError::InvalidImagePlacement);
    }
    Ok(Object::Real(real))
}

fn set_matrix(operation: &mut Operation, destination: PdfMatrix) -> Result<(), PdfEditError> {
    if matrix_from_operation(operation).is_none() {
        return Err(PdfEditError::ImagePlacementUnsupported);
    }
    if !valid_image_matrix(destination) {
        return Err(PdfEditError::InvalidImagePlacement);
    }
    for (operand, value) in operation.operands.iter_mut().zip([
        destination.a,
        destination.b,
        destination.c,
        destination.d,
        destination.e,
        destination.f,
    ]) {
        *operand = pdf_number_object(value)?;
    }
    Ok(())
}

fn operations_equal(left: &Operation, right: &Operation) -> bool {
    left.operator == right.operator && left.operands == right.operands
}

fn page_image_resources(
    document: &Document,
    page_id: ObjectId,
) -> Result<BTreeMap<Vec<u8>, ObjectId>, PdfEditError> {
    let (direct_resources, resource_ids) = document
        .get_page_resources(page_id)
        .map_err(|_| PdfEditError::ImageUnsupported)?;
    let mut resources = Vec::new();
    if let Some(resources_dictionary) = direct_resources {
        resources.push(resources_dictionary);
    }
    for resource_id in resource_ids {
        if let Ok(resources_dictionary) = document.get_dictionary(resource_id) {
            resources.push(resources_dictionary);
        }
    }
    let mut images = BTreeMap::new();
    for resources_dictionary in resources {
        let Ok(xobjects_value) = resources_dictionary.get(b"XObject") else {
            continue;
        };
        let Some(xobjects) = dereference_dictionary(document, xobjects_value) else {
            continue;
        };
        for (resource_name, value) in xobjects.iter() {
            if images.contains_key(resource_name) {
                continue;
            }
            let Ok(object_id) = value.as_reference() else {
                continue;
            };
            let Ok(stream) = document.get_object(object_id).and_then(Object::as_stream) else {
                continue;
            };
            if stream
                .dict
                .get(b"Subtype")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Image")
            {
                images.insert(resource_name.clone(), object_id);
            }
        }
    }
    Ok(images)
}

fn page_effective_resources_clone(
    document: &Document,
    page_id: ObjectId,
) -> Result<Dictionary, PdfEditError> {
    let (direct_resources, resource_ids) = document
        .get_page_resources(page_id)
        .map_err(|_| PdfEditError::ImagePlacementUnsupported)?;
    if let Some(resources) = direct_resources {
        return Ok(resources.clone());
    }
    if let Some(resource_id) = resource_ids.first() {
        return document
            .get_dictionary(*resource_id)
            .cloned()
            .map_err(|_| PdfEditError::ImagePlacementUnsupported);
    }
    Ok(Dictionary::new())
}

fn ext_g_state_dictionary_clone(
    document: &Document,
    resources: &Dictionary,
) -> Result<Dictionary, PdfEditError> {
    match resources.get(b"ExtGState") {
        Ok(Object::Dictionary(dictionary)) => Ok(dictionary.clone()),
        Ok(Object::Reference(object_id)) => document
            .get_dictionary(*object_id)
            .cloned()
            .map_err(|_| PdfEditError::ImagePlacementUnsupported),
        Err(_) => Ok(Dictionary::new()),
        _ => Err(PdfEditError::ImagePlacementUnsupported),
    }
}

fn owned_image_opacity(
    document: &Document,
    page_id: ObjectId,
    resource_name: &[u8],
) -> Option<PdfImageOpacity> {
    if !resource_name.starts_with(b"WFImgAlpha") || !valid_resource_name(resource_name) {
        return None;
    }
    let (direct_resources, resource_ids) = document.get_page_resources(page_id).ok()?;
    let mut resources = Vec::new();
    if let Some(dictionary) = direct_resources {
        resources.push(dictionary);
    }
    for resource_id in resource_ids {
        if let Ok(dictionary) = document.get_dictionary(resource_id) {
            resources.push(dictionary);
        }
    }
    for dictionary in resources {
        let Some(ext_g_states) = dictionary
            .get(b"ExtGState")
            .ok()
            .and_then(|value| dereference_dictionary(document, value))
        else {
            continue;
        };
        let Ok(object_id) = ext_g_states
            .get(resource_name)
            .and_then(Object::as_reference)
        else {
            continue;
        };
        let Ok(state) = document.get_dictionary(object_id) else {
            continue;
        };
        if !state
            .get(b"Type")
            .and_then(Object::as_name)
            .is_ok_and(|name| name == b"ExtGState")
        {
            return None;
        }
        let non_stroking = state.get(b"ca").ok().and_then(pdf_number)?;
        let stroking = state.get(b"CA").ok().and_then(pdf_number)?;
        if !valid_image_opacity(non_stroking)
            || !opacity_equivalent(non_stroking, stroking)
            || opacity_equivalent(non_stroking, 1.0)
        {
            return None;
        }
        return Some(PdfImageOpacity {
            value: non_stroking,
            resource_name: String::from_utf8(resource_name.to_vec()).ok()?,
            ext_g_state_object: object_id.into(),
        });
    }
    None
}

type IsolatedImagePlacement = (
    Content,
    PdfMatrix,
    ObjectId,
    Option<PdfNormalizedCrop>,
    Option<PdfImageOpacity>,
);

fn isolated_image_placement(
    document: &Document,
    selector: &ImagePlacementSelector,
) -> Result<IsolatedImagePlacement, PdfEditError> {
    let draw_offset =
        1 + if selector.expected_crop.is_some() {
            3
        } else {
            0
        } + if selector.expected_opacity.is_some() {
            1
        } else {
            0
        };
    if selector.matrix_operation_index == 0
        || selector.draw_operation_index != selector.matrix_operation_index + draw_offset
    {
        return Err(PdfEditError::ImagePlacementUnsupported);
    }
    let selected_page_id = page_id(document, selector.page_number)?;
    let target_id: ObjectId = selector.content_stream_object.into();
    if !document
        .get_page_contents(selected_page_id)
        .contains(&target_id)
    {
        return Err(PdfEditError::ImagePlacementNotFound);
    }
    let resources = page_image_resources(document, selected_page_id)?;
    let image_id = resources
        .get(selector.expected_resource_name.as_bytes())
        .copied()
        .ok_or(PdfEditError::ImagePlacementNotFound)?;
    if image_id != ObjectId::from(selector.expected_image_object) {
        return Err(PdfEditError::StaleImagePlacement);
    }
    let image_stream = document
        .get_object(image_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ImagePlacementNotFound)?;
    supported_image_dimensions(image_stream)?;
    let target_stream = document
        .get_object(target_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let plain_content = target_stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    let content =
        Content::decode(&plain_content).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
    if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
        return Err(PdfEditError::ContentOperationLimitExceeded);
    }
    let before = content
        .operations
        .get(selector.matrix_operation_index - 1)
        .ok_or(PdfEditError::StaleImagePlacement)?;
    let matrix_operation = content
        .operations
        .get(selector.matrix_operation_index)
        .ok_or(PdfEditError::StaleImagePlacement)?;
    let crop = if let Some(expected_crop) = selector.expected_crop {
        let rectangle = content
            .operations
            .get(selector.matrix_operation_index + 1)
            .and_then(crop_from_operation)
            .ok_or(PdfEditError::StaleImagePlacement)?;
        let clip = content
            .operations
            .get(selector.matrix_operation_index + 2)
            .ok_or(PdfEditError::StaleImagePlacement)?;
        let end_path = content
            .operations
            .get(selector.matrix_operation_index + 3)
            .ok_or(PdfEditError::StaleImagePlacement)?;
        if !crop_equivalent(rectangle, expected_crop)
            || clip.operator != "W"
            || !clip.operands.is_empty()
            || end_path.operator != "n"
            || !end_path.operands.is_empty()
        {
            return Err(PdfEditError::StaleImagePlacement);
        }
        Some(rectangle)
    } else {
        None
    };
    let opacity_operation_index = selector.matrix_operation_index
        + 1
        + if selector.expected_crop.is_some() {
            3
        } else {
            0
        };
    let opacity = if let Some(expected_opacity) = selector.expected_opacity.as_ref() {
        let operation = content
            .operations
            .get(opacity_operation_index)
            .ok_or(PdfEditError::StaleImagePlacement)?;
        let resource_name = operation
            .operands
            .first()
            .and_then(|value| value.as_name().ok())
            .ok_or(PdfEditError::StaleImagePlacement)?;
        if operation.operator != "gs"
            || operation.operands.len() != 1
            || resource_name != expected_opacity.resource_name.as_bytes()
        {
            return Err(PdfEditError::StaleImagePlacement);
        }
        let actual = owned_image_opacity(document, selected_page_id, resource_name)
            .ok_or(PdfEditError::StaleImagePlacement)?;
        if actual.ext_g_state_object != expected_opacity.ext_g_state_object
            || !opacity_equivalent(actual.value, expected_opacity.value)
        {
            return Err(PdfEditError::StaleImagePlacement);
        }
        Some(actual)
    } else {
        None
    };
    let draw = content
        .operations
        .get(selector.draw_operation_index)
        .ok_or(PdfEditError::StaleImagePlacement)?;
    let after = content
        .operations
        .get(selector.draw_operation_index + 1)
        .ok_or(PdfEditError::StaleImagePlacement)?;
    if before.operator != "q"
        || !before.operands.is_empty()
        || draw.operator != "Do"
        || draw.operands.len() != 1
        || !draw.operands[0]
            .as_name()
            .is_ok_and(|name| name == selector.expected_resource_name.as_bytes())
        || after.operator != "Q"
        || !after.operands.is_empty()
    {
        return Err(PdfEditError::StaleImagePlacement);
    }
    let matrix =
        matrix_from_operation(matrix_operation).ok_or(PdfEditError::ImagePlacementUnsupported)?;
    if !matrix_equivalent(matrix, selector.expected_matrix) {
        return Err(PdfEditError::StaleImagePlacement);
    }
    Ok((content, matrix, image_id, crop, opacity))
}

fn image_usages(document: &Document) -> Result<BTreeMap<ObjectId, Vec<ImageUsage>>, PdfEditError> {
    let mut usages: BTreeMap<ObjectId, Vec<ImageUsage>> = BTreeMap::new();
    for (page_number, page_id) in document.get_pages() {
        for (resource_name, object_id) in page_image_resources(document, page_id)? {
            usages.entry(object_id).or_default().push(ImageUsage {
                page_number,
                resource_name: String::from_utf8_lossy(&resource_name).to_string(),
            });
        }
    }
    for image_usages in usages.values_mut() {
        image_usages.sort();
        image_usages.dedup();
    }
    Ok(usages)
}

fn supported_image_dimensions(stream: &lopdf::Stream) -> Result<(u32, u32), PdfEditError> {
    if !stream
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .is_ok_and(|name| name == b"Image")
        || stream.dict.has(b"Mask")
        || stream.dict.has(b"SMask")
        || stream
            .dict
            .get(b"ImageMask")
            .and_then(Object::as_bool)
            .unwrap_or(false)
    {
        return Err(PdfEditError::ImageUnsupported);
    }
    let width = stream
        .dict
        .get(b"Width")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0 && *value <= u32::from(u16::MAX))
        .ok_or(PdfEditError::ImageUnsupported)?;
    let height = stream
        .dict
        .get(b"Height")
        .and_then(Object::as_i64)
        .ok()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0 && *value <= u32::from(u16::MAX))
        .ok_or(PdfEditError::ImageUnsupported)?;
    Ok((width, height))
}

fn image_filters(stream: &lopdf::Stream) -> Option<Vec<String>> {
    let Ok(filter) = stream.dict.get(b"Filter") else {
        return Some(Vec::new());
    };
    match filter {
        Object::Name(name) => Some(vec![String::from_utf8_lossy(name).to_string()]),
        Object::Array(filters) => filters
            .iter()
            .map(|value| {
                value
                    .as_name()
                    .ok()
                    .map(|name| String::from_utf8_lossy(name).to_string())
            })
            .collect(),
        _ => None,
    }
}

fn image_color_space(stream: &lopdf::Stream) -> Option<String> {
    match stream.dict.get(b"ColorSpace").ok()? {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(values) => values
            .first()?
            .as_name()
            .ok()
            .map(|name| String::from_utf8_lossy(name).to_string()),
        _ => None,
    }
}

fn inspect_image_placements(
    document: &Document,
) -> Result<(Vec<EditableImagePlacement>, bool, usize), PdfEditError> {
    let mut placements = Vec::new();
    let mut truncated = false;
    let mut unsupported = 0usize;
    for (page_number, page_id) in document.get_pages() {
        let resources = page_image_resources(document, page_id)?;
        for stream_id in document.get_page_contents(page_id) {
            let stream = match document.get_object(stream_id).and_then(Object::as_stream) {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let bytes = stream
                .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
                .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            let content =
                Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
                return Err(PdfEditError::ContentOperationLimitExceeded);
            }
            for (draw_operation_index, draw) in content.operations.iter().enumerate() {
                if draw.operator != "Do" || draw.operands.len() != 1 {
                    continue;
                }
                let Ok(resource_name_bytes) = draw.operands[0].as_name() else {
                    continue;
                };
                let Some(image_id) = resources.get(resource_name_bytes).copied() else {
                    continue;
                };
                let image_supported = document
                    .get_object(image_id)
                    .and_then(Object::as_stream)
                    .ok()
                    .is_some_and(|image| supported_image_dimensions(image).is_ok());
                let mut cursor = draw_operation_index;
                let opacity = content
                    .operations
                    .get(cursor.saturating_sub(1))
                    .filter(|operation| operation.operator == "gs")
                    .and_then(|operation| {
                        (operation.operands.len() == 1)
                            .then(|| operation.operands[0].as_name().ok())
                            .flatten()
                    })
                    .and_then(|name| owned_image_opacity(document, page_id, name));
                let has_foreign_graphics_state = content
                    .operations
                    .get(cursor.saturating_sub(1))
                    .is_some_and(|operation| operation.operator == "gs")
                    && opacity.is_none();
                if opacity.is_some() {
                    cursor -= 1;
                }
                let crop = (cursor >= 3)
                    .then(|| {
                        let rectangle = content
                            .operations
                            .get(cursor - 3)
                            .and_then(crop_from_operation)?;
                        let clip = content.operations.get(cursor - 2)?;
                        let end_path = content.operations.get(cursor - 1)?;
                        (clip.operator == "W"
                            && clip.operands.is_empty()
                            && end_path.operator == "n"
                            && end_path.operands.is_empty())
                        .then_some(rectangle)
                    })
                    .flatten();
                if crop.is_some() {
                    cursor -= 3;
                }
                let matrix_operation_index = cursor.saturating_sub(1);
                let isolated = !has_foreign_graphics_state
                    && matrix_operation_index >= 1
                    && content
                        .operations
                        .get(matrix_operation_index - 1)
                        .is_some_and(|operation| {
                            operation.operator == "q" && operation.operands.is_empty()
                        })
                    && content
                        .operations
                        .get(draw_operation_index + 1)
                        .is_some_and(|operation| {
                            operation.operator == "Q" && operation.operands.is_empty()
                        });
                let matrix = content
                    .operations
                    .get(matrix_operation_index)
                    .and_then(matrix_from_operation);
                let Ok(resource_name) = String::from_utf8(resource_name_bytes.to_vec()) else {
                    unsupported += 1;
                    continue;
                };
                if !valid_resource_name(resource_name_bytes) {
                    unsupported += 1;
                    continue;
                }
                let Some(matrix) = matrix else {
                    unsupported += 1;
                    continue;
                };
                if !image_supported || !isolated {
                    unsupported += 1;
                    continue;
                }
                if placements.len() >= MAX_EDITABLE_IMAGE_PLACEMENTS {
                    truncated = true;
                    continue;
                }
                placements.push(EditableImagePlacement {
                    selector: ImagePlacementSelector {
                        page_number,
                        content_stream_object: stream_id.into(),
                        matrix_operation_index,
                        draw_operation_index,
                        expected_resource_name: resource_name,
                        expected_image_object: image_id.into(),
                        expected_matrix: matrix,
                        expected_crop: crop,
                        expected_opacity: opacity.clone(),
                    },
                    matrix,
                    crop,
                    opacity: opacity.as_ref().map_or(1.0, |state| state.value),
                });
            }
        }
    }
    Ok((placements, truncated, unsupported))
}

fn inspect_editable_images(
    document: &Document,
) -> Result<(Vec<EditableImage>, bool, usize), PdfEditError> {
    let usages = image_usages(document)?;
    let mut editable = Vec::new();
    let mut truncated = false;
    let mut unsupported = 0usize;
    for (image_id, image_usages) in usages {
        let Ok(stream) = document.get_object(image_id).and_then(Object::as_stream) else {
            unsupported += 1;
            continue;
        };
        let Ok((width, height)) = supported_image_dimensions(stream) else {
            unsupported += 1;
            continue;
        };
        let Some(filters) = image_filters(stream) else {
            unsupported += 1;
            continue;
        };
        if editable.len() >= MAX_EDITABLE_IMAGES {
            truncated = true;
            continue;
        }
        let bits_per_component = stream
            .dict
            .get(b"BitsPerComponent")
            .and_then(Object::as_i64)
            .ok()
            .and_then(|value| u32::try_from(value).ok());
        editable.push(EditableImage {
            selector: ImageSelector {
                image_object: image_id.into(),
                expected_stream_sha256: sha256_bytes(&stream.content),
                expected_width: width,
                expected_height: height,
            },
            width,
            height,
            color_space: image_color_space(stream),
            bits_per_component,
            filters,
            usages: image_usages,
        });
    }
    Ok((editable, truncated, unsupported))
}

fn inspect_text_runs(
    document: &Document,
) -> Result<(Vec<EditableTextRun>, bool, usize), PdfEditError> {
    inspect_text_runs_for_page(document, None)
}

fn inspect_text_runs_for_page(
    document: &Document,
    text_page: Option<u32>,
) -> Result<(Vec<EditableTextRun>, bool, usize), PdfEditError> {
    let mut runs = Vec::new();
    let mut run_text_bytes = 0usize;
    let mut truncated = false;
    let mut unsupported = 0usize;
    for (page_number, page_id) in document.get_pages() {
        if text_page.is_some_and(|page| page != page_number) {
            continue;
        }
        let fonts = document
            .get_page_fonts(page_id)
            .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
        let mut font_state = text_show::FontState::default();
        let mut current_matrix = None;
        let page_style_eligible = styled_text::page_style_eligible(document, page_id);
        for stream_id in document.get_page_contents(page_id) {
            let stream = match document.get_object(stream_id).and_then(Object::as_stream) {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let bytes = stream
                .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
                .map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            let content =
                Content::decode(&bytes).map_err(|_| PdfEditError::ContentStreamUnsupported)?;
            if content.operations.len() > MAX_CONTENT_OPERATIONS_PER_STREAM {
                return Err(PdfEditError::ContentOperationLimitExceeded);
            }
            for (operation_index, operation) in content.operations.iter().enumerate() {
                font_state.apply(operation)?;
                match operation.operator.as_str() {
                    "Tm" => current_matrix = text_matrix(&operation.operands),
                    "Td" | "TD" | "T*" | "BT" | "ET" => current_matrix = None,
                    "Tj" | "TJ" | "'" | "\"" => {
                        // Only a known immediate matrix can describe a whole run.
                        // Do not reuse the previous run's origin after its advance.
                        let matrix = current_matrix.take();
                        let Some(font_name) = font_state.name.as_ref() else {
                            unsupported += 1;
                            continue;
                        };
                        let Ok(source_bytes) = text_show::bytes(operation) else {
                            unsupported += 1;
                            continue;
                        };
                        let Some(font) = fonts.get(font_name) else {
                            unsupported += 1;
                            continue;
                        };
                        let Ok(encoding) = font
                            .get_font_encoding_with_limit(document, MAX_DECOMPRESSED_STREAM_BYTES)
                        else {
                            unsupported += 1;
                            continue;
                        };
                        let Ok(text) = Document::decode_text(&encoding, &source_bytes) else {
                            unsupported += 1;
                            continue;
                        };
                        let encoded = Document::encode_text(&encoding, &text);
                        let exact_round_trip = Document::decode_text(&encoding, &encoded)
                            .is_ok_and(|decoded| decoded == text);
                        if !exact_round_trip
                            || text.len() > MAX_EDITABLE_TEXT_RUN_BYTES
                            || text.chars().count() > MAX_EDITABLE_TEXT_RUN_CHARS
                        {
                            unsupported += 1;
                            continue;
                        }
                        let Ok(font_resource) = String::from_utf8(font_name.clone()) else {
                            unsupported += 1;
                            continue;
                        };
                        let measurement = (operation.operator == "Tj")
                            .then_some(())
                            .and(font_state.size.zip(matrix.as_ref()))
                            .and_then(|(font_size, matrix)| {
                                measure_simple_text_run(
                                    document,
                                    font,
                                    &source_bytes,
                                    font_size,
                                    matrix,
                                )
                            });
                        let reflow_eligible = measurement.is_some()
                            && isolated_reflow_operation(&content, operation_index);
                        if runs.len() >= MAX_EDITABLE_TEXT_RUNS
                            || run_text_bytes.saturating_add(text.len()) > MAX_INSPECTION_TEXT_BYTES
                        {
                            truncated = true;
                            continue;
                        }
                        run_text_bytes += text.len();
                        runs.push(EditableTextRun {
                            selector: TextRunSelector {
                                page_number,
                                content_stream_object: stream_id.into(),
                                operation_index,
                                expected_font_resource: font_resource,
                                expected_text: text,
                            },
                            operator: operation.operator.clone(),
                            font_size: font_state.size,
                            style_eligible: page_style_eligible
                                && isolated_reflow_operation(&content, operation_index)
                                && matrix.as_ref().is_some_and(horizontal_unit_text_matrix),
                            text_matrix: matrix,
                            bounding_box: measurement.as_ref().map(|value| value.bounding_box),
                            font_metrics_source: measurement
                                .as_ref()
                                .map(|value| value.metrics_source.clone()),
                            reflow_eligible,
                            encoded_byte_length: source_bytes.len(),
                            exact_round_trip,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    Ok((runs, truncated, unsupported))
}

fn target_operation(
    document: &Document,
    page_id: ObjectId,
    stream_id: ObjectId,
    selector: &TextRunSelector,
) -> Result<Operation, PdfEditError> {
    if !document.get_page_contents(page_id).contains(&stream_id) {
        return Err(PdfEditError::ValidationFailed);
    }
    let stream = document
        .get_object(stream_id)
        .and_then(Object::as_stream)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let bytes = stream
        .get_plain_content_with_limit(MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let content = Content::decode(&bytes).map_err(|_| PdfEditError::ValidationFailed)?;
    content
        .operations
        .get(selector.operation_index)
        .cloned()
        .ok_or(PdfEditError::ValidationFailed)
}

fn decoded_tj_text(
    document: &Document,
    page_id: ObjectId,
    operation: &Operation,
    font_resource: &str,
) -> Result<String, PdfEditError> {
    let bytes = text_show::bytes(operation).map_err(|_| PdfEditError::ValidationFailed)?;
    let fonts = document
        .get_page_fonts(page_id)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    let font = fonts
        .get(font_resource.as_bytes())
        .ok_or(PdfEditError::ValidationFailed)?;
    let encoding = font
        .get_font_encoding_with_limit(document, MAX_DECOMPRESSED_STREAM_BYTES)
        .map_err(|_| PdfEditError::ValidationFailed)?;
    Document::decode_text(&encoding, &bytes).map_err(|_| PdfEditError::ValidationFailed)
}

fn save_new_pdf(document: &mut Document, output: &Path) -> Result<(), PdfEditError> {
    let parent = output.parent().ok_or(PdfEditError::OutputWriteFailed)?;
    let file_name = output.file_name().ok_or(PdfEditError::OutputWriteFailed)?;
    let temporary = parent.join(format!(
        ".{}.wellfriend-pdf-edit.tmp",
        file_name.to_string_lossy()
    ));
    if temporary.exists() {
        fs::remove_file(&temporary).map_err(|_| PdfEditError::OutputWriteFailed)?;
    }
    let file = File::options()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| PdfEditError::OutputWriteFailed)?;
    let mut writer = BufWriter::new(file);
    if document.save_to(&mut writer).is_err() || writer.flush().is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(PdfEditError::OutputWriteFailed);
    }
    drop(writer);
    if fs::rename(&temporary, output).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(PdfEditError::OutputWriteFailed);
    }
    Ok(())
}

fn changed_object_ids(
    before: &BTreeMap<ObjectId, Object>,
    after: &BTreeMap<ObjectId, Object>,
) -> Vec<ObjectId> {
    before
        .iter()
        .filter_map(|(id, object)| (after.get(id) != Some(object)).then_some(*id))
        .chain(after.keys().filter(|id| !before.contains_key(id)).copied())
        .collect()
}

fn pdf_objects_semantically_equal(before: &Object, after: &Object) -> bool {
    match (before, after) {
        (Object::Stream(before), Object::Stream(after)) => {
            before.dict == after.dict
                && before.content == after.content
                && before.allows_compression == after.allows_compression
        }
        _ => before == after,
    }
}

fn is_writer_housekeeping_object(object: &Object) -> bool {
    object_dictionary(object).is_some_and(|dictionary| {
        dictionary
            .get(b"Type")
            .and_then(Object::as_name)
            .is_ok_and(|name| matches!(name, b"XRef" | b"ObjStm"))
    })
}

fn semantic_object_count(objects: &BTreeMap<ObjectId, Object>) -> usize {
    objects
        .values()
        .filter(|object| !is_writer_housekeeping_object(object))
        .count()
}

fn signature_summaries(document: &Document) -> Vec<SignatureSummary> {
    document
        .objects
        .iter()
        .filter_map(|(id, object)| {
            let dictionary = object_dictionary(object)?;
            let is_signature = dictionary
                .get(b"Type")
                .and_then(Object::as_name)
                .is_ok_and(|name| name == b"Sig")
                || dictionary
                    .get(b"FT")
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == b"Sig");
            is_signature.then(|| SignatureSummary {
                object: (*id).into(),
                has_byte_range: dictionary
                    .get(b"ByteRange")
                    .and_then(Object::as_array)
                    .is_ok(),
            })
        })
        .collect()
}

fn object_graph_summary(document: &Document, signature_count: usize) -> PdfObjectGraphSummary {
    let pages = document.get_pages();
    let mut summary = PdfObjectGraphSummary {
        catalogs: document
            .objects
            .values()
            .filter_map(object_dictionary)
            .filter(|dict| {
                dict.get(b"Type")
                    .and_then(Object::as_name)
                    .is_ok_and(|name| name == b"Catalog")
            })
            .count(),
        pages: pages.len(),
        signature_objects: signature_count,
        ..Default::default()
    };
    for page_id in pages.values().copied() {
        summary.content_streams += document.get_page_contents(page_id).len();
        summary.font_resources += document
            .get_page_fonts(page_id)
            .map(|fonts| fonts.len())
            .unwrap_or(0);
        summary.images += document
            .get_page_images(page_id)
            .map(|images| images.len())
            .unwrap_or(0);
        if let Ok((resource, resource_ids)) = document.get_page_resources(page_id) {
            if let Some(resource) = resource {
                summary.xobjects += dictionary_entry_len(document, resource, b"XObject");
            }
            for resource_id in resource_ids {
                if let Ok(resource) = document.get_dictionary(resource_id) {
                    summary.xobjects += dictionary_entry_len(document, resource, b"XObject");
                }
            }
        }
        if let Ok(annotations) = document.get_page_annotations(page_id) {
            summary.annotations += annotations.len();
            summary.links += annotations
                .iter()
                .filter(|annotation| {
                    annotation
                        .get(b"Subtype")
                        .and_then(Object::as_name)
                        .is_ok_and(|name| name == b"Link")
                })
                .count();
        }
    }
    if let Some(catalog) = catalog_dictionary(document) {
        summary.outlines = usize::from(catalog.get(b"Outlines").is_ok());
        summary.names_trees = usize::from(catalog.get(b"Names").is_ok());
        summary.metadata_streams = usize::from(catalog.get(b"Metadata").is_ok());
        summary.acro_form_fields = catalog
            .get(b"AcroForm")
            .ok()
            .and_then(|value| dereference_dictionary(document, value))
            .and_then(|acro_form| acro_form.get(b"Fields").ok())
            .and_then(|fields| fields.as_array().ok())
            .map(Vec::len)
            .unwrap_or(0);
    }
    summary
}

fn catalog_dictionary(document: &Document) -> Option<&Dictionary> {
    let root = document.trailer.get(b"Root").ok()?;
    dereference_dictionary(document, root)
}

fn dereference_dictionary<'a>(
    document: &'a Document,
    object: &'a Object,
) -> Option<&'a Dictionary> {
    match object {
        Object::Reference(id) => document.get_dictionary(*id).ok(),
        Object::Dictionary(dictionary) => Some(dictionary),
        _ => None,
    }
}

fn dictionary_entry_len(document: &Document, dictionary: &Dictionary, key: &[u8]) -> usize {
    dictionary
        .get(key)
        .ok()
        .and_then(|value| dereference_dictionary(document, value))
        .map(|value| value.iter().count())
        .unwrap_or(0)
}

fn object_dictionary(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn text_matrix(operands: &[Object]) -> Option<TextMatrix> {
    let values = operands
        .iter()
        .take(6)
        .map(pdf_number)
        .collect::<Option<Vec<_>>>()?;
    (values.len() == 6).then(|| TextMatrix {
        a: values[0],
        b: values[1],
        c: values[2],
        d: values[3],
        e: values[4],
        f: values[5],
    })
}

fn pdf_number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(f64::from(*value)),
        _ => None,
    }
}

fn sha256_file(path: &Path) -> Result<String, PdfEditError> {
    let mut file = File::open(path).map_err(|_| PdfEditError::InputUnavailable)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| PdfEditError::InputUnavailable)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
