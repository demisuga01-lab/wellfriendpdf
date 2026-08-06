use std::collections::{BTreeSet, HashMap};
use std::io::{self, Cursor, Read};
use std::path::Path;
use std::sync::Arc;

use crate::content::{ContentOperation, ContentParser, StreamingContentTokenizer};
use crate::document::{PdfDocument, PdfPage};
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless_reader_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::images::decoder::{ImageDecoder, RawImage};
use crate::images::encoder::{ImageEncoder, ImageOutputFormat};
use crate::images::locator::{ImageLocateOptions, ImageLocator, ImageReference};
use crate::object::{PdfDictionary, PdfObject};
use crate::pubsec::PubSecKeyProvider;
use crate::reader::PdfReader;
use crate::render::contract::RevisionId;
use crate::render::{
    CanonicalDocument, DisplayList, EditDocumentView, InvalidationResult, PageRenderer,
    PixelBuffer, PixelFormat, ProgressiveRenderJob, RenderCache, RenderContract,
    RenderDocumentCache, RenderDocumentView, RenderMode, RenderPlan, RenderTile,
    SemanticDocumentView, TransactionInvalidationResult, TransactionWriteSet,
    ValidationDocumentView, Viewport, WHITE,
};
use crate::text::{TextExtractOptions, TextExtractor, TextFormatOptions};
use crate::{
    decode_scheduler::{
        estimate_image_decode_bytes, estimate_raw_stream_decode_bytes, DecodeSchedulerContext,
    },
    CancelToken,
};

#[derive(Debug, Clone, Default)]
pub struct PageResources {
    pub fonts: HashMap<String, PdfDictionary>,
    pub xobjects: HashMap<String, (u32, u16)>,
    pub xobject_subtypes: HashMap<String, String>,
    pub xobject_bboxes: HashMap<String, [f64; 4]>,
    pub xobject_matrices: HashMap<String, [f64; 6]>,
    pub color_spaces: HashMap<String, PdfObject>,
    pub ext_g_states: HashMap<String, PdfDictionary>,
    pub patterns: HashMap<String, PdfObject>,
    pub shadings: HashMap<String, PdfObject>,
    pub properties: HashMap<String, PdfObject>,
}

fn encode_contract_row(
    source: &[u8],
    destination: &mut [u8],
    format: PixelFormat,
    grayscale: bool,
) {
    destination.fill(0);
    let bytes_per_pixel = format.bytes_per_pixel();
    for (pixel_index, rgba) in source.chunks_exact(4).enumerate() {
        let offset = pixel_index * bytes_per_pixel;
        let red = rgba[0];
        let green = rgba[1];
        let blue = rgba[2];
        let alpha = rgba[3];
        let gray = ((u16::from(red) * 77 + u16::from(green) * 150 + u16::from(blue) * 29 + 128)
            >> 8) as u8;
        let (red, green, blue) = if grayscale {
            (gray, gray, gray)
        } else {
            (red, green, blue)
        };
        match format {
            PixelFormat::Rgba8 => {
                destination[offset..offset + 4].copy_from_slice(&[red, green, blue, alpha])
            }
            PixelFormat::Bgra8 => {
                destination[offset..offset + 4].copy_from_slice(&[blue, green, red, alpha])
            }
            PixelFormat::Rgb8 => {
                destination[offset..offset + 3].copy_from_slice(&[red, green, blue])
            }
            PixelFormat::Bgr8 => {
                destination[offset..offset + 3].copy_from_slice(&[blue, green, red])
            }
            PixelFormat::Gray8 => destination[offset] = gray,
        }
    }
}

struct JoinedContentStreams<'a> {
    readers: Vec<Box<dyn Read + 'a>>,
    index: usize,
    emit_separator: bool,
    pending_error: Option<io::Error>,
}

impl<'a> JoinedContentStreams<'a> {
    fn new(readers: Vec<Box<dyn Read + 'a>>) -> Self {
        Self {
            readers,
            index: 0,
            emit_separator: false,
            pending_error: None,
        }
    }
}

impl Read for JoinedContentStreams<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(err) = self.pending_error.take() {
            return Err(err);
        }
        if buf.is_empty() {
            return Ok(0);
        }

        let mut written = 0usize;
        while written < buf.len() {
            if self.emit_separator {
                buf[written] = b'\n';
                written += 1;
                self.emit_separator = false;
                if written == buf.len() {
                    break;
                }
            }

            if self.index >= self.readers.len() {
                break;
            }

            match self.readers[self.index].read(&mut buf[written..]) {
                Ok(0) => {
                    self.index += 1;
                    self.emit_separator = self.index < self.readers.len();
                }
                Ok(n) => written += n,
                Err(err) if written > 0 => {
                    self.pending_error = Some(err);
                    break;
                }
                Err(err) => return Err(err),
            }
        }

        Ok(written)
    }
}

/// A rectangular page region in PDF user-space points.
///
/// Coordinates use the same convention as Wellfriend's positioned layout model:
/// origin at the page's bottom-left, `x` increasing rightward, and `y`
/// increasing upward. Region extraction keeps an item when the item's center is
/// inside the rectangle or at least half of the item's area overlaps it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct PageRegion {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

impl PageRegion {
    pub fn new(x0: f64, y0: f64, x1: f64, y1: f64) -> Result<Self> {
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
            return Err(WellfriendError::MalformedPdf(
                "region coordinates must be finite numbers".to_string(),
            ));
        }
        if x0 >= x1 || y0 >= y1 {
            return Err(WellfriendError::MalformedPdf(
                "region must satisfy x0 < x1 and y0 < y1".to_string(),
            ));
        }
        Ok(Self { x0, y0, x1, y1 })
    }

    pub fn from_array(region: [f64; 4]) -> Result<Self> {
        Self::new(region[0], region[1], region[2], region[3])
    }

    pub fn as_array(self) -> [f64; 4] {
        [self.x0, self.y0, self.x1, self.y1]
    }

    pub fn width(self) -> f64 {
        (self.x1 - self.x0).max(0.0)
    }

    pub fn height(self) -> f64 {
        (self.y1 - self.y0).max(0.0)
    }

    pub fn contains_point(self, x: f64, y: f64) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }

    pub fn overlap_area(self, bbox: [f64; 4]) -> f64 {
        let bx0 = bbox[0].min(bbox[2]);
        let by0 = bbox[1].min(bbox[3]);
        let bx1 = bbox[0].max(bbox[2]);
        let by1 = bbox[1].max(bbox[3]);
        let x = (self.x1.min(bx1) - self.x0.max(bx0)).max(0.0);
        let y = (self.y1.min(by1) - self.y0.max(by0)).max(0.0);
        x * y
    }

    pub fn overlap_ratio_of(self, bbox: [f64; 4]) -> f64 {
        let area = bbox_area(bbox);
        if area <= 0.0 {
            return 0.0;
        }
        self.overlap_area(bbox) / area
    }

    pub fn keeps_bbox(self, bbox: [f64; 4]) -> bool {
        let cx = (bbox[0] + bbox[2]) / 2.0;
        let cy = (bbox[1] + bbox[3]) / 2.0;
        self.contains_point(cx, cy) || self.overlap_ratio_of(bbox) >= 0.5
    }

    fn clamp_to_page(self, page_box: [f64; 4]) -> Result<Self> {
        let page = normalize_bbox(page_box);
        let x0 = self.x0.max(page[0]);
        let y0 = self.y0.max(page[1]);
        let x1 = self.x1.min(page[2]);
        let y1 = self.y1.min(page[3]);
        if x0 >= x1 || y0 >= y1 {
            return Err(WellfriendError::MalformedPdf(format!(
                "region [{:.2},{:.2},{:.2},{:.2}] does not overlap page box [{:.2},{:.2},{:.2},{:.2}]",
                self.x0, self.y0, self.x1, self.y1, page[0], page[1], page[2], page[3]
            )));
        }
        Ok(Self { x0, y0, x1, y1 })
    }
}

/// Named extraction profiles. Profiles are convenience bundles over existing
/// engine options; they do not introduce a separate parser path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExtractionProfile {
    /// Speed-oriented plain text extraction.
    #[default]
    FastText,
    /// Prefer the layout analyzer for line/column-faithful text.
    LayoutFaithful,
    /// Preserve table structure and keep parse options table-friendly.
    TablesFocused,
    /// RAG-oriented parse defaults: omit furniture, normalize searchable text.
    RagChunks,
}

