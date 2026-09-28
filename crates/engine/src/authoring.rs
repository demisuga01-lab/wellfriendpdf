//! High-level PDF authoring API.
//!
//! Coordinates use native PDF user space: the origin is at the bottom-left of
//! the page, x grows to the right, and y grows upward. Use
//! [`PdfPageBuilder::pdf_y_from_top`] when a top-left UI coordinate is more
//! convenient.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

#[path = "authoring_fallback.rs"]
mod fallback;
#[path = "authoring_layout.rs"]
mod layout;
pub use fallback::{FontStackLinePreview, FontStackRunPreview};
#[path = "authoring_tables.rs"]
mod tables;
pub use tables::{
    inspect_authored_typed_table_grid_paint, inspect_authored_typed_table_sources,
    load_authored_typed_tables, mutate_authored_typed_table, AuthoredRetainedFont,
    AuthoredTypedCellAlignment, AuthoredTypedCellLayout, AuthoredTypedCellModel,
    AuthoredTypedCellPaint, AuthoredTypedCellRole, AuthoredTypedCellSourceBinding,
    AuthoredTypedCellSourceFragment, AuthoredTypedColor, AuthoredTypedGridPaintFragment,
    AuthoredTypedHeaderCellModel, AuthoredTypedHeaderModel, AuthoredTypedHeaderScope,
    AuthoredTypedTableGridPaintReport, AuthoredTypedTableModel, AuthoredTypedTableMutationReport,
    AuthoredTypedTableMutationRequest, AuthoredTypedTablePagination,
    AuthoredTypedTableRetainedContinuation, AuthoredTypedTableSourceReport, TableCaptionInfo,
    TableCellFragmentInfo, TableFlowReport, TableFragmentInfo, TableRowPageBreakInfo,
    TableRowSplitPolicy,
};
#[path = "authoring_notes.rs"]
mod notes;
pub use notes::{
    EndnoteItemInfo, EndnoteReport, FlowEndnote, FlowFootnote, FootnoteFragmentInfo,
    FootnoteNumbering, FootnotedParagraphReport, InsertedFootnoteMarkerInfo, NoteNumberScope,
    NoteNumberStyle, NumberedFootnote, NumberedFootnotedParagraphReport,
};
#[path = "authoring_fields.rs"]
mod fields;
pub use fields::{
    BodyAnchorInfo, BodyField, BodyFieldFormat, BodyFieldInfo, BodyFieldPart, FieldParagraphReport,
};
#[path = "authoring_outline.rs"]
mod outline;
pub use outline::PdfOutlineEntry;
#[path = "authoring_toc.rs"]
mod toc;
pub use toc::{
    TableOfContentsLeaderStyle, TableOfContentsLevelStyle, TableOfContentsPageSide,
    TableOfContentsReport, TableOfContentsRow, TableOfContentsStyle,
};
#[path = "authoring_index.rs"]
mod document_index;
pub use document_index::{
    DocumentIndexReport, DocumentIndexRow, DocumentIndexSort, DocumentIndexStyle, PdfIndexEntry,
};
#[path = "authoring_front_matter.rs"]
mod front_matter;
pub use front_matter::{FrontMatterReport, FrontMatterTableOfContentsReport};
#[path = "authoring_sections.rs"]
mod sections;
#[path = "authoring_structure.rs"]
mod structure;
pub use sections::{
    FlowSection, PageNumberField, PageNumberStyle, RunningText, RunningTextPart, SectionPageMaster,
};
#[cfg(test)]
#[path = "authoring_cff_tests.rs"]
mod cff_tests;
#[cfg(test)]
#[path = "authoring_layout_tests.rs"]
mod layout_tests;

use crate::content::{Color, ColorSpace, LineCap, LineDash, LineJoin};
use crate::error::{Result, WellfriendError};
use crate::filters::flate_encode;
use crate::fonts::encoding::{zapf_dingbats_name_to_unicode, Encoding};
use crate::fonts::glyph_list::glyph_name_to_unicode;
use crate::fonts::sfnt_subset::{subset_glyf_preserving_gids, SfntSubsetError, SfntSubsetMetrics};
use crate::fonts::{ShapeOptions, TextShaper};
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::object::{PdfDictionary, PdfObject};
use crate::render::get_fallback_font;
use crate::writer::{OutputObject, PdfWriter, WriterMode};
use sha2::{Digest, Sha256};

pub use crate::fonts::tab_stops::{TabAlignment, TabLeader, TabStop, TabStops};

const DEFAULT_FONT_SIZE: f64 = 12.0;
const DEFAULT_LINE_HEIGHT: f64 = 1.2;
const BUILTIN_UNICODE_RESOURCE_NAME: &str = "WellfriendUnicode";
const KAPPA: f64 = 0.552_284_749_830_793_6;

fn valid_language_subtag(value: &str, minimum: usize, maximum: usize) -> bool {
    (minimum..=maximum).contains(&value.len())
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn validate_authoring_language(language: &str) -> Result<String> {
    const GRANDFATHERED: &[&str] = &[
        "art-lojban",
        "cel-gaulish",
        "en-gb-oed",
        "i-ami",
        "i-bnn",
        "i-default",
        "i-enochian",
        "i-hak",
        "i-klingon",
        "i-lux",
        "i-mingo",
        "i-navajo",
        "i-pwn",
        "i-tao",
        "i-tay",
        "i-tsu",
        "no-bok",
        "no-nyn",
        "sgn-be-fr",
        "sgn-be-nl",
        "sgn-ch-de",
        "zh-guoyu",
        "zh-hakka",
        "zh-min",
        "zh-min-nan",
        "zh-xiang",
    ];
    let value = language.trim();
    if value.len() < 2 || value.len() > 255 || !value.is_ascii() {
        return Err(WellfriendError::invalid_input(
            "authoring document language must be a bounded BCP 47 tag",
        ));
    }
    let lower = value.to_ascii_lowercase();
    if GRANDFATHERED.contains(&lower.as_str()) {
        return Ok(value.to_string());
    }
    let subtags = value.split('-').collect::<Vec<_>>();
    if subtags.iter().any(|subtag| subtag.is_empty()) {
        return Err(WellfriendError::invalid_input(
            "authoring document language contains an empty BCP 47 subtag",
        ));
    }
    if subtags[0].eq_ignore_ascii_case("x") {
        if subtags.len() == 1
            || subtags[1..]
                .iter()
                .any(|subtag| !valid_language_subtag(subtag, 1, 8))
        {
            return Err(WellfriendError::invalid_input(
                "authoring private-use language tag is invalid",
            ));
        }
        return Ok(value.to_string());
    }
    if !(2..=8).contains(&subtags[0].len())
        || !subtags[0].bytes().all(|byte| byte.is_ascii_alphabetic())
    {
        return Err(WellfriendError::invalid_input(
            "authoring document language has an invalid primary subtag",
        ));
    }

    let mut index = 1usize;
    if subtags[0].len() <= 3 {
        let mut extlangs = 0usize;
        while index < subtags.len()
            && extlangs < 3
            && subtags[index].len() == 3
            && subtags[index]
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic())
        {
            index += 1;
            extlangs += 1;
        }
    }
    if index < subtags.len()
        && subtags[index].len() == 4
        && subtags[index]
            .bytes()
            .all(|byte| byte.is_ascii_alphabetic())
    {
        index += 1;
    }
    if index < subtags.len()
        && ((subtags[index].len() == 2
            && subtags[index]
                .bytes()
                .all(|byte| byte.is_ascii_alphabetic()))
            || (subtags[index].len() == 3
                && subtags[index].bytes().all(|byte| byte.is_ascii_digit())))
    {
        index += 1;
    }
    while index < subtags.len()
        && (((5..=8).contains(&subtags[index].len())
            && subtags[index]
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric()))
            || (subtags[index].len() == 4
                && subtags[index].as_bytes()[0].is_ascii_digit()
                && subtags[index]
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric())))
    {
        index += 1;
    }

    let mut extensions = BTreeSet::new();
    while index < subtags.len()
        && subtags[index].len() == 1
        && !subtags[index].eq_ignore_ascii_case("x")
        && subtags[index]
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        let singleton = subtags[index].to_ascii_lowercase();
        if !extensions.insert(singleton) {
            return Err(WellfriendError::invalid_input(
                "authoring document language repeats an extension singleton",
            ));
        }
        index += 1;
        let start = index;
        while index < subtags.len() && valid_language_subtag(subtags[index], 2, 8) {
            index += 1;
        }
        if index == start {
            return Err(WellfriendError::invalid_input(
                "authoring document language extension has no value",
            ));
        }
    }
    if index < subtags.len() && subtags[index].eq_ignore_ascii_case("x") {
        index += 1;
        let start = index;
        while index < subtags.len() && valid_language_subtag(subtags[index], 1, 8) {
            index += 1;
        }
        if index == start {
            return Err(WellfriendError::invalid_input(
                "authoring private-use language sequence is empty",
            ));
        }
    }
    if index != subtags.len() {
        return Err(WellfriendError::invalid_input(
            "authoring document language contains an invalid BCP 47 subtag sequence",
        ));
    }
    Ok(value.to_string())
}

/// A high-level PDF document builder.
///
/// The builder creates a fresh object graph and serializes it through
/// [`PdfWriter`]. The default writer mode is
/// [`WriterMode::XrefStreamWithObjStm`] for compact modern output.
#[derive(Debug, Clone)]
pub struct PdfBuilder {
    pages: Vec<PdfPageBuilder>,
    metadata: PdfMetadata,
    writer_mode: WriterMode,
    version: String,
    custom_fonts: Arc<Vec<CustomFont>>,
    font_stacks: Arc<Vec<Vec<FontFace>>>,
    images: Vec<AuthoredImage>,
    sections: Vec<FlowSection>,
    anchors: BTreeMap<String, fields::BodyAnchor>,
    next_field_plan_id: u64,
    fields_materialized: bool,
    outline: Vec<PdfOutlineEntry>,
    outline_item_count: usize,
    language: Option<String>,
    structures: Vec<structure::Element>,
    next_structure_id: u64,
    next_footnote_id: u64,
    notes_materialized: bool,
    section_masters_materialized: bool,
    authored_typed_tables: Vec<AuthoredTypedTableModel>,
}

impl Default for PdfBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PdfBuilder {
    /// Create an empty PDF document.
    pub fn new() -> Self {
        Self {
            pages: Vec::new(),
            metadata: PdfMetadata::default(),
            writer_mode: WriterMode::XrefStreamWithObjStm,
            version: "1.7".to_string(),
            custom_fonts: Arc::new(Vec::new()),
            font_stacks: Arc::new(Vec::new()),
            images: Vec::new(),
            sections: Vec::new(),
            anchors: BTreeMap::new(),
            next_field_plan_id: 0,
            fields_materialized: false,
            outline: Vec::new(),
            outline_item_count: 0,
            language: None,
            structures: Vec::new(),
            next_structure_id: 0,
            next_footnote_id: 0,
            notes_materialized: false,
            section_masters_materialized: false,
            authored_typed_tables: Vec::new(),
        }
    }

    /// Replace the document metadata dictionary.
    pub fn set_metadata(&mut self, metadata: PdfMetadata) -> &mut Self {
        self.metadata = metadata;
        self
    }

    pub fn metadata_mut(&mut self) -> &mut PdfMetadata {
        &mut self.metadata
    }

    /// Replace the document outline. Entries may reference anchors declared
    /// later; final serialization resolves every target atomically.
    pub fn set_outline(&mut self, entries: Vec<PdfOutlineEntry>) -> Result<&mut Self> {
        let count = outline::validate(&entries)?;
        self.outline = entries;
        self.outline_item_count = count;
        Ok(self)
    }

    pub fn clear_outline(&mut self) -> &mut Self {
        self.outline.clear();
        self.outline_item_count = 0;
        self
    }

    /// Set the document's bounded BCP 47 language token for catalog `/Lang`.
    pub fn set_language(&mut self, language: impl Into<String>) -> Result<&mut Self> {
        let language = language.into();
        self.language = Some(validate_authoring_language(&language)?);
        Ok(self)
    }

    pub fn clear_language(&mut self) -> &mut Self {
        self.language = None;
        self
    }

    pub fn set_title(&mut self, title: impl Into<String>) -> &mut Self {
        self.metadata.title = Some(title.into());
        self
    }

    pub fn set_author(&mut self, author: impl Into<String>) -> &mut Self {
        self.metadata.author = Some(author.into());
        self
    }

    pub fn set_subject(&mut self, subject: impl Into<String>) -> &mut Self {
        self.metadata.subject = Some(subject.into());
        self
    }

    pub fn set_keywords(&mut self, keywords: impl Into<String>) -> &mut Self {
        self.metadata.keywords = Some(keywords.into());
        self
    }

    pub fn set_creator(&mut self, creator: impl Into<String>) -> &mut Self {
        self.metadata.creator = Some(creator.into());
        self
    }

    /// Select the low-level writer mode used by [`Self::to_bytes`].
    pub fn with_writer_mode(mut self, mode: WriterMode) -> Self {
        self.writer_mode = mode;
        self
    }

    /// Set the preferred PDF header version. Defaults to `1.7`. The writer
    /// raises it when selected features require a newer version (for example,
    /// PDF 1.6 for embedded OpenType programs).
    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Add a page and return its drawing surface.
    pub fn add_page(&mut self, size: PageSize) -> &mut PdfPageBuilder {
        let mut page = PdfPageBuilder::new(size);
        page.custom_fonts = Arc::clone(&self.custom_fonts);
        page.font_stacks = Arc::clone(&self.font_stacks);
        self.pages.push(page);
        self.pages.last_mut().expect("page was just pushed")
    }

    /// Add a page with margins intended for paragraph/layout helpers.
    pub fn add_page_with_margins(
        &mut self,
        size: PageSize,
        margins: Margins,
    ) -> &mut PdfPageBuilder {
        let mut page = PdfPageBuilder::with_margins(size, margins);
        page.custom_fonts = Arc::clone(&self.custom_fonts);
        page.font_stacks = Arc::clone(&self.font_stacks);
        self.pages.push(page);
        self.pages.last_mut().expect("page was just pushed")
    }

    pub fn pages(&self) -> &[PdfPageBuilder] {
        &self.pages
    }

    pub fn pages_mut(&mut self) -> &mut [PdfPageBuilder] {
        &mut self.pages
    }

    /// Register an editable standalone TrueType or OpenType/CFF1 program.
    ///
    /// TrueType uses a CIDFontType2 descendant and a CIDToGIDMap. CFF1 uses
    /// CIDFontType0, FontFile3/OpenType and a native-charset Encoding CMap.
    /// Glyph subsetting is performed only when the font permits it; CFF1 is
    /// embedded whole. Both routes retain separate ToUnicode mappings.
    pub fn register_font_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<FontFace> {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "authoring: custom font bytes are empty".to_string(),
            ));
        }
        TrueTypeMetrics::parse(&bytes)?;
        crate::fonts::pdf_embedding::EmbeddingInfo::parse(&bytes)?;
        let id =
            CustomFontId(u32::try_from(self.custom_fonts.len()).map_err(|_| {
                WellfriendError::ResourceLimit("authoring custom font count".into())
            })?);
        let base_name = sanitize_pdf_name(
            &name.into(),
            &format!("WellfriendCustomFont{}", u64::from(id.0) + 1),
        );
        Arc::make_mut(&mut self.custom_fonts).push(CustomFont {
            id,
            base_name,
            bytes: bytes.into(),
        });
        for page in &mut self.pages {
            page.custom_fonts = Arc::clone(&self.custom_fonts);
        }
        Ok(FontFace::Custom(id))
    }

    /// Select a source-hash-bound face from a TTC/OTC (or standalone sfnt), then
    /// register its exact standalone bytes through the canonical authoring path.
    /// The receipt discloses container changes; it is not signature validation.
    pub fn register_font_face_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        selection: &crate::fonts::font_asset::FontFaceSelection,
    ) -> Result<(FontFace, crate::fonts::font_asset::FontPreparationReport)> {
        let prepared = crate::fonts::font_asset::prepare_font_asset(bytes, selection)?;
        let face = self.register_font_bytes(name, prepared.bytes)?;
        Ok((face, prepared.report))
    }

    /// Freeze a selected variable font and register its exact static output.
    pub fn register_font_instance_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: &[u8],
        request: &crate::fonts::font_instance::FontInstanceRequest,
    ) -> Result<(FontFace, crate::fonts::font_instance::FontInstanceReport)> {
        let prepared = crate::fonts::font_instance::prepare_font_instance(bytes, request)?;
        let face = self.register_font_bytes(name, prepared.bytes)?;
        Ok((face, prepared.report))
    }

    /// Historical alias for [`Self::register_font_bytes`]. It accepts the same
    /// editable standalone TrueType and OpenType/CFF1 programs.
    pub fn register_truetype_font_bytes(
        &mut self,
        name: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
    ) -> Result<FontFace> {
        self.register_font_bytes(name, bytes)
    }

    /// Register an ordered contextual fallback stack for TextStyle and the
    /// existing text/paragraph/table/flow methods. Standard-14 members explicitly
    /// opt into bundled TrueType equivalents rather than viewer-supplied fonts.
    /// No system-font search or undisclosed fallback is performed.
    pub fn register_font_stack(&mut self, fonts: &[FontFace]) -> Result<FontFace> {
        fallback::register(self, fonts)
    }

    /// Physical embedded faces after nested-stack flattening and explicit
    /// Standard-14 equivalent resolution, in preference order.
    pub fn font_stack_members(&self, font: FontFace) -> Result<&[FontFace]> {
        let FontFace::Fallback(id) = font else {
            return Err(WellfriendError::invalid_input(
                "font is not an authoring fallback stack",
            ));
        };
        self.font_stacks
            .get(id.0 as usize)
            .map(Vec::as_slice)
            .ok_or_else(|| {
                WellfriendError::invalid_input("authoring fallback stack is not registered")
            })
    }

    /// Register a JPEG image XObject. The source JPEG bytes are embedded
    /// directly with `DCTDecode`; they are decoded only to read dimensions and
    /// color channel count.
    pub fn add_jpeg_image(&mut self, bytes: impl Into<Vec<u8>>) -> Result<ImageHandle> {
        let bytes = bytes.into();
        let (_, width, height, channels) = ImageDecoder::decode_jpeg_with_info(&bytes)?;
        let color_space = ImageColorSpace::from_channels(channels)?;
        Ok(self.push_image(AuthoredImage {
            width,
            height,
            color_space,
            bits_per_component: 8,
            data: bytes,
            filter: ImageFilter::DctDecode,
            smask: None,
        }))
    }

    /// Register PNG bytes as an Image XObject. RGB/gray samples are Flate
    /// compressed; alpha is split into a PDF soft mask.
    pub fn add_png_image(&mut self, bytes: &[u8]) -> Result<ImageHandle> {
        let decoded = decode_png_for_authoring(bytes)?;
        self.add_raw_image(decoded)
    }

    /// Register interleaved RGB samples as a Flate-compressed Image XObject.
    pub fn add_rgb_image(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<ImageHandle> {
        self.add_raw_image(RawImage {
            width,
            height,
            channels: 3,
            bits_per_sample: 8,
            pixels,
        })
    }

    /// Register interleaved RGBA samples as a Flate-compressed Image XObject
    /// with an `SMask` for alpha.
    pub fn add_rgba_image(
        &mut self,
        width: u32,
        height: u32,
        pixels: Vec<u8>,
    ) -> Result<ImageHandle> {
        self.add_raw_image(RawImage {
            width,
            height,
            channels: 4,
            bits_per_sample: 8,
            pixels,
        })
    }

    fn add_raw_image(&mut self, raw: RawImage) -> Result<ImageHandle> {
        if !raw.is_valid() || raw.bits_per_sample != 8 {
            return Err(WellfriendError::MalformedPdf(
                "authoring: image samples must be non-empty 8-bit data".to_string(),
            ));
        }
        let authored = authored_image_from_raw(raw)?;
        Ok(self.push_image(authored))
    }

    fn push_image(&mut self, image: AuthoredImage) -> ImageHandle {
        let handle = ImageHandle(self.images.len() as u32);
        self.images.push(image);
        handle
    }

    fn image(&self, handle: ImageHandle) -> Result<&AuthoredImage> {
        self.images.get(handle.0 as usize).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "authoring: image handle {} was not registered on this document",
                handle.0
            ))
        })
    }

    fn custom_font(&self, id: CustomFontId) -> Result<&CustomFont> {
        self.custom_fonts.get(id.0 as usize).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "authoring: custom font handle {} was not registered on this document",
                id.0
            ))
        })
    }

    /// Serialize the authored document to PDF bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        if self.pages.is_empty() {
            return Err(WellfriendError::MalformedPdf(
                "authoring: cannot save a PDF with no pages".to_string(),
            ));
        }

        let materialized = notes::materialize(self)?;
        let materialized = sections::materialize(&materialized)?;
        let materialized = fields::materialize(&materialized)?;
        let font_plan = FontBuildPlan::from_builder(&materialized)?;
        let image_plan = ImageBuildPlan::from_builder(&materialized)?;
        let objects = AuthoredObjects::build(&materialized, &font_plan, &image_plan)?;
        PdfWriter::new(objects.objects, objects.catalog_number)
            .with_info(objects.info_number)
            .with_version(self.version.clone())
            .with_mode(self.writer_mode)
            .write()
    }

    /// Write the authored document to a file.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        std::fs::write(path, self.to_bytes()?)?;
        Ok(())
    }
}

