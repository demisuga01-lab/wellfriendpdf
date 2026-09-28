//! Executable, revision-bound edit postconditions. These are validation reports,
//! not proofs of arbitrary PDF semantics or independent visual equivalence.
use crate::{ContentEngine, PdfObject, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EditContract {
    /// Hash of the exact normalized planning bytes, not an encrypted transport.
    pub input_sha256: String,
    #[serde(default)]
    pub assertions: Vec<EditAssertion>,
    /// Optional bounded inventory of changed indirect objects. This is an
    /// object-number comparison, not a semantic diff after object renumbering.
    #[serde(default)]
    pub inventory_objects: bool,
    /// Optional fail-closed policy for the complete indirect-object delta.
    /// Existing objects may change or disappear only when explicitly listed;
    /// newly allocated objects require both permission and a numeric bound.
    #[serde(default)]
    pub object_change_policy: Option<ObjectChangePolicy>,
    /// Optional native-render preservation check. Pixels outside the declared
    /// top-left-origin device-space rectangles must remain byte-identical when
    /// input and output are rendered with the same exact contract.
    #[serde(default)]
    pub raster_preservation: Option<RasterPreservationPolicy>,
    /// Caller-supplied page-text oracles from a named external extractor. The
    /// SDK validates both the input baseline and expected output against its
    /// reopened extraction; it cannot attest how the caller produced them.
    #[serde(default)]
    pub text_oracles: Vec<ExternalTextOracle>,
    /// Caller-supplied independent output raster oracles. Every requested page
    /// must have an oracle and every comparison must pass before publication.
    #[serde(default)]
    pub render_oracle: Option<crate::universal_editing::UniversalRenderQualificationOptionsV2>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ObjectChangePolicy {
    /// Existing `(object, generation)` identities allowed to change or be
    /// removed. This exact-revision policy deliberately does not guess across
    /// object renumbering.
    #[serde(default)]
    pub allowed_existing_objects: Vec<[u32; 2]>,
    #[serde(default)]
    pub allow_new_objects: bool,
    /// Required upper bound when `allow_new_objects` is true.
    #[serde(default)]
    pub max_new_objects: usize,
    #[serde(default)]
    pub allow_removed_objects: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPreservationPolicy {
    #[serde(default = "default_raster_preservation_dpi")]
    pub dpi: u32,
    #[serde(default)]
    pub pages: Vec<RasterPagePreservation>,
    /// Counts both the input and output render. Zero selects the conservative
    /// default; callers may lower but not raise that ceiling.
    #[serde(default)]
    pub max_total_pixels: u64,
}

impl Default for RasterPreservationPolicy {
    fn default() -> Self {
        Self {
            dpi: default_raster_preservation_dpi(),
            pages: Vec::new(),
            max_total_pixels: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RasterPagePreservation {
    pub input_page: usize,
    pub output_page: usize,
    /// `[x, y, width, height]` in exact rendered RGBA pixel coordinates, with
    /// `(0, 0)` at the top-left. Overlap is allowed and normalized.
    #[serde(default)]
    pub allowed_changed_regions: Vec<[u32; 4]>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalTextOracle {
    pub id: String,
    pub extractor: String,
    pub extractor_version: String,
    pub input_page: usize,
    pub output_page: usize,
    pub input_text_sha256: String,
    pub output_text_sha256: String,
}

fn default_raster_preservation_dpi() -> u32 {
    144
}

fn default_raster_preservation_pixel_limit() -> u64 {
    200_000_000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditAssertion {
    TextCount {
        id: String,
        input_page: usize,
        output_page: usize,
        text: String,
        before: usize,
        after: usize,
    },
    /// Hash of the whole extracted Unicode page text; pagination must supply
    /// the new page number explicitly. No case/whitespace normalization occurs.
    PreservePageText {
        id: String,
        input_page: usize,
        output_page: usize,
    },
    /// Exact PDF value including literal reference numbers. Referenced objects
    /// are NOT covered implicitly; list those dependencies as separate checks.
    PreserveObject {
        id: String,
        input_object: [u32; 2],
        output_object: [u32; 2],
        expected_sha256: String,
    },
    /// Canonical rooted source graph, including stream bytes and all reachable
    /// indirect objects. Reference identities are replaced by deterministic
    /// first-encounter indices so equivalent renumbering does not fail.
    PreserveObjectGraph {
        id: String,
        input_object: [u32; 2],
        output_object: [u32; 2],
        expected_sha256: String,
        #[serde(default = "default_graph_object_limit")]
        max_objects: usize,
        #[serde(default = "default_graph_byte_limit")]
        max_bytes: usize,
    },
    /// Canonical rooted graph whose fully losslessly decoded stream payloads
    /// are compared independently of `/Length`, `/Filter`, `/DecodeParms` and
    /// `/DL`. All other direct values and reachable topology remain exact.
    PreserveDecodedObjectGraph {
        id: String,
        input_object: [u32; 2],
        output_object: [u32; 2],
        expected_sha256: String,
        #[serde(default = "default_graph_object_limit")]
        max_objects: usize,
        #[serde(default = "default_graph_byte_limit")]
        max_bytes: usize,
    },
    /// Preserve unrecognized content as opaque decoded bytes. Only this slice
    /// is asserted, not the graphics state or visual meaning around it.
    PreserveStreamSlice {
        id: String,
        input_object: [u32; 2],
        output_object: [u32; 2],
        input_range: [usize; 2],
        output_range: [usize; 2],
        expected_sha256: String,
    },
    PageCount {
        id: String,
        before: usize,
        after: usize,
    },
}
impl EditAssertion {
    fn id(&self) -> &str {
        match self {
            Self::TextCount { id, .. }
            | Self::PreservePageText { id, .. }
            | Self::PreserveObject { id, .. }
            | Self::PreserveObjectGraph { id, .. }
            | Self::PreserveDecodedObjectGraph { id, .. }
            | Self::PreserveStreamSlice { id, .. }
            | Self::PageCount { id, .. } => id,
        }
    }
}

fn default_graph_object_limit() -> usize {
    4096
}

fn default_graph_byte_limit() -> usize {
    64 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize)]
pub struct AssertionResult {
    pub id: String,
    pub passed: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ObjectChange {
    pub number: u32,
    pub generation: u16,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct EditContractReport {
    pub input_sha256: String,
    pub output_sha256: String,
    pub assertions: Vec<AssertionResult>,
    pub object_changes: Option<Vec<ObjectChange>>,
    pub raster_preservation: Option<Value>,
    pub text_oracle_validation: Option<Value>,
    pub render_validation: Option<serde_json::Value>,
    pub validation_scope: String,
}

fn validate_object_change_policy(policy: &ObjectChangePolicy) -> Result<BTreeSet<(u32, u16)>> {
    if policy.allowed_existing_objects.len() > 4096 {
        return Err(fail("edit contract object allowlist exceeds 4096 entries"));
    }
    if policy.max_new_objects > 100_000 {
        return Err(fail("edit contract new-object budget exceeds 100000"));
    }
    if policy.allow_new_objects != (policy.max_new_objects > 0) {
        return Err(fail(
            "edit contract new-object permission requires a positive bounded maximum",
        ));
    }
    let mut allowed = BTreeSet::new();
    for [number, generation] in &policy.allowed_existing_objects {
        let generation =
            u16::try_from(*generation).map_err(|_| fail("contract generation exceeds u16"))?;
        if !allowed.insert((*number, generation)) {
            return Err(fail("edit contract object allowlist contains duplicates"));
        }
    }
    Ok(allowed)
}

fn validate_raster_preservation_policy(policy: &RasterPreservationPolicy) -> Result<u64> {
    if !(24..=2400).contains(&policy.dpi) {
        return Err(fail("edit contract raster dpi must be in 24..=2400"));
    }
    if policy.pages.is_empty() || policy.pages.len() > 256 {
        return Err(fail(
            "edit contract raster preservation requires 1..=256 page mappings",
        ));
    }
    let pixel_limit = if policy.max_total_pixels == 0 {
        default_raster_preservation_pixel_limit()
    } else {
        policy.max_total_pixels
    };
    if pixel_limit > default_raster_preservation_pixel_limit() {
        return Err(fail(
            "edit contract raster preservation exceeds its 200000000-pixel ceiling",
        ));
    }
    let mut input_pages = BTreeSet::new();
    let mut output_pages = BTreeSet::new();
    let mut region_count = 0usize;
    for page in &policy.pages {
        if page.input_page == 0
            || page.output_page == 0
            || !input_pages.insert(page.input_page)
            || !output_pages.insert(page.output_page)
        {
            return Err(fail(
                "edit contract raster page mappings must be nonzero and one-to-one",
            ));
        }
        region_count = region_count
            .checked_add(page.allowed_changed_regions.len())
            .ok_or_else(|| fail("edit contract raster region count overflow"))?;
        if region_count > 4096 {
            return Err(fail("edit contract raster region budget exceeds 4096"));
        }
        for [x, y, width, height] in &page.allowed_changed_regions {
            if *width == 0
                || *height == 0
                || x.checked_add(*width).is_none()
                || y.checked_add(*height).is_none()
            {
                return Err(fail("edit contract raster region is empty or overflows"));
            }
        }
    }
    Ok(pixel_limit)
}

fn validate_text_oracles(oracles: &[ExternalTextOracle]) -> Result<()> {
    if oracles.len() > 256 {
        return Err(fail("edit contract text-oracle budget exceeds 256"));
    }
    let mut ids = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for oracle in oracles {
        if oracle.id.is_empty()
            || oracle.id.len() > 256
            || !ids.insert(oracle.id.as_str())
            || oracle.extractor.is_empty()
            || oracle.extractor.len() > 256
            || oracle.extractor_version.is_empty()
            || oracle.extractor_version.len() > 128
            || oracle.input_page == 0
            || oracle.output_page == 0
            || !valid_sha256(&oracle.input_text_sha256)
            || !valid_sha256(&oracle.output_text_sha256)
            || !identities.insert((
                oracle.extractor.as_str(),
                oracle.extractor_version.as_str(),
                oracle.input_page,
                oracle.output_page,
            ))
        {
            return Err(fail(
                "edit contract text oracles require unique bounded identities, pages and SHA-256 values",
            ));
        }
    }
    Ok(())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}
fn object(engine: &ContentEngine, reference: [u32; 2]) -> Result<PdfObject> {
    let generation =
        u16::try_from(reference[1]).map_err(|_| fail("contract generation exceeds u16"))?;
    engine
        .document()
        .reader()
        .get_object(reference[0], generation)
}
fn object_digest(value: &PdfObject) -> Result<String> {
    if let PdfObject::Stream { raw, .. } = value {
        if raw.len() > 64 * 1024 * 1024 {
            return Err(fail("contract object exceeds 64 MiB"));
        }
    }
    let mut bytes = Vec::new();
    crate::writer::serialize_object(value, &mut bytes);
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(fail("contract serialized object exceeds 64 MiB"));
    }
    Ok(digest(&bytes))
}

struct CanonicalGraphEncoder<'a> {
    engine: &'a ContentEngine,
    identities: BTreeMap<(u32, u16), u32>,
    hasher: Sha256,
    byte_count: usize,
    max_objects: usize,
    max_bytes: usize,
    stream_mode: CanonicalGraphStreamMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CanonicalGraphStreamMode {
    ExactRaw,
    DecodedEquivalent,
}

impl<'a> CanonicalGraphEncoder<'a> {
    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        let next_count = self
            .byte_count
            .checked_add(bytes.len())
            .ok_or_else(|| fail("edit contract canonical object byte count overflow"))?;
        if next_count > self.max_bytes {
            return Err(fail(
                "edit contract canonical object graph exceeds its byte budget",
            ));
        }
        self.hasher.update(bytes);
        self.byte_count = next_count;
        Ok(())
    }

    fn append_len(&mut self, length: usize) -> Result<()> {
        let length = u64::try_from(length)
            .map_err(|_| fail("edit contract canonical object length overflow"))?;
        self.append(&length.to_be_bytes())
    }

    fn encode_object(&mut self, object: &PdfObject, depth: usize) -> Result<()> {
        if depth > 256 {
            return Err(fail(
                "edit contract canonical object graph exceeds depth 256",
            ));
        }
        match object {
            PdfObject::Null => self.append(b"N"),
            PdfObject::Boolean(value) => self.append(if *value { b"B1" } else { b"B0" }),
            PdfObject::Integer(value) => {
                self.append(b"I")?;
                self.append(&value.to_be_bytes())
            }
            PdfObject::Real(value) => {
                self.append(b"F")?;
                self.append(&value.to_bits().to_be_bytes())
            }
            PdfObject::String(value) => {
                self.append(b"S")?;
                self.append_len(value.len())?;
                self.append(value)
            }
            PdfObject::Name(value) => {
                self.append(b"M")?;
                self.append_len(value.len())?;
                self.append(value.as_bytes())
            }
            PdfObject::Array(items) => {
                self.append(b"A")?;
                self.append_len(items.len())?;
                for item in items {
                    self.encode_object(item, depth + 1)?;
                }
                Ok(())
            }
            PdfObject::Dictionary(dictionary) => {
                self.append(b"D")?;
                self.append_len(dictionary.len())?;
                for (key, value) in dictionary.iter() {
                    self.append_len(key.len())?;
                    self.append(key.as_bytes())?;
                    self.encode_object(value, depth + 1)?;
                }
                Ok(())
            }
            PdfObject::Stream { dict, raw } => {
                if self.stream_mode == CanonicalGraphStreamMode::ExactRaw {
                    self.append(b"T")?;
                    self.append_len(dict.len())?;
                    for (key, value) in dict.iter() {
                        self.append_len(key.len())?;
                        self.append(key.as_bytes())?;
                        self.encode_object(value, depth + 1)?;
                    }
                    self.append_len(raw.len())?;
                    return self.append(raw);
                }
                if ["F", "FFilter", "FDecodeParms"]
                    .iter()
                    .any(|key| dict.contains_key(key))
                {
                    return Err(fail(
                        "edit contract decoded object graph does not resolve external-file streams",
                    ));
                }
                self.append(b"U")?;
                let semantic_entries = dict
                    .iter()
                    .filter(|(key, _)| {
                        !matches!(key.as_str(), "Length" | "Filter" | "DecodeParms" | "DL")
                    })
                    .collect::<Vec<_>>();
                self.append_len(semantic_entries.len())?;
                for (key, value) in semantic_entries {
                    self.append_len(key.len())?;
                    self.append(key.as_bytes())?;
                    self.encode_object(value, depth + 1)?;
                }
                crate::cancel::check_current_cancel(
                    "edit contract canonical decoded object graph",
                )?;
                let remaining = self.max_bytes.checked_sub(self.byte_count).ok_or_else(|| {
                    fail("edit contract canonical decoded graph exhausted its byte budget")
                })?;
                if remaining == 0 {
                    return Err(fail(
                        "edit contract canonical decoded graph exhausted its byte budget",
                    ));
                }
                let remaining = u64::try_from(remaining)
                    .map_err(|_| fail("edit contract decoded stream byte limit overflow"))?;
                let decoded = crate::filters::decode_stream_lossless_with_limits(
                    object,
                    self.engine.document().reader(),
                    &crate::filters::DecodeLimits {
                        max_decoded_bytes_per_stream: remaining,
                        max_decoded_bytes_per_document: remaining,
                        max_image_decoded_bytes: remaining,
                        scheduler_memory_budget_bytes: remaining,
                        ..Default::default()
                    },
                )?;
                if decoded.status != crate::filters::StreamDecodeStatus::Complete {
                    return Err(fail(
                        "edit contract decoded object graph requires every stream to decode completely",
                    ));
                }
                self.append_len(decoded.data.len())?;
                self.append(&decoded.data)
            }
            PdfObject::Reference { number, generation } => {
                self.append(b"R")?;
                let identity = (*number, *generation);
                if let Some(index) = self.identities.get(&identity).copied() {
                    self.append(b"-")?;
                    return self.append(&index.to_be_bytes());
                }
                if self.identities.len() >= self.max_objects {
                    return Err(fail(
                        "edit contract canonical object graph exceeds its object budget",
                    ));
                }
                let index = u32::try_from(self.identities.len())
                    .map_err(|_| fail("edit contract canonical object index overflow"))?;
                self.identities.insert(identity, index);
                self.append(b"+")?;
                self.append(&index.to_be_bytes())?;
                crate::cancel::check_current_cancel("edit contract canonical object graph")?;
                let target = self
                    .engine
                    .document()
                    .reader()
                    .get_object(*number, *generation)?;
                self.encode_object(&target, depth + 1)
            }
        }
    }
}

fn canonical_object_graph_digest_with_engine(
    engine: &ContentEngine,
    root: [u32; 2],
    max_objects: usize,
    max_bytes: usize,
) -> Result<String> {
    canonical_object_graph_digest_with_engine_mode(
        engine,
        root,
        max_objects,
        max_bytes,
        CanonicalGraphStreamMode::ExactRaw,
    )
}

fn canonical_decoded_object_graph_digest_with_engine(
    engine: &ContentEngine,
    root: [u32; 2],
    max_objects: usize,
    max_bytes: usize,
) -> Result<String> {
    canonical_object_graph_digest_with_engine_mode(
        engine,
        root,
        max_objects,
        max_bytes,
        CanonicalGraphStreamMode::DecodedEquivalent,
    )
}

fn canonical_object_graph_digest_with_engine_mode(
    engine: &ContentEngine,
    root: [u32; 2],
    max_objects: usize,
    max_bytes: usize,
    stream_mode: CanonicalGraphStreamMode,
) -> Result<String> {
    if max_objects == 0 || max_objects > 100_000 {
        return Err(fail(
            "edit contract canonical object graph limit must be in 1..=100000",
        ));
    }
    if max_bytes == 0 || max_bytes > 512 * 1024 * 1024 {
        return Err(fail(
            "edit contract canonical object byte limit must be in 1..=536870912",
        ));
    }
    let generation = u16::try_from(root[1]).map_err(|_| fail("contract generation exceeds u16"))?;
    let identity = (root[0], generation);
    let value = engine
        .document()
        .reader()
        .get_object(identity.0, identity.1)?;
    let mut encoder = CanonicalGraphEncoder {
        engine,
        identities: BTreeMap::from([(identity, 0)]),
        hasher: Sha256::new(),
        byte_count: 0,
        max_objects,
        max_bytes,
        stream_mode,
    };
    encoder.append(b"ROOT")?;
    encoder.encode_object(&value, 0)?;
    Ok(format!("{:x}", encoder.hasher.finalize()))
}

/// Hash a bounded rooted PDF object graph without embedding indirect object
/// numbers. Equivalent graphs remain comparable after canonical renumbering;
/// stream bytes and all direct values remain exact.
pub fn canonical_object_graph_digest(
    input: &[u8],
    root: [u32; 2],
    max_objects: usize,
    max_bytes: usize,
) -> Result<String> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    canonical_object_graph_digest_with_engine(&engine, root, max_objects, max_bytes)
}

/// Hash a bounded rooted graph after fully lossless stream decoding. Encoding
/// controls and encoded byte differences are ignored, while all remaining
/// dictionary values, decoded bytes, reference topology and scalar values stay
/// exact. Unsupported or incomplete decoding fails closed.
pub fn canonical_decoded_object_graph_digest(
    input: &[u8],
    root: [u32; 2],
    max_objects: usize,
    max_bytes: usize,
) -> Result<String> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    canonical_decoded_object_graph_digest_with_engine(&engine, root, max_objects, max_bytes)
}
fn slice_digest(engine: &ContentEngine, reference: [u32; 2], range: [usize; 2]) -> Result<String> {
    let value = object(engine, reference)?;
    let decoded = crate::filters::decode_stream_lossless_with_limits(
        &value,
        engine.document().reader(),
        &crate::filters::DecodeLimits {
            max_decoded_bytes_per_stream: 64 * 1024 * 1024,
            ..Default::default()
        },
    )?;
    if decoded.status != crate::filters::StreamDecodeStatus::Complete {
        return Err(fail("contract requires fully decoded stream"));
    }
    let bytes = decoded
        .data
        .get(range[0]..range[1])
        .ok_or_else(|| fail("contract stream slice is outside decoded bytes"))?;
    Ok(digest(bytes))
}
fn page_text(
    engine: &ContentEngine,
    cache: &mut BTreeMap<usize, String>,
    page: usize,
) -> Result<String> {
    if let Some(text) = cache.get(&page) {
        return Ok(text.clone());
    }
    let text = engine.get_page_text(page)?;
    if text.len() > 4_000_000
        || cache.values().map(String::len).sum::<usize>() + text.len() > 32_000_000
    {
        return Err(fail("contract extraction budget exceeded"));
    }
    cache.insert(page, text.clone());
    Ok(text)
}

pub fn validate_contract_input(input: &[u8], contract: &EditContract) -> Result<()> {
    if contract.input_sha256 != digest(input) {
        return Err(fail("edit contract belongs to another input revision"));
    }
    if contract.assertions.len() > 4096 {
        return Err(fail("edit contract assertion budget exceeded"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if let Some(policy) = &contract.object_change_policy {
        for (number, generation) in validate_object_change_policy(policy)? {
            engine
                .document()
                .reader()
                .get_object(number, generation)
                .map_err(|_| fail("edit contract allowed object is absent from input revision"))?;
        }
    }
    if let Some(policy) = &contract.raster_preservation {
        validate_raster_preservation_policy(policy)?;
        let page_count = engine.page_count()?;
        if policy.pages.iter().any(|page| page.input_page > page_count) {
            return Err(fail(
                "edit contract raster input page is outside the input revision",
            ));
        }
    }
    let mut ids = BTreeSet::new();
    let mut cache = BTreeMap::new();
    validate_text_oracles(&contract.text_oracles)?;
    let input_page_count = engine.page_count()?;
    for oracle in &contract.text_oracles {
        crate::cancel::check_current_cancel("edit contract input text oracle")?;
        if oracle.input_page > input_page_count
            || !digest(page_text(&engine, &mut cache, oracle.input_page)?.as_bytes())
                .eq_ignore_ascii_case(&oracle.input_text_sha256)
        {
            return Err(fail(&format!(
                "edit contract input text oracle failed: {}",
                oracle.id
            )));
        }
    }
    for assertion in &contract.assertions {
        crate::cancel::check_current_cancel("edit contract preconditions")?;
        if assertion.id().is_empty() || !ids.insert(assertion.id()) {
            return Err(fail("edit contract IDs must be nonempty and unique"));
        }
        let passed = match assertion {
            EditAssertion::TextCount {
                input_page,
                text,
                before,
                ..
            } => {
                if text.is_empty() {
                    return Err(fail("edit contract cannot count empty text"));
                }
                page_text(&engine, &mut cache, *input_page)?
                    .matches(text.as_str())
                    .count()
                    == *before
            }
            EditAssertion::PreservePageText { input_page, .. } => {
                page_text(&engine, &mut cache, *input_page)?;
                true
            }
            EditAssertion::PreserveObject {
                input_object,
                expected_sha256,
                ..
            } => {
                if !valid_sha256(expected_sha256) {
                    return Err(fail("edit contract expected object digest is malformed"));
                }
                object_digest(&object(&engine, *input_object)?)?
                    .eq_ignore_ascii_case(expected_sha256)
            }
            EditAssertion::PreserveObjectGraph {
                input_object,
                expected_sha256,
                max_objects,
                max_bytes,
                ..
            } => {
                if !valid_sha256(expected_sha256) {
                    return Err(fail(
                        "edit contract expected object-graph digest is malformed",
                    ));
                }
                canonical_object_graph_digest_with_engine(
                    &engine,
                    *input_object,
                    *max_objects,
                    *max_bytes,
                )?
                .eq_ignore_ascii_case(expected_sha256)
            }
            EditAssertion::PreserveDecodedObjectGraph {
                input_object,
                expected_sha256,
                max_objects,
                max_bytes,
                ..
            } => {
                if !valid_sha256(expected_sha256) {
                    return Err(fail(
                        "edit contract expected decoded object-graph digest is malformed",
                    ));
                }
                canonical_decoded_object_graph_digest_with_engine(
                    &engine,
                    *input_object,
                    *max_objects,
                    *max_bytes,
                )?
                .eq_ignore_ascii_case(expected_sha256)
            }
            EditAssertion::PreserveStreamSlice {
                input_object,
                input_range,
                expected_sha256,
                ..
            } => slice_digest(&engine, *input_object, *input_range)? == *expected_sha256,
            EditAssertion::PageCount { before, .. } => engine.page_count()? == *before,
        };
        if !passed {
            return Err(fail(&format!(
                "edit contract input precondition failed: {}",
                assertion.id()
            )));
        }
    }
    if let Some(oracle) = &contract.render_oracle {
        let pages: BTreeSet<_> = oracle.pages.iter().copied().collect();
        let references: BTreeSet<_> = oracle.reference_rasters.iter().map(|r| r.page).collect();
        if pages.is_empty()
            || pages != references
            || pages.len() != oracle.pages.len()
            || !oracle.require_exact
        {
            return Err(fail("edit contract render oracle requires explicit distinct pages, matching references and exact mode"));
        }
    }
    Ok(())
}

fn inventory(engine: &ContentEngine) -> Result<BTreeMap<(u32, u16), String>> {
    let reader = engine.document().reader();
    let mut result = BTreeMap::new();
    let mut budget = 0usize;
    for (number, generation) in reader.object_ids() {
        crate::cancel::check_current_cancel("edit contract object inventory")?;
        if result.len() >= 100_000 {
            return Err(fail("edit contract inventory exceeds 100000 objects"));
        }
        let value = reader.get_object(number, generation)?;
        if let PdfObject::Stream { raw, .. } = &value {
            budget = budget.saturating_add(raw.len());
        }
        if budget > 512 * 1024 * 1024 {
            return Err(fail(
                "edit contract inventory exceeds 512 MiB stream budget",
            ));
        }
        result.insert((number, generation), object_digest(&value)?);
    }
    Ok(result)
}

fn merged_active_intervals(active: &BTreeSet<(u32, u32, usize)>) -> Vec<(u32, u32)> {
    let mut merged = Vec::<(u32, u32)>::new();
    for &(start, end, _) in active {
        if let Some(last) = merged.last_mut() {
            if start <= last.1 {
                last.1 = last.1.max(end);
                continue;
            }
        }
        merged.push((start, end));
    }
    merged
}

fn verify_raster_preservation(
    before: &ContentEngine,
    after: &ContentEngine,
    policy: &RasterPreservationPolicy,
) -> Result<Value> {
    let pixel_limit = validate_raster_preservation_policy(policy)?;
    let output_page_count = after.page_count()?;
    if policy
        .pages
        .iter()
        .any(|page| page.output_page > output_page_count)
    {
        return Err(fail(
            "edit contract raster output page is outside the output revision",
        ));
    }
    let cancel = crate::cancel::current_cancel_token();
    let mode = crate::render::RenderMode::HighQuality;
    let mut total_rendered_pixels = 0u64;
    let mut reports = Vec::with_capacity(policy.pages.len());
    for mapping in &policy.pages {
        cancel.check("edit contract raster preservation page")?;
        let before_contract =
            before.default_render_contract(mapping.input_page, policy.dpi, mode)?;
        let after_contract =
            after.default_render_contract(mapping.output_page, policy.dpi, mode)?;
        if before_contract.width != after_contract.width
            || before_contract.height != after_contract.height
        {
            return Err(fail(
                "edit contract raster preservation page dimensions changed",
            ));
        }
        let mut comparable_after_contract = after_contract.clone();
        comparable_after_contract.document_revision = before_contract.document_revision;
        comparable_after_contract.page_identity = before_contract.page_identity;
        comparable_after_contract.page_number = before_contract.page_number;
        if comparable_after_contract != before_contract {
            return Err(fail(
                "edit contract raster preservation render semantics changed between revisions",
            ));
        }
        let width = before_contract.width;
        let height = before_contract.height;
        let pair_pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(2))
            .ok_or_else(|| fail("edit contract raster pixel count overflow"))?;
        total_rendered_pixels = total_rendered_pixels
            .checked_add(pair_pixels)
            .ok_or_else(|| fail("edit contract raster total pixel count overflow"))?;
        if total_rendered_pixels > pixel_limit {
            return Err(fail(
                "edit contract raster preservation exceeded its requested pixel budget",
            ));
        }
        let mut events = BTreeMap::<u32, Vec<(bool, (u32, u32, usize))>>::new();
        for (index, [x, y, region_width, region_height]) in
            mapping.allowed_changed_regions.iter().copied().enumerate()
        {
            let x1 = x
                .checked_add(region_width)
                .ok_or_else(|| fail("edit contract raster region x overflow"))?;
            let y1 = y
                .checked_add(region_height)
                .ok_or_else(|| fail("edit contract raster region y overflow"))?;
            if x1 > width || y1 > height {
                return Err(fail(
                    "edit contract raster region exceeds rendered page bounds",
                ));
            }
            let interval = (x, x1, index);
            events.entry(y).or_default().push((true, interval));
            events.entry(y1).or_default().push((false, interval));
        }
        let before_list = before.build_page_display_list(mapping.input_page, policy.dpi)?;
        let after_list = after.build_page_display_list(mapping.output_page, policy.dpi)?;
        if !before_list.is_fully_supported() || !after_list.is_fully_supported() {
            return Err(fail(
                "edit contract raster preservation requires fully supported native display lists",
            ));
        }
        let before_pixels = before.render_page_cancellable_with_mode(
            mapping.input_page,
            policy.dpi,
            &cancel,
            mode,
        )?;
        let after_pixels = after.render_page_cancellable_with_mode(
            mapping.output_page,
            policy.dpi,
            &cancel,
            mode,
        )?;
        if before_pixels.width != width
            || before_pixels.height != height
            || after_pixels.width != width
            || after_pixels.height != height
        {
            return Err(fail(
                "edit contract raster output dimensions disagree with the planned contract",
            ));
        }

        let before_rgba = before_pixels.rgba_bytes();
        let after_rgba = after_pixels.rgba_bytes();
        let expected_bytes = u64::from(width)
            .checked_mul(u64::from(height))
            .and_then(|pixels| pixels.checked_mul(4))
            .and_then(|bytes| usize::try_from(bytes).ok())
            .ok_or_else(|| fail("edit contract raster byte length overflow"))?;
        if before_rgba.len() != expected_bytes || after_rgba.len() != expected_bytes {
            return Err(fail(
                "edit contract raster buffer length disagrees with its dimensions",
            ));
        }
        let mut before_outside_hash = Sha256::new();
        let mut after_outside_hash = Sha256::new();
        let mut active = BTreeSet::<(u32, u32, usize)>::new();
        let mut intervals = Vec::new();
        let mut allowed_pixels = 0u64;
        let mut changed_outside_pixels = 0u64;
        for y in 0..height {
            if y % 256 == 0 {
                cancel.check("edit contract raster preservation scan")?;
            }
            if let Some(changes) = events.get(&y) {
                for (add, interval) in changes.iter().filter(|(add, _)| !*add) {
                    debug_assert!(!*add);
                    active.remove(interval);
                }
                for (add, interval) in changes.iter().filter(|(add, _)| *add) {
                    debug_assert!(*add);
                    active.insert(*interval);
                }
                intervals = merged_active_intervals(&active);
            }
            allowed_pixels = allowed_pixels
                .checked_add(
                    intervals
                        .iter()
                        .map(|(start, end)| u64::from(end - start))
                        .sum::<u64>(),
                )
                .ok_or_else(|| fail("edit contract allowed pixel count overflow"))?;
            let mut cursor = 0u32;
            for (start, end) in intervals
                .iter()
                .copied()
                .chain(std::iter::once((width, width)))
            {
                if cursor < start {
                    let row_start = u64::from(y)
                        .checked_mul(u64::from(width))
                        .and_then(|offset| offset.checked_add(u64::from(cursor)))
                        .and_then(|offset| offset.checked_mul(4))
                        .and_then(|offset| usize::try_from(offset).ok())
                        .ok_or_else(|| fail("edit contract raster byte offset overflow"))?;
                    let row_end = u64::from(y)
                        .checked_mul(u64::from(width))
                        .and_then(|offset| offset.checked_add(u64::from(start)))
                        .and_then(|offset| offset.checked_mul(4))
                        .and_then(|offset| usize::try_from(offset).ok())
                        .ok_or_else(|| fail("edit contract raster byte offset overflow"))?;
                    let before_slice = &before_rgba[row_start..row_end];
                    let after_slice = &after_rgba[row_start..row_end];
                    before_outside_hash.update(before_slice);
                    after_outside_hash.update(after_slice);
                    if before_slice != after_slice {
                        let changed = u64::try_from(
                            before_slice
                                .chunks_exact(4)
                                .zip(after_slice.chunks_exact(4))
                                .filter(|(left, right)| left != right)
                                .count(),
                        )
                        .map_err(|_| fail("edit contract changed pixel count overflow"))?;
                        changed_outside_pixels = changed_outside_pixels
                            .checked_add(changed)
                            .ok_or_else(|| fail("edit contract changed pixel count overflow"))?;
                    }
                }
                cursor = cursor.max(end);
            }
        }
        if changed_outside_pixels != 0 {
            return Err(fail(&format!(
                "edit contract changed {changed_outside_pixels} pixels outside declared regions on output page {}; output withheld",
                mapping.output_page
            )));
        }
        let total_page_pixels = u64::from(width) * u64::from(height);
        reports.push(json!({
            "input_page": mapping.input_page,
            "output_page": mapping.output_page,
            "width": width,
            "height": height,
            "allowed_region_pixels": allowed_pixels,
            "preserved_outside_pixels": total_page_pixels.saturating_sub(allowed_pixels),
            "changed_outside_pixels": changed_outside_pixels,
            "input_outside_sha256": format!("{:x}", before_outside_hash.finalize()),
            "output_outside_sha256": format!("{:x}", after_outside_hash.finalize()),
        }));
    }
    Ok(json!({
        "status": "native_pixels_outside_declared_regions_preserved",
        "dpi": policy.dpi,
        "total_rendered_pixels": total_rendered_pixels,
        "pages": reports,
        "scope": "same-engine exact RGBA comparison; not an independent-renderer proof",
    }))
}

/// Reopen the actual output, enforce every postcondition and return evidence.
/// No caller should publish output bytes until this function succeeds.
pub fn verify_edit_contract(
    input: &[u8],
    output: &[u8],
    contract: &EditContract,
) -> Result<EditContractReport> {
    validate_contract_input(input, contract)?;
    let before = ContentEngine::open_bytes(input.to_vec())?;
    let after = ContentEngine::open_bytes(output.to_vec())?;
    let mut before_text = BTreeMap::new();
    let mut after_text = BTreeMap::new();
    let mut results = Vec::new();
    for assertion in &contract.assertions {
        crate::cancel::check_current_cancel("edit contract postconditions")?;
        let passed = match assertion {
            EditAssertion::TextCount {
                output_page,
                text,
                after: count,
                ..
            } => {
                page_text(&after, &mut after_text, *output_page)?
                    .matches(text.as_str())
                    .count()
                    == *count
            }
            EditAssertion::PreservePageText {
                input_page,
                output_page,
                ..
            } => {
                page_text(&before, &mut before_text, *input_page)?
                    == page_text(&after, &mut after_text, *output_page)?
            }
            EditAssertion::PreserveObject {
                output_object,
                expected_sha256,
                ..
            } => object_digest(&object(&after, *output_object)?)?
                .eq_ignore_ascii_case(expected_sha256),
            EditAssertion::PreserveObjectGraph {
                output_object,
                expected_sha256,
                max_objects,
                max_bytes,
                ..
            } => canonical_object_graph_digest_with_engine(
                &after,
                *output_object,
                *max_objects,
                *max_bytes,
            )?
            .eq_ignore_ascii_case(expected_sha256),
            EditAssertion::PreserveDecodedObjectGraph {
                output_object,
                expected_sha256,
                max_objects,
                max_bytes,
                ..
            } => canonical_decoded_object_graph_digest_with_engine(
                &after,
                *output_object,
                *max_objects,
                *max_bytes,
            )?
            .eq_ignore_ascii_case(expected_sha256),
            EditAssertion::PreserveStreamSlice {
                output_object,
                output_range,
                expected_sha256,
                ..
            } => slice_digest(&after, *output_object, *output_range)? == *expected_sha256,
            EditAssertion::PageCount { after: count, .. } => after.page_count()? == *count,
        };
        if !passed {
            return Err(fail(&format!(
                "edit contract postcondition failed; output withheld: {}",
                assertion.id()
            )));
        }
        results.push(AssertionResult {
            id: assertion.id().into(),
            passed,
        });
    }
    let text_oracle_validation = if contract.text_oracles.is_empty() {
        None
    } else {
        let output_page_count = after.page_count()?;
        let mut reports = Vec::with_capacity(contract.text_oracles.len());
        for oracle in &contract.text_oracles {
            crate::cancel::check_current_cancel("edit contract output text oracle")?;
            if oracle.output_page > output_page_count {
                return Err(fail(&format!(
                    "edit contract output text oracle page disappeared: {}",
                    oracle.id
                )));
            }
            let observed =
                digest(page_text(&after, &mut after_text, oracle.output_page)?.as_bytes());
            if !observed.eq_ignore_ascii_case(&oracle.output_text_sha256) {
                return Err(fail(&format!(
                    "edit contract output text oracle failed; output withheld: {}",
                    oracle.id
                )));
            }
            reports.push(json!({
                "id": oracle.id,
                "extractor": oracle.extractor,
                "extractor_version": oracle.extractor_version,
                "input_page": oracle.input_page,
                "output_page": oracle.output_page,
                "input_text_sha256": oracle.input_text_sha256.to_ascii_lowercase(),
                "output_text_sha256": observed,
                "passed": true,
            }));
        }
        Some(json!({
            "status": "caller_supplied_text_oracles_matched",
            "oracles": reports,
            "provenance": "extractor identity/version and expected hashes are caller assertions; the SDK does not attest external execution",
        }))
    };
    let need_inventory = contract.inventory_objects || contract.object_change_policy.is_some();
    let all_object_changes: Option<Vec<ObjectChange>> = if need_inventory {
        let a = inventory(&before)?;
        let b = inventory(&after)?;
        let keys: BTreeSet<_> = a.keys().chain(b.keys()).copied().collect();
        Some(
            keys.into_iter()
                .filter_map(|(number, generation)| {
                    let key = (number, generation);
                    (a.get(&key) != b.get(&key)).then(|| ObjectChange {
                        number,
                        generation,
                        before_sha256: a.get(&key).cloned(),
                        after_sha256: b.get(&key).cloned(),
                    })
                })
                .collect(),
        )
    } else {
        None
    };
    if let Some(policy) = &contract.object_change_policy {
        let allowed = validate_object_change_policy(policy)?;
        let changes = all_object_changes
            .as_ref()
            .ok_or_else(|| fail("edit contract object delta was not collected"))?;
        let mut new_objects = 0usize;
        for change in changes {
            let key = (change.number, change.generation);
            match (&change.before_sha256, &change.after_sha256) {
                (Some(_), Some(_)) if !allowed.contains(&key) => {
                    return Err(fail(&format!(
                        "edit contract changed existing object {} {} outside its allowlist; output withheld",
                        change.number, change.generation
                    )));
                }
                (Some(_), None) if !policy.allow_removed_objects || !allowed.contains(&key) => {
                    return Err(fail(&format!(
                        "edit contract removed object {} {} without explicit permission; output withheld",
                        change.number, change.generation
                    )));
                }
                (None, Some(_)) => {
                    new_objects = new_objects
                        .checked_add(1)
                        .ok_or_else(|| fail("edit contract new-object count overflow"))?;
                }
                _ => {}
            }
        }
        if new_objects > policy.max_new_objects {
            return Err(fail(
                "edit contract exceeded its new-object budget; output withheld",
            ));
        }
    }
    let object_changes = if contract.inventory_objects || contract.object_change_policy.is_some() {
        all_object_changes
    } else {
        None
    };
    let raster_preservation = contract
        .raster_preservation
        .as_ref()
        .map(|policy| verify_raster_preservation(&before, &after, policy))
        .transpose()?;
    let render_validation = if let Some(oracle) = &contract.render_oracle {
        let report = crate::universal_editing::qualify_universal_render_v2(output, oracle)?;
        if report["status"] != "native_and_supplied_reference_qualification_passed" {
            return Err(fail(
                "edit contract output raster comparison failed; output withheld",
            ));
        }
        Some(report)
    } else {
        None
    };
    Ok(EditContractReport {
        input_sha256: digest(input),
        output_sha256: digest(output),
        assertions: results,
        object_changes,
        raster_preservation,
        text_oracle_validation,
        render_validation,
        validation_scope: "declared postconditions and same-engine exact pixels outside explicit device-space edit regions on normalized plaintext output; no undeclared preservation, redaction, font fidelity, independent-renderer equivalence or universal correctness claim".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(text: &str) -> Vec<u8> {
        use crate::authoring::*;
        let mut b = PdfBuilder::new();
        b.add_page(PageSize::custom(200.0, 200.0))
            .draw_text(
                text,
                10.0,
                40.0,
                &TextStyle::new(FontFace::BuiltinUnicode, 12.0),
            )
            .unwrap();
        b.to_bytes().unwrap()
    }
    fn stream_document(data: &[u8], flate: bool) -> Vec<u8> {
        use crate::writer::{OutputObject, PdfWriter};
        let reference = |number| PdfObject::Reference {
            number,
            generation: 0,
        };
        let mut catalog = crate::PdfDictionary::empty();
        catalog.insert("Type", PdfObject::Name("Catalog".into()));
        catalog.insert("Pages", reference(2));
        catalog.insert("WFContractStream", reference(3));
        let mut pages = crate::PdfDictionary::empty();
        pages.insert("Type", PdfObject::Name("Pages".into()));
        pages.insert("Count", PdfObject::Integer(0));
        pages.insert("Kids", PdfObject::Array(Vec::new()));
        let raw = if flate {
            crate::filters::flate_encode(data, 6)
        } else {
            data.to_vec()
        };
        let mut stream = crate::PdfDictionary::empty();
        stream.insert("Length", PdfObject::Integer(raw.len() as i64));
        stream.insert("WFMeaning", PdfObject::Name("ExactDecodedBytes".into()));
        if flate {
            stream.insert("Filter", PdfObject::Name("FlateDecode".into()));
        }
        PdfWriter::new(
            vec![
                OutputObject {
                    number: 1,
                    object: PdfObject::Dictionary(catalog),
                },
                OutputObject {
                    number: 2,
                    object: PdfObject::Dictionary(pages),
                },
                OutputObject {
                    number: 3,
                    object: PdfObject::Stream { dict: stream, raw },
                },
            ],
            1,
        )
        .write()
        .unwrap()
    }
    #[test]
    fn contracts_check_actual_output_and_reject_stale_input() {
        let input = document("OLD");
        let output = document("NEW");
        let mut contract = EditContract {
            input_sha256: digest(&input),
            assertions: vec![
                EditAssertion::TextCount {
                    id: "old_removed".into(),
                    input_page: 1,
                    output_page: 1,
                    text: "OLD".into(),
                    before: 1,
                    after: 0,
                },
                EditAssertion::TextCount {
                    id: "new_present".into(),
                    input_page: 1,
                    output_page: 1,
                    text: "NEW".into(),
                    before: 0,
                    after: 1,
                },
            ],
            ..Default::default()
        };
        assert!(verify_edit_contract(&input, &output, &contract).is_ok());
        assert!(verify_edit_contract(&input, &input, &contract).is_err());
        contract.input_sha256 = digest(&output);
        assert!(verify_edit_contract(&input, &output, &contract).is_err());
    }

    #[test]
    fn object_change_policy_rejects_undeclared_mutation_and_bounds_allocations() {
        let input = document("OLD");
        let output = document("NEW");
        let before = ContentEngine::open_bytes(input.clone()).unwrap();
        let after = ContentEngine::open_bytes(output.clone()).unwrap();
        let a = inventory(&before).unwrap();
        let b = inventory(&after).unwrap();
        let keys = a.keys().chain(b.keys()).copied().collect::<BTreeSet<_>>();
        let allowed_existing_objects = keys
            .iter()
            .filter(|key| a.get(key).is_some() && a.get(key) != b.get(key))
            .map(|(number, generation)| [*number, u32::from(*generation)])
            .collect::<Vec<_>>();
        let new_objects = keys
            .iter()
            .filter(|key| a.get(key).is_none() && b.get(key).is_some())
            .count();
        let removed_objects = keys
            .iter()
            .any(|key| a.get(key).is_some() && b.get(key).is_none());
        let mut contract = EditContract {
            input_sha256: digest(&input),
            assertions: vec![EditAssertion::TextCount {
                id: "replacement".into(),
                input_page: 1,
                output_page: 1,
                text: "NEW".into(),
                before: 0,
                after: 1,
            }],
            object_change_policy: Some(ObjectChangePolicy::default()),
            ..Default::default()
        };
        assert!(verify_edit_contract(&input, &output, &contract).is_err());

        contract.object_change_policy = Some(ObjectChangePolicy {
            allowed_existing_objects,
            allow_new_objects: new_objects > 0,
            max_new_objects: new_objects,
            allow_removed_objects: removed_objects,
        });
        let report = verify_edit_contract(&input, &output, &contract).unwrap();
        assert!(report.object_changes.is_some());
    }

    #[test]
    fn raster_preservation_rejects_pixels_outside_declared_regions() {
        let input = document("OLD");
        let output = document("NEW");
        let engine = ContentEngine::open_bytes(input.clone()).unwrap();
        let cancel = crate::cancel::current_cancel_token();
        let pixels = engine
            .render_page_cancellable_with_mode(
                1,
                72,
                &cancel,
                crate::render::RenderMode::HighQuality,
            )
            .unwrap();
        let mut contract = EditContract {
            input_sha256: digest(&input),
            raster_preservation: Some(RasterPreservationPolicy {
                dpi: 72,
                pages: vec![RasterPagePreservation {
                    input_page: 1,
                    output_page: 1,
                    allowed_changed_regions: Vec::new(),
                }],
                max_total_pixels: 0,
            }),
            ..Default::default()
        };
        assert!(verify_edit_contract(&input, &output, &contract).is_err());

        contract.raster_preservation.as_mut().unwrap().pages[0].allowed_changed_regions =
            vec![[0, 0, pixels.width, pixels.height]];
        let report = verify_edit_contract(&input, &output, &contract).unwrap();
        assert_eq!(
            report.raster_preservation.unwrap()["status"],
            "native_pixels_outside_declared_regions_preserved"
        );
    }

    #[test]
    fn canonical_object_graph_survives_root_renumbering_and_detects_value_changes() {
        let input = document("GRAPH");
        let engine = ContentEngine::open_bytes(input.clone()).unwrap();
        let reader = engine.document().reader();
        let (root_number, root_generation) = reader.root_reference().unwrap();
        let root = reader.get_object(root_number, root_generation).unwrap();
        let new_number = reader
            .object_ids()
            .into_iter()
            .map(|(number, _)| number)
            .max()
            .unwrap()
            .checked_add(1)
            .unwrap();
        let output = crate::writer::write_incremental_update(
            reader,
            vec![crate::writer::IncrementalObject {
                number: new_number,
                generation: 0,
                object: root.clone(),
            }],
        )
        .unwrap();
        let expected = canonical_object_graph_digest(
            &input,
            [root_number, u32::from(root_generation)],
            4096,
            64 * 1024 * 1024,
        )
        .unwrap();
        let contract = EditContract {
            input_sha256: digest(&input),
            assertions: vec![EditAssertion::PreserveObjectGraph {
                id: "catalog_graph".into(),
                input_object: [root_number, u32::from(root_generation)],
                output_object: [new_number, 0],
                expected_sha256: expected,
                max_objects: 4096,
                max_bytes: 64 * 1024 * 1024,
            }],
            ..Default::default()
        };
        assert!(verify_edit_contract(&input, &output, &contract).is_ok());

        let PdfObject::Dictionary(mut changed_root) = root else {
            panic!("catalog root must be a dictionary")
        };
        changed_root.insert("WFGraphChange", PdfObject::Boolean(true));
        let changed = crate::writer::write_incremental_update(
            reader,
            vec![crate::writer::IncrementalObject {
                number: new_number,
                generation: 0,
                object: PdfObject::Dictionary(changed_root),
            }],
        )
        .unwrap();
        assert!(verify_edit_contract(&input, &changed, &contract).is_err());
    }

    #[test]
    fn decoded_object_graph_accepts_reencoding_but_rejects_changed_decoded_bytes() {
        let input = stream_document(b"same decoded payload", false);
        let reencoded = stream_document(b"same decoded payload", true);
        let changed = stream_document(b"changed decoded payload", true);
        let exact_input = canonical_object_graph_digest(&input, [3, 0], 32, 1024 * 1024).unwrap();
        let exact_reencoded =
            canonical_object_graph_digest(&reencoded, [3, 0], 32, 1024 * 1024).unwrap();
        assert_ne!(exact_input, exact_reencoded);
        let expected =
            canonical_decoded_object_graph_digest(&input, [3, 0], 32, 1024 * 1024).unwrap();
        assert_eq!(
            expected,
            canonical_decoded_object_graph_digest(&reencoded, [3, 0], 32, 1024 * 1024).unwrap()
        );
        let contract = EditContract {
            input_sha256: digest(&input),
            assertions: vec![EditAssertion::PreserveDecodedObjectGraph {
                id: "decoded_stream_graph".into(),
                input_object: [3, 0],
                output_object: [3, 0],
                expected_sha256: expected,
                max_objects: 32,
                max_bytes: 1024 * 1024,
            }],
            ..Default::default()
        };
        assert!(verify_edit_contract(&input, &reencoded, &contract).is_ok());
        assert!(verify_edit_contract(&input, &changed, &contract).is_err());
    }

    #[test]
    fn supplied_text_oracle_binds_input_and_reopened_output() {
        let input = document("OLD ORACLE");
        let output = document("NEW ORACLE");
        let before = ContentEngine::open_bytes(input.clone()).unwrap();
        let after = ContentEngine::open_bytes(output.clone()).unwrap();
        let input_text_sha256 = digest(before.get_page_text(1).unwrap().as_bytes());
        let output_text_sha256 = digest(after.get_page_text(1).unwrap().as_bytes());
        let mut contract = EditContract {
            input_sha256: digest(&input),
            text_oracles: vec![ExternalTextOracle {
                id: "external_page_1".into(),
                extractor: "independent-test-extractor".into(),
                extractor_version: "1.0".into(),
                input_page: 1,
                output_page: 1,
                input_text_sha256,
                output_text_sha256,
            }],
            ..Default::default()
        };
        let report = verify_edit_contract(&input, &output, &contract).unwrap();
        assert_eq!(
            report.text_oracle_validation.unwrap()["status"],
            "caller_supplied_text_oracles_matched"
        );
        contract.text_oracles[0].output_text_sha256 = digest(b"wrong output");
        assert!(verify_edit_contract(&input, &output, &contract).is_err());
    }
}