impl ExtractionProfile {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "fast-text" | "fast" | "text" => Some(Self::FastText),
            "layout-faithful" | "layout" | "faithful" => Some(Self::LayoutFaithful),
            "tables-focused" | "tables" | "table" => Some(Self::TablesFocused),
            "rag-chunks" | "rag" | "chunks" => Some(Self::RagChunks),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::FastText => "fast-text",
            Self::LayoutFaithful => "layout-faithful",
            Self::TablesFocused => "tables-focused",
            Self::RagChunks => "rag-chunks",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FastText => "speed-optimized plain text extraction",
            Self::LayoutFaithful => "layout-aware reading order and spacing",
            Self::TablesFocused => "table-preserving document parsing",
            Self::RagChunks => {
                "RAG-ready text with furniture omitted and searchable text normalized"
            }
        }
    }

    fn apply_parse_options(self, options: &mut crate::parse::ParseOptions) {
        match self {
            Self::FastText => {}
            Self::LayoutFaithful => {
                options.omit_furniture = false;
            }
            Self::TablesFocused => {
                options.omit_furniture = true;
            }
            Self::RagChunks => {
                options.omit_furniture = true;
                options.dehyphenate = true;
                options.normalize_ligatures = true;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RegionWord {
    pub text: String,
    pub page: usize,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

#[derive(Debug, Clone)]
pub struct PlacedImageReference {
    pub image: ImageReference,
    pub bbox: [f64; 4],
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RegionImage {
    pub page: usize,
    pub name: String,
    pub bbox: [f64; 4],
    pub width: u32,
    pub height: u32,
    pub bits_per_component: u8,
    pub color_space: String,
    pub filters: Vec<String>,
    pub inline: bool,
    pub mask: bool,
    pub soft_mask: bool,
}

impl From<&PlacedImageReference> for RegionImage {
    fn from(value: &PlacedImageReference) -> Self {
        let image = &value.image;
        Self {
            page: image.page_number,
            name: image.xobject_name.clone(),
            bbox: value.bbox,
            width: image.width,
            height: image.height,
            bits_per_component: image.bits_per_component,
            color_space: image.color_space.clone(),
            filters: image.filter.clone(),
            inline: image.is_inline,
            mask: image.is_mask,
            soft_mask: image.is_smask,
        }
    }
}

/// Fetch a resource sub-dictionary (e.g. `/Font`, `/ColorSpace`, `/Pattern`),
/// resolving an indirect reference when the entry is one. Real-world PDFs
/// (notably pdf.js-generated files) often store these sub-dictionaries as
/// indirect objects, e.g. `/ColorSpace 12 0 R`, so a direct `get_dict` lookup
/// would miss them and leave the corresponding resources empty.
fn resolve_subdict(
    resources: &PdfDictionary,
    key: &str,
    reader: &PdfReader,
) -> Option<PdfDictionary> {
    match resources.get(key) {
        Some(PdfObject::Dictionary(d)) => Some(d.clone()),
        Some(obj @ PdfObject::Reference { .. }) => match reader.resolve(obj.clone()) {
            Ok(PdfObject::Dictionary(d)) => Some(d),
            _ => None,
        },
        _ => None,
    }
}

impl PageResources {
    pub fn from_dict(resources: &PdfDictionary, reader: &PdfReader) -> Self {
        let mut page_resources = PageResources::default();

        if let Some(font_dict) = resolve_subdict(resources, "Font", reader) {
            for (name, value) in font_dict.entries() {
                match reader.resolve(value.clone()) {
                    Ok(PdfObject::Dictionary(dict)) => {
                        page_resources.fonts.insert(name.clone(), dict);
                    }
                    Ok(other) => {
                        log::warn!(
                            "PageResources: font '{}' resolved to non-dict {}",
                            name,
                            other.variant_name()
                        );
                    }
                    Err(err) => {
                        log::warn!("PageResources: could not resolve font '{}': {}", name, err);
                    }
                }
            }
        }

        if let Some(xobject_dict) = resolve_subdict(resources, "XObject", reader) {
            for (name, value) in xobject_dict.entries() {
                if let Some(reference) = value.as_reference() {
                    page_resources.xobjects.insert(name.clone(), reference);
                    if let Ok(PdfObject::Stream { dict, .. }) =
                        reader.get_object(reference.0, reference.1)
                    {
                        if let Some(subtype) = dict.get_name("Subtype") {
                            page_resources
                                .xobject_subtypes
                                .insert(name.clone(), subtype.to_string());
                            if subtype == "Form" {
                                if let Some(bbox) = numeric_array_4(&dict, "BBox") {
                                    page_resources.xobject_bboxes.insert(name.clone(), bbox);
                                }
                                if let Some(matrix) = numeric_array_6(&dict, "Matrix") {
                                    page_resources.xobject_matrices.insert(name.clone(), matrix);
                                }
                            }
                        }
                    }
                } else {
                    log::warn!(
                        "PageResources: XObject '{}' is not an indirect reference",
                        name
                    );
                }
            }
        }

        if let Some(color_space_dict) = resolve_subdict(resources, "ColorSpace", reader) {
            for (name, value) in color_space_dict.entries() {
                let resolved = match reader.resolve(value.clone()) {
                    Ok(object) => object,
                    Err(err) => {
                        log::warn!(
                            "PageResources: could not resolve ColorSpace '{}': {}",
                            name,
                            err
                        );
                        value.clone()
                    }
                };
                page_resources.color_spaces.insert(name.clone(), resolved);
            }
        }

        if let Some(ext_g_state_dict) = resolve_subdict(resources, "ExtGState", reader) {
            for (name, value) in ext_g_state_dict.entries() {
                match reader.resolve(value.clone()) {
                    Ok(PdfObject::Dictionary(dict)) => {
                        page_resources.ext_g_states.insert(name.clone(), dict);
                    }
                    Ok(other) => {
                        log::warn!(
                            "PageResources: ExtGState '{}' resolved to non-dict {}",
                            name,
                            other.variant_name()
                        );
                    }
                    Err(err) => {
                        log::warn!("PageResources: ExtGState '{}' error: {}", name, err);
                    }
                }
            }
        }

        if let Some(pattern_dict) = resolve_subdict(resources, "Pattern", reader) {
            for (name, value) in pattern_dict.entries() {
                page_resources.patterns.insert(name.clone(), value.clone());
            }
        }

        if let Some(shading_dict) = resolve_subdict(resources, "Shading", reader) {
            for (name, value) in shading_dict.entries() {
                page_resources.shadings.insert(name.clone(), value.clone());
            }
        }

        if let Some(properties_dict) = resolve_subdict(resources, "Properties", reader) {
            for (name, value) in properties_dict.entries() {
                page_resources
                    .properties
                    .insert(name.clone(), value.clone());
            }
        }

        page_resources
    }
}

fn numeric_array_4(dict: &PdfDictionary, key: &str) -> Option<[f64; 4]> {
    let arr = dict.get(key)?.as_array()?;
    let values: Vec<f64> = arr.iter().filter_map(PdfObject::as_number).collect();
    (values.len() >= 4).then(|| [values[0], values[1], values[2], values[3]])
}

fn numeric_array_6(dict: &PdfDictionary, key: &str) -> Option<[f64; 6]> {
    let arr = dict.get(key)?.as_array()?;
    let values: Vec<f64> = arr.iter().filter_map(PdfObject::as_number).collect();
    (values.len() >= 6).then(|| {
        [
            values[0], values[1], values[2], values[3], values[4], values[5],
        ]
    })
}

/// Parse a `/Resources` object (a direct dictionary or an indirect reference)
/// into a [`PageResources`]. Used when rendering Form XObjects that carry their
/// own resource dictionary.
///
/// Returns an empty [`PageResources`] when the object does not resolve to a
/// dictionary. Never panics on malformed input.
pub(crate) fn parse_resources_from_obj(res_obj: &PdfObject, reader: &PdfReader) -> PageResources {
    let dict = match res_obj {
        PdfObject::Dictionary(d) => d.clone(),
        PdfObject::Reference { number, generation } => {
            match reader.get_and_resolve(*number, *generation) {
                Ok(PdfObject::Dictionary(d)) => d,
                _ => return PageResources::default(),
            }
        }
        _ => return PageResources::default(),
    };
    PageResources::from_dict(&dict, reader)
}

#[derive(Clone)]
pub struct ContentEngine {
    doc: Arc<PdfDocument>,
    canonical: CanonicalDocument,
}

impl ContentEngine {
    fn from_document(doc: PdfDocument) -> Self {
        let canonical = CanonicalDocument::from_document(&doc);
        Self {
            doc: Arc::new(doc),
            canonical,
        }
    }

    pub fn open_path(path: impl AsRef<Path>) -> Result<Self> {
        let doc = PdfDocument::open_path(path)?;
        Ok(Self::from_document(doc))
    }

    pub fn open_bytes(data: Vec<u8>) -> Result<Self> {
        let doc = PdfDocument::open_bytes(data)?;
        Ok(Self::from_document(doc))
    }

    /// Open a PDF from bytes, supplying a password for encrypted PDFs.
    ///
    /// For non-encrypted PDFs the password is ignored. For encrypted PDFs with
    /// an empty user password, pass `b""` (or just call [`open_bytes`]).
    ///
    /// [`open_bytes`]: ContentEngine::open_bytes
    pub fn open_bytes_with_password(data: Vec<u8>, password: &[u8]) -> Result<Self> {
        let doc = PdfDocument::open_bytes_with_password(data, password)?;
        Ok(Self::from_document(doc))
    }

    /// Open a PDF from a file path, supplying a password for encrypted PDFs.
    pub fn open_path_with_password(path: impl AsRef<Path>, password: &[u8]) -> Result<Self> {
        let doc = PdfDocument::open_path_with_password(path, password)?;
        Ok(Self::from_document(doc))
    }

    /// Open a public-key encrypted PDF from bytes using an explicit provider.
    pub fn open_bytes_with_pubsec_provider(
        data: Vec<u8>,
        provider: &PubSecKeyProvider,
    ) -> Result<Self> {
        let doc = PdfDocument::open_bytes_with_pubsec_provider(data, provider)?;
        Ok(Self::from_document(doc))
    }

    /// True if the underlying reader has an active encryption (decryption)
    /// context — i.e. the document was encrypted and successfully unlocked.
    pub fn is_encrypted(&self) -> bool {
        self.doc.reader().is_encrypted()
    }

    pub fn document(&self) -> &PdfDocument {
        &self.doc
    }

    /// Canonical immutable source identity shared by all lazy views.
    pub fn canonical_document(&self) -> &CanonicalDocument {
        &self.canonical
    }

    /// Map known changed PDF object references to canonical source identities
    /// and invalidate only their proven render-page dependencies. Unknown
    /// objects deliberately fall back to a conservative full cache reset.
    pub fn invalidate_render_cache_for_changed_objects(
        &self,
        cache: &mut RenderDocumentCache,
        changed_objects: &[(u32, u16)],
    ) -> InvalidationResult {
        let changed_ids = self
            .canonical
            .object_identities()
            .iter()
            .filter(|identity| {
                changed_objects.iter().any(|(number, generation)| {
                    *number == identity.number && *generation == identity.generation
                })
            })
            .map(|identity| identity.id)
            .collect::<Vec<_>>();
        cache.invalidate_sources(self.canonical.revision(), &changed_ids)
    }

    /// Drive narrow cache invalidation from a transaction report's write-set.
    ///
    /// Maps the report's `affected_objects` strings to canonical identities and
    /// evicts only the proven page/tile dependencies. If any object ref cannot
    /// be mapped, the cache conservatively resets to prevent stale pixels.
    ///
    /// This is the canonical integration point: callers pass the transaction
    /// report fields directly and get a typed invalidation result back.
    pub fn invalidate_for_transaction(
        &self,
        cache: &mut RenderDocumentCache,
        affected_objects: &[String],
        affected_pages: &[usize],
        next_revision: RevisionId,
    ) -> TransactionInvalidationResult {
        let write_set = TransactionWriteSet::from_transaction_report(
            affected_objects,
            affected_pages,
            next_revision,
        );
        write_set.invalidate(cache, self.canonical.object_identities())
    }

    /// Lazily expose only render-required source state.
    pub fn render_view(&self) -> RenderDocumentView<'_> {
        RenderDocumentView::new(self)
    }

    /// Lazily expose source-linked edit state without constructing semantic or
    /// validation models.
    pub fn edit_view(&self) -> EditDocumentView<'_> {
        EditDocumentView::new(self)
    }

    /// Lazily expose semantic analysis. Ordinary rendering never constructs it.
    pub fn semantic_view(&self) -> SemanticDocumentView<'_> {
        SemanticDocumentView::new(self)
    }

    /// Lazily expose validation work. Ordinary rendering never constructs it.
    pub fn validation_view(&self) -> ValidationDocumentView<'_> {
        ValidationDocumentView::new(self)
    }

    /// Build the canonical default render contract for a full page.
    pub fn default_render_contract(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<RenderContract> {
        let viewport = self.page_viewport(page_number, dpi)?;
        self.render_contract_for_tile(
            page_number,
            dpi,
            render_mode,
            RenderTile::full(viewport.width_px, viewport.height_px),
        )
    }

    pub(crate) fn render_contract_for_tile(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
        tile: RenderTile,
    ) -> Result<RenderContract> {
        let page = self.get_page(page_number)?;
        let viewport = self.page_viewport(page_number, dpi)?;
        let page_identity = self.canonical.page_identity_for(&page);
        let mut contract = RenderContract::for_viewport(
            self.canonical.revision(),
            page_identity.object.id,
            page_number,
            &viewport,
            tile,
            render_mode,
        );
        contract.optional_content =
            crate::optional_content::OptionalContentContext::from_document(self.document())
                .visibility_fingerprint()
                .to_string()
                .into();
        Ok(contract)
    }

    /// Compile a packed retained plan. Native high-level resource payloads are
    /// represented explicitly in the plan's cold table until their backend
    /// compiler is available; callers can inspect this through the plan rather
    /// than falling back silently.
    pub fn compile_render_plan(&self, contract: RenderContract) -> Result<RenderPlan> {
        contract.validate()?;
        if contract.document_revision != self.canonical.revision() {
            return Err(WellfriendError::invalid_input(
                "render contract belongs to a different document revision",
            ));
        }
        let list = self.build_page_display_list(contract.page_number, contract.dpi)?;
        RenderPlan::compile(list, contract)
    }

    /// Render through a fully specified contract into the engine's canonical
    /// RGBA surface. Contracts that request a different output layout must use
    /// [`render_page_into_buffer`](Self::render_page_into_buffer) so the caller
    /// receives bytes matching the requested format and stride.
    pub fn render_page_with_contract(
        &self,
        contract: &RenderContract,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<PixelBuffer> {
        if contract.pixel_format != PixelFormat::Rgba8
            || contract.alpha_mode != crate::render::AlphaMode::Premultiplied
            || contract.grayscale
            || contract.reverse_byte_order
        {
            return Err(WellfriendError::UnsupportedFeature(
                "render_page_with_contract returns canonical premultiplied RGBA; use render_page_into_buffer for the requested surface layout".to_string(),
            ));
        }
        let (buffer, _) = self.render_contract_pixels(contract, cancel)?;
        Ok(buffer)
    }

    /// Render into a caller-owned byte surface. The core validates document
    /// revision, page identity, clip bounds, output dimensions, stride, format,
    /// and buffer length before writing a single byte. Unsupported semantic
    /// policy fields remain typed refusals rather than ignored requests.
    pub fn render_page_into_buffer(
        &self,
        contract: &RenderContract,
        cancel: &crate::cancel::CancelToken,
        output: &mut [u8],
    ) -> Result<()> {
        let (buffer, _) = self.render_contract_pixels(contract, cancel)?;
        let required = contract
            .stride
            .checked_mul(contract.height as usize)
            .ok_or_else(|| {
                WellfriendError::ResourceLimit("render surface byte length overflows".to_string())
            })?;
        if output.len() < required {
            return Err(WellfriendError::invalid_input(format!(
                "caller surface has {} bytes but contract requires at least {required}",
                output.len()
            )));
        }
        let source = buffer.rgba_bytes();
        let source_row_bytes = contract.width as usize * 4;
        for row in 0..contract.height as usize {
            let src = &source[row * source_row_bytes..(row + 1) * source_row_bytes];
            let dst = &mut output[row * contract.stride..(row + 1) * contract.stride];
            encode_contract_row(src, dst, contract.pixel_format, contract.grayscale);
        }
        Ok(())
    }

    fn render_contract_pixels(
        &self,
        contract: &RenderContract,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<(PixelBuffer, RenderTile)> {
        contract.validate()?;
        if contract.document_revision != self.canonical.revision() {
            return Err(WellfriendError::invalid_input(
                "render contract belongs to a different document revision",
            ));
        }
        if contract.reverse_byte_order {
            return Err(WellfriendError::UnsupportedFeature(
                "reverse_byte_order is not implemented for the CPU surface encoder".to_string(),
            ));
        }
        let full_viewport = self.page_viewport(contract.page_number, contract.dpi)?;
        let full_tile = RenderTile::full(full_viewport.width_px, full_viewport.height_px);
        let tile = match contract.clip {
            Some(clip) => {
                if clip.x < 0 || clip.y < 0 {
                    return Err(WellfriendError::invalid_input(
                        "render contract clip origin must be non-negative device coordinates",
                    ));
                }
                let tile = RenderTile {
                    x: clip.x as u32,
                    y: clip.y as u32,
                    width: clip.width,
                    height: clip.height,
                };
                let end_x = tile.x.checked_add(tile.width).ok_or_else(|| {
                    WellfriendError::invalid_input("render contract clip x range overflows")
                })?;
                let end_y = tile.y.checked_add(tile.height).ok_or_else(|| {
                    WellfriendError::invalid_input("render contract clip y range overflows")
                })?;
                if tile.width == 0
                    || tile.height == 0
                    || end_x > full_viewport.width_px
                    || end_y > full_viewport.height_px
                {
                    return Err(WellfriendError::invalid_input(
                        "render contract clip lies outside the requested page viewport",
                    ));
                }
                tile
            }
            None => full_tile,
        };
        let expected = self.render_contract_for_tile(
            contract.page_number,
            contract.dpi,
            contract.render_mode(),
            tile,
        )?;
        let mut normalized = contract.clone();
        normalized.pixel_format = expected.pixel_format;
        normalized.alpha_mode = expected.alpha_mode;
        normalized.stride = expected.stride;
        normalized.grayscale = expected.grayscale;
        // Accept caller-specified policies that are actively implemented:
        // PrintProfile, AnnotationRenderPolicy, and FormRenderPolicy now
        // influence rendering rather than being validation-only metadata.
        let mut accepted_expected = expected.clone();
        accepted_expected.print_profile = contract.print_profile;
        accepted_expected.annotations = contract.annotations;
        accepted_expected.forms = contract.forms;
        if normalized != accepted_expected {
            return Err(WellfriendError::UnsupportedFeature(
                "the requested render contract contains semantic policy fields not yet implemented by the active CPU renderer".to_string(),
            ));
        }
        let buffer = if tile == full_tile {
            PageRenderer::render_page_cancellable_with_contract_policies(
                self,
                contract.page_number,
                contract.dpi,
                cancel,
                contract.render_mode(),
                contract.print_profile,
                contract.annotations,
                contract.forms,
            )?
        } else {
            // For sub-page tiles, render the full page with contract policies
            // then crop to the requested tile. This preserves correct annotation
            // visibility while honoring the tile clip.
            let full_buf = PageRenderer::render_page_cancellable_with_contract_policies(
                self,
                contract.page_number,
                contract.dpi,
                cancel,
                contract.render_mode(),
                contract.print_profile,
                contract.annotations,
                contract.forms,
            )?;
            crate::render::page_renderer::crop_buffer_for_contract(&full_buf, tile)?
        };
        if buffer.width != contract.width || buffer.height != contract.height {
            return Err(WellfriendError::MalformedPdf(
                "render contract output dimensions diverged from the active viewport".to_string(),
            ));
        }
        Ok((buffer, tile))
    }

    pub fn page_count(&self) -> Result<usize> {
        self.doc.page_count()
    }

    pub fn get_page_content(&self, page_number: usize) -> Result<Vec<ContentOperation>> {
        self.validate_page(page_number)?;
        let limits = DecodeLimits::default();
        let scheduler = DecodeSchedulerContext::new(&limits);
        if let Some(streams) = self.doc.content_stream_ranges(page_number)? {
            let mut readers = Vec::with_capacity(streams.len());
            for (stream_index, stream) in streams.into_iter().enumerate() {
                let estimate = stream
                    .dict
                    .get("Length")
                    .and_then(PdfObject::as_integer)
                    .and_then(|value| usize::try_from(value).ok())
                    .map(estimate_raw_stream_decode_bytes)
                    .unwrap_or(1);
                let (status, bytes) = scheduler.run(
                    estimate,
                    &CancelToken::none(),
                    "text extraction content stream decode",
                    || {
                        let decoded = decode_stream_lossless_reader_with_limits(
                            &stream.dict,
                            stream.reader,
                            Some(self.doc.reader()),
                            &limits,
                        )?;
                        let mut bytes = Vec::new();
                        let status = decoded.status;
                        let mut reader = decoded.reader;
                        reader.read_to_end(&mut bytes)?;
                        Ok((status, bytes))
                    },
                )?;
                if let StreamDecodeStatus::StoppedAtImageFilter(filter) = &status {
                    log::warn!("page content stream stopped at image filter {filter}");
                }
                if stream_index > 0 {
                    readers.push(Box::new(Cursor::new(vec![b'\n'])) as Box<dyn Read>);
                }
                readers.push(Box::new(Cursor::new(bytes)) as Box<dyn Read>);
            }
            let tokens = StreamingContentTokenizer::new(JoinedContentStreams::new(readers));
            return ContentParser::parse_tokens_propagating_io(tokens);
        }
        let page = self.doc.get_page(page_number)?;
        let estimate = estimate_raw_stream_decode_bytes(page.contents.len().saturating_mul(1024));
        let bytes = scheduler.run(
            estimate,
            &CancelToken::none(),
            "text extraction fallback content decode",
            || {
                self.doc
                    .get_page_content_bytes_with_limits(page_number, &limits)
            },
        )?;
        ContentParser::parse(&bytes)
    }

    pub fn get_page_resources(&self, page_number: usize) -> Result<PageResources> {
        self.validate_page(page_number)?;
        let page = self.doc.get_page(page_number)?;
        Ok(PageResources::from_dict(&page.resources, self.doc.reader()))
    }

    pub fn get_page(&self, page_number: usize) -> Result<PdfPage> {
        self.validate_page(page_number)?;
        self.doc.get_page(page_number)
    }

    pub fn get_page_text(&self, page_number: usize) -> Result<String> {
        let extractor = TextExtractor::new();
        let options = TextExtractOptions {
            pages: Some(vec![page_number]),
            format: TextFormatOptions {
                include_page_markers: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let text = extractor.extract(self, &options)?;
        if should_prefer_structured_column_text(&text) {
            match self.get_page_text_structured(page_number) {
                Ok(structured) if !structured.trim().is_empty() => Ok(structured),
                _ => Ok(text),
            }
        } else {
            Ok(text)
        }
    }

    /// Extract one page's text using a named convenience profile.
    pub fn get_page_text_with_profile(
        &self,
        page_number: usize,
        profile: ExtractionProfile,
    ) -> Result<String> {
        match profile {
            ExtractionProfile::FastText => self.get_page_text(page_number),
            ExtractionProfile::LayoutFaithful
            | ExtractionProfile::TablesFocused
            | ExtractionProfile::RagChunks => self.get_page_text_structured(page_number),
        }
    }

    /// Run geometric layout analysis (XY-cut segmentation + reading order) on a
    /// page, returning the structured [`PageLayout`](crate::analysis::layout::PageLayout)
    /// (blocks → lines, in reading order). This is **additive** — it does not
    /// affect [`get_page_text`](Self::get_page_text) or the default extraction
    /// path. See [`crate::analysis::layout`].
    pub fn analyze_page_layout(
        &self,
        page_number: usize,
    ) -> Result<crate::analysis::layout::PageLayout> {
        self.analyze_page_layout_with(
            page_number,
            &crate::analysis::layout::LayoutConfig::default(),
        )
    }

    /// Layout analysis with an explicit [`LayoutConfig`](crate::analysis::layout::LayoutConfig).
    pub fn analyze_page_layout_with(
        &self,
        page_number: usize,
        config: &crate::analysis::layout::LayoutConfig,
    ) -> Result<crate::analysis::layout::PageLayout> {
        let chunks = self.collect_page_text_chunks(page_number)?;
        Ok(crate::analysis::layout::analyze_page(&chunks, config))
    }

    /// Collect positioned text runs for a page with provenance-friendly flags
    /// preserved (`ActualText`, invisible OCR-layer text, RTL/vertical writing
    /// mode, font name, font size, and run geometry). This is the shared
    /// low-level input for layout analysis, semantic text, search, and
    /// redaction-preview quads.
    pub fn collect_page_text_chunks(
        &self,
        page_number: usize,
    ) -> Result<Vec<crate::text::TextChunk>> {
        let ops = self.get_page_content(page_number)?;
        let resources = self.get_page_resources(page_number)?;
        let mut collector = crate::text::TextCollector::new(resources, self.doc.reader());
        Ok(collector.collect(&ops))
    }

    /// Collect positioned text runs with active marked-content IDs. This is used
    /// by the Reference Renderer semantic model bridge so `/StructTreeRoot` MCIDs attach
    /// to the same character/quad model used by search and redaction planning.
    pub fn collect_page_marked_text_chunks(
        &self,
        page_number: usize,
    ) -> Result<Vec<crate::text::MarkedTextChunk>> {
        let ops = self.get_page_content(page_number)?;
        let resources = self.get_page_resources(page_number)?;
        let mut collector = crate::text::TextCollector::new(resources, self.doc.reader());
        Ok(collector.collect_marked(&ops))
    }

    /// Structured (layout-aware) text for a page: the page's text in
    /// reading order recovered by XY-cut segmentation, with blocks separated by
    /// a blank line. Correct for multi-column pages where the default
    /// top-to-bottom dump (and plain `pdftotext`) interleaves columns.
    pub fn get_page_text_structured(&self, page_number: usize) -> Result<String> {
        Ok(self.analyze_page_layout(page_number)?.text())
    }

    /// Extract text constrained to a page region in PDF user-space points.
    ///
    /// A line is included when its center lies inside the region or at least
    /// 50% of its line box overlaps the region. Blocks are preserved by joining
    /// included lines with newlines and included blocks with blank lines.
    pub fn extract_text_in_region(&self, page_number: usize, region: PageRegion) -> Result<String> {
        let region = self.validated_region_for_page(page_number, region)?;
        let layout = self.analyze_page_layout(page_number)?;
        let mut blocks = Vec::new();
        for block in layout.blocks {
            let lines: Vec<String> = block
                .lines
                .into_iter()
                .filter(|line| region.keeps_bbox(bbox_from_layout(line.bbox)))
                .map(|line| line.text)
                .collect();
            if !lines.is_empty() {
                blocks.push(lines.join("\n"));
            }
        }
        Ok(blocks.join("\n\n"))
    }

    /// Positioned words for a page from the semantic text model. Word boxes are
    /// derived from the contributing text runs/characters rather than from a
    /// whole-line proportional split, which makes the region/search/redaction
    /// surfaces map back to tighter source quads.
    pub fn extract_page_words(&self, page_number: usize) -> Result<Vec<RegionWord>> {
        let document = self.extract_text_semantic_model(
            &[page_number],
            crate::text::TextSemanticOptions::default(),
        )?;
        Ok(document
            .pages
            .into_iter()
            .flat_map(|page| {
                page.blocks.into_iter().flat_map(move |block| {
                    block.lines.into_iter().flat_map(move |line| {
                        line.words.into_iter().map(move |word| RegionWord {
                            text: word.text,
                            page: page.page,
                            x0: word.quad.x0,
                            y0: word.quad.y0,
                            x1: word.quad.x1,
                            y1: word.quad.y1,
                        })
                    })
                })
            })
            .collect())
    }

    /// Extract positioned words constrained to a page region.
    pub fn extract_words_in_region(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<Vec<RegionWord>> {
        let region = self.validated_region_for_page(page_number, region)?;
        Ok(self
            .extract_page_words(page_number)?
            .into_iter()
            .filter(|word| region.keeps_bbox([word.x0, word.y0, word.x1, word.y1]))
            .collect())
    }

    /// Extract semantic structure for selected pages. Tagged PDFs use the
    /// authored `/StructTreeRoot` and MCID links; untagged PDFs fall back to the
    /// geometric layout analyzer. Additive; the default text path is unchanged.
    pub fn extract_semantic_document(
        &self,
        pages: &[usize],
    ) -> Result<crate::semantic::SemanticDocument> {
        crate::semantic::extract_semantic_document(self, pages)
    }

    /// Recover a conservative semantic graph from `/ParentTree` and page MCIDs
    /// when tagged PDFs have incomplete or broken `/StructTreeRoot` hierarchy.
    ///
    /// The report distinguishes spec-derived, repaired, inferred, orphan, and
    /// conflicting content. It never rewrites raw text and is safe to call when
    /// no ParentTree exists; the returned status reports the exact fallback.
    pub fn recover_parenttree_semantics(
        &self,
        pages: &[usize],
    ) -> Result<crate::semantic_intelligence::ParentTreeRecoveryReport> {
        crate::semantic_intelligence::recover_parenttree_semantics(self, pages)
    }

    /// Report optional Semantic Intelligence local/cloud layout backend template
    /// availability. No model is loaded and no cloud call is made.
    pub fn layout_backend_availability_report(
        &self,
        local: &crate::semantic_intelligence::LayoutLocalBackendConfig,
        cloud: &crate::semantic_intelligence::CloudLayoutBackendConfig,
    ) -> crate::semantic_intelligence::LayoutAvailabilityReport {
        crate::semantic_intelligence::layout_backend_availability_report(local, cloud)
    }

    /// Readable text view of [`extract_semantic_document`](Self::extract_semantic_document).
    pub fn extract_semantic_text(&self, pages: &[usize]) -> Result<String> {
        Ok(self.extract_semantic_document(pages)?.to_text())
    }

    /// Build the Native Renderer semantic text model: pages -> blocks -> paragraphs
    /// -> lines -> words/spans/chars with geometry, confidence, and provenance.
    /// This model is additive and leaves the legacy flat extraction path
    /// unchanged.
    pub fn extract_text_semantic_model(
        &self,
        pages: &[usize],
        options: crate::text::TextSemanticOptions,
    ) -> Result<crate::text::TextSemanticDocument> {
        let page_numbers = if pages.is_empty() {
            (1..=self.page_count()?).collect::<Vec<_>>()
        } else {
            pages.to_vec()
        };
        let structure = if options.include_structure {
            Some(crate::semantic::extract_text_structure_context(
                self,
                &page_numbers,
                options.max_structure_nodes,
                options.max_mcid_entries,
            )?)
        } else {
            None
        };
        let mut out = Vec::with_capacity(page_numbers.len());
        for page_number in page_numbers {
            let page_box = self.page_box(page_number)?;
            if options.include_structure {
                let chunks = self.collect_page_marked_text_chunks(page_number)?;
                out.push(crate::text::build_text_semantic_page_from_marked_chunks(
                    page_number,
                    page_box,
                    chunks,
                    structure.as_ref(),
                    &options,
                ));
            } else {
                let chunks = self.collect_page_text_chunks(page_number)?;
                out.push(crate::text::build_text_semantic_page(
                    page_number,
                    page_box,
                    chunks,
                    &options,
                ));
            }
        }
        Ok(crate::text::build_text_semantic_document(out, Vec::new()))
    }

    /// Search selected pages through the semantic model and return source
    /// character quads for highlighting/redaction previews. This does not apply
    /// redactions; it prepares stable geometry for the editing phase.
    pub fn search_text(
        &self,
        pages: &[usize],
        query: &str,
        options: crate::text::TextSearchOptions,
    ) -> Result<Vec<crate::text::TextSearchMatch>> {
        let semantic_options = crate::text::TextSemanticOptions {
            mode: crate::text::TextExtractionMode::SearchText,
            include_hidden: options.include_hidden,
            ..crate::text::TextSemanticOptions::default()
        };
        let document = self.extract_text_semantic_model(pages, semantic_options)?;
        Ok(document.search(query, &options))
    }

    /// Detect and extract tables on a page (the `extract-tables` tool — a
    /// capability Poppler's CLIs lack). Tries ruled-grid detection from drawn
    /// lines first, then falls back to borderless inference from text alignment.
    /// See [`crate::analysis::tables`].
    pub fn extract_tables(
        &self,
        page_number: usize,
    ) -> Result<Vec<crate::analysis::tables::Table>> {
        let semantic = self.extract_semantic_document(&[page_number])?;
        if semantic.tagged && !semantic.tables.is_empty() {
            return Ok(semantic.tables);
        }

        let ops = self.get_page_content(page_number)?;
        let resources = self.get_page_resources(page_number)?;
        let mut collector = crate::text::TextCollector::new(resources, self.doc.reader());
        let chunks = collector.collect(&ops);
        let graphics = crate::analysis::graphics::collect_graphics(&ops);
        // Filter to tables worth *reporting*: ruled/semantic always qualify;
        // borderless (alignment-only) candidates must be regular dense grids,
        // not key/value forms, prose columns, or lists. This gate is applied
        // only here (the extract-tables reporting surface, shared by the CLI and
        // the Python binding) and deliberately not inside the shared
        // `detect_tables`, so the parse/field path keeps borderless regions it
        // needs for label→value pairing. See `is_reportable_table`.
        Ok(crate::analysis::tables::detect_tables(&chunks, &graphics)
            .into_iter()
            .filter(crate::analysis::tables::is_reportable_table)
            .collect())
    }

    /// Extract tables constrained to a page region.
    pub fn extract_tables_in_region(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<Vec<crate::analysis::tables::Table>> {
        let region = self.validated_region_for_page(page_number, region)?;
        Ok(self
            .extract_tables(page_number)?
            .into_iter()
            .filter(|table| region.keeps_bbox(table.bbox))
            .collect())
    }

    /// Build a typed, ordered **document model** for the selected pages: each
    /// recovered block is classified (heading/paragraph/list/figure/caption/
    /// table/header/footer/page-number) and placed in a robust reading order
    /// (tagged-PDF authored order when present, else a geometric precedence
    /// graph). A capability beyond Poppler's CLIs. See [`crate::docmodel`].
    pub fn build_document_model(&self, pages: &[usize]) -> Result<crate::docmodel::DocumentModel> {
        crate::docmodel::build_document_model(self, pages)
    }

    /// Parse this PDF into the canonical [`crate::parse::Document`] model — the
    /// single structured representation every output format (Markdown / JSON /
    /// HTML) is serialized from. Wraps [`Self::build_document_model`] with
    /// metadata, a per-page view, provenance, and inline-styled text.
    pub fn parse_document(
        &self,
        options: &crate::parse::ParseOptions,
    ) -> Result<crate::parse::Document> {
        crate::parse::parse(self, options)
    }

    /// Parse with a named extraction profile.
    pub fn parse_document_with_profile(
        &self,
        profile: ExtractionProfile,
        options: &crate::parse::ParseOptions,
    ) -> Result<crate::parse::Document> {
        let mut options = options.clone();
        profile.apply_parse_options(&mut options);
        self.parse_document(&options)
    }

    /// Build the Advanced Rendering shared editable document model. This is the model
    /// conversion and edit-planning surfaces consume before writing PDF, Office,
    /// HTML, Markdown, JSON, or RAG-oriented output.
    pub fn build_editable_document(
        &self,
        options: &crate::editable::EditableBuildOptions,
    ) -> Result<crate::editable::EditableDocument> {
        crate::editable::build_editable_document(self, options)
    }

    /// Serialize selected pages to Markdown, optionally disabling heading
    /// detection for callers that want a flat text-like Markdown export.
    pub fn to_markdown_with_options(
        &self,
        pages: &[usize],
        detect_headings: bool,
        serialize_options: &crate::parse::SerializeOptions,
    ) -> Result<String> {
        let selected = if pages.is_empty() {
            (1..=self.page_count()?).collect::<Vec<_>>()
        } else {
            pages.to_vec()
        };
        for &page in &selected {
            self.validate_page(page)?;
        }

        if detect_headings {
            let parse_options = crate::parse::ParseOptions {
                pages: selected,
                ..crate::parse::ParseOptions::default()
            };
            return Ok(self
                .parse_document(&parse_options)?
                .to_markdown(serialize_options));
        }

        let mut out = String::new();
        for page in selected {
            out.push_str(&self.get_page_text(page)?);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// Extract structured **key-value / form fields** (invoice number, date,
    /// total, line items; receipt merchant/amount; form label→value pairs).
    ///
    /// Combines exact AcroForm fields with a pure-Rust spatial label→value
    /// engine and document-type profiles. Operates on the canonical model, so it
    /// works identically on digital-born and OCR'd pages. See [`crate::extract`].
    pub fn extract_fields(
        &self,
        options: &crate::extract::ExtractOptions,
    ) -> Result<crate::extract::ExtractedFields> {
        crate::extract::extract_fields(self, options)
    }

    /// The page's crop box `[x0, y0, x1, y1]` in PDF user-space points.
    pub fn page_box(&self, page_number: usize) -> Result<[f64; 4]> {
        self.page_crop_box(page_number)
    }

    /// Clamp a region to the page box, returning a clean error if there is no
    /// overlap. This is the effective region used by scoped extraction.
    pub fn clamp_region_to_page(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<PageRegion> {
        self.validated_region_for_page(page_number, region)
    }

    /// Visible page size `(width, height)` in user-space units, from the page's
    /// `/CropBox` (falling back to `/MediaBox`). Used by the document-model
    /// layer for margin-band (header/footer) detection and page-area thresholds.
    pub(crate) fn page_dimensions(&self, page_number: usize) -> Result<(f64, f64)> {
        self.validate_page(page_number)?;
        let page = self.doc.get_page(page_number)?;
        let b = page.crop_box;
        Ok(((b[2] - b[0]).abs(), (b[3] - b[1]).abs()))
    }

    /// The page's `/Rotate` value, normalized to one of `0`, `90`, `180`, `270`
    /// (clockwise). Used by the document-model layer to normalize text/graphics
    /// coordinates into upright reading orientation before layout analysis.
    pub(crate) fn page_rotation(&self, page_number: usize) -> Result<i32> {
        self.validate_page(page_number)?;
        let page = self.doc.get_page(page_number)?;
        Ok(page.rotate.rem_euclid(360))
    }

    /// The page's crop box `[x0, y0, x1, y1]` in user space (falls back to the
    /// media box). The origin needed to rotate coordinates about the page.
    pub(crate) fn page_crop_box(&self, page_number: usize) -> Result<[f64; 4]> {
        self.validate_page(page_number)?;
        let page = self.doc.get_page(page_number)?;
        Ok(page.crop_box)
    }

    /// External hyperlinks on a page: each `/Link` annotation with a URI action
    /// (`/A << /S /URI /URI (…) >>`, or a direct `/URI`), as `(rect, uri)` where
    /// `rect` is the annotation's `/Rect` `[x0,y0,x1,y1]` in user space (y-up).
    /// Used by the digital-born pass to attach `[text](href)` links to the blocks
    /// the link rectangles overlap. Best-effort: a malformed annotation is
    /// skipped, never an error. Never resolves remote targets.
    pub(crate) fn page_links(&self, page_number: usize) -> Result<Vec<([f64; 4], String)>> {
        self.validate_page(page_number)?;
        let page = self.doc.get_page(page_number)?;
        let reader = self.doc.reader();
        let page_obj = reader.get_and_resolve(page.object_number, page.generation_number)?;
        let Some(page_dict) = page_obj.as_dict() else {
            return Ok(Vec::new());
        };
        let Some(annots_obj) = page_dict.get("Annots") else {
            return Ok(Vec::new());
        };
        let annots = reader.resolve(annots_obj.clone())?;
        let Some(items) = annots.as_array() else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        for item in items {
            let Ok(resolved) = reader.resolve(item.clone()) else {
                continue;
            };
            let Some(adict) = resolved.as_dict() else {
                continue;
            };
            if adict.get_name("Subtype") != Some("Link") {
                continue;
            }
            let Some(rect) = rect_from_obj(adict.get("Rect"), reader) else {
                continue;
            };
            if let Some(uri) = link_uri(adict, reader) {
                out.push((rect, uri));
            }
        }
        Ok(out)
    }

    pub fn get_text_range(
        &self,
        start_page: usize,
        end_page: usize,
    ) -> Result<Vec<(usize, String)>> {
        self.validate_page(start_page)?;
        self.validate_page(end_page)?;
        let mut results = Vec::new();
        for page in start_page..=end_page {
            match self.get_page_text(page) {
                Ok(text) => results.push((page, text)),
                Err(err) => log::warn!("get_text_range: page {} failed: {}", page, err),
            }
        }
        Ok(results)
    }

    /// Stream text for selected 1-based pages to a caller-provided callback.
    ///
    /// An empty `pages` slice means all pages. This keeps only the active page's
    /// extracted text resident while preserving the existing all-at-once
    /// convenience APIs for callers that want a collected `String`/`Vec`.
    pub fn for_each_page_text<F>(&self, pages: &[usize], mut sink: F) -> Result<()>
    where
        F: FnMut(usize, &str) -> Result<()>,
    {
        let selected = if pages.is_empty() {
            (1..=self.page_count()?).collect::<Vec<_>>()
        } else {
            pages.to_vec()
        };
        for page in selected {
            self.validate_page(page)?;
            let text = self.get_page_text(page)?;
            sink(page, &text)?;
        }
        Ok(())
    }

    pub fn get_all_text(&self) -> Result<Vec<(usize, String)>> {
        let count = self.page_count()?;
        if count == 0 {
            return Ok(Vec::new());
        }
        self.get_text_range(1, count)
    }

    pub fn page_has_text_layer(&self, page_number: usize) -> Result<bool> {
        let operations = self.get_page_content(page_number)?;
        Ok(operations
            .iter()
            .any(|operation| operation.operator == "Tj" || operation.operator == "TJ"))
    }

    /// Find all image XObjects on a single page.
    pub fn find_page_images(&self, page_number: usize) -> Result<Vec<ImageReference>> {
        self.validate_page(page_number)?;
        let opts = ImageLocateOptions::default();
        ImageLocator::find_page_images(self, page_number, &opts)
    }

    /// Find image placements constrained to a page region.
    ///
    /// The returned entries carry both the decodable image reference and the
    /// user-space bounding box for the specific placement. Inline images do not
    /// currently carry reliable placement boxes and are omitted from this
    /// region-filtered surface.
    pub fn find_page_images_in_region(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<Vec<PlacedImageReference>> {
        let region = self.validated_region_for_page(page_number, region)?;
        let images = self.find_page_images(page_number)?;
        let image_names: BTreeSet<String> = images
            .iter()
            .filter(|image| !image.is_inline)
            .map(|image| image.xobject_name.clone())
            .collect();
        if image_names.is_empty() {
            return Ok(Vec::new());
        }

        let ops = self.get_page_content(page_number)?;
        let graphics = crate::analysis::graphics::collect_graphics_with_images(&ops, &image_names);
        let mut out = Vec::new();
        for placement in graphics.images {
            let bbox = [
                placement.bbox.x0,
                placement.bbox.y0,
                placement.bbox.x1,
                placement.bbox.y1,
            ];
            if !region.keeps_bbox(bbox) {
                continue;
            }
            if let Some(image) = images
                .iter()
                .find(|image| image.xobject_name == placement.name)
            {
                out.push(PlacedImageReference {
                    image: image.clone(),
                    bbox,
                });
            }
        }
        Ok(out)
    }

    /// Serializable image-placement metadata constrained to a page region.
    pub fn find_page_image_regions(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<Vec<RegionImage>> {
        Ok(self
            .find_page_images_in_region(page_number, region)?
            .iter()
            .map(RegionImage::from)
            .collect())
    }

    /// Find all image XObjects in the entire document.
    pub fn find_all_images(&self, options: &ImageLocateOptions) -> Result<Vec<ImageReference>> {
        ImageLocator::find_all_images(self, options)
    }

    /// Decode a single image from its ImageReference.
    ///
    /// Inline images (BI/ID/EI) are decoded from the pixel bytes captured on the
    /// reference; XObject images are decoded from their PDF object.
    pub fn decode_image(&self, image: &ImageReference) -> Result<RawImage> {
        self.decode_image_with_limits(image, &DecodeLimits::default())
    }

    pub fn decode_image_with_limits(
        &self,
        image: &ImageReference,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        // TODO: parallel-decode multi-image pages (decode is currently serial per call).
        if image.is_inline {
            return self.decode_inline_image_with_limits(image, limits);
        }
        let scheduler = DecodeSchedulerContext::new(limits);
        scheduler.run(
            estimate_image_decode_bytes(image),
            &CancelToken::none(),
            "image extraction XObject decode",
            || ImageDecoder::decode_with_limits(image, self.document().reader(), limits),
        )
    }

    /// Decode an inline image from the raw data captured during location.
    pub fn decode_inline_image(&self, image: &ImageReference) -> Result<RawImage> {
        self.decode_inline_image_with_limits(image, &DecodeLimits::default())
    }

    pub fn decode_inline_image_with_limits(
        &self,
        image: &ImageReference,
        limits: &DecodeLimits,
    ) -> Result<RawImage> {
        let data = image.inline_data.as_ref().ok_or_else(|| {
            WellfriendError::UnsupportedFeature(format!(
                "inline image '{}' has no captured pixel data",
                image.xobject_name
            ))
        })?;
        let filters: Vec<&str> = data.filters.iter().map(String::as_str).collect();
        let scheduler = DecodeSchedulerContext::new(limits);
        scheduler.run(
            estimate_image_decode_bytes(image),
            &CancelToken::none(),
            "image extraction inline image decode",
            || {
                ImageDecoder::decode_inline_with_limits(
                    &data.bytes,
                    image.width,
                    image.height,
                    data.bits_per_component,
                    &image.color_space,
                    &filters,
                    None,
                    limits,
                )
            },
        )
    }

    /// Encode a decoded RawImage to the specified format.
    pub fn encode_image(
        image: &RawImage,
        format: ImageOutputFormat,
        quality: Option<u8>,
    ) -> Result<Vec<u8>> {
        ImageEncoder::encode(image, &format, quality)
    }

    /// Convenience: decode + encode in one call.
    pub fn extract_image_bytes(
        &self,
        image: &ImageReference,
        format: ImageOutputFormat,
        quality: Option<u8>,
    ) -> Result<Vec<u8>> {
        if let Ok(Some((bytes, _ext))) =
            ImageEncoder::keep_original(image, self.document().reader(), &format)
        {
            return Ok(bytes);
        }

        let raw = self.decode_image(image)?;
        ImageEncoder::encode(&raw, &format, quality)
    }

    /// Create a PixelBuffer sized to render the given page at the given DPI.
    pub fn create_page_buffer(&self, page_number: usize, dpi: u32) -> Result<PixelBuffer> {
        self.create_page_buffer_with_mode(page_number, dpi, RenderMode::Compat)
    }

    /// Create a PixelBuffer sized to render the given page with an explicit render mode.
    pub fn create_page_buffer_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        let viewport = self.page_viewport(page_number, dpi)?;
        Ok(PixelBuffer::new_filled_with_mode(
            viewport.width_px,
            viewport.height_px,
            WHITE,
            render_mode,
        ))
    }

    /// Create a Viewport for the given page at the given DPI.
    ///
    /// Rejects a page whose final pixel count (post-DPI, post-rotation) would
    /// exceed [`max_render_pixels`] BEFORE any buffer is allocated, so a hostile
    /// PDF declaring a giant `/MediaBox` (e.g. `[0 0 200000 200000]`) returns a
    /// clean [`WellfriendError::ResourceLimit`] instead of attempting a multi-hundred-
    /// gigabyte allocation that aborts the process.
    pub fn page_viewport(&self, page_number: usize, dpi: u32) -> Result<Viewport> {
        self.validate_page(page_number)?;
        let page = self.get_page(page_number)?;
        let viewport = Viewport::new_rotated_with_user_unit(
            effective_page_box(&page),
            dpi,
            page_rotation_u32(page.rotate),
            page.user_unit,
        );
        let pixels = viewport.width_px as u64 * viewport.height_px as u64;
        let cap = max_render_pixels();
        if pixels > cap {
            return Err(WellfriendError::ResourceLimit(format!(
                "page {} would render {} pixels ({}x{}) at {} DPI, exceeding the limit of {} \
                 pixels; lower the DPI or the page is abusively large",
                page_number, pixels, viewport.width_px, viewport.height_px, dpi, cap
            )));
        }
        Ok(viewport)
    }

    /// Render a page to a PixelBuffer at the given DPI.
    pub fn render_page(&self, page_number: usize, dpi: u32) -> Result<PixelBuffer> {
        PageRenderer::render_page(self, page_number, dpi)
    }

    /// Render a page with an explicit render mode.
    ///
    /// [`RenderMode::Compat`] is byte-for-byte the default Poppler-compatible
    /// path used by [`render_page`](Self::render_page). [`RenderMode::HighQuality`]
    /// keeps the same geometry/AA coverage but composites RGB in linear light.
    pub fn render_page_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        PageRenderer::render_page_with_mode(self, page_number, dpi, render_mode)
    }

    /// Render a page with a cancellation token threaded into the hot loops.
    ///
    /// The token is polled periodically while executing the page content
    /// stream (and any nested Form XObjects / tiling patterns). When the token
    /// is cancelled — e.g. by a server request-timeout timer — rendering bails
    /// out early with [`WellfriendError::Cancelled`] instead of running to
    /// completion, freeing the worker thread promptly.
    pub fn render_page_cancellable(
        &self,
        page_number: usize,
        dpi: u32,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<PixelBuffer> {
        PageRenderer::render_page_cancellable(self, page_number, dpi, cancel)
    }

    /// Render a page with cancellation and an explicit render mode.
    pub fn render_page_cancellable_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        cancel: &crate::cancel::CancelToken,
        render_mode: RenderMode,
    ) -> Result<PixelBuffer> {
        PageRenderer::render_page_cancellable_with_mode(self, page_number, dpi, cancel, render_mode)
    }

    /// Render a page with cancellation and a caller-owned per-document cache.
    ///
    /// Use this when rendering multiple pages from one document sequentially.
    /// It preserves the same output semantics as
    /// [`render_page_cancellable_with_mode`](Self::render_page_cancellable_with_mode)
    /// while avoiding repeated font/glyph resolver reconstruction.
    pub fn render_page_cancellable_with_mode_and_cache(
        &self,
        page_number: usize,
        dpi: u32,
        cancel: &crate::cancel::CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<PixelBuffer> {
        PageRenderer::render_page_cancellable_with_mode_and_cache(
            self,
            page_number,
            dpi,
            cancel,
            render_mode,
            cache,
        )
    }

    /// Build the conservative Release Packaging display list for a page.
    ///
    /// This is a replayable vector display list for pages whose drawing
    /// operations fit the current subset. The returned list also records
    /// unsupported operations so callers can inspect why a page still needs the
    /// immediate renderer.
    pub fn build_page_display_list(&self, page_number: usize, dpi: u32) -> Result<DisplayList> {
        PageRenderer::build_display_list(self, page_number, dpi)
    }

    /// Render a page through the display-list CPU device when fully supported.
    ///
    /// Returns `Ok(None)` for pages containing operations that are still handled
    /// by the existing immediate renderer.
    pub fn render_page_display_list_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<Option<PixelBuffer>> {
        PageRenderer::render_page_display_list_with_mode(self, page_number, dpi, render_mode)
    }

    /// Render a page through display-list replay with cancellation.
    pub fn render_page_display_list_cancellable_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        cancel: &crate::cancel::CancelToken,
        render_mode: RenderMode,
    ) -> Result<Option<PixelBuffer>> {
        let list = PageRenderer::build_display_list(self, page_number, dpi)?;
        if !list.is_fully_supported() {
            Ok(None)
        } else {
            Ok(Some(
                PageRenderer::render_display_list_cancellable_with_mode(
                    self,
                    page_number,
                    dpi,
                    &list,
                    cancel,
                    render_mode,
                )?,
            ))
        }
    }

    /// Render a display-list page with cancellation and a reusable document cache.
    pub fn render_page_display_list_cancellable_with_mode_and_cache(
        &self,
        page_number: usize,
        dpi: u32,
        cancel: &crate::cancel::CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<Option<PixelBuffer>> {
        let (list, _) =
            PageRenderer::get_or_build_display_list_with_cache(self, page_number, dpi, cache)?;
        if !list.is_fully_supported() {
            Ok(None)
        } else {
            Ok(Some(
                PageRenderer::render_display_list_cancellable_with_mode_and_cache(
                    self,
                    page_number,
                    dpi,
                    list.as_ref(),
                    cancel,
                    render_mode,
                    cache,
                )?,
            ))
        }
    }

    /// Render one pixel-space page tile through retained display-list replay
    /// when the page is fully captured by the display-list model.
    pub fn render_page_display_list_tile_cancellable_with_mode_and_cache(
        &self,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        cancel: &crate::cancel::CancelToken,
        render_mode: RenderMode,
        cache: &mut RenderDocumentCache,
    ) -> Result<Option<PixelBuffer>> {
        PageRenderer::render_page_display_list_tile_cancellable_with_mode_and_cache(
            self,
            page_number,
            dpi,
            tile,
            cancel,
            render_mode,
            cache,
        )
    }

    /// Render a pixel-space page tile through the display-list path where
    /// possible, falling back to the compatibility renderer only when the list
    /// reports an explicit unsupported reason.
    pub fn render_page_tile_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        tile: RenderTile,
        render_mode: RenderMode,
        cache: Option<&mut RenderCache>,
    ) -> Result<PixelBuffer> {
        PageRenderer::render_page_tile_with_mode(self, page_number, dpi, tile, render_mode, cache)
    }

    /// Render a page as deterministic vertical bands.
    pub fn render_page_bands_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        band_height: u32,
        render_mode: RenderMode,
    ) -> Result<Vec<PixelBuffer>> {
        PageRenderer::render_page_bands_with_mode(self, page_number, dpi, band_height, render_mode)
    }

    /// Return the sparse Prepress CMM Separation/DeviceN plate report produced by
    /// the render interpreter for fill/stroke paths.
    pub fn prepress_plate_report(
        &self,
        page_number: usize,
        dpi: u32,
    ) -> Result<crate::prepress::SeparationFramebufferReport> {
        PageRenderer::prepress_plate_report(self, page_number, dpi)
    }

    /// Create an in-process progressive render job that can checkpoint at tile
    /// boundaries and resume without re-rendering completed tiles.
    pub fn progressive_render_job_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        tile_width: u32,
        tile_height: u32,
        render_mode: RenderMode,
    ) -> Result<ProgressiveRenderJob> {
        ProgressiveRenderJob::new(
            self.clone(),
            page_number,
            dpi,
            render_mode,
            tile_width,
            tile_height,
        )
    }

    pub fn progressive_render_job_with_viewport_hint(
        &self,
        page_number: usize,
        dpi: u32,
        tile_width: u32,
        tile_height: u32,
        render_mode: RenderMode,
        viewport_hint: Option<RenderTile>,
    ) -> Result<ProgressiveRenderJob> {
        ProgressiveRenderJob::new_with_viewport_hint(
            self.clone(),
            page_number,
            dpi,
            render_mode,
            tile_width,
            tile_height,
            viewport_hint,
        )
    }

    /// Verify every digital signature field in the document (the `verify-sig`
    /// tool — `pdfsig`-equivalent). See [`crate::signature`] for the precise
    /// scope (cryptographic validity + coverage + cert details; no trust-chain
    /// or revocation checking).
    pub fn verify_signatures(&self) -> Result<Vec<crate::signature::SignatureReport>> {
        crate::signature::verify_signatures(&self.doc)
    }

    /// Verify signatures and evaluate signer trust against the trust anchors in
    /// `options`. A signature is reported `Trusted` only when integrity verifies,
    /// the signer chains to a configured anchor (in validity, not revoked), and
    /// it covers the whole file. With no anchors, trust is `NotVerified`.
    pub fn verify_signatures_with_options(
        &self,
        options: &crate::signature::VerifyOptions,
    ) -> Result<Vec<crate::signature::SignatureReport>> {
        crate::signature::verify_signatures_with_options(&self.doc, options)
    }

    /// Verify signatures and return a portable bundle containing only the
    /// evidence accepted by the validation pipeline for offline replay.
    pub fn verify_signatures_with_options_and_evidence(
        &self,
        options: &crate::signature::VerifyOptions,
    ) -> Result<crate::signature::SignatureValidationOutcome> {
        crate::signature::verify_signatures_with_options_and_evidence(&self.doc, options)
    }

    /// Apply an RSA/SHA-256 detached CMS digital signature as an incremental
    /// update, preserving the original file bytes as an exact prefix.
    pub fn sign(
        &self,
        signer: &crate::signature::PdfSigner,
        options: &crate::signature::SignatureOptions,
    ) -> Result<Vec<u8>> {
        crate::signature::sign_document(&self.doc, signer, options)
    }

    /// Append PAdES long-term-validation material as a catalog `/DSS`
    /// incremental update.
    pub fn add_ltv_material(&self, material: &crate::signature::LtvMaterial) -> Result<Vec<u8>> {
        crate::signature::add_ltv_material(&self.doc, material)
    }

    /// Export the given 1-based pages to a single self-contained HTML or XML
    /// document (the `to-html` tool — `pdftohtml`-equivalent). See
    /// [`crate::html`] for the modes (complex / simple / xml).
    pub fn export_html(
        &self,
        pages: &[usize],
        options: &crate::html::HtmlOptions,
    ) -> Result<String> {
        for &p in pages {
            self.validate_page(p)?;
        }
        crate::html::HtmlExporter::export(self, pages, options)
    }

    /// Render a page to an SVG document (`pdftocairo -svg`-equivalent).
    ///
    /// Pages using only path/text/solid-fill/clip operations become true
    /// scalable vector SVG (text emitted as glyph outlines); pages using
    /// images, shadings, patterns, Form XObjects, or soft masks fall back to a
    /// single embedded rasterized page image (see [`crate::render::svg`]). The
    /// returned [`crate::render::SvgPage`] reports which path was taken.
    pub fn render_page_svg(&self, page_number: usize, dpi: u32) -> Result<crate::render::SvgPage> {
        crate::render::render_page_svg(self, page_number, dpi)
    }

    /// Render a single page to a PostScript page body (the building block of the
    /// `render --format ps` / `pdftops` equivalent). See
    /// [`crate::render::postscript`]. Pages using only path/text/solid-fill/clip
    /// operations become true vector PostScript (text as glyph outlines); pages
    /// using images, shadings, patterns, Form XObjects, or soft masks fall back
    /// to a single embedded rasterised page image.
    pub fn render_page_ps(&self, page_number: usize, dpi: u32) -> Result<crate::render::PsPage> {
        crate::render::render_page_ps(self, page_number, dpi)
    }

    /// Render the given 1-based pages to a complete, DSC-conformant multi-page
    /// PostScript document (`%!PS-Adobe-3.0`). The `is_rasterized` count is the
    /// number of pages that took the rasterize-embed fallback.
    pub fn render_document_ps(&self, pages: &[usize], dpi: u32) -> Result<(String, usize)> {
        let mut ps_pages = Vec::with_capacity(pages.len());
        let mut rasterized = 0usize;
        for &p in pages {
            let page = self.render_page_ps(p, dpi)?;
            if page.is_rasterized {
                rasterized += 1;
            }
            ps_pages.push(page);
        }
        Ok((crate::render::assemble_ps_document(&ps_pages), rasterized))
    }

    /// Render a single page to a conforming EPS document (`%!PS-Adobe-3.0
    /// EPSF-3.0`) with a precise `%%BoundingBox` and no `showpage`/
    /// `setpagedevice` (the `render --format eps` / `pdftops -eps` /
    /// `pdftocairo -eps` equivalent). Returns `(eps, is_rasterized)`.
    pub fn render_page_eps(&self, page_number: usize, dpi: u32) -> Result<(String, bool)> {
        let page = self.render_page_ps(page_number, dpi)?;
        let rasterized = page.is_rasterized;
        Ok((crate::render::assemble_eps_document(&page), rasterized))
    }

    /// Render a page and encode it as PNG using fast compression.
    pub fn render_page_png_fast(&self, page_number: usize, dpi: u32) -> Result<Vec<u8>> {
        // NOTE: line width 0 renders as 1px (PDF hairline spec). Verified in tests.
        let buf = self.render_page(page_number, dpi)?;
        ImageEncoder::encode_png_fast(&buf.to_raw_image())
    }

    /// Render a page with an explicit render mode and encode it as PNG.
    pub fn render_page_png_fast_with_mode(
        &self,
        page_number: usize,
        dpi: u32,
        render_mode: RenderMode,
    ) -> Result<Vec<u8>> {
        let buf = self.render_page_with_mode(page_number, dpi, render_mode)?;
        ImageEncoder::encode_png_fast(&buf.to_raw_image())
    }

    /// Render through a fully specified contract and encode the resulting
    /// canonical RGBA surface as PNG. Unsupported contract semantics are
    /// rejected by `render_page_with_contract`; they are never ignored.
    pub fn render_page_png_with_contract(
        &self,
        contract: &RenderContract,
        cancel: &crate::cancel::CancelToken,
    ) -> Result<Vec<u8>> {
        let buf = self.render_page_with_contract(contract, cancel)?;
        ImageEncoder::encode_png_fast(&buf.to_raw_image())
    }

    /// Build a new PDF containing exactly the given 1-based pages, in the
    /// order given (duplicates and arbitrary ordering are honoured). Underlies
    /// the `extract-pages` tool. Output is unencrypted (see [`crate::writer`]).
    pub fn extract_pages(&self, page_indices: &[usize]) -> Result<Vec<u8>> {
        for &p in page_indices {
            self.validate_page(p)?;
        }
        crate::writer::build_subset(&self.doc, page_indices)
    }

    /// Build a single-page PDF for the given 1-based page. Underlies the
    /// `split` tool, which calls this once per page.
    pub fn extract_single_page(&self, page_number: usize) -> Result<Vec<u8>> {
        self.validate_page(page_number)?;
        crate::writer::build_subset(&self.doc, &[page_number])
    }

    /// Gather document metadata and structural facts (the `info` tool —
    /// `pdfinfo`-equivalent). Works on encrypted documents (they are decrypted
    /// on open).
    pub fn document_info(&self) -> Result<crate::info::DocumentInfo> {
        crate::info::DocumentInfo::gather(&self.doc)
    }

    /// Enumerate every distinct font used in the document (the `fonts` tool —
    /// `pdffonts`-equivalent), walking all resource scopes and deduping by
    /// object id.
    pub fn list_fonts(&self) -> Result<Vec<crate::fonts_report::FontInfo>> {
        crate::fonts_report::list_fonts(&self.doc)
    }

    /// Enumerate every embedded file attachment (the `detach` tool —
    /// `pdfdetach`-equivalent), from both the name tree and file-attachment
    /// annotations, deduped by embedded-file stream object id.
    pub fn list_attachments(&self) -> Result<Vec<crate::attachments::Attachment>> {
        crate::attachments::list_attachments(&self.doc)
    }

    /// Extract the (filter-decoded) bytes of an embedded file attachment.
    pub fn extract_attachment(
        &self,
        attachment: &crate::attachments::Attachment,
    ) -> Result<Vec<u8>> {
        crate::attachments::extract_attachment(&self.doc, attachment)
    }

    fn validate_page(&self, page_number: usize) -> Result<()> {
        if page_number == 0 {
            return Err(WellfriendError::MalformedPdf(
                "page_number is 1-indexed; 0 is invalid".to_string(),
            ));
        }
        let count = self.doc.page_count()?;
        if page_number > count {
            return Err(WellfriendError::MalformedPdf(format!(
                "page {} out of range (document has {} pages)",
                page_number, count
            )));
        }
        Ok(())
    }

    fn validated_region_for_page(
        &self,
        page_number: usize,
        region: PageRegion,
    ) -> Result<PageRegion> {
        self.validate_page(page_number)?;
        region.clamp_to_page(self.page_crop_box(page_number)?)
    }
}

/// Default ceiling on the pixel count of a single rendered page (width * height
/// after DPI and rotation). 100 megapixels admits normal high-DPI pages while
/// keeping the 4-byte-per-pixel buffer around 400 MB before renderer overhead.
/// The cap exists to turn a hostile giant `/MediaBox` into a clean error rather
/// than a process abort from a failed multi-hundred-gigabyte allocation.
pub const DEFAULT_MAX_RENDER_PIXELS: u64 = 100_000_000;

/// The active per-page render pixel cap. Overridable at runtime via the
/// `WELLFRIENDPDF_MAX_RENDER_PIXELS` environment variable (a positive integer); falls
/// back to [`DEFAULT_MAX_RENDER_PIXELS`] when unset, empty, zero, or unparsable.
/// Keeping this an env-var keeps the engine API free of a config object while
/// still letting the CLI/server/benchmark tune the bound.
pub fn max_render_pixels() -> u64 {
    std::env::var("WELLFRIENDPDF_MAX_RENDER_PIXELS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_RENDER_PIXELS)
}

/// Default cap on the number of pixels (`width × height`) an embedded image may
/// declare before its decoded buffer is allocated. Mirrors the render pixel cap
/// so untrusted input is bounded end-to-end: the *decode* layer (image codecs,
/// bit-depth expansion, CCITT/JBIG2 sinks) is capped, not just the render layer.
pub const DEFAULT_MAX_DECODE_PIXELS: u64 = DEFAULT_MAX_RENDER_PIXELS;

/// The active image-decode pixel cap. Overridable via `WELLFRIENDPDF_MAX_DECODE_PIXELS`
/// (a positive integer); falls back to [`DEFAULT_MAX_DECODE_PIXELS`] when unset,
/// empty, zero, or unparsable. A hostile image header declaring enormous
/// dimensions is rejected with a clean error *before* allocation rather than
/// OOMing the process.
pub fn max_decode_pixels() -> u64 {
    std::env::var("WELLFRIENDPDF_MAX_DECODE_PIXELS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&v| v > 0)
        .unwrap_or(DEFAULT_MAX_DECODE_PIXELS)
}

fn should_prefer_structured_column_text(text: &str) -> bool {
    let mut nonempty_lines = 0usize;
    let mut wide_join_lines = 0usize;
    let mut long_lines = 0usize;

    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        nonempty_lines += 1;
        if line.contains("  ") {
            wide_join_lines += 1;
        }
        if line.chars().count() > 100 {
            long_lines += 1;
        }
    }

    if nonempty_lines < 24 {
        return false;
    }

    let wide_ratio = wide_join_lines as f64 / nonempty_lines as f64;
    let long_ratio = long_lines as f64 / nonempty_lines as f64;
    wide_ratio >= 0.45 && long_ratio < 0.05
}

fn page_rotation_u32(rotation: i32) -> u32 {
    rotation.rem_euclid(360) as u32
}

fn normalize_bbox(bbox: [f64; 4]) -> [f64; 4] {
    [
        bbox[0].min(bbox[2]),
        bbox[1].min(bbox[3]),
        bbox[0].max(bbox[2]),
        bbox[1].max(bbox[3]),
    ]
}

fn bbox_area(bbox: [f64; 4]) -> f64 {
    let b = normalize_bbox(bbox);
    ((b[2] - b[0]).max(0.0)) * ((b[3] - b[1]).max(0.0))
}

fn bbox_from_layout(bbox: crate::analysis::layout::BBox) -> [f64; 4] {
    [bbox.x0, bbox.y0, bbox.x1, bbox.y1]
}

/// Parse an annotation `/Rect` `[x0 y0 x1 y1]` (resolving indirect refs and
/// normalizing so `x0<=x1, y0<=y1`). `None` if it is not a 4-number array.
fn rect_from_obj(obj: Option<&crate::object::PdfObject>, reader: &PdfReader) -> Option<[f64; 4]> {
    let resolved = reader.resolve(obj?.clone()).ok()?;
    let arr = resolved.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let mut v = [0.0f64; 4];
    for (i, item) in arr.iter().enumerate() {
        let n = reader.resolve(item.clone()).ok()?;
        v[i] = n.as_number()?;
    }
    Some([
        v[0].min(v[2]),
        v[1].min(v[3]),
        v[0].max(v[2]),
        v[1].max(v[3]),
    ])
}

/// Extract the URI from a `/Link` annotation: either its `/A << /S /URI /URI … >>`
/// action or a direct `/URI`. Returns `None` for GoTo/internal links.
fn link_uri(adict: &crate::object::PdfDictionary, reader: &PdfReader) -> Option<String> {
    use crate::info::decode_pdf_text_string;
    // Direct /URI on the annotation (older style).
    if let Some(crate::object::PdfObject::String(bytes)) = adict.get("URI") {
        return Some(decode_pdf_text_string(bytes));
    }
    // /A action dictionary (may be indirect).
    let action = reader.resolve(adict.get("A")?.clone()).ok()?;
    let act_dict = action.as_dict()?;
    if act_dict.get_name("S") != Some("URI") {
        return None;
    }
    match reader.resolve(act_dict.get("URI")?.clone()).ok()? {
        crate::object::PdfObject::String(bytes) => Some(decode_pdf_text_string(&bytes)),
        _ => None,
    }
}

fn effective_page_box(page: &PdfPage) -> [f64; 4] {
    intersect_boxes(page.media_box, page.crop_box).unwrap_or(page.media_box)
}

fn intersect_boxes(media: [f64; 4], crop: [f64; 4]) -> Option<[f64; 4]> {
    let result = [
        media[0].max(crop[0]),
        media[1].max(crop[1]),
        media[2].min(crop[2]),
        media[3].min(crop[3]),
    ];

    if result[0] >= result[2] || result[1] >= result[3] {
        None
    } else {
        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(media_box: [f64; 4], crop_box: [f64; 4]) -> PdfPage {
        PdfPage {
            page_number: 1,
            object_number: 1,
            generation_number: 0,
            media_box,
            crop_box,
            rotate: 0,
            resources: PdfDictionary::empty(),
            contents: Vec::new(),
            user_unit: 1.0,
        }
    }

    #[test]
    fn intersect_boxes_clips_cropbox_to_mediabox() {
        let media = [0.0, 0.0, 612.0, 792.0];
        let crop = [-10.0, -10.0, 100.0, 100.0];

        assert_eq!(intersect_boxes(media, crop), Some([0.0, 0.0, 100.0, 100.0]));
    }

    #[test]
    fn intersect_boxes_identical_cropbox_returns_mediabox() {
        let media = [0.0, 0.0, 612.0, 792.0];

        assert_eq!(intersect_boxes(media, media), Some(media));
    }

    #[test]
    fn effective_page_box_ignores_invalid_cropbox() {
        let page = page([0.0, 0.0, 200.0, 200.0], [250.0, 250.0, 300.0, 300.0]);

        assert_eq!(effective_page_box(&page), [0.0, 0.0, 200.0, 200.0]);
    }

    #[test]
    fn page_region_keeps_center_or_majority_overlap() {
        let region = PageRegion::new(0.0, 0.0, 100.0, 100.0).unwrap();

        assert!(region.keeps_bbox([25.0, 25.0, 50.0, 50.0]));
        assert!(region.keeps_bbox([80.0, 10.0, 110.0, 50.0]));
        assert!(!region.keeps_bbox([120.0, 10.0, 180.0, 50.0]));
    }

    #[test]
    fn page_region_clamps_partial_overlap_and_rejects_miss() {
        let region = PageRegion::new(-10.0, -10.0, 20.0, 20.0).unwrap();
        assert_eq!(
            region
                .clamp_to_page([0.0, 0.0, 100.0, 100.0])
                .unwrap()
                .as_array(),
            [0.0, 0.0, 20.0, 20.0]
        );

        let miss = PageRegion::new(120.0, 120.0, 140.0, 140.0).unwrap();
        assert!(miss.clamp_to_page([0.0, 0.0, 100.0, 100.0]).is_err());
    }

    #[test]
    fn extraction_profile_parses_aliases() {
        assert_eq!(
            ExtractionProfile::parse("layout-faithful"),
            Some(ExtractionProfile::LayoutFaithful)
        );
        assert_eq!(
            ExtractionProfile::parse("rag"),
            Some(ExtractionProfile::RagChunks)
        );
        assert_eq!(ExtractionProfile::parse("unknown"), None);
    }

    #[test]
    fn structured_column_text_fallback_detects_row_joined_columns() {
        let mut text = String::new();
        for i in 0..30 {
            text.push_str(&format!("left {i}  middle {i}  right {i}\n"));
        }

        assert!(should_prefer_structured_column_text(&text));
    }

    #[test]
    fn structured_column_text_fallback_ignores_ordinary_long_lines() {
        let mut text = String::new();
        for i in 0..30 {
            text.push_str(&format!(
                "section {i} has an intentionally long sentence with enough words to exceed one hundred characters but no column join\n"
            ));
        }

        assert!(!should_prefer_structured_column_text(&text));
    }

    #[test]
    fn contract_renders_into_caller_owned_surface_with_clip_and_format_conversion() {
        use crate::{AuthorPageSize, PdfBuilder, TextStyle};

        let mut builder = PdfBuilder::new();
        builder
            .add_page(AuthorPageSize::LETTER)
            .draw_text("surface", 12.0, 780.0, &TextStyle::default())
            .expect("write surface fixture");
        let engine = ContentEngine::open_bytes(builder.to_bytes().expect("serialize fixture"))
            .expect("open surface fixture");
        let mut contract = engine
            .default_render_contract(1, 72, RenderMode::Compat)
            .expect("default contract");
        contract.clip = Some(crate::render::DeviceClip {
            x: 0,
            y: 0,
            width: 32,
            height: 24,
        });
        contract.width = 32;
        contract.height = 24;
        contract.pixel_format = PixelFormat::Bgra8;
        contract.stride = 32 * 4 + 8;
        let mut surface = vec![0xAA; contract.stride * contract.height as usize];
        engine
            .render_page_into_buffer(&contract, &CancelToken::none(), &mut surface)
            .expect("render into BGRA caller surface");
        assert!(surface
            .chunks_exact(contract.stride)
            .all(|row| row[32 * 4..].iter().all(|v| *v == 0)));
        assert!(engine
            .render_page_with_contract(&contract, &CancelToken::none())
            .is_err());
    }
}