/// Document information dictionary fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PdfMetadata {
    pub title: Option<String>,
    pub author: Option<String>,
    pub subject: Option<String>,
    pub keywords: Option<String>,
    pub creator: Option<String>,
}

impl PdfMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = Some(author.into());
        self
    }

    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    pub fn keywords(mut self, keywords: impl Into<String>) -> Self {
        self.keywords = Some(keywords.into());
        self
    }

    pub fn creator(mut self, creator: impl Into<String>) -> Self {
        self.creator = Some(creator.into());
        self
    }

    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.author.is_none()
            && self.subject.is_none()
            && self.keywords.is_none()
            && self.creator.is_none()
    }
}

/// Handle returned by [`PdfBuilder::add_jpeg_image`],
/// [`PdfBuilder::add_png_image`], or raw image registration helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImageHandle(u32);

impl ImageHandle {
    pub fn index(self) -> u32 {
        self.0
    }
}

/// Handle for a document-registered custom font.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CustomFontId(u32);

/// Handle for a document-registered immutable fallback stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FontStackId(u32);

impl FontStackId {
    pub fn index(self) -> u32 {
        self.0
    }
}

impl CustomFontId {
    pub fn index(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone)]
struct CustomFont {
    id: CustomFontId,
    base_name: String,
    bytes: Arc<[u8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageColorSpace {
    DeviceGray,
    DeviceRGB,
    DeviceCMYK,
}

impl ImageColorSpace {
    fn from_channels(channels: u8) -> Result<Self> {
        match channels {
            1 => Ok(Self::DeviceGray),
            3 => Ok(Self::DeviceRGB),
            4 => Ok(Self::DeviceCMYK),
            _ => Err(WellfriendError::UnsupportedFeature(format!(
                "authoring: unsupported image channel count {channels}"
            ))),
        }
    }

    fn pdf_name(self) -> &'static str {
        match self {
            Self::DeviceGray => "DeviceGray",
            Self::DeviceRGB => "DeviceRGB",
            Self::DeviceCMYK => "DeviceCMYK",
        }
    }
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

#[derive(Debug, Clone)]
struct AuthoredImage {
    width: u32,
    height: u32,
    color_space: ImageColorSpace,
    bits_per_component: u8,
    data: Vec<u8>,
    filter: ImageFilter,
    smask: Option<AuthoredSoftMask>,
}

#[derive(Debug, Clone)]
struct AuthoredSoftMask {
    width: u32,
    height: u32,
    data: Vec<u8>,
}

/// Page dimensions in PDF points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageSize {
    pub width: f64,
    pub height: f64,
}

impl PageSize {
    pub const LETTER: Self = Self {
        width: 612.0,
        height: 792.0,
    };
    pub const LEGAL: Self = Self {
        width: 612.0,
        height: 1008.0,
    };
    pub const A3: Self = Self {
        width: 841.8898,
        height: 1190.5512,
    };
    pub const A4: Self = Self {
        width: 595.2756,
        height: 841.8898,
    };
    pub const A5: Self = Self {
        width: 419.5276,
        height: 595.2756,
    };

    pub fn custom(width: f64, height: f64) -> Self {
        Self { width, height }
    }

    pub fn inches(width: f64, height: f64) -> Self {
        Self {
            width: width * 72.0,
            height: height * 72.0,
        }
    }

    pub fn mm(width: f64, height: f64) -> Self {
        const POINTS_PER_MM: f64 = 72.0 / 25.4;
        Self {
            width: width * POINTS_PER_MM,
            height: height * POINTS_PER_MM,
        }
    }

    pub fn landscape(self) -> Self {
        Self {
            width: self.width.max(self.height),
            height: self.width.min(self.height),
        }
    }

    pub fn portrait(self) -> Self {
        Self {
            width: self.width.min(self.height),
            height: self.width.max(self.height),
        }
    }
}

/// Page margins in points, used by layout helpers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub left: f64,
    pub right: f64,
    pub top: f64,
    pub bottom: f64,
}

impl Default for Margins {
    fn default() -> Self {
        Self::all(0.0)
    }
}

impl Margins {
    pub fn all(value: f64) -> Self {
        Self {
            left: value,
            right: value,
            top: value,
            bottom: value,
        }
    }

    pub fn vertical_horizontal(vertical: f64, horizontal: f64) -> Self {
        Self {
            left: horizontal,
            right: horizontal,
            top: vertical,
            bottom: vertical,
        }
    }
}

/// One authored PDF page.
#[derive(Debug, Clone)]
pub struct PdfPageBuilder {
    size: PageSize,
    margins: Margins,
    commands: Vec<PageCommand>,
    // Shared document registry, not a fallback-font copy. Updated atomically
    // on registration; cloned documents retain copy-on-write font ownership.
    custom_fonts: Arc<Vec<CustomFont>>,
    font_stacks: Arc<Vec<Vec<FontFace>>>,
    section_index: Option<usize>,
    suppress_section_master: bool,
    footnotes: Vec<notes::FootnoteFragment>,
    footnote_reserved_height: f64,
    links: Vec<fields::AuthoredLink>,
}

impl PdfPageBuilder {
    pub fn new(size: PageSize) -> Self {
        Self {
            size,
            margins: Margins::default(),
            commands: Vec::new(),
            custom_fonts: Arc::new(Vec::new()),
            font_stacks: Arc::new(Vec::new()),
            section_index: None,
            suppress_section_master: false,
            footnotes: Vec::new(),
            footnote_reserved_height: 0.0,
            links: Vec::new(),
        }
    }

    pub fn with_margins(size: PageSize, margins: Margins) -> Self {
        Self {
            size,
            margins,
            commands: Vec::new(),
            custom_fonts: Arc::new(Vec::new()),
            font_stacks: Arc::new(Vec::new()),
            section_index: None,
            suppress_section_master: false,
            footnotes: Vec::new(),
            footnote_reserved_height: 0.0,
            links: Vec::new(),
        }
    }

    pub fn size(&self) -> PageSize {
        self.size
    }

    pub fn margins(&self) -> Margins {
        self.margins
    }

    pub fn set_margins(&mut self, margins: Margins) -> &mut Self {
        self.margins = margins;
        self
    }

    /// Convert a distance from the top page edge into native PDF y space.
    pub fn pdf_y_from_top(&self, y_from_top: f64) -> f64 {
        self.size.height - y_from_top
    }

    /// Draw one text run at a baseline position. Hard separators require
    /// `draw_paragraph`; they are not flattened onto this single baseline.
    pub fn draw_text(
        &mut self,
        text: impl Into<String>,
        x: f64,
        y: f64,
        style: &TextStyle,
    ) -> Result<&mut Self> {
        let text = text.into();
        layout::validate_single_line(&text, x, y, style)?;
        if matches!(style.font, FontFace::Fallback(_)) {
            let command = fallback::single_command(self, &text, x, y, style, None)?;
            self.commands.push(command);
            return Ok(self);
        }
        self.font_program(style.font)?;
        if crate::fonts::logical_carrier::is_text(&text) {
            self.commands.push(PageCommand::LogicalBreak {
                text,
                x,
                y,
                size: style.size,
            });
            return Ok(self);
        }
        validate_text_for_font(&text, &style.font)?;
        self.commands.push(PageCommand::Text {
            text,
            x,
            y,
            style: style.clone(),
            bidi: None,
            logical_text: None,
            suppress_actual_text: false,
            font_asset: self.custom_font_asset(style.font),
            shaped: None,
        });
        Ok(self)
    }

    /// Preserve paragraph-derived UAX #9 levels and joining context. Only
    /// embedded Unicode fonts support this route; no Standard-14 downgrade.
    pub fn draw_text_resolved(
        &mut self,
        text: impl Into<String>,
        x: f64,
        y: f64,
        style: &TextStyle,
        bidi: &crate::fonts::shaper::LineBidi,
    ) -> Result<&mut Self> {
        let text = text.into();
        if !style.font.is_embedded_unicode() || bidi.levels.len() != text.len() {
            return Err(WellfriendError::invalid_input(
                "resolved text requires an embedded Unicode font and matching levels",
            ));
        }
        bidi.context.validate()?;
        if text.chars().any(crate::fonts::hard_break::is_hard_break) {
            return Err(WellfriendError::invalid_input(
                "resolved authoring expects one visible logical line without hard separators",
            ));
        }
        if matches!(style.font, FontFace::Fallback(_)) {
            layout::validate_single_line(&text, x, y, style)?;
            let command = fallback::single_command(self, &text, x, y, style, Some(bidi))?;
            self.commands.push(command);
            return Ok(self);
        }
        self.draw_text(text, x, y, style)?;
        if let Some(PageCommand::Text { bidi: saved, .. }) = self.commands.last_mut() {
            *saved = Some(bidi.clone());
        }
        Ok(self)
    }

    /// Draw one text run where y is measured from the top page edge.
    pub fn draw_text_from_top(
        &mut self,
        text: impl Into<String>,
        x: f64,
        y_from_top: f64,
        style: &TextStyle,
    ) -> Result<&mut Self> {
        self.draw_text(text, x, self.pdf_y_from_top(y_from_top), style)
    }

    pub fn draw_text_line(
        &mut self,
        text: impl Into<String>,
        x: f64,
        y: f64,
        style: &TextStyle,
    ) -> Result<&mut Self> {
        self.draw_text(text, x, y, style)
    }

    /// Preview contextual font choices and measured lines without appending
    /// commands. Uses the same font-stack and paragraph path as drawing.
    pub fn preview_font_stack(
        &self,
        text: &str,
        width: f64,
        style: &TextStyle,
    ) -> Result<Vec<FontStackLinePreview>> {
        fallback::preview(self, text, width, style)
    }

    /// Return the width of a text run in points for the selected style.
    pub fn text_width(&self, text: &str, style: &TextStyle) -> Result<f64> {
        layout::text_width(self, text, style)
    }

    /// Break text into visible lines that fit `max_width` in points, preserving
    /// spaces and blank lines. Hard separator scalars are omitted from this
    /// presentation-only return value; `draw_paragraph` retains them in PDF text.
    pub fn wrap_text(&self, text: &str, max_width: f64, style: &TextStyle) -> Result<Vec<String>> {
        Ok(layout::prepare(self, text, max_width, style)?
            .into_iter()
            .map(|line| line.visual)
            .collect())
    }

    /// Draw wrapped paragraph text on this exact page. Returns the emitted
    /// lines. U+000C is rejected because a page builder cannot create its
    /// physical successor; use [`FlowDocument::add_paragraph`] for form feed.
    pub fn draw_paragraph(
        &mut self,
        text: &str,
        x: f64,
        y: f64,
        max_width: f64,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
    ) -> Result<Vec<String>> {
        if text.chars().any(crate::fonts::hard_break::is_form_feed) {
            return Err(WellfriendError::UnsupportedFeature(
                "a page-local paragraph cannot own a physical page break; use FlowDocument::add_paragraph"
                    .into(),
            ));
        }
        let lines = layout::prepare_with_tabs(self, text, max_width, style, &paragraph.tab_stops)?;
        let line_height = paragraph.line_height_points(style.size)?;
        let commands = lines
            .iter()
            .enumerate()
            .map(|(idx, line)| {
                line.command(
                    line.aligned_x(x, max_width, paragraph.align),
                    y - idx as f64 * line_height,
                    style,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        self.commands.extend(commands);
        Ok(lines.into_iter().map(|line| line.visual).collect())
    }

    pub fn draw_line(
        &mut self,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        style: &GraphicsStyle,
    ) -> &mut Self {
        self.commands.push(PageCommand::Path {
            path: PathBuilder::new().move_to(x1, y1).line_to(x2, y2),
            style: style.clone().stroke_only_if_unpainted(),
        });
        self
    }

    pub fn draw_rect(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        style: &GraphicsStyle,
    ) -> &mut Self {
        self.commands.push(PageCommand::Rect {
            x,
            y,
            width,
            height,
            style: style.clone(),
        });
        self
    }

    pub fn draw_rounded_rect(
        &mut self,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        radius: f64,
        style: &GraphicsStyle,
    ) -> &mut Self {
        let radius = radius
            .max(0.0)
            .min(width.abs() / 2.0)
            .min(height.abs() / 2.0);
        let x2 = x + width;
        let y2 = y + height;
        let k = radius * KAPPA;
        let path = PathBuilder::new()
            .move_to(x + radius, y)
            .line_to(x2 - radius, y)
            .curve_to(x2 - radius + k, y, x2, y + radius - k, x2, y + radius)
            .line_to(x2, y2 - radius)
            .curve_to(x2, y2 - radius + k, x2 - radius + k, y2, x2 - radius, y2)
            .line_to(x + radius, y2)
            .curve_to(x + radius - k, y2, x, y2 - radius + k, x, y2 - radius)
            .line_to(x, y + radius)
            .curve_to(x, y + radius - k, x + radius - k, y, x + radius, y)
            .close();
        self.draw_path(path, style)
    }

    pub fn draw_circle(
        &mut self,
        cx: f64,
        cy: f64,
        radius: f64,
        style: &GraphicsStyle,
    ) -> &mut Self {
        self.draw_ellipse(cx, cy, radius, radius, style)
    }

    pub fn draw_ellipse(
        &mut self,
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
        style: &GraphicsStyle,
    ) -> &mut Self {
        let kx = rx * KAPPA;
        let ky = ry * KAPPA;
        let path = PathBuilder::new()
            .move_to(cx + rx, cy)
            .curve_to(cx + rx, cy + ky, cx + kx, cy + ry, cx, cy + ry)
            .curve_to(cx - kx, cy + ry, cx - rx, cy + ky, cx - rx, cy)
            .curve_to(cx - rx, cy - ky, cx - kx, cy - ry, cx, cy - ry)
            .curve_to(cx + kx, cy - ry, cx + rx, cy - ky, cx + rx, cy)
            .close();
        self.draw_path(path, style)
    }

    pub fn draw_polygon(&mut self, points: &[(f64, f64)], style: &GraphicsStyle) -> &mut Self {
        if points.is_empty() {
            return self;
        }
        let mut path = PathBuilder::new().move_to(points[0].0, points[0].1);
        for &(x, y) in &points[1..] {
            path = path.line_to(x, y);
        }
        self.draw_path(path.close(), style)
    }

    pub fn draw_path(&mut self, path: PathBuilder, style: &GraphicsStyle) -> &mut Self {
        self.commands.push(PageCommand::Path {
            path,
            style: style.clone(),
        });
        self
    }

    /// Place a registered image in the rectangle `(x, y, width, height)`.
    pub fn draw_image(
        &mut self,
        image: ImageHandle,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> &mut Self {
        self.commands.push(PageCommand::Image {
            image,
            x,
            y,
            width,
            height,
        });
        self
    }

    fn fonts_used(&self) -> Vec<FontFace> {
        let mut out = Vec::new();
        for command in self.commands.iter().flat_map(text_commands) {
            if let PageCommand::Text { style, .. } = command {
                push_unique_font(&mut out, style.font);
            } else if matches!(command, PageCommand::LogicalBreak { .. }) {
                push_unique_font(&mut out, FontFace::BuiltinUnicode);
            }
        }
        out
    }

    fn font_program(&self, font: FontFace) -> Result<Option<&[u8]>> {
        match font {
            FontFace::Standard(_) => Ok(None),
            FontFace::BuiltinUnicode => Ok(Some(builtin_unicode_font_bytes()?)),
            FontFace::Custom(id) => self
                .custom_fonts
                .get(id.0 as usize)
                .map(|font| Some(font.bytes.as_ref()))
                .ok_or_else(|| {
                    WellfriendError::invalid_input(
                        "authoring custom font is not registered on this page's document",
                    )
                }),
            FontFace::Fallback(_) => Err(WellfriendError::invalid_input(
                "fallback stack requires resolved font runs",
            )),
        }
    }

    fn custom_font_asset(&self, font: FontFace) -> Option<Arc<[u8]>> {
        if let FontFace::Custom(id) = font {
            self.custom_fonts
                .get(id.0 as usize)
                .map(|font| Arc::clone(&font.bytes))
        } else {
            None
        }
    }

    fn images_used(&self) -> Vec<ImageHandle> {
        let mut out = Vec::new();
        for command in &self.commands {
            if let PageCommand::Image { image, .. } = command {
                push_unique_image(&mut out, *image);
            }
        }
        out
    }
}

/// Text font selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FontFace {
    Standard(StandardFont),
    /// Bundled Liberation Sans, embedded as a Type0 TrueType font with
    /// ToUnicode. This is the Part-1 Unicode authoring baseline.
    BuiltinUnicode,
    /// Document-registered custom TrueType or OpenType/CFF1 font.
    Custom(CustomFontId),
    /// Explicit contextual fallback, resolved into physical embedded runs
    /// before a drawing command is appended.
    Fallback(FontStackId),
}

impl Default for FontFace {
    fn default() -> Self {
        Self::Standard(StandardFont::Helvetica)
    }
}

impl FontFace {
    fn is_embedded_unicode(self) -> bool {
        matches!(
            self,
            Self::BuiltinUnicode | Self::Custom(_) | Self::Fallback(_)
        )
    }
}

/// The PDF Standard-14 font faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StandardFont {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
    Symbol,
    ZapfDingbats,
}

impl StandardFont {
    pub fn base_font_name(self) -> &'static str {
        match self {
            Self::Helvetica => "Helvetica",
            Self::HelveticaBold => "Helvetica-Bold",
            Self::HelveticaOblique => "Helvetica-Oblique",
            Self::HelveticaBoldOblique => "Helvetica-BoldOblique",
            Self::TimesRoman => "Times-Roman",
            Self::TimesBold => "Times-Bold",
            Self::TimesItalic => "Times-Italic",
            Self::TimesBoldItalic => "Times-BoldItalic",
            Self::Courier => "Courier",
            Self::CourierBold => "Courier-Bold",
            Self::CourierOblique => "Courier-Oblique",
            Self::CourierBoldOblique => "Courier-BoldOblique",
            Self::Symbol => "Symbol",
            Self::ZapfDingbats => "ZapfDingbats",
        }
    }

    fn fallback_font_name(self) -> &'static str {
        self.base_font_name()
    }

    fn built_in_encoding(self) -> Option<&'static str> {
        match self {
            Self::Symbol => Some("SymbolEncoding"),
            Self::ZapfDingbats => Some("ZapfDingbatsEncoding"),
            _ => None,
        }
    }
}

/// Text drawing style.
#[derive(Debug, Clone, PartialEq)]
pub struct TextStyle {
    pub font: FontFace,
    pub size: f64,
    pub fill: Color,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font: FontFace::default(),
            size: DEFAULT_FONT_SIZE,
            fill: Color::black(),
        }
    }
}

impl TextStyle {
    pub fn new(font: FontFace, size: f64) -> Self {
        Self {
            font,
            size,
            fill: Color::black(),
        }
    }

    pub fn standard(font: StandardFont, size: f64) -> Self {
        Self::new(FontFace::Standard(font), size)
    }

    pub fn unicode(size: f64) -> Self {
        Self::new(FontFace::BuiltinUnicode, size)
    }

    pub fn custom(font: CustomFontId, size: f64) -> Self {
        Self::new(FontFace::Custom(font), size)
    }

    pub fn fill(mut self, color: Color) -> Self {
        self.fill = color;
        self
    }
}

/// Paragraph alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// Paragraph helper options.
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphStyle {
    pub align: TextAlign,
    /// Multiplier over the text size. `1.2` is the default.
    pub line_height: f64,
    /// Inline-axis tab positions. U+0009 remains logical text and is emitted as
    /// exact positioned fields rather than a font glyph or guessed spaces.
    pub tab_stops: TabStops,
}

impl Default for ParagraphStyle {
    fn default() -> Self {
        Self {
            align: TextAlign::Left,
            line_height: DEFAULT_LINE_HEIGHT,
            tab_stops: TabStops::default(),
        }
    }
}

impl ParagraphStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }

    pub fn line_height(mut self, line_height: f64) -> Self {
        self.line_height = line_height;
        self
    }

    pub fn tab_stops(mut self, tab_stops: TabStops) -> Self {
        self.tab_stops = tab_stops;
        self
    }

    fn line_height_points(&self, font_size: f64) -> Result<f64> {
        let value = font_size * self.line_height;
        if !font_size.is_finite()
            || font_size <= 0.0
            || !self.line_height.is_finite()
            || self.line_height <= 0.0
            || !value.is_finite()
            || value <= 0.0
        {
            return Err(WellfriendError::invalid_input(
                "invalid authoring line height",
            ));
        }
        Ok(value)
    }
}

/// One table column with fixed width in PDF points.
#[derive(Debug, Clone, PartialEq)]
pub struct TableColumn {
    pub width: f64,
    pub align: TextAlign,
}

impl TableColumn {
    pub fn new(width: f64) -> Self {
        Self {
            width,
            align: TextAlign::Left,
        }
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = align;
        self
    }
}

/// Styling shared by authored tables.
#[derive(Debug, Clone, PartialEq)]
pub struct TableStyle {
    pub border_color: Color,
    pub header_fill: Color,
    pub row_fill: Option<Color>,
    pub padding: f64,
    pub line_width: f64,
    pub paragraph: ParagraphStyle,
}

impl Default for TableStyle {
    fn default() -> Self {
        Self {
            border_color: Color::device_rgb(0.28, 0.32, 0.36),
            header_fill: Color::device_rgb(0.9, 0.93, 0.96),
            row_fill: None,
            padding: 4.0,
            line_width: 0.5,
            paragraph: ParagraphStyle::new(),
        }
    }
}

impl TableStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn padding(mut self, padding: f64) -> Self {
        self.padding = padding.max(0.0);
        self
    }

    pub fn border(mut self, color: Color, line_width: f64) -> Self {
        self.border_color = color;
        self.line_width = line_width.max(0.0);
        self
    }

    pub fn header_fill(mut self, color: Color) -> Self {
        self.header_fill = color;
        self
    }

    pub fn row_fill(mut self, color: Option<Color>) -> Self {
        self.row_fill = color;
        self
    }
}

/// Semantic scope for an authored table-header cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableHeaderScope {
    Row,
    Column,
    Both,
}

/// A text cell in an authored table.
#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub text: String,
    pub style: Option<TextStyle>,
    pub background: Option<Color>,
    pub align: Option<TextAlign>,
    /// Optional semantic header scope. Cells in the dedicated header row
    /// default to `Column`; body cells remain data unless explicitly marked.
    pub header_scope: Option<TableHeaderScope>,
    /// Number of consecutive fixed grid columns occupied by this cell.
    pub column_span: usize,
    /// Number of consecutive body rows occupied by this cell. The dedicated
    /// repeatable header is a separate row group and cannot span into body rows.
    pub row_span: usize,
    /// Optional stable typed-value identity and exact value/formula. These two
    /// fields must be present together; evaluated text replaces `text` only in
    /// the private layout clone.
    pub typed_id: Option<String>,
    pub typed_value: Option<crate::typed_tables::TableValue>,
}

impl TableCell {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            style: None,
            background: None,
            align: None,
            header_scope: None,
            column_span: 1,
            row_span: 1,
            typed_id: None,
            typed_value: None,
        }
    }

    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = Some(style);
        self
    }

    pub fn background(mut self, color: Color) -> Self {
        self.background = Some(color);
        self
    }

    pub fn align(mut self, align: TextAlign) -> Self {
        self.align = Some(align);
        self
    }

    pub fn header_scope(mut self, scope: TableHeaderScope) -> Self {
        self.header_scope = Some(scope);
        self
    }

    pub fn row_header(self) -> Self {
        self.header_scope(TableHeaderScope::Row)
    }

    pub fn column_span(mut self, columns: usize) -> Self {
        self.column_span = columns;
        self
    }

    pub fn row_span(mut self, rows: usize) -> Self {
        self.row_span = rows;
        self
    }

    pub fn typed(id: impl Into<String>, value: crate::typed_tables::TableValue) -> Self {
        Self {
            typed_id: Some(id.into()),
            typed_value: Some(value),
            ..Self::text("")
        }
    }
}

impl From<&str> for TableCell {
    fn from(value: &str) -> Self {
        Self::text(value)
    }
}

impl From<String> for TableCell {
    fn from(value: String) -> Self {
        Self::text(value)
    }
}

/// A table row.
#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    pub cells: Vec<TableCell>,
    /// A physical page transition owned by this row. It is honored only by
    /// `FlowDocument`; page-local table drawing rejects it.
    pub page_break_before: Option<FlowPageBreak>,
}

impl TableRow {
    pub fn new(cells: Vec<TableCell>) -> Self {
        Self {
            cells,
            page_break_before: None,
        }
    }

    pub fn page_break_before(mut self, policy: FlowPageBreak) -> Self {
        self.page_break_before = Some(policy);
        self
    }
}

/// Fixed-column table renderer with wrapped text and repeatable headers.
#[derive(Debug, Clone, PartialEq)]
pub struct TableBuilder {
    columns: Vec<TableColumn>,
    header: Option<TableRow>,
    rows: Vec<TableRow>,
    body_style: TextStyle,
    header_style: TextStyle,
    style: TableStyle,
    row_split_policy: TableRowSplitPolicy,
    caption: Option<String>,
    caption_style: TextStyle,
    caption_paragraph: ParagraphStyle,
    summary: Option<String>,
    identity: Option<String>,
}

#[derive(Debug, Clone)]
struct TableCellPlacement<'a> {
    source: Option<(usize, &'a TableCell)>,
    columns: std::ops::Range<usize>,
    row_start: usize,
    row_span: usize,
}

impl TableBuilder {
    pub fn new(columns: Vec<TableColumn>) -> Self {
        Self {
            columns,
            header: None,
            rows: Vec::new(),
            body_style: TextStyle::standard(StandardFont::Helvetica, 9.0),
            header_style: TextStyle::standard(StandardFont::HelveticaBold, 9.0),
            style: TableStyle::default(),
            row_split_policy: TableRowSplitPolicy::default(),
            caption: None,
            caption_style: TextStyle::standard(StandardFont::HelveticaBold, 9.0),
            caption_paragraph: ParagraphStyle::new(),
            summary: None,
            identity: None,
        }
    }

    pub fn body_style(mut self, style: TextStyle) -> Self {
        self.body_style = style;
        self
    }

    /// Rows that fit a fresh page stay whole. Oversized rows may split at
    /// measured line boundaries according to this policy. Headers stay whole.
    pub fn row_split_policy(mut self, policy: TableRowSplitPolicy) -> Self {
        self.row_split_policy = policy;
        self
    }

    pub fn header_style(mut self, style: TextStyle) -> Self {
        self.header_style = style;
        self
    }

    pub fn style(mut self, style: TableStyle) -> Self {
        self.style = style;
        self
    }

    /// Add a visible caption before the table. The caption is kept with the
    /// first semantic header/body fragment when it can fit a fresh page.
    pub fn caption(mut self, text: impl Into<String>) -> Self {
        self.caption = Some(text.into());
        self
    }

    pub fn caption_style(mut self, style: TextStyle, paragraph: ParagraphStyle) -> Self {
        self.caption_style = style;
        self.caption_paragraph = paragraph;
        self
    }

    /// Add a bounded accessibility summary to the `/Table` element.
    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Stable identity used by exact typed-value/formula evaluation.
    pub fn identity(mut self, identity: impl Into<String>) -> Self {
        self.identity = Some(identity.into());
        self
    }

    pub fn set_header<I, C>(&mut self, cells: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: Into<TableCell>,
    {
        self.header = Some(TableRow::new(cells.into_iter().map(Into::into).collect()));
        self
    }

    pub fn add_row<I, C>(&mut self, cells: I) -> &mut Self
    where
        I: IntoIterator<Item = C>,
        C: Into<TableCell>,
    {
        self.rows
            .push(TableRow::new(cells.into_iter().map(Into::into).collect()));
        self
    }

    /// Append a fully configured row, including an optional physical
    /// page-break-before policy.
    pub fn push_row(&mut self, row: TableRow) -> &mut Self {
        self.rows.push(row);
        self
    }

    pub fn rows(&self) -> &[TableRow] {
        &self.rows
    }

    pub fn columns(&self) -> &[TableColumn] {
        &self.columns
    }

    /// Draw the whole table on one page at a top-left anchor and return the
    /// consumed height. Long tables should use [`FlowDocument::add_table`].
    pub fn draw_on_page(&self, page: &mut PdfPageBuilder, x: f64, top_y: f64) -> Result<f64> {
        tables::draw_on_page(self, page, x, top_y)
    }

    fn total_width(&self) -> f64 {
        self.columns.iter().map(|col| col.width.max(0.0)).sum()
    }

    fn validate(&self) -> Result<()> {
        self.row_split_policy.validate()?;
        if self.columns.len() > 10_000 || self.rows.len() > 100_000 {
            return Err(WellfriendError::ResourceLimit(
                "authoring table dimensions".into(),
            ));
        }
        if self.columns.is_empty() && (self.header.is_some() || !self.rows.is_empty())
            || self.columns.iter().any(|column| {
                !column.width.is_finite() || column.width - self.style.padding * 2.0 <= 0.0
            })
            || !self.total_width().is_finite()
            || !self.style.padding.is_finite()
            || self.style.padding < 0.0
            || !self.style.line_width.is_finite()
            || self.style.line_width < 0.0
            || !(self.style.padding * 2.0 + self.body_style.size).is_finite()
            || !self.body_style.size.is_finite()
            || self.body_style.size <= 0.0
            || self
                .rows
                .iter()
                .chain(self.header.iter())
                .flat_map(|row| &row.cells)
                .any(|cell| {
                    cell.text
                        .chars()
                        .any(crate::fonts::hard_break::is_form_feed)
                })
            || self.caption.as_ref().is_some_and(|caption| {
                caption.is_empty()
                    || caption.len() > 16 * 1024 * 1024
                    || caption.chars().any(crate::fonts::hard_break::is_form_feed)
            })
            || self.summary.as_ref().is_some_and(|summary| {
                summary.is_empty()
                    || summary.len() > 16 * 1024
                    || summary.chars().any(|ch| ch == '\0')
            })
            || self.identity.as_ref().is_some_and(|identity| {
                identity.is_empty() || identity.len() > 16 * 1024 || identity.contains('\0')
            })
            || (self.header.is_none()
                && self.rows.is_empty()
                && (self.caption.is_some() || self.summary.is_some()))
        {
            return Err(WellfriendError::invalid_input("invalid authoring table geometry/cells; physical page breaks require row-level pagination, not cell text"));
        }
        if let Some(header) = &self.header {
            if header.cells.iter().any(|cell| cell.row_span != 1) {
                return Err(WellfriendError::invalid_input(
                    "the repeatable authored table header cannot span into body rows",
                ));
            }
            self.cell_placements(header)?;
        }
        self.body_cell_placements()?;
        self.typed_values()?;
        Ok(())
    }

    fn typed_values(&self) -> Result<BTreeMap<String, String>> {
        if self.header.as_ref().is_some_and(|row| {
            row.cells
                .iter()
                .any(|cell| cell.typed_id.is_some() || cell.typed_value.is_some())
        }) {
            return Err(WellfriendError::invalid_input(
                "authored repeatable headers cannot own typed values",
            ));
        }
        let grid = self.body_cell_placements()?;
        let mut entries = Vec::new();
        for placements in &grid {
            for placement in placements {
                let Some((_, cell)) = placement.source else {
                    continue;
                };
                match (&cell.typed_id, &cell.typed_value) {
                    (None, None) => {}
                    (Some(id), Some(value))
                        if !id.is_empty()
                            && id.len() <= 16 * 1024
                            && !id.chars().any(|ch| ch == '\0') =>
                    {
                        entries.push((
                            id.as_str(),
                            placement.row_start,
                            placement.columns.start,
                            value,
                        ))
                    }
                    _ => {
                        return Err(WellfriendError::invalid_input(
                            "authored typed table cell requires a bounded nonempty identity and value",
                        ));
                    }
                }
            }
        }
        if entries.is_empty() {
            return Ok(BTreeMap::new());
        }
        let identity = self.identity.as_deref().ok_or_else(|| {
            WellfriendError::invalid_input(
                "authored typed table cells require a stable table identity",
            )
        })?;
        crate::typed_tables::evaluate_values(identity, &entries)
    }

    fn resolved_typed_values(&self) -> Result<(Self, BTreeMap<String, String>)> {
        self.validate()?;
        let values = self.typed_values()?;
        let mut resolved = self.clone();
        if !values.is_empty() {
            for row in &mut resolved.rows {
                for cell in &mut row.cells {
                    if let Some(id) = &cell.typed_id {
                        cell.text = values.get(id).cloned().ok_or_else(|| {
                            WellfriendError::invalid_input(
                                "authored typed value lost its evaluated result",
                            )
                        })?;
                    }
                }
            }
        }
        Ok((resolved, values))
    }

    fn cell_placements<'a>(
        &'a self,
        row: &'a TableRow,
    ) -> Result<Vec<(Option<(usize, &'a TableCell)>, std::ops::Range<usize>)>> {
        let mut placements = Vec::with_capacity(row.cells.len().max(self.columns.len()));
        let mut column = 0usize;
        for (source_index, cell) in row.cells.iter().enumerate() {
            if cell.column_span == 0 || cell.row_span == 0 {
                return Err(WellfriendError::invalid_input(
                    "authored table row and column spans must be positive",
                ));
            }
            let end = column.checked_add(cell.column_span).ok_or_else(|| {
                WellfriendError::ResourceLimit("authored table column span overflow".into())
            })?;
            if end > self.columns.len() {
                return Err(WellfriendError::invalid_input(
                    "authored table cells exceed the fixed column grid",
                ));
            }
            placements.push((Some((source_index, cell)), column..end));
            column = end;
        }
        while column < self.columns.len() {
            placements.push((None, column..column + 1));
            column += 1;
        }
        Ok(placements)
    }

    fn body_cell_placements(&self) -> Result<Vec<Vec<TableCellPlacement<'_>>>> {
        let mut occupied_until = vec![0usize; self.columns.len()];
        let mut rows = Vec::with_capacity(self.rows.len());
        for (row_index, row) in self.rows.iter().enumerate() {
            if row.page_break_before.is_some() && occupied_until.iter().any(|end| *end > row_index)
            {
                return Err(WellfriendError::invalid_input(
                    "authored row page break cannot split an active row span",
                ));
            }
            let mut placements = Vec::with_capacity(row.cells.len().max(self.columns.len()));
            let mut cursor = 0usize;
            for (source_index, cell) in row.cells.iter().enumerate() {
                if cell.column_span == 0 || cell.row_span == 0 {
                    return Err(WellfriendError::invalid_input(
                        "authored table row and column spans must be positive",
                    ));
                }
                let row_end = row_index.checked_add(cell.row_span).ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored table row span overflow".into())
                })?;
                if row_end > self.rows.len() {
                    return Err(WellfriendError::invalid_input(
                        "authored table row span exceeds the body row grid",
                    ));
                }
                let start = (cursor..self.columns.len())
                    .find(|&candidate| {
                        candidate
                            .checked_add(cell.column_span)
                            .is_some_and(|end| end <= self.columns.len())
                            && (candidate..candidate + cell.column_span)
                                .all(|column| occupied_until[column] <= row_index)
                    })
                    .ok_or_else(|| {
                        WellfriendError::invalid_input(
                            "authored table cell cannot fit the remaining row grid beside active row spans",
                        )
                    })?;
                let end = start + cell.column_span;
                for column in start..end {
                    occupied_until[column] = row_end;
                }
                placements.push(TableCellPlacement {
                    source: Some((source_index, cell)),
                    columns: start..end,
                    row_start: row_index,
                    row_span: cell.row_span,
                });
                cursor = end;
            }
            for column in 0..self.columns.len() {
                if occupied_until[column] <= row_index {
                    occupied_until[column] = row_index + 1;
                    placements.push(TableCellPlacement {
                        source: None,
                        columns: column..column + 1,
                        row_start: row_index,
                        row_span: 1,
                    });
                }
            }
            placements.sort_by_key(|placement| placement.columns.start);
            rows.push(placements);
        }
        Ok(rows)
    }

    fn has_row_spans(&self) -> bool {
        self.rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.row_span > 1)
    }

    #[cfg(test)]
    fn measure_row(&self, page: &PdfPageBuilder, row: &TableRow, header: bool) -> Result<f64> {
        let (resolved, _) = self.resolved_typed_values()?;
        let resolved_row = if header {
            resolved.header.as_ref().ok_or_else(|| {
                WellfriendError::invalid_input("authored table has no header row to measure")
            })?
        } else {
            let index = self
                .rows
                .iter()
                .position(|candidate| std::ptr::eq(candidate, row))
                .or_else(|| self.rows.iter().position(|candidate| candidate == row))
                .ok_or_else(|| {
                    WellfriendError::invalid_input(
                        "authored row measurement requires a row owned by this table",
                    )
                })?;
            &resolved.rows[index]
        };
        let prepared = tables::PreparedRow::new(
            &resolved,
            page,
            resolved_row,
            header,
            &mut tables::Budget::default(),
        )?;
        prepared.remaining_height(&vec![0; prepared.cell_count()])
    }

    fn cell_style(&self, cell: Option<&TableCell>, header: bool) -> TextStyle {
        cell.and_then(|cell| cell.style.clone()).unwrap_or_else(|| {
            if header {
                self.header_style.clone()
            } else {
                self.body_style.clone()
            }
        })
    }
}

/// Single-column layout helper that creates pages as content overflows.
#[derive(Debug, Clone)]
pub struct FlowDocument {
    builder: PdfBuilder,
    page_size: PageSize,
    margins: Margins,
    current_page: usize,
    current_section: usize,
    document_footnote_next: usize,
    document_footnote_started: bool,
    section_footnote_next: usize,
    section_footnote_started: bool,
    outline_heading_path: Vec<String>,
    cursor_y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowPageBreak {
    NextPage,
    NextOddPage,
    NextEvenPage,
}
impl FlowPageBreak {
    fn accepts(self, page: usize) -> bool {
        match self {
            Self::NextPage => true,
            Self::NextOddPage => page % 2 == 1,
            Self::NextEvenPage => page.is_multiple_of(2),
        }
    }
}

fn heading_presentation(level: u8) -> (TextStyle, ParagraphStyle, f64) {
    let size = match level {
        0 | 1 => 22.0,
        2 => 16.0,
        _ => 13.0,
    };
    (
        TextStyle::standard(StandardFont::HelveticaBold, size)
            .fill(Color::device_rgb(0.08, 0.12, 0.16)),
        ParagraphStyle::new().line_height(if level <= 1 { 1.15 } else { 1.2 }),
        if level <= 1 { 8.0 } else { 5.0 },
    )
}

impl FlowDocument {
    pub fn new(page_size: PageSize, margins: Margins) -> Self {
        Self::from_section_unchecked(FlowSection::new(page_size, margins))
    }

    /// Create a flow document whose first page is owned by an explicit section.
    pub fn from_section(section: FlowSection) -> Result<Self> {
        section.validate()?;
        Ok(Self::from_section_unchecked(section))
    }

    fn from_section_unchecked(section: FlowSection) -> Self {
        let mut builder = PdfBuilder::new();
        let page_size = section.page_size;
        let margins = section.effective_margins(1);
        let note_start = section.footnote_numbering.start;
        builder.sections.push(section);
        builder
            .add_page_with_margins(page_size, margins)
            .section_index = Some(0);
        Self {
            builder,
            page_size,
            margins,
            current_page: 0,
            current_section: 0,
            document_footnote_next: note_start,
            document_footnote_started: false,
            section_footnote_next: note_start,
            section_footnote_started: false,
            outline_heading_path: Vec::new(),
            cursor_y: page_size.height - margins.top,
        }
    }

    pub fn builder(&self) -> &PdfBuilder {
        &self.builder
    }

    pub fn builder_mut(&mut self) -> &mut PdfBuilder {
        &mut self.builder
    }

    /// Replace the final document outline while retaining forward references
    /// to anchors that may be declared by later flow content.
    pub fn set_outline(&mut self, entries: Vec<PdfOutlineEntry>) -> Result<&mut Self> {
        self.builder.set_outline(entries)?;
        self.outline_heading_path.clear();
        Ok(self)
    }

    pub fn sections(&self) -> &[FlowSection] {
        &self.builder.sections
    }

    /// Update the active section before serialization. This supports running
    /// text using a font registered through `builder_mut()` after construction.
    pub fn current_section_mut(&mut self) -> &mut FlowSection {
        &mut self.builder.sections[self.current_section]
    }

    pub fn into_builder(self) -> PdfBuilder {
        self.builder
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.builder.save(path)
    }

    pub fn add_heading(&mut self, text: &str, level: u8) -> Result<&mut Self> {
        let (style, paragraph, spacer) = heading_presentation(level);
        self.add_structured_paragraph(
            text,
            &style,
            &paragraph,
            structure::Role::heading(level),
            Some(text.to_string()),
        )?;
        self.add_spacer(spacer);
        Ok(self)
    }

    /// Atomically bind the current position, paint a heading and append its
    /// hierarchical document-outline entry. Levels must start at one and may
    /// increase by at most one; decreasing levels closes prior branches.
    pub fn add_outlined_heading(
        &mut self,
        text: &str,
        level: u8,
        anchor: impl Into<String>,
    ) -> Result<BodyAnchorInfo> {
        let anchor = anchor.into();
        outline::validate_heading_append(
            &self.builder,
            &self.outline_heading_path,
            text,
            &anchor,
            level,
        )?;
        let (style, paragraph, spacer) = heading_presentation(level);
        let width = self.content_width()?;
        let lines = layout::prepare(self.current_page_ref(), text, width, &style)?;
        if lines.is_empty() {
            return Err(WellfriendError::invalid_input(
                "outlined heading produced no line",
            ));
        }
        let line_height = paragraph.line_height_points(style.size)?;
        let previous_path = self.outline_heading_path.clone();
        outline::append_heading(
            &mut self.builder,
            &mut self.outline_heading_path,
            text.to_string(),
            anchor.clone(),
            level,
        )?;
        let mut anchor_info = None;
        let result = self.append_transaction(|flow| {
            let heading_structure = structure::register(
                &mut flow.builder,
                structure::Role::heading(level),
                None,
                Some(text.to_string()),
            )?;
            for (index, line) in lines.into_iter().enumerate() {
                crate::cancel::check_current_cancel("outlined heading layout")?;
                let height = line.occupied_height(line_height);
                flow.ensure_space(height)?;
                if index == 0 {
                    anchor_info = Some(fields::add_anchor(flow, anchor.clone())?);
                }
                let x = line.aligned_x(flow.margins.left, width, paragraph.align);
                let command = line.command(x, flow.cursor_y - line.metrics.ascent, &style)?;
                flow.current_page_mut()
                    .commands
                    .push(PageCommand::BeginStructure(heading_structure));
                flow.current_page_mut().commands.push(command);
                flow.current_page_mut()
                    .commands
                    .push(PageCommand::EndStructure(heading_structure));
                flow.cursor_y -= height;
            }
            flow.add_spacer(spacer);
            Ok(())
        });
        if let Err(error) = result {
            if anchor_info.is_some() {
                self.builder.anchors.remove(&anchor);
            }
            outline::rollback_heading(
                &mut self.builder,
                &mut self.outline_heading_path,
                previous_path,
                &anchor,
            )?;
            return Err(error);
        }
        anchor_info.ok_or_else(|| {
            WellfriendError::invalid_input("outlined heading produced no anchorable line")
        })
    }

    /// Paint the currently configured document outline as a transactional,
    /// paginated table of contents with clickable deferred page values.
    pub fn add_table_of_contents(
        &mut self,
        style: &TableOfContentsStyle,
    ) -> Result<TableOfContentsReport> {
        toc::append(self, style)
    }

    /// Build a table of contents in an isolated front-matter section and
    /// prepend it before all existing pages. An automatic suppressed parity
    /// page keeps every pre-existing odd/even body page on the same side.
    pub fn prepend_table_of_contents(
        &mut self,
        section: FlowSection,
        style: &TableOfContentsStyle,
    ) -> Result<FrontMatterTableOfContentsReport> {
        front_matter::prepend_table_of_contents(self, section, style)
    }

    /// Stage arbitrary fresh-authoring blocks in one or more front sections,
    /// then atomically prepend them while preserving every body page's parity.
    pub fn prepend_front_matter<F>(
        &mut self,
        section: FlowSection,
        author: F,
    ) -> Result<FrontMatterReport>
    where
        F: FnOnce(&mut FlowDocument) -> Result<()>,
    {
        front_matter::prepend(self, section, author)
    }

    /// Append a transactional back-of-document index. Every occurrence must
    /// reference an already-declared anchor so physical page ordering and
    /// same-page deduplication are stable before any index page is mutated.
    pub fn add_document_index(
        &mut self,
        entries: &[PdfIndexEntry],
        style: &DocumentIndexStyle,
    ) -> Result<DocumentIndexReport> {
        document_index::append(self, entries, style)
    }

    pub fn add_paragraph(
        &mut self,
        text: &str,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
    ) -> Result<&mut Self> {
        self.add_structured_paragraph(text, style, paragraph, structure::Role::Paragraph, None)
    }

    fn add_structured_paragraph(
        &mut self,
        text: &str,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
        role: structure::Role,
        title: Option<String>,
    ) -> Result<&mut Self> {
        let width = self.content_width()?;
        let lines = layout::prepare_with_tabs(
            self.current_page_ref(),
            text,
            width,
            style,
            &paragraph.tab_stops,
        )?;
        let line_height = paragraph.line_height_points(style.size)?;
        self.append_transaction(|flow| {
            let structure_id = if lines.is_empty() {
                None
            } else {
                Some(structure::register(&mut flow.builder, role, None, title)?)
            };
            for line in lines {
                crate::cancel::check_current_cancel("authoring flow paragraph")?;
                let force_page = line
                    .logical
                    .chars()
                    .next_back()
                    .is_some_and(crate::fonts::hard_break::is_form_feed);
                let height = line.occupied_height(line_height);
                flow.ensure_space(height)?;
                let x = line.aligned_x(flow.margins.left, width, paragraph.align);
                let command = line.command(x, flow.cursor_y - line.metrics.ascent, style)?;
                if let Some(structure_id) = structure_id {
                    flow.current_page_mut()
                        .commands
                        .push(PageCommand::BeginStructure(structure_id));
                }
                flow.current_page_mut().commands.push(command);
                if let Some(structure_id) = structure_id {
                    flow.current_page_mut()
                        .commands
                        .push(PageCommand::EndStructure(structure_id));
                }
                flow.cursor_y -= height;
                if force_page {
                    flow.add_page_break_to(FlowPageBreak::NextPage);
                }
            }
            Ok(())
        })
    }

    /// Add a paragraph whose explicit source ranges own page footnotes. The
    /// selected marker text remains part of the body paragraph; each note is
    /// laid out in the reserved page-bottom region and may continue on later
    /// pages without overlapping subsequent flow content.
    pub fn add_paragraph_with_footnotes(
        &mut self,
        text: &str,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
        footnotes: &[FlowFootnote],
    ) -> Result<FootnotedParagraphReport> {
        let mut report = FootnotedParagraphReport::default();
        self.append_transaction(|flow| {
            report = notes::append_paragraph(flow, text, style, paragraph, footnotes)?;
            Ok(())
        })?;
        Ok(report)
    }

    /// Insert automatically numbered marker text at exact original UTF-8
    /// offsets, then lay out those inserted ranges through the same footnote
    /// transaction. The report preserves the original-to-enriched mapping.
    pub fn add_numbered_footnoted_paragraph(
        &mut self,
        text: &str,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
        footnotes: &[NumberedFootnote],
    ) -> Result<NumberedFootnotedParagraphReport> {
        let mut report = NumberedFootnotedParagraphReport::default();
        self.append_transaction(|flow| {
            report = notes::append_numbered_paragraph(flow, text, style, paragraph, footnotes)?;
            Ok(())
        })?;
        Ok(report)
    }

    /// Bind a stable name to the current flow position. The final PDF catalog
    /// publishes it as a named XYZ destination, and deferred body fields may
    /// reference its physical or section page before or after declaration.
    pub fn add_anchor(&mut self, name: impl Into<String>) -> Result<BodyAnchorInfo> {
        fields::add_anchor(self, name.into())
    }

    /// Add a paragraph containing fixed-capacity deferred document/section or
    /// named-anchor page fields. Final values resolve on a private clone.
    pub fn add_field_paragraph(
        &mut self,
        parts: &[BodyFieldPart],
        style: &TextStyle,
        paragraph: &ParagraphStyle,
    ) -> Result<FieldParagraphReport> {
        let mut report = FieldParagraphReport::default();
        self.append_transaction(|flow| {
            let structure_id =
                structure::register(&mut flow.builder, structure::Role::Paragraph, None, None)?;
            report = fields::append_paragraph(flow, parts, style, paragraph, Some(structure_id))?;
            Ok(())
        })?;
        Ok(report)
    }

    /// Begin an endnote collection on a later physical page and flow every
    /// typed note through the same paragraph/font/page transaction.
    pub fn add_endnotes(
        &mut self,
        endnotes: &[FlowEndnote],
        break_before: FlowPageBreak,
    ) -> Result<EndnoteReport> {
        let mut report = EndnoteReport::default();
        self.append_transaction(|flow| {
            report = notes::append_endnotes(flow, endnotes, break_before)?;
            Ok(())
        })?;
        Ok(report)
    }

    pub fn add_list<I, S>(
        &mut self,
        items: I,
        ordered: bool,
        style: &TextStyle,
        paragraph: &ParagraphStyle,
    ) -> Result<&mut Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut collected_items = Vec::new();
        for item in items.into_iter() {
            if collected_items.len() >= 100_000 {
                return Err(WellfriendError::ResourceLimit(
                    "authoring list item count".into(),
                ));
            }
            collected_items.push(item.as_ref().to_string());
        }
        let base_left = self.margins.left;
        let content_width = self.content_width()?;
        let line_height = paragraph.line_height_points(style.size)?;
        self.append_transaction(|flow| {
            let list_structure = if collected_items.is_empty() {
                None
            } else {
                Some(structure::register(
                    &mut flow.builder,
                    structure::Role::List,
                    None,
                    None,
                )?)
            };
            for (idx, item) in collected_items.into_iter().enumerate() {
                crate::cancel::check_current_cancel("authoring list")?;
                let item_structure = structure::register(
                    &mut flow.builder,
                    structure::Role::ListItem,
                    list_structure,
                    None,
                )?;
                let label_structure = structure::register(
                    &mut flow.builder,
                    structure::Role::ListLabel,
                    Some(item_structure),
                    None,
                )?;
                let marker = if ordered {
                    format!("{}.", idx + 1)
                } else {
                    "*".to_string()
                };
                let marker_width = flow.current_page_ref().text_width(&marker, style)?;
                let indent = (marker_width + 6.0).max(18.0);
                let width = content_width - indent;
                let lines = layout::prepare_with_tabs(
                    flow.current_page_ref(),
                    &item,
                    width,
                    style,
                    &paragraph.tab_stops,
                )?;
                let body_structure = if lines.is_empty() {
                    None
                } else {
                    Some(structure::register(
                        &mut flow.builder,
                        structure::Role::ListBody,
                        Some(item_structure),
                        None,
                    )?)
                };
                let first_height = lines
                    .first()
                    .map_or(line_height, |line| line.occupied_height(line_height));
                flow.ensure_space(first_height)?;
                let marker_ascent = lines.first().map_or(style.size, |line| line.metrics.ascent);
                let marker_y = flow.cursor_y - marker_ascent;
                flow.current_page_mut()
                    .commands
                    .push(PageCommand::BeginStructure(label_structure));
                flow.current_page_mut().draw_text(
                    marker,
                    base_left + indent - 6.0 - marker_width,
                    marker_y,
                    style,
                )?;
                flow.current_page_mut()
                    .commands
                    .push(PageCommand::EndStructure(label_structure));
                if lines.is_empty() {
                    flow.cursor_y -= first_height;
                }
                for line in lines {
                    let force_page = line
                        .logical
                        .chars()
                        .next_back()
                        .is_some_and(crate::fonts::hard_break::is_form_feed);
                    let height = line.occupied_height(line_height);
                    flow.ensure_space(height)?;
                    let command = line.command(
                        base_left + indent + line.metrics.left_pad,
                        flow.cursor_y - line.metrics.ascent,
                        style,
                    )?;
                    if let Some(body_structure) = body_structure {
                        flow.current_page_mut()
                            .commands
                            .push(PageCommand::BeginStructure(body_structure));
                    }
                    flow.current_page_mut().commands.push(command);
                    if let Some(body_structure) = body_structure {
                        flow.current_page_mut()
                            .commands
                            .push(PageCommand::EndStructure(body_structure));
                    }
                    flow.cursor_y -= height;
                    if force_page {
                        flow.add_page_break_to(FlowPageBreak::NextPage);
                    }
                }
            }
            Ok(())
        })
    }

    pub fn add_image(&mut self, image: ImageHandle, width: f64, height: f64) -> Result<&mut Self> {
        self.add_decorative_image(image, width, height)
    }

    /// Add an image that is intentionally excluded from the logical reading
    /// order. Use [`FlowDocument::add_figure`] for meaningful content.
    pub fn add_decorative_image(
        &mut self,
        image: ImageHandle,
        width: f64,
        height: f64,
    ) -> Result<&mut Self> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(WellfriendError::invalid_input(
                "flow image dimensions must be finite and positive",
            ));
        }
        self.builder.image(image)?;
        self.append_transaction(|flow| {
            flow.ensure_space(height)?;
            let x = flow.margins.left;
            let y = flow.cursor_y - height;
            flow.current_page_mut()
                .commands
                .push(PageCommand::BeginArtifact);
            flow.current_page_mut()
                .draw_image(image, x, y, width, height);
            flow.current_page_mut()
                .commands
                .push(PageCommand::EndArtifact);
            flow.cursor_y = y;
            Ok(())
        })
    }

    /// Add a meaningful image as a `/Figure` structure element with required
    /// alternate text.
    pub fn add_figure(
        &mut self,
        image: ImageHandle,
        width: f64,
        height: f64,
        alternate_text: impl Into<String>,
    ) -> Result<&mut Self> {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return Err(WellfriendError::invalid_input(
                "flow figure dimensions must be finite and positive",
            ));
        }
        self.builder.image(image)?;
        let alternate_text = alternate_text.into();
        self.append_transaction(|flow| {
            let figure = structure::register_figure(&mut flow.builder, alternate_text)?;
            flow.ensure_space(height)?;
            let x = flow.margins.left;
            let y = flow.cursor_y - height;
            flow.current_page_mut()
                .commands
                .push(PageCommand::BeginStructure(figure));
            flow.current_page_mut()
                .draw_image(image, x, y, width, height);
            flow.current_page_mut()
                .commands
                .push(PageCommand::EndStructure(figure));
            flow.cursor_y = y;
            Ok(())
        })
    }

    pub fn add_table(&mut self, table: &TableBuilder) -> Result<&mut Self> {
        self.add_table_with_report(table)?;
        Ok(self)
    }

    /// Append one atomic table transaction and disclose every header/body
    /// fragment, its page, geometry and original cell UTF-8 ranges.
    pub fn add_table_with_report(&mut self, table: &TableBuilder) -> Result<TableFlowReport> {
        let mut report = TableFlowReport::default();
        self.append_transaction(|flow| {
            report = tables::append_flow(flow, table)?;
            Ok(())
        })?;
        Ok(report)
    }

    pub fn add_spacer(&mut self, height: f64) -> &mut Self {
        let height = height.max(0.0);
        if self.cursor_y - height < self.current_bottom() {
            self.add_page_break();
        } else {
            self.cursor_y -= height;
        }
        self
    }

    pub fn add_page_break(&mut self) -> &mut Self {
        self.add_page_break_to(FlowPageBreak::NextPage)
    }

    /// Begin a section on the next physical page. Section masters and page
    /// fields are resolved only in the private serialization clone.
    pub fn start_section(&mut self, section: FlowSection) -> Result<&mut Self> {
        self.start_section_on(section, FlowPageBreak::NextPage)
    }

    /// Begin a section on the requested physical parity. Intervening blank
    /// pages remain in the previous section; the destination uses the new page
    /// size, margins, numbering and first-page master.
    pub fn start_section_on(
        &mut self,
        section: FlowSection,
        policy: FlowPageBreak,
    ) -> Result<&mut Self> {
        section.validate()?;
        while !policy.accepts(self.builder.pages.len() + 1) {
            self.add_section_page(self.current_section, true);
        }
        let section_index = self.builder.sections.len();
        self.builder.sections.push(section);
        self.current_section = section_index;
        let numbering = &self.builder.sections[section_index].footnote_numbering;
        self.section_footnote_next = numbering.start;
        self.section_footnote_started = false;
        if numbering.scope == NoteNumberScope::Document && !self.document_footnote_started {
            self.document_footnote_next = numbering.start;
        }
        self.add_section_page(section_index, false);
        Ok(self)
    }

    /// Start on a later physical page, optionally inserting an owned blank page
    /// so the destination has the requested one-based PDF page parity.
    pub fn add_page_break_to(&mut self, policy: FlowPageBreak) -> &mut Self {
        loop {
            let accepted = policy.accepts(self.builder.pages.len() + 1);
            self.add_section_page(self.current_section, !accepted);
            let page = self.current_page + 1;
            if policy.accepts(page) {
                break;
            }
        }
        self
    }

    fn add_section_page(&mut self, section_index: usize, suppress_master: bool) {
        // A caller may replace the backing builder (for example to install an
        // already prepared font registry). Such a builder need not carry the
        // FlowDocument's private section table. Pagination must remain safe in
        // that supported composition case instead of indexing an empty table.
        let (size, margins, owned_section) =
            if let Some(section) = self.builder.sections.get(section_index) {
                (
                    section.page_size,
                    section.effective_margins(self.builder.pages.len() + 1),
                    Some(section_index),
                )
            } else {
                (self.page_size, self.margins, None)
            };
        let page = self.builder.add_page_with_margins(size, margins);
        page.section_index = owned_section;
        page.suppress_section_master = suppress_master;
        self.current_page = self.builder.pages.len() - 1;
        self.page_size = size;
        self.margins = margins;
        self.cursor_y = size.height - margins.top;
    }

    fn ensure_space(&mut self, height: f64) -> Result<()> {
        if !height.is_finite()
            || height <= 0.0
            || !self.cursor_y.is_finite()
            || !self.page_size.height.is_finite()
            || !self.margins.top.is_finite()
            || !self.margins.bottom.is_finite()
        {
            return Err(WellfriendError::invalid_input(
                "invalid authoring flow dimensions",
            ));
        }
        let usable = self.page_size.height - self.margins.top - self.margins.bottom;
        if !usable.is_finite() || usable <= 0.0 {
            return Err(WellfriendError::invalid_input(
                "invalid authoring flow page extent",
            ));
        }
        if height > usable {
            return Err(WellfriendError::ResourceLimit(format!(
                "authoring: flow block height {} exceeds usable page height",
                fmt_num(height)
            )));
        }
        if self.cursor_y - height < self.current_bottom() {
            self.add_page_break();
        }
        Ok(())
    }

    /// Roll back only append-only drawing state, not a full document/font/image
    /// clone, when shaping, layout, cancellation or serialization planning fails.
    fn append_transaction(
        &mut self,
        operation: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<&mut Self> {
        if self.current_page >= self.builder.pages.len() {
            return Err(WellfriendError::invalid_input(
                "authoring flow current page is outside the backing builder",
            ));
        }
        let lengths = self
            .builder
            .pages
            .iter()
            .map(|page| {
                (
                    page.commands.len(),
                    page.footnotes.len(),
                    page.footnote_reserved_height,
                    page.links.len(),
                )
            })
            .collect::<Vec<_>>();
        let cursor = self.cursor_y;
        let current = self.current_page;
        let section = self.current_section;
        let section_count = self.builder.sections.len();
        let next_footnote_id = self.builder.next_footnote_id;
        let next_field_plan_id = self.builder.next_field_plan_id;
        let structure_count = self.builder.structures.len();
        let next_structure_id = self.builder.next_structure_id;
        let authored_typed_table_count = self.builder.authored_typed_tables.len();
        let document_footnote_next = self.document_footnote_next;
        let document_footnote_started = self.document_footnote_started;
        let section_footnote_next = self.section_footnote_next;
        let section_footnote_started = self.section_footnote_started;
        if let Err(error) = operation(self) {
            self.builder.pages.truncate(lengths.len());
            self.builder.sections.truncate(section_count);
            for (page, (commands, notes, reserved, links)) in
                self.builder.pages.iter_mut().zip(lengths)
            {
                page.commands.truncate(commands);
                page.footnotes.truncate(notes);
                page.footnote_reserved_height = reserved;
                page.links.truncate(links);
            }
            self.cursor_y = cursor;
            self.current_page = current;
            self.current_section = section;
            self.builder.next_footnote_id = next_footnote_id;
            self.builder.next_field_plan_id = next_field_plan_id;
            self.builder.structures.truncate(structure_count);
            self.builder.next_structure_id = next_structure_id;
            self.builder
                .authored_typed_tables
                .truncate(authored_typed_table_count);
            self.document_footnote_next = document_footnote_next;
            self.document_footnote_started = document_footnote_started;
            self.section_footnote_next = section_footnote_next;
            self.section_footnote_started = section_footnote_started;
            return Err(error);
        }
        Ok(self)
    }

    fn content_width(&self) -> Result<f64> {
        let width = self.page_size.width - self.margins.left - self.margins.right;
        if ![
            self.page_size.width,
            self.margins.left,
            self.margins.right,
            width,
        ]
        .iter()
        .all(|n| n.is_finite())
            || width <= 0.0
        {
            return Err(WellfriendError::invalid_input(
                "invalid authoring flow width",
            ));
        }
        Ok(width)
    }

    fn current_bottom(&self) -> f64 {
        self.margins.bottom + self.current_page_ref().footnote_reserved_height
    }

    fn current_page_ref(&self) -> &PdfPageBuilder {
        &self.builder.pages[self.current_page]
    }

    fn current_page_mut(&mut self) -> &mut PdfPageBuilder {
        &mut self.builder.pages[self.current_page]
    }
}

/// Stroke/fill state for vector drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphicsStyle {
    pub stroke: Option<Color>,
    pub fill: Option<Color>,
    pub line_width: f64,
    pub line_cap: LineCap,
    pub line_join: LineJoin,
    pub dash: LineDash,
}

impl Default for GraphicsStyle {
    fn default() -> Self {
        Self {
            stroke: Some(Color::black()),
            fill: None,
            line_width: 1.0,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            dash: LineDash::default(),
        }
    }
}

impl GraphicsStyle {
    pub fn stroke(color: Color, line_width: f64) -> Self {
        Self {
            stroke: Some(color),
            line_width,
            ..Default::default()
        }
    }

    pub fn fill(color: Color) -> Self {
        Self {
            stroke: None,
            fill: Some(color),
            ..Default::default()
        }
    }

    pub fn fill_stroke(fill: Color, stroke: Color, line_width: f64) -> Self {
        Self {
            stroke: Some(stroke),
            fill: Some(fill),
            line_width,
            ..Default::default()
        }
    }

    pub fn line_cap(mut self, cap: LineCap) -> Self {
        self.line_cap = cap;
        self
    }

    pub fn line_join(mut self, join: LineJoin) -> Self {
        self.line_join = join;
        self
    }

    pub fn dash(mut self, pattern: Vec<f64>, phase: f64) -> Self {
        self.dash = LineDash { pattern, phase };
        self
    }

    fn stroke_only_if_unpainted(mut self) -> Self {
        if self.stroke.is_none() && self.fill.is_none() {
            self.stroke = Some(Color::black());
        }
        self.fill = None;
        self
    }
}

/// Arbitrary path builder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PathBuilder {
    segments: Vec<PathSegment>,
}

impl PathBuilder {
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    pub fn move_to(mut self, x: f64, y: f64) -> Self {
        self.segments.push(PathSegment::MoveTo(x, y));
        self
    }

    pub fn line_to(mut self, x: f64, y: f64) -> Self {
        self.segments.push(PathSegment::LineTo(x, y));
        self
    }

    pub fn curve_to(mut self, x1: f64, y1: f64, x2: f64, y2: f64, x3: f64, y3: f64) -> Self {
        self.segments
            .push(PathSegment::CurveTo(x1, y1, x2, y2, x3, y3));
        self
    }

    pub fn close(mut self) -> Self {
        self.segments.push(PathSegment::Close);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PathSegment {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    CurveTo(f64, f64, f64, f64, f64, f64),
    Close,
}

#[derive(Debug, Clone)]
enum PageCommand {
    BeginArtifact,
    /// Static table-cell paint owned by a retained authored table. Text remains
    /// in its independent TH/TD structure scope; this artifact marks only the
    /// grid/fill rectangle so a later row transaction can bind and relocate it
    /// without treating visual similarity as ownership evidence.
    BeginOwnedTableCellArtifact {
        table: String,
        row: usize,
        column: usize,
        row_span: usize,
        column_span: usize,
    },
    EndArtifact,
    BeginStructure(u64),
    BeginTypedCellStructure {
        element: u64,
        region: [f64; 4],
    },
    EndStructure(u64),
    DeferredField(fields::DeferredFieldLine),
    Text {
        text: String,
        x: f64,
        y: f64,
        style: TextStyle,
        bidi: Option<crate::fonts::shaper::LineBidi>,
        logical_text: Option<String>,
        /// Paint glyphs without adding a private /ActualText carrier. Used by
        /// cluster-preserving semantic fragments after one sibling fragment
        /// has taken ownership of the complete logical source span.
        suppress_actual_text: bool,
        font_asset: Option<Arc<[u8]>>,
        shaped: Option<Arc<crate::fonts::ShapedRun>>,
    },
    TextGroup {
        logical_text: String,
        runs: Vec<PageCommand>,
    },
    LogicalBreak {
        text: String,
        x: f64,
        y: f64,
        size: f64,
    },
    Rect {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        style: GraphicsStyle,
    },
    Path {
        path: PathBuilder,
        style: GraphicsStyle,
    },
    Image {
        image: ImageHandle,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
}

// Groups are created internally from text/logical children plus an optional
// leading artifact path group for tab decoration, never recursively nested
// user commands. Reuse one traversal for resource and glyph planning.
fn text_commands(command: &PageCommand) -> std::slice::Iter<'_, PageCommand> {
    match command {
        PageCommand::TextGroup { runs, .. } => runs.iter(),
        _ => std::slice::from_ref(command).iter(),
    }
}

struct AuthoredObjects {
    objects: Vec<OutputObject>,
    catalog_number: u32,
    info_number: Option<u32>,
}

impl AuthoredObjects {
    fn build(
        builder: &PdfBuilder,
        font_plan: &FontBuildPlan,
        image_plan: &ImageBuildPlan,
    ) -> Result<Self> {
        let catalog_number = 1u32;
        let pages_number = 2u32;
        let page_count = builder.pages.len();
        let page_start = 3u32;
        let content_start = page_start + page_count as u32;
        let mut next = content_start + page_count as u32;

        let mut image_objects = Vec::new();
        let mut image_refs = HashMap::new();
        for handle in &image_plan.images {
            let image = builder.image(*handle)?;
            let smask_number = if image.smask.is_some() {
                Some(alloc(&mut next))
            } else {
                None
            };
            let image_number = alloc(&mut next);
            image_refs.insert(*handle, image_number);
            if let (Some(number), Some(mask)) = (smask_number, image.smask.as_ref()) {
                image_objects.push(OutputObject {
                    number,
                    object: PdfObject::Stream {
                        dict: smask_image_dict(mask),
                        raw: mask.data.clone(),
                    },
                });
            }
            image_objects.push(OutputObject {
                number: image_number,
                object: PdfObject::Stream {
                    dict: authored_image_dict(image, smask_number),
                    raw: image.data.clone(),
                },
            });
        }

        let mut font_objects = Vec::new();
        let mut font_refs = HashMap::new();
        for font in &font_plan.fonts {
            let built = build_font_objects(*font, builder, &mut next, font_plan)?;
            font_refs.insert(*font, built.top_object);
            font_objects.extend(built.objects);
        }

        let mut annotation_objects = Vec::new();
        let mut annotation_refs = Vec::with_capacity(builder.pages.len());
        let mut annotation_structure_marks = Vec::new();
        let mut next_struct_parent = u32::try_from(builder.pages.len())
            .map_err(|_| WellfriendError::ResourceLimit("authored ParentTree page count".into()))?;
        for (page_index, page) in builder.pages.iter().enumerate() {
            let mut refs = Vec::with_capacity(page.links.len());
            let page_number = page_start
                .checked_add(u32::try_from(page_index).map_err(|_| {
                    WellfriendError::ResourceLimit("authored annotation page index".into())
                })?)
                .ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored annotation page number".into())
                })?;
            for link in &page.links {
                let number = alloc(&mut next);
                let struct_parent = if let Some(element) = link.structure_id {
                    let key = next_struct_parent;
                    next_struct_parent = next_struct_parent.checked_add(1).ok_or_else(|| {
                        WellfriendError::ResourceLimit(
                            "authored annotation StructParent identity overflow".into(),
                        )
                    })?;
                    annotation_structure_marks.push(structure::AnnotationMark {
                        element,
                        page: page_index,
                        object: number,
                        struct_parent: key,
                    });
                    Some(key)
                } else {
                    None
                };
                refs.push(number);
                annotation_objects.push(OutputObject {
                    number,
                    object: PdfObject::Dictionary(fields::link_annotation_dict(
                        link,
                        page_number,
                        struct_parent,
                    )?),
                });
            }
            annotation_refs.push(refs);
        }

        let destination_tree = fields::destination_tree_objects(builder, page_start, &mut next)?;
        let outline_objects = outline::build(builder, &mut next)?;
        let structure_objects = structure::build(
            builder,
            page_start,
            &mut next,
            &annotation_structure_marks,
            next_struct_parent,
        )?;
        let authored_typed_tables = tables::build_typed_table_registry(builder, &mut next)?;

        let info_number = if builder.metadata.is_empty() {
            None
        } else {
            let number = next;
            next += 1;
            Some(number)
        };

        let mut objects = Vec::new();
        objects.push(OutputObject {
            number: catalog_number,
            object: PdfObject::Dictionary(catalog_dict(
                pages_number,
                sections::page_labels(builder)?,
                destination_tree.names,
                outline_objects.root,
                structure_objects.root,
                builder.language.as_deref(),
                authored_typed_tables.root,
            )),
        });
        objects.push(OutputObject {
            number: pages_number,
            object: PdfObject::Dictionary(pages_tree_dict(page_start, page_count)),
        });

        let resource_refs = PageResourceRefs {
            font_plan,
            font_refs: &font_refs,
            image_plan,
            image_refs: &image_refs,
        };

        for (idx, page) in builder.pages.iter().enumerate() {
            let page_number = page_start + idx as u32;
            let content_number = content_start + idx as u32;
            let content = build_content_stream_with_structure(
                page,
                font_plan,
                image_plan,
                &structure_objects,
            )?;
            objects.push(OutputObject {
                number: page_number,
                object: PdfObject::Dictionary(page_dict(
                    pages_number,
                    content_number,
                    page,
                    &resource_refs,
                    &annotation_refs[idx],
                    structure_objects.page_is_marked(idx).then_some(idx),
                )?),
            });
            objects.push(OutputObject {
                number: content_number,
                object: PdfObject::Stream {
                    dict: PdfDictionary::empty(),
                    raw: content,
                },
            });
        }

        objects.extend(image_objects);
        objects.extend(font_objects);
        objects.extend(annotation_objects);
        objects.extend(destination_tree.objects);
        objects.extend(outline_objects.objects);
        objects.extend(structure_objects.objects);
        objects.extend(authored_typed_tables.objects);
        if let Some(number) = info_number {
            objects.push(OutputObject {
                number,
                object: PdfObject::Dictionary(info_dict(&builder.metadata)),
            });
        } else {
            let _ = next;
        }

        Ok(Self {
            objects,
            catalog_number,
            info_number,
        })
    }
}

struct BuiltFontObjects {
    top_object: u32,
    objects: Vec<OutputObject>,
}

struct PageResourceRefs<'a> {
    font_plan: &'a FontBuildPlan,
    font_refs: &'a HashMap<FontFace, u32>,
    image_plan: &'a ImageBuildPlan,
    image_refs: &'a HashMap<ImageHandle, u32>,
}

#[derive(Debug)]
struct FontBuildPlan {
    fonts: Vec<FontFace>,
    resource_names: HashMap<FontFace, String>,
    embedded: HashMap<FontFace, EmbeddedFontPlan>,
}

#[derive(Debug, Clone)]
struct EmbeddedFontPlan {
    cids: HashMap<char, u16>,
    entries: Vec<CidEntry>,
    shaped_runs: HashMap<(String, Option<crate::fonts::shaper::LineBidi>), ShapedTextPlan>,
}

#[derive(Debug, Clone)]
struct FontProgramSelection {
    base_name: String,
    bytes: Vec<u8>,
    subset: Option<SfntSubsetMetrics>,
    fallback: Option<SfntSubsetFallback>,
}

#[derive(Debug, Clone)]
struct SfntSubsetFallback {
    code: &'static str,
    reason: String,
}

#[derive(Debug, Clone)]
struct CidEntry {
    cid: u16,
    glyph_id: u16,
    unicode: String,
    width: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct ShapedTextPlan {
    glyphs: Vec<ShapedTextGlyph>,
    actual_text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct ShapedTextGlyph {
    cid: u16,
    advance: f64,
    offset_x: f64,
    offset_y: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct GlyphCidKey {
    glyph_id: u16,
    unicode: String,
}

#[derive(Debug, Default)]
struct EmbeddedFontPlanBuilder {
    cids: HashMap<char, u16>,
    glyph_cids: HashMap<GlyphCidKey, u16>,
    entries: Vec<CidEntry>,
    shaped_runs: HashMap<(String, Option<crate::fonts::shaper::LineBidi>), ShapedTextPlan>,
}

impl FontBuildPlan {
    fn from_builder(builder: &PdfBuilder) -> Result<Self> {
        let mut fonts = Vec::new();
        let mut embedded_builders: HashMap<FontFace, EmbeddedFontPlanBuilder> = HashMap::new();
        let mut metrics_by_font = HashMap::new();

        for page in &builder.pages {
            for font in page.fonts_used() {
                if let FontFace::Custom(id) = font {
                    builder.custom_font(id)?;
                }
                push_unique_font(&mut fonts, font);
            }
            for command in page.commands.iter().flat_map(text_commands) {
                crate::cancel::check_current_cancel("authoring font plan")?;
                if let PageCommand::LogicalBreak { text, .. } = command {
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        metrics_by_font.entry(FontFace::BuiltinUnicode)
                    {
                        e.insert(TrueTypeMetrics::parse(builtin_unicode_font_bytes()?)?);
                    }
                    embedded_builders
                        .entry(FontFace::BuiltinUnicode)
                        .or_default()
                        .add_logical_breaks(text, &metrics_by_font[&FontFace::BuiltinUnicode])?;
                }
                if let PageCommand::Text {
                    text,
                    style,
                    bidi,
                    font_asset,
                    shaped: prepared,
                    ..
                } = command
                {
                    if let FontFace::Custom(id) = style.font {
                        let registered = &builder.custom_font(id)?.bytes;
                        let asset_matches = font_asset.as_ref().is_some_and(|asset| {
                            Arc::ptr_eq(asset, registered) || asset.as_ref() == registered.as_ref()
                        });
                        // Empty source carriers deliberately contain no shaped
                        // glyph plan and may be constructed before a page can
                        // borrow its registered custom-font asset. The font ID
                        // is still resolved against this builder immediately;
                        // only non-empty/custom-shaped commands require an
                        // attached byte-for-byte asset witness.
                        if !asset_matches && !text.is_empty() {
                            return Err(WellfriendError::invalid_input(
                                "authored custom font asset differs from this document's registered font"));
                        }
                    }
                    if !style.font.is_embedded_unicode() {
                        continue;
                    }
                    let font_bytes = font_bytes_for_face(builder, style.font)?;
                    if let std::collections::hash_map::Entry::Vacant(e) =
                        metrics_by_font.entry(style.font)
                    {
                        e.insert(TrueTypeMetrics::parse(font_bytes)?);
                    }
                    let metrics = &metrics_by_font[&style.font];
                    let embedded = embedded_builders.entry(style.font).or_default();
                    if embedded
                        .shaped_runs
                        .contains_key(&(text.clone(), bidi.clone()))
                    {
                        continue;
                    }
                    let shaped = if let Some(prepared) = prepared {
                        prepared.as_ref().clone()
                    } else if let Some(bidi) = bidi {
                        TextShaper::shape_resolved(font_bytes, text, bidi, &Default::default())?
                    } else {
                        TextShaper::shape(font_bytes, text, ShapeOptions::default())?
                    };
                    if crate::fonts::shaper::has_missing_glyphs(font_bytes, text, &shaped)? {
                        return Err(WellfriendError::UnsupportedFeature(
                            "authoring font lacks shaped glyph coverage".into(),
                        ));
                    }
                    if shaped.used_complex_shaping {
                        embedded.add_shaped_run(text, &shaped, metrics, bidi.as_ref())?;
                    } else {
                        for ch in text.chars() {
                            embedded.add_char(ch, metrics)?;
                        }
                    }
                }
            }
        }

        let mut resource_names = HashMap::new();
        for (idx, font) in fonts.iter().enumerate() {
            resource_names.insert(*font, format!("F{}", idx + 1));
        }

        let mut embedded = HashMap::new();
        for font in &fonts {
            if !font.is_embedded_unicode() {
                continue;
            }
            let builder = embedded_builders.remove(font).unwrap_or_default();
            embedded.insert(*font, builder.finish());
        }

        Ok(Self {
            fonts,
            resource_names,
            embedded,
        })
    }

    #[cfg(test)]
    fn from_pages(pages: &[PdfPageBuilder]) -> Result<Self> {
        let mut builder = PdfBuilder::new();
        builder.pages = pages.to_vec();
        Self::from_builder(&builder)
    }

    fn resource_name(&self, font: FontFace) -> Result<&str> {
        self.resource_names
            .get(&font)
            .map(String::as_str)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf("authoring: font was not registered".to_string())
            })
    }

    fn embedded_plan(&self, font: FontFace) -> Result<&EmbeddedFontPlan> {
        self.embedded.get(&font).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!(
                "authoring: embedded font {font:?} was not planned"
            ))
        })
    }

    fn cid_for(&self, font: FontFace, ch: char) -> Result<u16> {
        self.embedded_plan(font)?
            .cids
            .get(&ch)
            .copied()
            .ok_or_else(|| {
                WellfriendError::MalformedPdf(format!(
                    "authoring: Unicode character {ch:?} has no CID for {font:?}"
                ))
            })
    }

    #[cfg(test)]
    fn shaped_run(&self, font: FontFace, text: &str) -> Option<&ShapedTextPlan> {
        self.shaped_run_resolved(font, text, None)
    }
    fn shaped_run_resolved(
        &self,
        font: FontFace,
        text: &str,
        bidi: Option<&crate::fonts::shaper::LineBidi>,
    ) -> Option<&ShapedTextPlan> {
        self.embedded
            .get(&font)?
            .shaped_runs
            .get(&(text.to_owned(), bidi.cloned()))
    }
}

impl EmbeddedFontPlan {
    fn requested_glyphs(&self) -> BTreeSet<u16> {
        let mut glyphs = BTreeSet::new();
        glyphs.insert(0);
        for entry in &self.entries {
            glyphs.insert(entry.glyph_id);
        }
        glyphs
    }
}

impl EmbeddedFontPlanBuilder {
    fn add_logical_breaks(&mut self, text: &str, metrics: &TrueTypeMetrics<'_>) -> Result<()> {
        let face = &metrics.face;
        let space = face.glyph_index(' ').ok_or_else(|| {
            WellfriendError::invalid_input("authoring logical carrier lacks a space glyph")
        })?;
        if space.0 == 0 || face.tables().glyf.is_none() || face.glyph_bounding_box(space).is_some()
        {
            return Err(WellfriendError::invalid_input(
                "authoring logical carrier requires an empty TrueType outline",
            ));
        }
        for (index, ch) in text.chars().enumerate() {
            if index % 1024 == 0 {
                crate::cancel::check_current_cancel("authoring logical carrier mapping")?;
            }
            if !crate::fonts::logical_carrier::is_scalar(ch) {
                return Err(WellfriendError::invalid_input(
                    "invalid authoring logical carrier scalar",
                ));
            }
            if !self.cids.contains_key(&ch) {
                let cid = self.push_entry(space.0, ch.to_string(), 0.0)?;
                self.cids.insert(ch, cid);
            }
        }
        Ok(())
    }

    fn add_char(&mut self, ch: char, metrics: &TrueTypeMetrics<'_>) -> Result<u16> {
        if let Some(cid) = self.cids.get(&ch).copied() {
            return Ok(cid);
        }
        let glyph_id = metrics.glyph_id(ch);
        let width = metrics.glyph_width_by_gid(glyph_id);
        let cid = self.push_entry(glyph_id, ch.to_string(), width)?;
        self.cids.insert(ch, cid);
        Ok(cid)
    }

    fn add_shaped_run(
        &mut self,
        text: &str,
        shaped: &crate::fonts::ShapedRun,
        metrics: &TrueTypeMetrics<'_>,
        bidi: Option<&crate::fonts::shaper::LineBidi>,
    ) -> Result<()> {
        let clusters = cluster_boundaries(text, shaped);
        let mut glyphs = Vec::with_capacity(shaped.glyphs.len());
        let mut mapped_clusters = BTreeSet::new();
        for glyph in &shaped.glyphs {
            if ![glyph.advance, glyph.offset_x, glyph.offset_y]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err(WellfriendError::invalid_input(
                    "non-finite shaped glyph position",
                ));
            }
            let unicode = if mapped_clusters.insert(glyph.cluster) {
                cluster_text(text, glyph.cluster, &clusters)
            } else {
                String::new()
            };
            let key = GlyphCidKey {
                glyph_id: glyph.glyph_id,
                unicode: unicode.clone(),
            };
            let cid = if let Some(cid) = self.glyph_cids.get(&key).copied() {
                cid
            } else {
                // /W describes the reusable glyph. Contextual advances belong
                // to occurrences, not the first context that allocated this CID.
                let width = metrics.glyph_width_by_gid(glyph.glyph_id);
                let cid = self.push_entry(glyph.glyph_id, unicode.clone(), width)?;
                self.glyph_cids.insert(key, cid);
                cid
            };
            glyphs.push(ShapedTextGlyph {
                cid,
                advance: glyph.advance,
                offset_x: glyph.offset_x,
                offset_y: glyph.offset_y,
            });
        }
        self.shaped_runs.insert(
            (text.to_string(), bidi.cloned()),
            ShapedTextPlan {
                glyphs,
                actual_text: text.to_string(),
            },
        );
        Ok(())
    }

    fn push_entry(&mut self, glyph_id: u16, unicode: String, width: f64) -> Result<u16> {
        let cid = u16::try_from(self.entries.len() + 1).map_err(|_| {
            WellfriendError::ResourceLimit(
                "authoring: too many unique font CIDs for embedded Type0 font".to_string(),
            )
        })?;
        self.entries.push(CidEntry {
            cid,
            glyph_id,
            unicode,
            width,
        });
        Ok(cid)
    }

    fn finish(self) -> EmbeddedFontPlan {
        EmbeddedFontPlan {
            cids: self.cids,
            entries: self.entries,
            shaped_runs: self.shaped_runs,
        }
    }
}

fn font_bytes_for_face(builder: &PdfBuilder, font: FontFace) -> Result<&[u8]> {
    match font {
        FontFace::BuiltinUnicode => builtin_unicode_font_bytes(),
        FontFace::Custom(id) => Ok(&builder.custom_font(id)?.bytes),
        FontFace::Standard(_) => Err(WellfriendError::MalformedPdf(
            "authoring: Standard14 font has no embedded font plan".to_string(),
        )),
        FontFace::Fallback(_) => Err(WellfriendError::invalid_input(
            "unresolved authoring fallback stack",
        )),
    }
}

fn cluster_boundaries(text: &str, shaped: &crate::fonts::ShapedRun) -> Vec<usize> {
    let mut clusters: Vec<usize> = shaped
        .glyphs
        .iter()
        .filter_map(|glyph| usize::try_from(glyph.cluster).ok())
        .filter(|idx| *idx <= text.len() && text.is_char_boundary(*idx))
        .collect();
    clusters.push(0);
    clusters.push(text.len());
    clusters.sort_unstable();
    clusters.dedup();
    clusters
}

fn cluster_text(text: &str, cluster: u32, clusters: &[usize]) -> String {
    let Ok(start) = usize::try_from(cluster) else {
        return "\u{FFFD}".to_string();
    };
    if start > text.len() || !text.is_char_boundary(start) {
        return "\u{FFFD}".to_string();
    }
    let end = clusters
        .iter()
        .copied()
        .find(|candidate| *candidate > start)
        .unwrap_or(text.len());
    if end <= start || end > text.len() || !text.is_char_boundary(end) {
        return "\u{FFFD}".to_string();
    }
    text[start..end].to_string()
}

fn push_unique_font(fonts: &mut Vec<FontFace>, font: FontFace) {
    if !fonts.contains(&font) {
        fonts.push(font);
    }
}

#[derive(Debug)]
struct ImageBuildPlan {
    images: Vec<ImageHandle>,
    resource_names: HashMap<ImageHandle, String>,
}

impl ImageBuildPlan {
    fn from_builder(builder: &PdfBuilder) -> Result<Self> {
        let mut images = Vec::new();
        for page in &builder.pages {
            for image in page.images_used() {
                builder.image(image)?;
                push_unique_image(&mut images, image);
            }
        }
        let mut resource_names = HashMap::new();
        for (idx, image) in images.iter().enumerate() {
            resource_names.insert(*image, format!("Im{}", idx + 1));
        }
        Ok(Self {
            images,
            resource_names,
        })
    }

    fn resource_name(&self, image: ImageHandle) -> Result<&str> {
        self.resource_names
            .get(&image)
            .map(String::as_str)
            .ok_or_else(|| {
                WellfriendError::MalformedPdf("authoring: image was not registered".to_string())
            })
    }
}

fn push_unique_image(images: &mut Vec<ImageHandle>, image: ImageHandle) {
    if !images.contains(&image) {
        images.push(image);
    }
}

fn catalog_dict(
    pages_number: u32,
    page_labels: Option<PdfObject>,
    names: Option<PdfObject>,
    outlines: Option<u32>,
    structure_root: Option<u32>,
    language: Option<&str>,
    authored_typed_tables: Option<u32>,
) -> PdfDictionary {
    let mut catalog = dict(&[
        ("Type", PdfObject::Name("Catalog".to_string())),
        ("Pages", reference(pages_number)),
    ]);
    if let Some(page_labels) = page_labels {
        catalog.insert("PageLabels", page_labels);
    }
    if let Some(names) = names {
        catalog.insert("Names", names);
    }
    if let Some(outlines) = outlines {
        catalog.insert("Outlines", reference(outlines));
        catalog.insert("PageMode", PdfObject::Name("UseOutlines".into()));
    }
    if let Some(structure_root) = structure_root {
        catalog.insert("StructTreeRoot", reference(structure_root));
        catalog.insert(
            "MarkInfo",
            PdfObject::Dictionary(dict(&[("Marked", PdfObject::Boolean(true))])),
        );
    }
    if let Some(language) = language {
        catalog.insert("Lang", PdfObject::String(language.as_bytes().to_vec()));
    }
    if let Some(authored_typed_tables) = authored_typed_tables {
        catalog.insert(
            "WellfriendAuthoredTypedTables",
            reference(authored_typed_tables),
        );
    }
    catalog
}

fn pages_tree_dict(page_start: u32, page_count: usize) -> PdfDictionary {
    let kids = (0..page_count)
        .map(|idx| reference(page_start + idx as u32))
        .collect();
    dict(&[
        ("Type", PdfObject::Name("Pages".to_string())),
        ("Count", PdfObject::Integer(page_count as i64)),
        ("Kids", PdfObject::Array(kids)),
    ])
}

fn page_dict(
    parent: u32,
    contents: u32,
    page: &PdfPageBuilder,
    resource_refs: &PageResourceRefs<'_>,
    annotations: &[u32],
    struct_parents: Option<usize>,
) -> Result<PdfDictionary> {
    let mut fonts = PdfDictionary::empty();
    for font in page.fonts_used() {
        let resource = resource_refs.font_plan.resource_name(font)?;
        let Some(number) = resource_refs.font_refs.get(&font).copied() else {
            return Err(WellfriendError::MalformedPdf(
                "authoring: font object missing".to_string(),
            ));
        };
        fonts.insert(resource, reference(number));
    }

    let mut xobjects = PdfDictionary::empty();
    for image in page.images_used() {
        let resource = resource_refs.image_plan.resource_name(image)?;
        let Some(number) = resource_refs.image_refs.get(&image).copied() else {
            return Err(WellfriendError::MalformedPdf(
                "authoring: image object missing".to_string(),
            ));
        };
        xobjects.insert(resource, reference(number));
    }

    let mut resources = PdfDictionary::empty();
    if !fonts.is_empty() {
        resources.insert("Font", PdfObject::Dictionary(fonts));
    }
    if !xobjects.is_empty() {
        resources.insert("XObject", PdfObject::Dictionary(xobjects));
    }
    resources.insert(
        "ProcSet",
        PdfObject::Array(vec![
            PdfObject::Name("PDF".to_string()),
            PdfObject::Name("Text".to_string()),
            PdfObject::Name("ImageB".to_string()),
            PdfObject::Name("ImageC".to_string()),
            PdfObject::Name("ImageI".to_string()),
        ]),
    );

    let mut page_dictionary = dict(&[
        ("Type", PdfObject::Name("Page".to_string())),
        ("Parent", reference(parent)),
        (
            "MediaBox",
            PdfObject::Array(vec![
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                pdf_number(page.size.width),
                pdf_number(page.size.height),
            ]),
        ),
        ("Resources", PdfObject::Dictionary(resources)),
        ("Contents", reference(contents)),
    ]);
    if !annotations.is_empty() {
        page_dictionary.insert(
            "Annots",
            PdfObject::Array(annotations.iter().copied().map(reference).collect()),
        );
    }
    if let Some(struct_parents) = struct_parents {
        page_dictionary.insert(
            "StructParents",
            PdfObject::Integer(i64::try_from(struct_parents).map_err(|_| {
                WellfriendError::ResourceLimit("authored page StructParents key".into())
            })?),
        );
    }
    Ok(page_dictionary)
}

fn info_dict(metadata: &PdfMetadata) -> PdfDictionary {
    let mut info = PdfDictionary::empty();
    if let Some(value) = &metadata.title {
        info.insert("Title", PdfObject::String(pdf_text_string(value)));
    }
    if let Some(value) = &metadata.author {
        info.insert("Author", PdfObject::String(pdf_text_string(value)));
    }
    if let Some(value) = &metadata.subject {
        info.insert("Subject", PdfObject::String(pdf_text_string(value)));
    }
    if let Some(value) = &metadata.keywords {
        info.insert("Keywords", PdfObject::String(pdf_text_string(value)));
    }
    if let Some(value) = &metadata.creator {
        info.insert("Creator", PdfObject::String(pdf_text_string(value)));
    }
    info.insert(
        "Producer",
        PdfObject::String(pdf_text_string("Wellfriend PDF SDK")),
    );
    info
}

fn authored_image_dict(image: &AuthoredImage, smask_number: Option<u32>) -> PdfDictionary {
    let mut dict = dict(&[
        ("Type", PdfObject::Name("XObject".to_string())),
        ("Subtype", PdfObject::Name("Image".to_string())),
        ("Width", PdfObject::Integer(i64::from(image.width))),
        ("Height", PdfObject::Integer(i64::from(image.height))),
        (
            "ColorSpace",
            PdfObject::Name(image.color_space.pdf_name().to_string()),
        ),
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
        dict.insert("SMask", reference(number));
    }
    dict
}

fn smask_image_dict(mask: &AuthoredSoftMask) -> PdfDictionary {
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

fn authored_image_from_raw(raw: RawImage) -> Result<AuthoredImage> {
    let expected = raw.byte_count();
    if raw.pixels.len() != expected {
        return Err(WellfriendError::MalformedPdf(format!(
            "authoring: image data has {} bytes but expected {expected}",
            raw.pixels.len()
        )));
    }

    let (samples, color_space, smask) = match raw.channels {
        1 => (raw.pixels, ImageColorSpace::DeviceGray, None),
        2 => {
            let mut samples = Vec::with_capacity(raw.pixel_count());
            let mut alpha = Vec::with_capacity(raw.pixel_count());
            for px in raw.pixels.chunks_exact(2) {
                samples.push(px[0]);
                alpha.push(px[1]);
            }
            (
                samples,
                ImageColorSpace::DeviceGray,
                Some(AuthoredSoftMask {
                    width: raw.width,
                    height: raw.height,
                    data: flate_encode(&alpha, 9),
                }),
            )
        }
        3 => (raw.pixels, ImageColorSpace::DeviceRGB, None),
        4 => {
            let mut samples = Vec::with_capacity(raw.pixel_count() * 3);
            let mut alpha = Vec::with_capacity(raw.pixel_count());
            for px in raw.pixels.chunks_exact(4) {
                samples.extend_from_slice(&px[..3]);
                alpha.push(px[3]);
            }
            (
                samples,
                ImageColorSpace::DeviceRGB,
                Some(AuthoredSoftMask {
                    width: raw.width,
                    height: raw.height,
                    data: flate_encode(&alpha, 9),
                }),
            )
        }
        channels => {
            return Err(WellfriendError::UnsupportedFeature(format!(
                "authoring: unsupported raw image channel count {channels}"
            )))
        }
    };

    Ok(AuthoredImage {
        width: raw.width,
        height: raw.height,
        color_space,
        bits_per_component: 8,
        data: flate_encode(&samples, 9),
        filter: ImageFilter::FlateDecode,
        smask,
    })
}

fn decode_png_for_authoring(bytes: &[u8]) -> Result<RawImage> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|err| {
        WellfriendError::MalformedPdf(format!("authoring: cannot read PNG: {err}"))
    })?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|err| {
        WellfriendError::MalformedPdf(format!("authoring: cannot decode PNG: {err}"))
    })?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(WellfriendError::UnsupportedFeature(format!(
            "authoring: PNG bit depth {:?} is not supported after expansion",
            info.bit_depth
        )));
    }
    let channels = match info.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => {
            return Err(WellfriendError::UnsupportedFeature(
                "authoring: indexed PNG did not expand to samples".to_string(),
            ))
        }
    };
    Ok(RawImage {
        width: info.width,
        height: info.height,
        channels,
        bits_per_sample: 8,
        pixels: buf[..info.buffer_size()].to_vec(),
    })
}

fn build_font_objects(
    font: FontFace,
    builder: &PdfBuilder,
    next: &mut u32,
    plan: &FontBuildPlan,
) -> Result<BuiltFontObjects> {
    match font {
        FontFace::Standard(standard) => {
            let number = alloc(next);
            Ok(BuiltFontObjects {
                top_object: number,
                objects: vec![OutputObject {
                    number,
                    object: PdfObject::Dictionary(standard_font_dict(standard)?),
                }],
            })
        }
        FontFace::BuiltinUnicode => build_embedded_type0_font(
            next,
            font,
            BUILTIN_UNICODE_RESOURCE_NAME,
            builtin_unicode_font_bytes()?,
            plan,
        ),
        FontFace::Custom(id) => {
            let custom = builder.custom_font(id)?;
            debug_assert_eq!(custom.id, id);
            build_embedded_type0_font(next, font, &custom.base_name, &custom.bytes, plan)
        }
        FontFace::Fallback(_) => Err(WellfriendError::invalid_input(
            "unresolved fallback font in object writer",
        )),
    }
}

fn standard_font_dict(font: StandardFont) -> Result<PdfDictionary> {
    let widths = standard_widths(font)?;
    let mut font_dict = dict(&[
        ("Type", PdfObject::Name("Font".to_string())),
        ("Subtype", PdfObject::Name("Type1".to_string())),
        (
            "BaseFont",
            PdfObject::Name(font.base_font_name().to_string()),
        ),
        ("FirstChar", PdfObject::Integer(32)),
        ("LastChar", PdfObject::Integer(255)),
        (
            "Widths",
            PdfObject::Array(widths.into_iter().map(pdf_number).collect()),
        ),
    ]);
    if font.built_in_encoding().is_none() {
        font_dict.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
    }
    Ok(font_dict)
}

fn build_embedded_type0_font(
    next: &mut u32,
    font: FontFace,
    base_name: &str,
    font_bytes: &[u8],
    plan: &FontBuildPlan,
) -> Result<BuiltFontObjects> {
    let type0_number = alloc(next);
    let descendant_number = alloc(next);
    let descriptor_number = alloc(next);
    let font_file_number = alloc(next);
    let to_unicode_number = alloc(next);
    let cid_to_gid_number = alloc(next);

    let metrics = TrueTypeMetrics::parse(font_bytes)?;
    let embedding = crate::fonts::pdf_embedding::EmbeddingInfo::parse(font_bytes)?;
    let font_program = select_embedded_font_program(base_name, font, font_bytes, plan, &embedding)?;
    let embedded = plan.embedded_plan(font)?;
    let encoding_map = embedding
        .cff
        .as_ref()
        .map(|cff| {
            cff.encoding(
                embedded
                    .entries
                    .iter()
                    .map(|entry| (entry.cid, entry.glyph_id)),
                false,
            )
        })
        .transpose()?;
    let embedded_base_name = font_program.base_name.as_str();
    let cmap_name = format!("{embedded_base_name}ToUnicode");

    let mut objects = Vec::new();
    objects.push(OutputObject {
        number: type0_number,
        object: PdfObject::Dictionary(dict(&[
            ("Type", PdfObject::Name("Font".to_string())),
            ("Subtype", PdfObject::Name("Type0".to_string())),
            ("BaseFont", PdfObject::Name(embedded_base_name.to_string())),
            (
                "Encoding",
                if encoding_map.is_some() {
                    reference(cid_to_gid_number)
                } else {
                    PdfObject::Name("Identity-H".to_string())
                },
            ),
            (
                "DescendantFonts",
                PdfObject::Array(vec![reference(descendant_number)]),
            ),
            ("ToUnicode", reference(to_unicode_number)),
        ])),
    });

    objects.push(OutputObject {
        number: descendant_number,
        object: PdfObject::Dictionary(cid_font_dict(
            embedded_base_name,
            descriptor_number,
            cid_to_gid_number,
            font,
            plan,
            &metrics,
            &embedding,
        )?),
    });
    objects.push(OutputObject {
        number: descriptor_number,
        object: PdfObject::Dictionary(font_descriptor_dict(
            embedded_base_name,
            font_file_number,
            &metrics,
            embedding.cff.is_some(),
        )),
    });

    let mut font_file_dict = PdfDictionary::empty();
    if embedding.cff.is_some() {
        font_file_dict.insert("Subtype", PdfObject::Name("OpenType".into()));
    } else {
        font_file_dict.insert(
            "Length1",
            PdfObject::Integer(font_program.bytes.len() as i64),
        );
    }
    annotate_subset_font_stream(&mut font_file_dict, &font_program);
    objects.push(OutputObject {
        number: font_file_number,
        object: PdfObject::Stream {
            dict: font_file_dict,
            raw: font_program.bytes,
        },
    });

    objects.push(OutputObject {
        number: to_unicode_number,
        object: PdfObject::Stream {
            dict: PdfDictionary::empty(),
            raw: build_to_unicode_cmap(font, plan, &cmap_name)?,
        },
    });
    let (map_dict, map_bytes) = if let Some(map) = encoding_map {
        map
    } else {
        (PdfDictionary::empty(), build_cid_to_gid_map(font, plan)?)
    };
    objects.push(OutputObject {
        number: cid_to_gid_number,
        object: PdfObject::Stream {
            dict: map_dict,
            raw: map_bytes,
        },
    });

    Ok(BuiltFontObjects {
        top_object: type0_number,
        objects,
    })
}

fn select_embedded_font_program(
    base_name: &str,
    font: FontFace,
    font_bytes: &[u8],
    plan: &FontBuildPlan,
    embedding: &crate::fonts::pdf_embedding::EmbeddingInfo,
) -> Result<FontProgramSelection> {
    let embedded = plan.embedded_plan(font)?;
    if embedding.cff.is_some() || !embedding.may_subset {
        return Ok(FontProgramSelection {
            base_name: embedding
                .cff
                .as_ref()
                .map_or_else(|| base_name.into(), |cff| cff.postscript_name.clone()),
            bytes: font_bytes.to_vec(),
            subset: None,
            fallback: Some(SfntSubsetFallback {
                code: if embedding.cff.is_some() {
                    "font.subset.full.opentype_cff"
                } else {
                    "font.subset.preserved.no_subsetting"
                },
                reason: if embedding.cff.is_some() {
                    "CFF1 embedded whole with native CID encoding"
                } else {
                    "font prohibits subsetting; exact full program retained"
                }
                .into(),
            }),
        });
    }
    let requested_glyphs = embedded.requested_glyphs();
    match subset_glyf_preserving_gids(font_bytes, &requested_glyphs) {
        Ok(subset) => {
            let base_name =
                deterministic_subset_font_name(base_name, font_bytes, embedded, &subset.metrics);
            Ok(FontProgramSelection {
                base_name,
                bytes: subset.bytes,
                subset: Some(subset.metrics),
                fallback: None,
            })
        }
        Err(err) => Ok(FontProgramSelection {
            base_name: base_name.to_string(),
            bytes: font_bytes.to_vec(),
            subset: None,
            fallback: Some(subset_fallback(err)),
        }),
    }
}

fn subset_fallback(err: SfntSubsetError) -> SfntSubsetFallback {
    SfntSubsetFallback {
        code: err.code(),
        reason: err.to_string(),
    }
}

fn deterministic_subset_font_name(
    base_name: &str,
    font_bytes: &[u8],
    embedded: &EmbeddedFontPlan,
    metrics: &SfntSubsetMetrics,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(base_name.as_bytes());
    hasher.update(font_bytes);
    hasher.update(metrics.strategy.as_bytes());
    for entry in &embedded.entries {
        hasher.update(entry.cid.to_be_bytes());
        hasher.update(entry.glyph_id.to_be_bytes());
        hasher.update(entry.unicode.as_bytes());
    }
    let digest = hasher.finalize();
    let tag: String = digest[..6]
        .iter()
        .map(|byte| char::from(b'A' + (byte % 26)))
        .collect();
    let clean_base = sanitize_pdf_name(base_name, "WellfriendSubsetFont");
    format!("{tag}+{clean_base}")
}

fn annotate_subset_font_stream(dict: &mut PdfDictionary, selection: &FontProgramSelection) {
    let mut info = PdfDictionary::empty();
    if let Some(metrics) = &selection.subset {
        info.insert("Enabled", PdfObject::Boolean(true));
        info.insert(
            "Strategy",
            PdfObject::String(metrics.strategy.as_bytes().to_vec()),
        );
        info.insert(
            "OriginalBytes",
            PdfObject::Integer(metrics.original_bytes as i64),
        );
        info.insert(
            "OutputBytes",
            PdfObject::Integer(metrics.subset_bytes as i64),
        );
        info.insert(
            "GlyphsRequested",
            PdfObject::Integer(metrics.glyphs_requested as i64),
        );
        info.insert(
            "GlyphsEmbedded",
            PdfObject::Integer(metrics.glyphs_embedded as i64),
        );
    } else {
        info.insert("Enabled", PdfObject::Boolean(false));
        if let Some(fallback) = &selection.fallback {
            info.insert(
                "FallbackCode",
                PdfObject::String(fallback.code.as_bytes().to_vec()),
            );
            info.insert(
                "FallbackReason",
                PdfObject::String(fallback.reason.as_bytes().to_vec()),
            );
        }
    }
    dict.insert("WellfriendSubset", PdfObject::Dictionary(info));
}

fn cid_font_dict(
    base_name: &str,
    descriptor_number: u32,
    cid_to_gid_number: u32,
    font: FontFace,
    plan: &FontBuildPlan,
    metrics: &TrueTypeMetrics,
    embedding: &crate::fonts::pdf_embedding::EmbeddingInfo,
) -> Result<PdfDictionary> {
    let widths = if embedding.cff.is_some() {
        let mut widths = BTreeMap::new();
        for entry in &plan.embedded_plan(font)?.entries {
            widths.insert(embedding.cid(entry.cid, entry.glyph_id)?, entry.width);
        }
        widths
            .into_iter()
            .flat_map(|(cid, width)| {
                [
                    PdfObject::Integer(i64::from(cid)),
                    PdfObject::Array(vec![pdf_number(width)]),
                ]
            })
            .collect()
    } else {
        unicode_width_array(font, plan, metrics)?
    };
    let mut result = dict(&[
        ("Type", PdfObject::Name("Font".to_string())),
        (
            "Subtype",
            PdfObject::Name(
                if embedding.cff.is_some() {
                    "CIDFontType0"
                } else {
                    "CIDFontType2"
                }
                .to_string(),
            ),
        ),
        ("BaseFont", PdfObject::Name(base_name.to_string())),
        (
            "CIDSystemInfo",
            PdfObject::Dictionary(embedding.system().dictionary()),
        ),
        ("FontDescriptor", reference(descriptor_number)),
        ("DW", PdfObject::Integer(500)),
        ("W", PdfObject::Array(widths)),
    ]);
    if embedding.cff.is_none() {
        result.insert("CIDToGIDMap", reference(cid_to_gid_number));
    }
    Ok(result)
}

fn font_descriptor_dict(
    base_name: &str,
    font_file_number: u32,
    metrics: &TrueTypeMetrics,
    cff: bool,
) -> PdfDictionary {
    dict(&[
        ("Type", PdfObject::Name("FontDescriptor".to_string())),
        ("FontName", PdfObject::Name(base_name.to_string())),
        ("Flags", PdfObject::Integer(32)),
        (
            "FontBBox",
            PdfObject::Array(metrics.bbox.iter().copied().map(pdf_number).collect()),
        ),
        ("ItalicAngle", PdfObject::Integer(0)),
        ("Ascent", pdf_number(metrics.ascender)),
        ("Descent", pdf_number(metrics.descender)),
        ("CapHeight", pdf_number(metrics.cap_height)),
        ("StemV", PdfObject::Integer(80)),
        (
            if cff { "FontFile3" } else { "FontFile2" },
            reference(font_file_number),
        ),
    ])
}

fn unicode_width_array(
    font: FontFace,
    plan: &FontBuildPlan,
    _metrics: &TrueTypeMetrics,
) -> Result<Vec<PdfObject>> {
    let embedded = plan.embedded_plan(font)?;
    if embedded.entries.is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::with_capacity(embedded.entries.len() + 1);
    items.push(PdfObject::Integer(1));
    let widths = embedded
        .entries
        .iter()
        .map(|entry| {
            debug_assert!(entry.cid > 0);
            // Zero is an intentional width for logical carriers and marks,
            // not a missing-metric sentinel. Every entry has a planned width.
            pdf_number(entry.width)
        })
        .collect();
    items.push(PdfObject::Array(widths));
    Ok(items)
}

fn build_to_unicode_cmap(font: FontFace, plan: &FontBuildPlan, cmap_name: &str) -> Result<Vec<u8>> {
    let embedded = plan.embedded_plan(font)?;
    let mut out = String::new();
    out.push_str("/CIDInit /ProcSet findresource begin\n");
    out.push_str("12 dict begin\nbegincmap\n");
    out.push_str("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n");
    out.push_str(&format!("/CMapName /{cmap_name} def\n/CMapType 2 def\n"));
    out.push_str("1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n");

    for chunk in embedded.entries.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for entry in chunk {
            out.push_str(&format!(
                "<{:04X}> <{}>\n",
                entry.cid,
                utf16be_hex_for_str(&entry.unicode)
            ));
        }
        out.push_str("endbfchar\n");
    }

    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    Ok(out.into_bytes())
}

fn build_cid_to_gid_map(font: FontFace, plan: &FontBuildPlan) -> Result<Vec<u8>> {
    let embedded = plan.embedded_plan(font)?;
    let max_cid = embedded
        .entries
        .iter()
        .map(|entry| entry.cid)
        .max()
        .unwrap_or(0);
    let mut bytes = vec![0u8; (usize::from(max_cid) + 1) * 2];
    for entry in &embedded.entries {
        let offset = usize::from(entry.cid) * 2;
        bytes[offset..offset + 2].copy_from_slice(&entry.glyph_id.to_be_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
fn build_content_stream(
    page: &PdfPageBuilder,
    plan: &FontBuildPlan,
    image_plan: &ImageBuildPlan,
) -> Result<Vec<u8>> {
    build_content_stream_with_structure(page, plan, image_plan, &structure::Built::untagged(1))
}

fn build_content_stream_with_structure(
    page: &PdfPageBuilder,
    plan: &FontBuildPlan,
    image_plan: &ImageBuildPlan,
    structures: &structure::Built,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut artifact_depth = 0usize;
    let mut active_structure = None;
    let mut next_mcid = 0u32;
    for command in &page.commands {
        match command {
            PageCommand::BeginArtifact | PageCommand::BeginOwnedTableCellArtifact { .. } => {
                if artifact_depth != 0 || active_structure.is_some() {
                    return Err(WellfriendError::invalid_input(
                        "nested or intersecting authored artifacts",
                    ));
                }
                artifact_depth = 1;
                if let PageCommand::BeginOwnedTableCellArtifact {
                    table,
                    row,
                    column,
                    row_span,
                    column_span,
                } = command
                {
                    if table.is_empty()
                        || table.len() > 16 * 1024
                        || table.chars().any(|ch| ch == '\0')
                        || *row > 1_000_000
                        || *column > 10_000
                        || *row_span == 0
                        || *column_span == 0
                        || *row_span > 1_000_000
                        || *column_span > 10_000
                    {
                        return Err(WellfriendError::invalid_input(
                            "invalid owned authored table-cell artifact",
                        ));
                    }
                    out.extend_from_slice(
                        format!(
                            "/Artifact << /WFRowGrid true /WFGridTableID <{}> /WFRow {} /WFColumn {} /WFRowSpan {} /WFColSpan {} >> BDC\n",
                            hex_string(&pdf_text_string(table)),
                            row,
                            column,
                            row_span,
                            column_span,
                        )
                        .as_bytes(),
                    );
                } else {
                    out.extend_from_slice(b"/Artifact BMC\n");
                }
            }
            PageCommand::EndArtifact => {
                if artifact_depth != 1 {
                    return Err(WellfriendError::invalid_input(
                        "unbalanced authored artifact",
                    ));
                }
                artifact_depth = 0;
                out.extend_from_slice(b"EMC\n");
            }
            PageCommand::BeginStructure(element)
            | PageCommand::BeginTypedCellStructure { element, .. } => {
                if artifact_depth != 0 || active_structure.is_some() {
                    return Err(WellfriendError::invalid_input(
                        "nested or intersecting authored structure scopes",
                    ));
                }
                let role = structures.role_name(*element)?;
                if let Some((table, cell)) = structures.typed_cell_owner(*element)? {
                    let region = match command {
                        PageCommand::BeginTypedCellStructure { region, .. } => {
                            if !region.iter().all(|value| value.is_finite())
                                || region[0] >= region[2]
                                || region[1] >= region[3]
                                || region[0] < -1e-7
                                || region[1] < -1e-7
                                || region[2] > page.size.width + 1e-7
                                || region[3] > page.size.height + 1e-7
                            {
                                return Err(WellfriendError::invalid_input(
                                    "invalid authored typed-cell content region",
                                ));
                            }
                            format!(
                                " /WFLeft {} /WFBottom {} /WFRight {} /WFTop {}",
                                fmt_num(region[0]),
                                fmt_num(region[1]),
                                fmt_num(region[2]),
                                fmt_num(region[3])
                            )
                        }
                        _ => {
                            return Err(WellfriendError::invalid_input(
                                "authored typed-cell owner requires a bounded content region",
                            ));
                        }
                    };
                    out.extend_from_slice(
                        format!(
                            "/{role} << /MCID {next_mcid} /WFTableID <{}> /WFCellID <{}>{region} >> BDC\n",
                            hex_string(&pdf_text_string(table)),
                            hex_string(&pdf_text_string(cell)),
                        )
                        .as_bytes(),
                    );
                } else {
                    if matches!(command, PageCommand::BeginTypedCellStructure { .. }) {
                        return Err(WellfriendError::invalid_input(
                            "bounded typed-cell structure command has no typed owner",
                        ));
                    }
                    out.extend_from_slice(
                        format!("/{role} << /MCID {next_mcid} >> BDC\n").as_bytes(),
                    );
                }
                next_mcid = next_mcid.checked_add(1).ok_or_else(|| {
                    WellfriendError::ResourceLimit("authored content MCID overflow".into())
                })?;
                active_structure = Some(*element);
            }
            PageCommand::EndStructure(element) => {
                if active_structure != Some(*element) {
                    return Err(WellfriendError::invalid_input(
                        "unbalanced authored structure scope during serialization",
                    ));
                }
                out.extend_from_slice(b"EMC\n");
                active_structure = None;
            }
            PageCommand::DeferredField(_) => {
                return Err(WellfriendError::invalid_input(
                    "unmaterialized authored body field reached serialization",
                ));
            }
            PageCommand::TextGroup { logical_text, runs } => {
                let mut body_start = 0usize;
                if matches!(runs.first(), Some(PageCommand::BeginArtifact)) {
                    out.extend_from_slice(b"/Artifact BMC\n");
                    let mut closed = false;
                    for (index, run) in runs.iter().enumerate().skip(1) {
                        match run {
                            PageCommand::Path { path, style } => {
                                write_graphics_state(&mut out, style);
                                write_path(&mut out, path);
                                out.extend_from_slice(
                                    format!("{}\nQ\n", paint_operator(style)).as_bytes(),
                                );
                            }
                            PageCommand::EndArtifact => {
                                out.extend_from_slice(b"EMC\n");
                                body_start = index + 1;
                                closed = true;
                                break;
                            }
                            _ => {
                                return Err(WellfriendError::invalid_input(
                                    "invalid leading tab decoration artifact",
                                ));
                            }
                        }
                    }
                    if !closed {
                        return Err(WellfriendError::invalid_input(
                            "unclosed tab decoration artifact",
                        ));
                    }
                }
                write_shaped_actual_text(&mut out, logical_text);
                for run in runs.iter().skip(body_start) {
                    match run {
                        PageCommand::Text {
                            text,
                            x,
                            y,
                            style,
                            bidi,
                            ..
                        } => write_text_command_owned(
                            &mut out,
                            text,
                            *x,
                            *y,
                            style,
                            plan,
                            bidi.as_ref(),
                            None,
                            false,
                        )?,
                        PageCommand::LogicalBreak { text, x, y, size } => {
                            let resource = plan.resource_name(FontFace::BuiltinUnicode)?;
                            let encoded =
                                encode_text_for_font(text, FontFace::BuiltinUnicode, plan)?;
                            out.extend_from_slice(
                                format!(
                                    "q\nBT /{} {} Tf 0 Tc 0 Tw 100 Tz 0 Ts 0 Tr 1 0 0 1 {} {} Tm <{}> Tj ET\nQ\n",
                                    resource,
                                    fmt_num(*size),
                                    fmt_num(*x),
                                    fmt_num(*y),
                                    hex_string(&encoded)
                                )
                                .as_bytes(),
                            );
                        }
                        _ => {
                            return Err(WellfriendError::invalid_input(
                                "invalid nested authoring text-group command",
                            ));
                        }
                    }
                }
                out.extend_from_slice(b"EMC\n");
            }
            PageCommand::Text {
                text,
                x,
                y,
                style,
                bidi,
                logical_text,
                suppress_actual_text,
                ..
            } => {
                write_text_command_owned(
                    &mut out,
                    text,
                    *x,
                    *y,
                    style,
                    plan,
                    bidi.as_ref(),
                    logical_text.as_deref(),
                    !*suppress_actual_text,
                )?;
            }
            PageCommand::LogicalBreak { text, x, y, size } => {
                let resource = plan.resource_name(FontFace::BuiltinUnicode)?;
                let encoded = encode_text_for_font(text, FontFace::BuiltinUnicode, plan)?;
                out.extend_from_slice(b"q\n");
                write_shaped_actual_text(&mut out, text);
                out.extend_from_slice(
                    format!(
                    "BT /{} {} Tf 0 Tc 0 Tw 100 Tz 0 Ts 0 Tr 1 0 0 1 {} {} Tm <{}> Tj ET\nEMC\nQ\n",
                    resource, fmt_num(*size), fmt_num(*x), fmt_num(*y), hex_string(&encoded))
                    .as_bytes(),
                );
            }
            PageCommand::Rect {
                x,
                y,
                width,
                height,
                style,
            } => {
                write_graphics_state(&mut out, style);
                out.extend_from_slice(
                    format!(
                        "{} {} {} {} re\n{}\nQ\n",
                        fmt_num(*x),
                        fmt_num(*y),
                        fmt_num(*width),
                        fmt_num(*height),
                        paint_operator(style)
                    )
                    .as_bytes(),
                );
            }
            PageCommand::Path { path, style } => {
                write_graphics_state(&mut out, style);
                write_path(&mut out, path);
                out.extend_from_slice(format!("{}\nQ\n", paint_operator(style)).as_bytes());
            }
            PageCommand::Image {
                image,
                x,
                y,
                width,
                height,
            } => {
                write_image_command(&mut out, *image, *x, *y, *width, *height, image_plan)?;
            }
        }
    }
    if artifact_depth != 0 {
        return Err(WellfriendError::invalid_input("unclosed authored artifact"));
    }
    if active_structure.is_some() {
        return Err(WellfriendError::invalid_input(
            "unclosed authored structure during serialization",
        ));
    }
    Ok(out)
}

#[cfg(test)]
fn write_text_command(
    out: &mut Vec<u8>,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    plan: &FontBuildPlan,
) -> Result<()> {
    write_text_command_resolved(out, text, x, y, style, plan, None, None)
}
#[cfg(test)]
fn write_text_command_resolved(
    out: &mut Vec<u8>,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    plan: &FontBuildPlan,
    bidi: Option<&crate::fonts::shaper::LineBidi>,
    logical_text: Option<&str>,
) -> Result<()> {
    write_text_command_owned(out, text, x, y, style, plan, bidi, logical_text, true)
}

#[allow(clippy::too_many_arguments)]
fn write_text_command_owned(
    out: &mut Vec<u8>,
    text: &str,
    x: f64,
    y: f64,
    style: &TextStyle,
    plan: &FontBuildPlan,
    bidi: Option<&crate::fonts::shaper::LineBidi>,
    logical_text: Option<&str>,
    owns_logical_text: bool,
) -> Result<()> {
    layout::validate_single_line(text, x, y, style)?;
    let resource = plan.resource_name(style.font)?;
    out.extend_from_slice(b"q\n");
    write_fill_color(out, &style.fill);
    if let Some(shaped) = plan.shaped_run_resolved(style.font, text, bidi) {
        if owns_logical_text {
            write_shaped_actual_text(out, logical_text.unwrap_or(&shaped.actual_text));
        }
        out.extend_from_slice(
            format!(
                "BT /{} {} Tf\n0 Tc 0 Tw 100 Tz 0 Ts\n",
                resource,
                fmt_num(style.size)
            )
            .as_bytes(),
        );
        let mut pen = x;
        for glyph in &shaped.glyphs {
            let gx = pen + glyph.offset_x * style.size / 1000.0;
            let gy = y + glyph.offset_y * style.size / 1000.0;
            let next_pen = pen + glyph.advance * style.size / 1000.0;
            if ![gx, gy, next_pen].iter().all(|n| n.is_finite()) {
                return Err(WellfriendError::invalid_input(
                    "authored glyph position overflow",
                ));
            }
            out.extend_from_slice(
                format!(
                    "1 0 0 1 {} {} Tm <{:04X}> Tj\n",
                    fmt_num(gx),
                    fmt_num(gy),
                    glyph.cid
                )
                .as_bytes(),
            );
            pen = next_pen;
        }
        out.extend_from_slice(b"ET\n");
        if owns_logical_text {
            out.extend_from_slice(b"EMC\n");
        }
        out.extend_from_slice(b"Q\n");
        return Ok(());
    }
    let encoded = encode_text_for_font(text, style.font, plan)?;
    if owns_logical_text {
        if let Some(logical) = logical_text {
            write_shaped_actual_text(out, logical);
        }
    }
    out.extend_from_slice(
        format!(
            "BT /{} {} Tf {} {} Td <{}> Tj ET\n",
            resource,
            fmt_num(style.size),
            fmt_num(x),
            fmt_num(y),
            hex_string(&encoded)
        )
        .as_bytes(),
    );
    if owns_logical_text && logical_text.is_some() {
        out.extend_from_slice(b"EMC\n");
    }
    out.extend_from_slice(b"Q\n");
    Ok(())
}

fn write_shaped_actual_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(
        format!(
            "/Span << /ActualText <{}> >> BDC\n",
            utf16be_hex_with_bom(text)
        )
        .as_bytes(),
    );
}

fn write_image_command(
    out: &mut Vec<u8>,
    image: ImageHandle,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    image_plan: &ImageBuildPlan,
) -> Result<()> {
    let resource = image_plan.resource_name(image)?;
    out.extend_from_slice(
        format!(
            "q\n{} 0 0 {} {} {} cm\n/{} Do\nQ\n",
            fmt_num(width),
            fmt_num(height),
            fmt_num(x),
            fmt_num(y),
            resource
        )
        .as_bytes(),
    );
    Ok(())
}

fn write_graphics_state(out: &mut Vec<u8>, style: &GraphicsStyle) {
    out.extend_from_slice(b"q\n");
    out.extend_from_slice(format!("{} w\n", fmt_num(style.line_width.max(0.0))).as_bytes());
    out.extend_from_slice(format!("{} J\n", style.line_cap.clone() as i32).as_bytes());
    out.extend_from_slice(format!("{} j\n", style.line_join.clone() as i32).as_bytes());
    if style.dash.pattern.is_empty() {
        out.extend_from_slice(b"[] 0 d\n");
    } else {
        out.push(b'[');
        for (idx, value) in style.dash.pattern.iter().enumerate() {
            if idx > 0 {
                out.push(b' ');
            }
            out.extend_from_slice(fmt_num(*value).as_bytes());
        }
        out.extend_from_slice(format!("] {} d\n", fmt_num(style.dash.phase)).as_bytes());
    }
    if let Some(color) = &style.stroke {
        write_stroke_color(out, color);
    }
    if let Some(color) = &style.fill {
        write_fill_color(out, color);
    }
}

fn write_path(out: &mut Vec<u8>, path: &PathBuilder) {
    for segment in &path.segments {
        match *segment {
            PathSegment::MoveTo(x, y) => {
                out.extend_from_slice(format!("{} {} m\n", fmt_num(x), fmt_num(y)).as_bytes());
            }
            PathSegment::LineTo(x, y) => {
                out.extend_from_slice(format!("{} {} l\n", fmt_num(x), fmt_num(y)).as_bytes());
            }
            PathSegment::CurveTo(x1, y1, x2, y2, x3, y3) => {
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
            PathSegment::Close => out.extend_from_slice(b"h\n"),
        }
    }
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

fn paint_operator(style: &GraphicsStyle) -> &'static str {
    match (style.fill.is_some(), style.stroke.is_some()) {
        (true, true) => "B",
        (true, false) => "f",
        (false, true) => "S",
        (false, false) => "n",
    }
}

fn validate_text_for_font(text: &str, font: &FontFace) -> Result<()> {
    if let FontFace::Standard(standard) = font {
        for ch in text.chars() {
            encode_standard_char(*standard, ch).ok_or_else(|| {
                WellfriendError::UnsupportedFeature(format!(
                    "authoring: character {ch:?} is not encodable in {}; use FontFace::BuiltinUnicode",
                    standard.base_font_name()
                ))
            })?;
        }
    }
    Ok(())
}

fn encode_text_for_font(text: &str, font: FontFace, plan: &FontBuildPlan) -> Result<Vec<u8>> {
    match font {
        FontFace::Fallback(_) => Err(WellfriendError::invalid_input(
            "unresolved fallback font in text encoder",
        )),
        FontFace::Standard(standard) => text
            .chars()
            .map(|ch| {
                encode_standard_char(standard, ch).ok_or_else(|| {
                    WellfriendError::UnsupportedFeature(format!(
                        "authoring: character {ch:?} is not encodable in {}",
                        standard.base_font_name()
                    ))
                })
            })
            .collect(),
        FontFace::BuiltinUnicode | FontFace::Custom(_) => {
            let mut bytes = Vec::with_capacity(text.len() * 2);
            for ch in text.chars() {
                let cid = plan.cid_for(font, ch)?;
                bytes.extend_from_slice(&cid.to_be_bytes());
            }
            Ok(bytes)
        }
    }
}

fn standard_widths(font: StandardFont) -> Result<Vec<f64>> {
    (32u8..=255)
        .map(|byte| {
            let ch = decode_standard_byte(font, byte);
            fallback_glyph_width(font.fallback_font_name(), ch)
        })
        .collect()
}

fn encode_standard_char(font: StandardFont, ch: char) -> Option<u8> {
    match font.built_in_encoding() {
        None => encode_win_ansi_char(ch),
        Some(encoding) => {
            (0u8..=255).find(|byte| decode_symbolic_byte(encoding, *byte) == Some(ch))
        }
    }
}

fn decode_standard_byte(font: StandardFont, byte: u8) -> char {
    match font.built_in_encoding() {
        None => decode_win_ansi(byte),
        Some(encoding) => decode_symbolic_byte(encoding, byte).unwrap_or(' '),
    }
}

fn decode_symbolic_byte(encoding: &str, byte: u8) -> Option<char> {
    let glyph = Encoding::lookup(encoding, byte);
    if glyph == ".notdef" {
        return None;
    }
    glyph_name_to_unicode(glyph).or_else(|| zapf_dingbats_name_to_unicode(glyph))
}

fn fallback_glyph_width(font_name: &str, ch: char) -> Result<f64> {
    let bytes = get_fallback_font(font_name).ok_or_else(|| {
        WellfriendError::MalformedPdf(format!("authoring: missing fallback font {font_name}"))
    })?;
    let metrics = TrueTypeMetrics::parse(bytes)?;
    Ok(metrics.glyph_width(ch))
}

fn builtin_unicode_font_bytes() -> Result<&'static [u8]> {
    // The built-in Unicode face must actually cover the complex scripts that
    // its public name promises. DejaVu Sans is already bundled, licensed and
    // used by the renderer for broad Unicode fallback; Liberation Sans is a
    // Latin-centric Standard-14 substitute and rejects Arabic/Hebrew shaping.
    get_fallback_font("Symbol").ok_or_else(|| {
        WellfriendError::MalformedPdf("authoring: bundled DejaVu Sans font missing".to_string())
    })
}

struct TrueTypeMetrics<'a> {
    face: ttf_parser::Face<'a>,
    scale: f64,
    ascender: f64,
    descender: f64,
    cap_height: f64,
    bbox: [f64; 4],
}

impl<'a> TrueTypeMetrics<'a> {
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let face = ttf_parser::Face::parse(bytes, 0).map_err(|err| {
            WellfriendError::MalformedPdf(format!("authoring: cannot parse bundled font: {err:?}"))
        })?;
        let units = f64::from(face.units_per_em());
        let scale = 1000.0 / units;
        let bbox = face.global_bounding_box();
        let ascender = f64::from(face.ascender()) * scale;
        let descender = f64::from(face.descender()) * scale;
        let cap_height = face
            .capital_height()
            .map(|value| f64::from(value) * scale)
            .unwrap_or(ascender);
        Ok(Self {
            face,
            scale,
            ascender,
            descender,
            cap_height,
            bbox: [
                f64::from(bbox.x_min) * scale,
                f64::from(bbox.y_min) * scale,
                f64::from(bbox.x_max) * scale,
                f64::from(bbox.y_max) * scale,
            ],
        })
    }

    fn glyph_id(&self, ch: char) -> u16 {
        self.face.glyph_index(ch).map(|gid| gid.0).unwrap_or(0)
    }

    fn glyph_width(&self, ch: char) -> f64 {
        self.glyph_width_by_gid(self.glyph_id(ch))
    }

    fn glyph_width_by_gid(&self, gid: u16) -> f64 {
        self.face
            .glyph_hor_advance(ttf_parser::GlyphId(gid))
            .map(|advance| f64::from(advance) * self.scale)
            .unwrap_or(500.0)
    }
}

fn encode_win_ansi_char(ch: char) -> Option<u8> {
    if ('\u{20}'..='\u{7e}').contains(&ch) {
        return Some(ch as u8);
    }
    WIN_ANSI_EXTRA
        .iter()
        .find_map(|(byte, mapped)| (*mapped == ch).then_some(*byte))
}

fn decode_win_ansi(byte: u8) -> char {
    WIN_ANSI_EXTRA
        .iter()
        .find_map(|(candidate, ch)| (*candidate == byte).then_some(*ch))
        .unwrap_or(byte as char)
}

const WIN_ANSI_EXTRA: &[(u8, char)] = &[
    (0x80, '\u{20AC}'),
    (0x82, '\u{201A}'),
    (0x83, '\u{0192}'),
    (0x84, '\u{201E}'),
    (0x85, '\u{2026}'),
    (0x86, '\u{2020}'),
    (0x87, '\u{2021}'),
    (0x88, '\u{02C6}'),
    (0x89, '\u{2030}'),
    (0x8A, '\u{0160}'),
    (0x8B, '\u{2039}'),
    (0x8C, '\u{0152}'),
    (0x8E, '\u{017D}'),
    (0x91, '\u{2018}'),
    (0x92, '\u{2019}'),
    (0x93, '\u{201C}'),
    (0x94, '\u{201D}'),
    (0x95, '\u{2022}'),
    (0x96, '\u{2013}'),
    (0x97, '\u{2014}'),
    (0x98, '\u{02DC}'),
    (0x99, '\u{2122}'),
    (0x9A, '\u{0161}'),
    (0x9B, '\u{203A}'),
    (0x9C, '\u{0153}'),
    (0x9E, '\u{017E}'),
    (0x9F, '\u{0178}'),
];

fn reference(number: u32) -> PdfObject {
    PdfObject::Reference {
        number,
        generation: 0,
    }
}

fn alloc(next: &mut u32) -> u32 {
    let number = *next;
    *next += 1;
    number
}

fn dict(entries: &[(&str, PdfObject)]) -> PdfDictionary {
    let mut map = BTreeMap::new();
    for (key, value) in entries {
        map.insert((*key).to_string(), value.clone());
    }
    PdfDictionary::new(map)
}

fn pdf_number(value: f64) -> PdfObject {
    if is_integerish(value) {
        PdfObject::Integer(value.round() as i64)
    } else {
        PdfObject::Real(round_pdf_num(value))
    }
}

fn fmt_num(value: f64) -> String {
    let value = round_pdf_num(value);
    if is_integerish(value) {
        return format!("{}", value.round() as i64);
    }
    let mut s = format!("{value:.4}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    if s == "-0" {
        s = "0".to_string();
    }
    s
}

fn round_pdf_num(value: f64) -> f64 {
    if value.abs() < 0.000_000_1 {
        0.0
    } else {
        (value * 10_000.0).round() / 10_000.0
    }
}

fn is_integerish(value: f64) -> bool {
    (value - value.round()).abs() < 0.000_000_1
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

fn utf16be_hex_for_str(value: &str) -> String {
    let mut out = String::new();
    for unit in value.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out
}

fn utf16be_hex_with_bom(value: &str) -> String {
    let mut out = String::from("FEFF");
    out.push_str(&utf16be_hex_for_str(value));
    out
}

fn pdf_text_string(value: &str) -> Vec<u8> {
    let mut bytes = vec![0xfe, 0xff];
    for unit in value.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    bytes
}

fn sanitize_pdf_name(value: &str, fallback: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
            out.push(ch);
        }
    }
    if out.is_empty() {
        fallback.to_string()
    } else {
        out
    }
}

#[cfg(test)]
#[path = "authoring_context_tests.rs"]
mod context_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContentEngine, ContentParser};

    #[test]
    fn content_stream_has_expected_text_and_graphics_operators() {
        let mut page = PdfPageBuilder::new(PageSize::LETTER);
        let text = TextStyle::standard(StandardFont::Helvetica, 12.0)
            .fill(Color::device_rgb(0.1, 0.2, 0.3));
        page.draw_text("Hello", 72.0, 720.0, &text).unwrap();
        page.draw_rect(
            72.0,
            680.0,
            144.0,
            24.0,
            &GraphicsStyle::fill_stroke(
                Color::device_rgb(0.9, 0.9, 0.9),
                Color::device_rgb(0.0, 0.0, 0.0),
                2.0,
            ),
        );
        page.draw_line(
            72.0,
            660.0,
            216.0,
            660.0,
            &GraphicsStyle::stroke(Color::black(), 1.0),
        );

        let plan = FontBuildPlan::from_pages(&[page.clone()]).unwrap();
        let image_plan = ImageBuildPlan {
            images: Vec::new(),
            resource_names: HashMap::new(),
        };
        let content = build_content_stream(&page, &plan, &image_plan).unwrap();
        let ops = ContentParser::parse(&content).unwrap();
        let names: Vec<_> = ops.iter().map(|op| op.operator.as_str()).collect();
        assert!(names.windows(2).any(|pair| pair == ["BT", "Tf"]));
        assert!(names.contains(&"Tj"));
        assert!(names.contains(&"re"));
        assert!(names.contains(&"m"));
        assert!(names.contains(&"l"));
        assert!(names.contains(&"S"));
        assert!(names.contains(&"B"));
    }

    #[test]
    fn authored_object_graph_reopens_with_pages_and_resources() {
        let mut doc = PdfBuilder::new();
        doc.set_title("Authored");
        doc.add_page(PageSize::LETTER)
            .draw_text(
                "Hello object graph",
                72.0,
                720.0,
                &TextStyle::standard(StandardFont::Helvetica, 12.0),
            )
            .unwrap();
        let bytes = doc.to_bytes().unwrap();
        let engine = ContentEngine::open_bytes(bytes).unwrap();
        assert_eq!(engine.page_count().unwrap(), 1);
        let page = engine.document().get_pages().unwrap().remove(0);
        assert!(page.resources.get_dict("Font").is_some());
        assert!(!page.contents.is_empty());
    }

    #[test]
    fn standard_and_unicode_text_extract() {
        let mut doc = PdfBuilder::new();
        let page = doc.add_page(PageSize::LETTER);
        page.draw_text(
            "Standard text",
            72.0,
            720.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap();
        page.draw_text(
            "Unicode cafe \u{03c0}",
            72.0,
            690.0,
            &TextStyle::unicode(12.0),
        )
        .unwrap();
        page.draw_text(
            "\u{03b1}\u{03b2}",
            72.0,
            660.0,
            &TextStyle::standard(StandardFont::Symbol, 12.0),
        )
        .unwrap();

        let engine = ContentEngine::open_bytes(doc.to_bytes().unwrap()).unwrap();
        let text = engine.get_page_text(1).unwrap();
        assert!(text.contains("Standard text"), "{text}");
        assert!(text.contains("Unicode cafe \u{03c0}"), "{text}");
        assert!(text.contains("\u{03b1}\u{03b2}"), "{text}");
    }

    #[test]
    fn complex_script_authoring_uses_shaped_cids_and_actual_text() {
        let mut doc = PdfBuilder::new();
        let font = doc
            .register_font_bytes(
                "DejaVuSansFontSubsystem",
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf"))
                    .as_slice(),
            )
            .unwrap();
        let arabic = "\u{0633}\u{0644}\u{0627}\u{0645}";
        doc.add_page(PageSize::LETTER)
            .draw_text(arabic, 72.0, 720.0, &TextStyle::new(font, 18.0))
            .unwrap();

        let plan = FontBuildPlan::from_builder(&doc).unwrap();
        let shaped = plan
            .shaped_run(font, arabic)
            .expect("Arabic text should use shaped CID path");
        assert!(!shaped.glyphs.is_empty());
        let embedded = plan.embedded_plan(font).unwrap();
        assert!(embedded.entries.iter().all(|entry| entry.glyph_id > 0));

        let image_plan = ImageBuildPlan {
            images: Vec::new(),
            resource_names: HashMap::new(),
        };
        let content = String::from_utf8(
            build_content_stream(&doc.pages[0], &plan, &image_plan).expect("content"),
        )
        .unwrap();
        assert!(
            content.contains("/ActualText <FEFF0633064406270645>"),
            "{content}"
        );
        assert!(content.contains("BDC"), "{content}");

        let cmap = String::from_utf8(build_to_unicode_cmap(font, &plan, "FontSubsystem").unwrap())
            .expect("cmap is text");
        assert!(cmap.contains("0633") || cmap.contains("0644"), "{cmap}");

        let cid_to_gid = build_cid_to_gid_map(font, &plan).unwrap();
        for glyph in &shaped.glyphs {
            let entry = embedded
                .entries
                .iter()
                .find(|entry| entry.cid == glyph.cid)
                .expect("cid entry");
            let offset = usize::from(glyph.cid) * 2;
            let mapped = u16::from_be_bytes([cid_to_gid[offset], cid_to_gid[offset + 1]]);
            assert_eq!(mapped, entry.glyph_id);
        }
    }

    #[test]
    fn authored_positions_match_contextual_advances_and_mark_offsets() {
        let bytes = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf"));
        let mut doc = PdfBuilder::new();
        let font = doc
            .register_font_bytes("PositionFixture", bytes.as_slice())
            .unwrap();
        let text = "AV AVA x\u{0301} שלום";
        doc.add_page(PageSize::LETTER)
            .draw_text(text, 20.0, 90.0, &TextStyle::new(font, 18.0))
            .unwrap();
        let plan = FontBuildPlan::from_builder(&doc).unwrap();
        let saved = plan.shaped_run(font, text).unwrap();
        let shaped =
            crate::fonts::TextShaper::shape(bytes, text, crate::fonts::ShapeOptions::default())
                .unwrap();
        assert_eq!(saved.glyphs.len(), shaped.glyphs.len());
        let mut content = Vec::new();
        write_text_command(
            &mut content,
            text,
            20.0,
            90.0,
            &TextStyle::new(font, 18.0),
            &plan,
        )
        .unwrap();
        let content = String::from_utf8(content).unwrap();
        let mut pen = 20.0;
        for (actual, expected) in saved.glyphs.iter().zip(&shaped.glyphs) {
            assert_eq!(
                (actual.advance, actual.offset_x, actual.offset_y),
                (expected.advance, expected.offset_x, expected.offset_y)
            );
            let command = format!(
                "1 0 0 1 {} {} Tm <{:04X}> Tj",
                fmt_num(pen + expected.offset_x * 18.0 / 1000.0),
                fmt_num(90.0 + expected.offset_y * 18.0 / 1000.0),
                actual.cid
            );
            assert!(content.contains(&command));
            pen += expected.advance * 18.0 / 1000.0;
        }
    }

    #[test]
    fn embedded_truetype_authoring_uses_real_glyf_subset() {
        let font_bytes =
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf")).as_slice();
        let mut doc = PdfBuilder::new();
        let font = doc
            .register_font_bytes("DejaVuSubsetFontFailureAnalysis", font_bytes)
            .unwrap();
        doc.add_page(PageSize::LETTER)
            .draw_text("Hello subset", 72.0, 720.0, &TextStyle::new(font, 18.0))
            .unwrap();

        let plan = FontBuildPlan::from_builder(&doc).unwrap();
        let mut next = 1;
        let built = build_embedded_type0_font(
            &mut next,
            font,
            "DejaVuSubsetFontFailureAnalysis",
            font_bytes,
            &plan,
        )
        .unwrap();
        let font_file = built
            .objects
            .iter()
            .find_map(|object| match &object.object {
                PdfObject::Stream { dict, raw }
                    if dict.get_name("WellfriendSubset").is_none()
                        && dict.get_integer("Length1").is_some() =>
                {
                    Some((dict, raw))
                }
                PdfObject::Stream { dict, raw }
                    if matches!(dict.get("WellfriendSubset"), Some(PdfObject::Dictionary(_))) =>
                {
                    Some((dict, raw))
                }
                _ => None,
            })
            .expect("embedded font file stream");
        assert!(font_file.1.len() < font_bytes.len());
        assert_eq!(
            font_file.0.get_integer("Length1").unwrap() as usize,
            font_file.1.len()
        );
        let subset_info = match font_file.0.get("WellfriendSubset") {
            Some(PdfObject::Dictionary(dict)) => dict,
            other => panic!("missing subset info: {other:?}"),
        };
        assert_eq!(subset_info.get_bool("Enabled"), Some(true));
        assert!(
            ttf_parser::Face::parse(font_file.1, 0).is_ok(),
            "subset sfnt must parse"
        );

        let engine = ContentEngine::open_bytes(doc.to_bytes().unwrap()).unwrap();
        let text = engine.get_page_text(1).unwrap();
        assert!(text.contains("Hello subset"), "{text}");
    }

    #[test]
    fn embedded_truetype_subset_authoring_is_deterministic() {
        fn build() -> Vec<u8> {
            let font_bytes =
                include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf"))
                    .as_slice();
            let mut doc = PdfBuilder::new();
            let font = doc
                .register_font_bytes("DejaVuSubsetDeterministic", font_bytes)
                .unwrap();
            doc.add_page(PageSize::LETTER)
                .draw_text(
                    "Deterministic subset",
                    72.0,
                    720.0,
                    &TextStyle::new(font, 18.0),
                )
                .unwrap();
            doc.to_bytes().unwrap()
        }
        assert_eq!(build(), build());
    }

    #[test]
    fn arabic_shaped_authoring_survives_subset_embedding() {
        let font_bytes =
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/fonts/DejaVuSans.ttf")).as_slice();
        let mut doc = PdfBuilder::new();
        let font = doc
            .register_font_bytes("DejaVuArabicSubsetFontFailureAnalysis", font_bytes)
            .unwrap();
        let arabic = "\u{0633}\u{0644}\u{0627}\u{0645}";
        doc.add_page(PageSize::LETTER)
            .draw_text(arabic, 72.0, 720.0, &TextStyle::new(font, 18.0))
            .unwrap();

        let plan = FontBuildPlan::from_builder(&doc).unwrap();
        assert!(plan.shaped_run(font, arabic).is_some());
        let mut next = 1;
        let built = build_embedded_type0_font(
            &mut next,
            font,
            "DejaVuArabicSubsetFontFailureAnalysis",
            font_bytes,
            &plan,
        )
        .unwrap();
        let subset_font_bytes = built
            .objects
            .iter()
            .find_map(|object| match &object.object {
                PdfObject::Stream { dict, raw }
                    if matches!(dict.get("WellfriendSubset"), Some(PdfObject::Dictionary(_))) =>
                {
                    Some(raw)
                }
                _ => None,
            })
            .expect("subset font file");
        assert!(subset_font_bytes.len() < font_bytes.len());

        let engine = ContentEngine::open_bytes(doc.to_bytes().unwrap()).unwrap();
        let text = engine.get_page_text(1).unwrap();
        assert!(text.contains(arabic), "{text}");
    }

    #[test]
    fn wrapping_and_alignment_use_measured_widths() {
        let mut page = PdfPageBuilder::new(PageSize::LETTER);
        let style = TextStyle::standard(StandardFont::Helvetica, 12.0);
        let lines = page
            .draw_paragraph(
                "alpha beta gamma delta",
                100.0,
                700.0,
                80.0,
                &style,
                &ParagraphStyle::new().align(TextAlign::Center),
            )
            .unwrap();
        assert!(lines.len() >= 2);

        let plan = FontBuildPlan::from_pages(&[page.clone()]).unwrap();
        let image_plan = ImageBuildPlan {
            images: Vec::new(),
            resource_names: HashMap::new(),
        };
        let content =
            String::from_utf8(build_content_stream(&page, &plan, &image_plan).unwrap()).unwrap();
        assert!(
            content.contains(" Td <"),
            "paragraph should emit positioned text: {content}"
        );
        assert!(
            page.text_width(&lines[0], &style).unwrap() <= 80.0,
            "wrapped line fits"
        );
    }

    #[test]
    fn authored_output_is_deterministic() {
        fn build() -> Vec<u8> {
            let mut doc = PdfBuilder::new();
            let page = doc.add_page(PageSize::A4);
            page.draw_text(
                "Deterministic",
                72.0,
                720.0,
                &TextStyle::standard(StandardFont::TimesRoman, 14.0),
            )
            .unwrap();
            page.draw_circle(
                120.0,
                620.0,
                20.0,
                &GraphicsStyle::fill(Color::device_rgb(0.2, 0.4, 0.7)),
            );
            doc.to_bytes().unwrap()
        }
        assert_eq!(build(), build());
    }
}
