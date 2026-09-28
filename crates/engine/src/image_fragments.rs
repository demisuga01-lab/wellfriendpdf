//! Source-preserving image relocation. A capsule replays the source graphics
//! state without replaying preceding paint or searchable text. This is a native
//! PDF transaction, not rasterization or redaction. Story layout integration is
//! separate: callers must explicitly approve destination geometry and stacking.
use crate::advanced_editing::ocr_carriers::{self, OcrCapture, OcrCarrierSelection};
use crate::content::operation::{ContentOperation, Operand};
use crate::content::parser::ContentParser;
use crate::content::tokenizer::{ContentToken, ContentTokenizer};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{ContentEngine, PdfDictionary, PdfObject, PdfReader, Result, WellfriendError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_TOTAL: usize = 256 * 1024 * 1024;
const MAX_OPS: usize = 1_000_000;
type Ref = (u32, u16);
type BatchCaptureKey = (usize, usize, usize);
#[path = "image_fragment_ocr.rs"]
mod ocr;
#[path = "image_fragment_stories.rs"]
pub(crate) mod stories;
fn fail(s: impl Into<String>) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn reference(r: Ref) -> PdfObject {
    PdfObject::Reference {
        number: r.0,
        generation: r.1,
    }
}
fn array(r: [f64; 4]) -> PdfObject {
    PdfObject::Array(r.into_iter().map(PdfObject::Real).collect())
}
fn valid_rect(r: [f64; 4]) -> bool {
    r.iter().all(|v| v.is_finite() && v.abs() <= 1e9) && r[2] > r[0] && r[3] > r[1]
}
fn key_valid(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageFragmentSource {
    /// Exact identity returned by universal_image_occurrences_v2.
    Occurrence {
        page: usize,
        /// Distinguishes repeated uses of one stream on the same page.
        content_stream_index: usize,
        occurrence_id: String,
    },
    /// Rebound ownership returned after a previous move. Offsets are rediscovered
    /// from marked content, never persisted across PDF revisions.
    Owned { binding: ImageFragmentBinding },
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ImageFragmentBinding {
    pub key: String,
    pub page: usize,
    pub rect: [f64; 4],
    pub content_sha256: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageFragmentStack {
    Background,
    Foreground,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageFragmentMove {
    pub input_sha256: String,
    pub source: ImageFragmentSource,
    pub target_page: usize,
    pub target_rect: [f64; 4],
    /// Required rather than defaulted: moving content changes painting order.
    pub stack: ImageFragmentStack,
    /// Initial capture only. An owned image/OCR group always moves intact.
    #[serde(default)]
    pub ocr: Option<OcrCarrierSelection>,
    #[serde(default)]
    pub signature_policy_override: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageFragmentPreview {
    pub input_sha256: String,
    pub plan_sha256: String,
    pub key: String,
    pub source_page: usize,
    pub source_rect: [f64; 4],
    pub target_page: usize,
    pub target_rect: [f64; 4],
    pub stack: ImageFragmentStack,
    pub source_state_bytes: usize,
    #[serde(default)]
    pub ocr_spans: usize,
    #[serde(default)]
    pub ocr_source_text: String,
    pub changed_pages: Vec<usize>,
    pub exact_limits: Vec<String>,
    pub qualification: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct ImageFragmentReport {
    pub preview: ImageFragmentPreview,
    pub binding: ImageFragmentBinding,
    pub output_sha256: String,
    pub source_occurrence_removed: bool,
    pub historical_bytes_removed: bool,
    pub output_reopened: bool,
}

fn decode(reader: &PdfReader, id: Ref) -> Result<Vec<u8>> {
    let object = reader.get_object(id.0, id.1)?;
    if object.as_stream().is_some_and(|(d, _)| d.contains_key("F")) {
        return Err(fail(
            "external-file content streams cannot be captured as local image fragments",
        ));
    }
    let decoded = decode_stream_lossless_with_limits(
        &object,
        reader,
        &DecodeLimits {
            max_decoded_bytes_per_stream: MAX_BYTES as u64,
            ..Default::default()
        },
    )?;
    if decoded.status != StreamDecodeStatus::Complete {
        return Err(fail("image fragment content stream is opaque"));
    }
    Ok(decoded.data)
}
fn resolve(reader: &PdfReader, value: &PdfObject) -> Result<PdfObject> {
    let mut v = value.clone();
    let mut seen = BTreeSet::new();
    while let Some(r) = v.as_reference() {
        if seen.len() >= 128 || !seen.insert(r) {
            return Err(fail("cyclic image fragment resource"));
        }
        v = reader.get_object(r.0, r.1)?;
    }
    Ok(v)
}
fn dict(reader: &PdfReader, value: Option<&PdfObject>) -> Result<PdfDictionary> {
    match value {
        None => Ok(PdfDictionary::empty()),
        Some(v) => resolve(reader, v)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("invalid image fragment resource dictionary")),
    }
}

/// Canonical lexical framing, with dictionaries/arrays and inline data opaque
/// to operator dispatch. Parse each complete non-image operation structurally.
pub(crate) fn operations(
    data: &[u8],
    mut visit: impl FnMut(usize, usize, &ContentOperation, bool) -> Result<()>,
) -> Result<()> {
    let mut tokens = ContentTokenizer::new(data);
    let mut start = None;
    let mut containers = Vec::new();
    let mut inline = false;
    let mut count = 0;
    while let Some(token) = tokens.next_spanned()? {
        count += 1;
        if count % 256 == 0 {
            crate::cancel::check_current_cancel("image fragment token scan")?;
        }
        if count > MAX_OPS * 8 {
            return Err(fail("image fragment token budget exceeded"));
        }
        let first = *start.get_or_insert(token.start);
        if inline {
            if matches!(&token.token,ContentToken::Operator(op) if op=="EI") {
                visit(
                    first,
                    token.end,
                    &ContentOperation::new("BI", Vec::new()),
                    true,
                )?;
                start = None;
                inline = false;
            }
            continue;
        }
        match &token.token {
            ContentToken::DictStart | ContentToken::ArrayStart => {
                if containers.len() >= 128 {
                    return Err(fail("image fragment operand depth exceeded"));
                }
                containers.push(matches!(&token.token, ContentToken::DictStart));
            }
            ContentToken::DictEnd | ContentToken::ArrayEnd
                if containers.pop() != Some(matches!(&token.token, ContentToken::DictEnd)) =>
            {
                return Err(fail("image fragment operand delimiter mismatch"));
            }
            ContentToken::Operator(op) if containers.is_empty() => {
                if op == "BI" {
                    if first != token.start {
                        return Err(fail("operands before inline image"));
                    }
                    inline = true;
                    continue;
                }
                let parsed = ContentParser::parse(&data[first..token.end])?;
                if parsed.len() != 1 {
                    return Err(fail("image fragment operation boundary mismatch"));
                }
                visit(first, token.end, &parsed[0], false)?;
                start = None;
            }
            ContentToken::Operator(_) => {
                return Err(fail("operator in image fragment operand container"))
            }
            _ => {}
        }
    }
    if inline || start.is_some() || !containers.is_empty() {
        return Err(fail("incomplete image fragment content operation"));
    }
    Ok(())
}

#[derive(Default)]
struct State {
    modes: Vec<i64>,
    mode: i64,
    in_text: bool,
    // Only optional-content/artifact scopes can own a selected untagged image.
    marks: Vec<bool>,
    logical_properties: BTreeSet<String>,
    prefix: Vec<u8>,
    captured: Option<Vec<u8>>,
    batch_selected: BTreeSet<BatchCaptureKey>,
    batch_captured: BTreeMap<BatchCaptureKey, Arc<Vec<u8>>>,
    batch_stream: Option<usize>,
    operations: usize,
    search_only: bool,
    /// Private story preflight has bound every selected logical scope to the
    /// approved Figure leaf. Captured capsules strip those source delimiters;
    /// the enclosing story transaction regenerates their MCIDs.
    allow_logical: bool,
}
impl State {
    fn feed_batch(&mut self, data: &[u8], stream: usize) -> Result<()> {
        if self.batch_selected.is_empty() || self.batch_stream.is_some() {
            return Err(fail("image fragment batch capture state is not configured"));
        }
        self.batch_stream = Some(stream);
        let result = self.feed(data, None, true);
        self.batch_stream = None;
        result
    }

    fn feed(&mut self, data: &[u8], selected: Option<[usize; 2]>, capture: bool) -> Result<()> {
        operations(data, |start, end, op, inline| {
            if self.search_only
                && (inline
                    || matches!(
                        op.operator.as_str(),
                        "Do" | "sh" | "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*"
                    ))
            {
                return Err(fail("native OCR search program contains visual paint"));
            }
            self.operations += 1;
            if self.operations > MAX_OPS {
                return Err(fail("image fragment operation budget exceeded"));
            }
            let batch_key = self.batch_stream.map(|stream| (stream, start, end));
            let batch_mode = !self.batch_selected.is_empty();
            let before = capture
                && if batch_mode {
                    self.batch_captured.len() < self.batch_selected.len()
                } else {
                    self.captured.is_none()
                };
            let chosen = if batch_mode {
                batch_key.is_some_and(|key| self.batch_selected.contains(&key))
            } else {
                selected == Some([start, end])
            };
            if chosen {
                if self.in_text
                    || (!self.allow_logical && self.marks.iter().any(|v| !*v))
                    || (!inline && op.operator != "Do")
                {
                    return Err(fail(
                        "selected image has text/logical marked-content ownership",
                    ));
                }
                if !before {
                    return Err(fail("image fragment selection is duplicated"));
                }
                let mut bytes = self.prefix.clone();
                bytes.extend_from_slice(&data[start..end]);
                bytes.push(b'\n');
                for keep in self.marks.iter().rev() {
                    if *keep {
                        bytes.extend_from_slice(b"EMC\n");
                    }
                }
                for _ in &self.modes {
                    bytes.extend_from_slice(b"Q\n");
                }
                if bytes.len() > MAX_BYTES {
                    return Err(fail("image capsule byte budget exceeded"));
                }
                if let Some(key) = batch_key.filter(|_| batch_mode) {
                    if self.batch_captured.insert(key, Arc::new(bytes)).is_some() {
                        return Err(fail("image fragment batch selection is duplicated"));
                    }
                } else {
                    self.captured = Some(bytes);
                }
            }
            if inline {
                return Ok(());
            }
            let mut retain = false;
            match op.operator.as_str() {
                "q" => {
                    if !op.operands.is_empty() || self.modes.len() >= 4096 {
                        return Err(fail("invalid image fragment q state"));
                    }
                    self.modes.push(self.mode);
                    retain = true;
                }
                "Q" => {
                    if !op.operands.is_empty() {
                        return Err(fail("operands before Q"));
                    }
                    self.mode = self
                        .modes
                        .pop()
                        .ok_or_else(|| fail("image fragment graphics-state underflow"))?;
                    retain = true;
                }
                "BT" => {
                    if self.in_text || !op.operands.is_empty() {
                        return Err(fail("invalid image fragment text nesting"));
                    }
                    self.in_text = true;
                }
                "ET" => {
                    if !self.in_text || !op.operands.is_empty() {
                        return Err(fail("invalid image fragment text end"));
                    }
                    self.in_text = false;
                }
                "Tr" => {
                    if op.operands.len() != 1 {
                        return Err(fail("invalid image fragment text render mode"));
                    }
                    self.mode = op.operands[0]
                        .as_integer()
                        .filter(|v| (0..=7).contains(v))
                        .ok_or_else(|| fail("invalid text render mode"))?;
                }
                "Tj" | "TJ" | "'" | "\"" => {
                    if !self.in_text {
                        return Err(fail("text paint outside text object"));
                    }
                    let has_glyphs = op.operands.iter().any(|operand| match operand {
                        Operand::String(bytes) => !bytes.is_empty(),
                        Operand::Array(items) => items
                            .iter()
                            .any(|item| matches!(item,Operand::String(bytes) if !bytes.is_empty())),
                        _ => false,
                    });
                    if self.search_only && has_glyphs && self.mode != 3 {
                        return Err(fail("native OCR search glyphs are not invisible"));
                    }
                    if before && self.mode >= 4 {
                        return Err(fail("text-derived clipping requires outline capture before moving this image"));
                    }
                }
                "BMC" | "BDC" => {
                    if self.marks.len() >= 256 {
                        return Err(fail("image fragment marked-content depth exceeded"));
                    }
                    let tag = op.operands.first().and_then(Operand::as_name);
                    if self.search_only
                        && (tag == Some("OC")
                            || (op.operator == "BDC"
                                && op
                                    .operands
                                    .get(1)
                                    .and_then(Operand::as_dictionary)
                                    .is_none())
                            || op
                                .operands
                                .get(1)
                                .and_then(Operand::as_dictionary)
                                .is_some_and(|d| {
                                    d.iter().any(|(key, value)| {
                                        key == "MCID" && !matches!(value, Operand::Null)
                                    })
                                }))
                    {
                        return Err(fail("native OCR search program retains unapproved semantic or optional-content ownership"));
                    }
                    let logical = op
                        .operands
                        .get(1)
                        .and_then(Operand::as_dictionary)
                        .is_some_and(|d| {
                            d.iter()
                                .any(|(k, _)| matches!(k.as_str(), "MCID" | "ActualText"))
                        })
                        || op
                            .operands
                            .get(1)
                            .and_then(Operand::as_name)
                            .is_some_and(|name| self.logical_properties.contains(name));
                    let keep = matches!(tag, Some("OC" | "Artifact")) && !logical;
                    if op.operands.len() != if op.operator == "BMC" { 1 } else { 2 }
                        || tag.is_none()
                    {
                        return Err(fail("invalid marked-content operands"));
                    }
                    self.marks.push(keep);
                    retain = keep;
                }
                "EMC" => {
                    retain = self
                        .marks
                        .pop()
                        .ok_or_else(|| fail("image fragment marked-content underflow"))?;
                }
                "s" | "b" | "b*" => {
                    if before {
                        self.prefix.extend_from_slice(b"h n\n");
                    }
                }
                "S" | "f" | "F" | "f*" | "B" | "B*" => {
                    if before {
                        self.prefix.extend_from_slice(b"n\n");
                    }
                }
                "Do" | "sh" | "MP" | "DP" => {}
                "Tc" | "Tw" | "Tz" | "TL" | "Tf" | "Ts" | "Td" | "TD" | "Tm" | "T*" => {}
                "cm" | "w" | "J" | "j" | "M" | "d" | "ri" | "i" | "gs" | "CS" | "cs" | "SC"
                | "SCN" | "sc" | "scn" | "G" | "g" | "RG" | "rg" | "K" | "k" | "m" | "l" | "c"
                | "v" | "y" | "h" | "re" | "W" | "W*" | "n" => retain = true,
                _ => {
                    return Err(fail(format!(
                        "image fragment cannot replay operator {}",
                        op.operator
                    )))
                }
            }
            if before && retain {
                self.prefix.extend_from_slice(&data[start..end]);
                self.prefix.push(b'\n');
            }
            if self.prefix.len() > MAX_BYTES {
                return Err(fail("image fragment state-prefix budget exceeded"));
            }
            Ok(())
        })
    }
    fn finish(&self) -> Result<()> {
        if self.in_text || !self.modes.is_empty() || !self.marks.is_empty() {
            return Err(fail(
                "unbalanced page state cannot be isolated for image movement",
            ));
        }
        Ok(())
    }
}
pub(crate) fn validate_carrier_program(program: &[u8]) -> Result<()> {
    let mut state = State::default();
    state.search_only = true;
    state.feed(program, None, false)?;
    state.finish()
}

fn state_for_resources(reader: &PdfReader, resources: &PdfDictionary) -> Result<State> {
    let mut state = State::default();
    let properties = dict(reader, resources.get("Properties"))?;
    if properties.iter().count() > 4096 {
        return Err(fail("image fragment property budget exceeded"));
    }
    for (name, value) in properties.iter() {
        crate::cancel::check_current_cancel("image fragment logical properties")?;
        let resolved = resolve(reader, value)?;
        if resolved
            .as_dict()
            .is_some_and(|d| d.contains_key("ActualText") || d.contains_key("MCID"))
        {
            state.logical_properties.insert(name.clone());
        }
    }
    Ok(state)
}
fn source_state(reader: &PdfReader, page: &crate::document::PdfPage) -> Result<State> {
    state_for_resources(reader, &page.resources)
}
fn owned_bytes(key: &str, rect: [f64; 4], size: [f64; 2]) -> Vec<u8> {
    // PDF integer parsing does not retain the sign of -0. Keep owner-byte
    // rebinding stable without changing any nonzero coordinate.
    let rect = rect.map(|v| if v == 0.0 { 0.0 } else { v });
    format!("/WFImageFragment << /Key ({key}) /Rect [{} {} {} {}] >> BDC\nq\n{} 0 0 {} {} {} cm\n/WFIF{key} Do\nQ\nEMC",
        rect[0],rect[1],rect[2],rect[3],(rect[2]-rect[0])/size[0],(rect[3]-rect[1])/size[1],rect[0],rect[1]).into_bytes()
}
fn isolation_dict(key: &str, role: &str) -> PdfDictionary {
    let mut dict = PdfDictionary::empty();
    dict.insert(
        "WFImageIsolationKey",
        PdfObject::String(key.as_bytes().to_vec()),
    );
    dict.insert("WFImageIsolationRole", PdfObject::Name(role.into()));
    dict
}
fn owns_isolation(reader: &PdfReader, id: Ref, key: &str, role: &str) -> Result<bool> {
    let object = reader.get_object(id.0, id.1)?;
    Ok(object.as_stream().is_some_and(|(dict, _)| {
        matches!(dict.get("WFImageIsolationKey"), Some(PdfObject::String(v)) if v == key.as_bytes())
            && dict.get_name("WFImageIsolationRole") == Some(role)
    }))
}
#[derive(Clone)]
struct Owned {
    binding: ImageFragmentBinding,
    stream: usize,
    range: [usize; 2],
    form: Ref,
    size: [f64; 2],
}
fn owned_on_page(
    reader: &PdfReader,
    page: &crate::document::PdfPage,
    buffers: &[Vec<u8>],
) -> Result<Vec<Owned>> {
    let resources = dict(reader, page.resources.get("XObject"))?;
    let mut found = Vec::new();
    let mut active = None;
    for (index, data) in buffers.iter().enumerate() {
        operations(data, |start, end, op, _| {
            if op.operator == "BDC"
                && op.operands.first().and_then(Operand::as_name) == Some("WFImageFragment")
            {
                if active.is_some() {
                    return Err(fail("nested image fragment owners"));
                }
                let props = op
                    .operands
                    .get(1)
                    .and_then(Operand::as_dictionary)
                    .ok_or_else(|| fail("invalid image fragment owner"))?;
                let key = props
                    .iter()
                    .find(|(k, _)| k == "Key")
                    .and_then(|(_, v)| v.as_bytes())
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .filter(|k| key_valid(k))
                    .ok_or_else(|| fail("invalid image fragment key"))?
                    .to_string();
                let values = props
                    .iter()
                    .find(|(k, _)| k == "Rect")
                    .and_then(|(_, v)| v.as_array())
                    .ok_or_else(|| fail("missing image fragment rectangle"))?;
                let rect: Vec<f64> = values
                    .iter()
                    .map(|v| {
                        v.as_number()
                            .ok_or_else(|| fail("invalid image fragment rectangle"))
                    })
                    .collect::<Result<_>>()?;
                let rect: [f64; 4] = rect
                    .try_into()
                    .map_err(|_| fail("invalid image fragment rectangle"))?;
                if !valid_rect(rect) {
                    return Err(fail("invalid image fragment rectangle"));
                }
                active = Some((start, key, rect));
            } else if op.operator == "EMC" {
                if let Some((begin, key, rect)) = active.take() {
                    let form = resources
                        .get(&format!("WFIF{key}"))
                        .and_then(PdfObject::as_reference)
                        .ok_or_else(|| fail("image fragment form resource missing"))?;
                    let object = reader.get_object(form.0, form.1)?;
                    let (d, _) = object
                        .as_stream()
                        .ok_or_else(|| fail("image fragment resource is not a stream"))?;
                    if d.get_name("Subtype") != Some("Form")
                        || d.get_bool("WFImageCapsule") != Some(true)
                    {
                        return Err(fail("image fragment resource is not an owned capsule"));
                    }
                    let size = d
                        .get("WFImageSize")
                        .and_then(PdfObject::as_array)
                        .ok_or_else(|| fail("image capsule dimensions missing"))?;
                    if size.len() != 2 {
                        return Err(fail("image capsule dimensions malformed"));
                    }
                    let size = [
                        size[0].as_number().unwrap_or(f64::NAN),
                        size[1].as_number().unwrap_or(f64::NAN),
                    ];
                    if size.iter().any(|v| !v.is_finite() || *v <= 0.0 || *v > 2e9) {
                        return Err(fail("image capsule dimensions malformed"));
                    }
                    let expected = owned_bytes(&key, rect, size);
                    if data.get(begin..end) != Some(expected.as_slice()) {
                        return Err(fail(
                            "owned image fragment content changed outside the native writer",
                        ));
                    }
                    found.push(Owned {
                        binding: ImageFragmentBinding {
                            key,
                            page: page.page_number,
                            rect,
                            content_sha256: hash(&expected),
                        },
                        stream: index,
                        range: [begin, end],
                        form,
                        size,
                    });
                    if found.len() > 4096 {
                        return Err(fail("image fragment owner budget exceeded"));
                    }
                }
            }
            Ok(())
        })?;
        if active.is_some() {
            return Err(fail("image fragment owner crosses content streams"));
        }
    }
    Ok(found)
}
fn page_buffers(
    reader: &PdfReader,
    page: &crate::document::PdfPage,
    total: &mut usize,
) -> Result<Vec<Vec<u8>>> {
    if page.contents.len() > 4096 {
        return Err(fail("image fragment page stream limit exceeded"));
    }
    let mut out = Vec::new();
    for id in &page.contents {
        crate::cancel::check_current_cancel("image fragment page decode")?;
        let bytes = decode(reader, *id)?;
        *total = total.saturating_add(bytes.len());
        if *total > MAX_TOTAL {
            return Err(fail("image fragment document decode budget exceeded"));
        }
        out.push(bytes);
    }
    Ok(out)
}
/// Native owner inventory used for save/reopen and subsequent moves.
pub fn image_fragment_bindings(input: &[u8]) -> Result<Vec<ImageFragmentBinding>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut total = 0;
    let mut keys = BTreeSet::new();
    let mut out = Vec::new();
    for page in engine.document().get_pages()? {
        crate::cancel::check_current_cancel("image fragment owner inventory")?;
        for owned in owned_on_page(reader, &page, &page_buffers(reader, &page, &mut total)?)? {
            if !keys.insert(owned.binding.key.clone()) || out.len() >= 4096 {
                return Err(fail("duplicated/excessive image fragment ownership"));
            }
            out.push(owned.binding);
        }
    }
    Ok(out)
}

struct Prepared {
    preview: ImageFragmentPreview,
    source: crate::document::PdfPage,
    target: crate::document::PdfPage,
    source_buffers: Arc<Vec<Vec<u8>>>,
    stream: usize,
    range: [usize; 2],
    capsule: Option<Arc<Vec<u8>>>,
    form: Option<Ref>,
    size: [f64; 2],
    ocr: Option<OcrCapture>,
    nested: Option<NestedPrepared>,
}

#[derive(Clone)]
struct NestedPrepared {
    occurrence: crate::universal_editing::UniversalImageOccurrenceV2,
    /// Selected image plus only its required state inside the leaf Form.
    leaf_program: Arc<Vec<u8>>,
    /// Program for invocation_path[index], indexed by the child invocation.
    /// Index zero is the page program stored in Prepared::capsule; entries here
    /// cover Form-owned invocations 1..path.len().
    parent_programs: Vec<Arc<Vec<u8>>>,
    captured_bytes: usize,
}
fn prepare(input: &[u8], request: &ImageFragmentMove) -> Result<Prepared> {
    let context = CaptureContext::new(input, request.signature_policy_override, false)?;
    prepare_known(&context, request, None, None, None, None)
}
#[derive(Default)]
struct PageBufferCache {
    total: usize,
    pages: BTreeMap<usize, Arc<Vec<Vec<u8>>>>,
}
/// Immutable original-revision document, permission decision and bounded page
/// buffers shared only within one capture batch. No cache survives a mutation.
struct CaptureContext<'a> {
    input: &'a [u8],
    engine: ContentEngine,
    revision: String,
    signature_policy_override: bool,
    tagged_story: bool,
    buffers: RefCell<PageBufferCache>,
    validated_pages: RefCell<BTreeSet<usize>>,
    owned_fragments: RefCell<BTreeMap<usize, Arc<Vec<Owned>>>>,
}
impl<'a> CaptureContext<'a> {
    fn new(input: &'a [u8], signature_policy_override: bool, tagged_story: bool) -> Result<Self> {
        let engine = ContentEngine::open_bytes(input.to_vec())?;
        let reader = engine.document().reader();
        if reader.is_encrypted() {
            return Err(fail(
                "image fragment relocation requires an authorized decrypted editing revision",
            ));
        }
        let root = reader
            .root_reference()
            .ok_or_else(|| fail("image fragment document has no catalog"))?;
        if !tagged_story
            && reader
                .get_object(root.0, root.1)?
                .as_dict()
                .is_some_and(|d| d.contains_key("StructTreeRoot"))
        {
            return Err(fail(
                "tagged image movement requires Figure/MCID ownership migration",
            ));
        }
        let policy = crate::secure_mutation::analyze_edit_policy(
            &engine,
            crate::secure_mutation::EditOperation::ContentEdit,
        )?;
        if !signature_policy_override
            && !matches!(
                policy.decision,
                crate::secure_mutation::EditPolicyDecision::SafeIncremental
                    | crate::secure_mutation::EditPolicyDecision::IncrementalWithWarning
            )
        {
            return Err(fail("image fragment requires signature-policy approval"));
        }
        Ok(Self {
            input,
            engine,
            revision: hash(input),
            signature_policy_override,
            tagged_story,
            buffers: RefCell::new(PageBufferCache::default()),
            validated_pages: RefCell::new(BTreeSet::new()),
            owned_fragments: RefCell::new(BTreeMap::new()),
        })
    }
    fn page_buffers(&self, page: &crate::document::PdfPage) -> Result<Arc<Vec<Vec<u8>>>> {
        crate::cancel::check_current_cancel("image capture page cache")?;
        let mut cache = self.buffers.borrow_mut();
        if let Some(bytes) = cache.pages.get(&page.page_number) {
            return Ok(Arc::clone(bytes));
        }
        let bytes = Arc::new(page_buffers(
            self.engine.document().reader(),
            page,
            &mut cache.total,
        )?);
        cache.pages.insert(page.page_number, Arc::clone(&bytes));
        Ok(bytes)
    }

    fn validate_page_state(&self, page: &crate::document::PdfPage) -> Result<()> {
        if self.validated_pages.borrow().contains(&page.page_number) {
            return Ok(());
        }
        let mut state = source_state(self.engine.document().reader(), page)?;
        for bytes in self.page_buffers(page)?.iter() {
            state.feed(bytes, None, false)?;
        }
        state.finish()?;
        self.validated_pages.borrow_mut().insert(page.page_number);
        Ok(())
    }

    fn owned_on_page(
        &self,
        page: &crate::document::PdfPage,
        buffers: &[Vec<u8>],
    ) -> Result<Arc<Vec<Owned>>> {
        if let Some(owned) = self.owned_fragments.borrow().get(&page.page_number) {
            return Ok(Arc::clone(owned));
        }
        let owned = Arc::new(owned_on_page(
            self.engine.document().reader(),
            page,
            buffers,
        )?);
        self.owned_fragments
            .borrow_mut()
            .insert(page.page_number, Arc::clone(&owned));
        Ok(owned)
    }
}

fn capture_occurrence_capsules(
    context: &CaptureContext<'_>,
    selections: &[(usize, usize, [usize; 2])],
    max_total_bytes: usize,
) -> Result<BTreeMap<(usize, usize, usize, usize), Arc<Vec<u8>>>> {
    let mut by_page = BTreeMap::<usize, BTreeSet<BatchCaptureKey>>::new();
    for (page, stream, range) in selections {
        if range[0] >= range[1]
            || !by_page
                .entry(*page)
                .or_default()
                .insert((*stream, range[0], range[1]))
        {
            return Err(fail(
                "image fragment batch contains a duplicate or empty source range",
            ));
        }
    }
    let mut captured = BTreeMap::new();
    let mut total = 0usize;
    for (page_number, selected) in by_page {
        crate::cancel::check_current_cancel("story image page batch capture")?;
        let page = context.engine.document().get_page(page_number)?;
        let buffers = context.page_buffers(&page)?;
        if selected.iter().any(|(stream, start, end)| {
            buffers
                .get(*stream)
                .is_none_or(|bytes| *start >= *end || *end > bytes.len())
        }) {
            return Err(fail("image fragment batch range leaves its source stream"));
        }
        let mut state = source_state(context.engine.document().reader(), &page)?;
        state.allow_logical = context.tagged_story;
        state.batch_selected = selected;
        for (stream, bytes) in buffers.iter().enumerate() {
            state.feed_batch(bytes, stream)?;
        }
        state.finish()?;
        if state.batch_captured.len() != state.batch_selected.len() {
            return Err(fail(
                "image fragment batch did not bind every source operation",
            ));
        }
        for ((stream, start, end), bytes) in state.batch_captured {
            total = total.checked_add(bytes.len()).ok_or_else(|| {
                WellfriendError::ResourceLimit(
                    "image fragment batch capsule byte count overflow".to_string(),
                )
            })?;
            if total > max_total_bytes {
                return Err(fail("image fragment batch capsule budget exceeded"));
            }
            captured.insert((page_number, stream, start, end), bytes);
        }
        context.validated_pages.borrow_mut().insert(page_number);
    }
    Ok(captured)
}

fn nested_resource_chain(
    reader: &PdfReader,
    page_resources: &PdfDictionary,
    path: &[crate::advanced_editing::VectorFormInvocation],
) -> Result<Vec<PdfDictionary>> {
    let mut chain = vec![page_resources.clone()];
    let mut inherited = page_resources.clone();
    for invocation in path {
        let object = reader.get_object(invocation.form_object, invocation.form_generation)?;
        let (form, _) = object
            .as_stream()
            .ok_or_else(|| fail("nested image invocation target is not a Form stream"))?;
        if form.get_name("Subtype") != Some("Form") {
            return Err(fail("nested image invocation target is not a Form XObject"));
        }
        inherited = match form.get("Resources") {
            Some(resources) => dict(reader, Some(resources))?,
            None => inherited,
        };
        chain.push(inherited.clone());
    }
    Ok(chain)
}

fn capture_form_program(
    reader: &PdfReader,
    resources: &PdfDictionary,
    owner: Ref,
    range: [usize; 2],
    allow_logical: bool,
) -> Result<Arc<Vec<u8>>> {
    let bytes = decode(reader, owner)?;
    if range[0] >= range[1] || range[1] > bytes.len() {
        return Err(fail("nested image operation leaves its Form stream"));
    }
    let mut state = state_for_resources(reader, resources)?;
    state.allow_logical = allow_logical;
    state.feed(&bytes, Some(range), true)?;
    state.finish()?;
    Ok(Arc::new(state.captured.take().ok_or_else(|| {
        fail("nested image operation did not bind to canonical token boundaries")
    })?))
}

fn capture_nested_occurrence(
    context: &CaptureContext<'_>,
    source: &crate::document::PdfPage,
    source_buffers: &[Vec<u8>],
    selected: &crate::universal_editing::UniversalImageOccurrenceV2,
    allow_logical: bool,
) -> Result<(Arc<Vec<u8>>, NestedPrepared)> {
    let path = &selected.invocation_path;
    if path.is_empty() || path.len() > 64 {
        return Err(fail("invalid nested image invocation path"));
    }
    let outer = &path[0];
    if source.contents.get(selected.content_stream_index)
        != Some(&(outer.owner_stream_object, outer.owner_stream_generation))
        || path.last().is_none_or(|leaf| {
            (leaf.form_object, leaf.form_generation)
                != (
                    selected.owner_stream_object,
                    selected.owner_stream_generation,
                )
        })
        || path.windows(2).any(|pair| {
            (pair[0].form_object, pair[0].form_generation)
                != (pair[1].owner_stream_object, pair[1].owner_stream_generation)
        })
    {
        return Err(fail("nested image invocation owner chain is stale"));
    }
    let reader = context.engine.document().reader();
    let resources = nested_resource_chain(reader, &source.resources, path)?;
    let leaf_program = capture_form_program(
        reader,
        resources
            .last()
            .ok_or_else(|| fail("nested image leaf resources missing"))?,
        (
            selected.owner_stream_object,
            selected.owner_stream_generation,
        ),
        [selected.operation_byte_start, selected.operation_byte_end],
        allow_logical,
    )?;
    let mut parent_programs = Vec::with_capacity(path.len().saturating_sub(1));
    for (index, invocation) in path.iter().enumerate().skip(1) {
        let program = capture_form_program(
            reader,
            resources
                .get(index)
                .ok_or_else(|| fail("nested image parent resources missing"))?,
            (
                invocation.owner_stream_object,
                invocation.owner_stream_generation,
            ),
            [
                invocation.owner_operation_byte_start,
                invocation.owner_operation_byte_end,
            ],
            allow_logical,
        )?;
        parent_programs.push(program);
    }
    let outer_range = [
        outer.owner_operation_byte_start,
        outer.owner_operation_byte_end,
    ];
    if source_buffers
        .get(selected.content_stream_index)
        .is_none_or(|bytes| outer_range[0] >= outer_range[1] || outer_range[1] > bytes.len())
    {
        return Err(fail("nested image outer invocation leaves its page stream"));
    }
    let mut page_state = source_state(reader, source)?;
    page_state.allow_logical = allow_logical;
    for (stream, bytes) in source_buffers.iter().enumerate() {
        page_state.feed(
            bytes,
            (stream == selected.content_stream_index).then_some(outer_range),
            true,
        )?;
    }
    page_state.finish()?;
    let page_program = Arc::new(
        page_state
            .captured
            .take()
            .ok_or_else(|| fail("nested image outer invocation was not captured"))?,
    );
    let captured_bytes = std::iter::once(page_program.len())
        .chain(std::iter::once(leaf_program.len()))
        .chain(parent_programs.iter().map(|program| program.len()))
        .try_fold(0usize, |total, length| total.checked_add(length))
        .ok_or_else(|| fail("nested image capsule byte count overflow"))?;
    if captured_bytes > MAX_TOTAL {
        return Err(fail("nested image capsule chain exceeds aggregate budget"));
    }
    Ok((
        page_program,
        NestedPrepared {
            occurrence: selected.clone(),
            leaf_program,
            parent_programs,
            captured_bytes,
        },
    ))
}

fn prepare_known(
    context: &CaptureContext<'_>,
    request: &ImageFragmentMove,
    occurrences: Option<&[crate::universal_editing::UniversalImageOccurrenceV2]>,
    owners: Option<&[ImageFragmentBinding]>,
    story_owner: Option<&str>,
    occurrence_capsules: Option<&BTreeMap<(usize, usize, usize, usize), Arc<Vec<u8>>>>,
) -> Result<Prepared> {
    if context.revision != request.input_sha256
        || !valid_rect(request.target_rect)
        || context.signature_policy_override != request.signature_policy_override
    {
        return Err(fail(
            "stale image fragment revision, policy or invalid destination",
        ));
    }
    let input = context.input;
    let engine = &context.engine;
    let reader = engine.document().reader();
    let source_page = match &request.source {
        ImageFragmentSource::Occurrence { page, .. } => *page,
        ImageFragmentSource::Owned { binding } => binding.page,
    };
    let source = engine.document().get_page(source_page)?;
    let target = engine.document().get_page(request.target_page)?;
    if source.rotate.rem_euclid(360) != target.rotate.rem_euclid(360)
        || (source.user_unit - target.user_unit).abs() > 1e-9
    {
        return Err(fail(
            "image fragment pages require matching rotation and UserUnit",
        ));
    }
    for p in [&source, &target] {
        if !valid_rect(p.crop_box)
            || reader
                .get_object(p.object_number, p.generation_number)?
                .as_dict()
                .is_some_and(|d| d.contains_key("Group"))
        {
            return Err(fail(
                "image fragment page geometry/group needs an explicit compositing migration",
            ));
        }
    }
    let r = request.target_rect;
    let crop = target.crop_box;
    if r[0] < crop[0] || r[1] < crop[1] || r[2] > crop[2] || r[3] > crop[3] {
        return Err(fail("image fragment destination leaves the crop box"));
    }
    let source_buffers = context.page_buffers(&source)?;
    if request.ocr.is_some() && matches!(&request.source, ImageFragmentSource::Owned { .. }) {
        return Err(fail(
            "new OCR selections require a source occurrence; owned groups already carry their OCR",
        ));
    }
    if request.ocr.is_some() && context.tagged_story && story_owner.is_none() {
        return Err(fail(
            "tagged OCR needs approved whole-Figure ownership in a story transaction",
        ));
    }
    if request.ocr.is_some()
        && story_owner.is_none()
        && crate::linked_stories::load_linked_stories(input)?
            .iter()
            .any(|story| {
                story
                    .request
                    .frames
                    .iter()
                    .any(|frame| frame.page == source_page)
            })
    {
        return Err(fail("OCR source page participates in a saved story; coordinated story-range rebinding is required"));
    }
    let (key, rect, stream, range, form, size, capsule, nested) = match &request.source {
        ImageFragmentSource::Occurrence {
            page,
            content_stream_index,
            occurrence_id,
        } => {
            let inventory = match occurrences {
                Some(v) => std::borrow::Cow::Borrowed(v),
                None => std::borrow::Cow::Owned(
                    crate::universal_editing::universal_image_occurrences_v2(input, &[*page])?,
                ),
            };
            let mut candidates = inventory.iter().filter(|v| {
                v.occurrence_id == *occurrence_id
                    && v.page == *page
                    && v.content_stream_index == *content_stream_index
            });
            let selected = candidates
                .next()
                .ok_or_else(|| fail("image occurrence is stale or absent"))?;
            if candidates.next().is_some() {
                return Err(fail("image occurrence identity is ambiguous"));
            }
            if !valid_rect(selected.bbox) {
                return Err(fail("image occurrence source binding is invalid"));
            }
            let (range, capsule, nested) = if selected.invocation_path.is_empty() {
                if source.contents.get(selected.content_stream_index)
                    != Some(&(
                        selected.owner_stream_object,
                        selected.owner_stream_generation,
                    ))
                {
                    return Err(fail("image occurrence page owner is stale"));
                }
                let range = [selected.operation_byte_start, selected.operation_byte_end];
                let capsule_key = (
                    source_page,
                    selected.content_stream_index,
                    range[0],
                    range[1],
                );
                let capsule = if let Some(capsules) = occurrence_capsules {
                    capsules
                        .get(&capsule_key)
                        .cloned()
                        .ok_or_else(|| fail("story image batch capsule is missing"))?
                } else {
                    let mut state = source_state(reader, &source)?;
                    state.allow_logical = context.tagged_story && story_owner.is_some();
                    for (i, b) in source_buffers.iter().enumerate() {
                        state.feed(
                            b,
                            (i == selected.content_stream_index).then_some(range),
                            true,
                        )?;
                    }
                    state.finish()?;
                    Arc::new(state.captured.take().ok_or_else(|| {
                        fail("image operation did not bind to canonical token boundaries")
                    })?)
                };
                (range, capsule, None)
            } else {
                if occurrence_capsules.is_some() {
                    return Err(fail(
                        "nested Form story images require the nested batch transaction",
                    ));
                }
                let (capsule, nested) = capture_nested_occurrence(
                    context,
                    &source,
                    &source_buffers,
                    selected,
                    context.tagged_story && story_owner.is_some(),
                )?;
                let outer = &selected.invocation_path[0];
                (
                    [
                        outer.owner_operation_byte_start,
                        outer.owner_operation_byte_end,
                    ],
                    capsule,
                    Some(nested),
                )
            };
            let key = hash(
                format!(
                    "image-fragment:{}:{}:{}",
                    request.input_sha256, content_stream_index, occurrence_id
                )
                .as_bytes(),
            );
            (
                key,
                selected.bbox,
                selected.content_stream_index,
                range,
                None,
                [
                    selected.bbox[2] - selected.bbox[0],
                    selected.bbox[3] - selected.bbox[1],
                ],
                Some(capsule),
                nested,
            )
        }
        ImageFragmentSource::Owned { binding } => {
            if !key_valid(&binding.key) {
                return Err(fail("invalid image fragment key"));
            }
            // A standalone image move must not leave saved story metadata
            // pointing at a different position. Story batches have already
            // validated ownership on their common original revision.
            if story_owner.is_none() && crate::linked_stories::load_linked_stories(input)?.iter().any(|s|s.request.figures.iter().any(|f|
                matches!(&f.source,ImageFragmentSource::Owned{binding:owned} if owned.key==binding.key))) {
                return Err(fail("image belongs to a saved linked story; move it through that story transaction"));
            }
            let all = match owners {
                Some(v) => std::borrow::Cow::Borrowed(v),
                None => std::borrow::Cow::Owned(image_fragment_bindings(input)?),
            };
            if all.iter().find(|v| v.key == binding.key) != Some(binding) {
                return Err(fail("owned image fragment binding changed"));
            }
            let owned = context
                .owned_on_page(&source, &source_buffers)?
                .iter()
                .find(|v| v.binding.key == binding.key)
                .cloned()
                .ok_or_else(|| fail("owned image fragment is absent"))?;
            context.validate_page_state(&source)?;
            (
                binding.key.clone(),
                binding.rect,
                owned.stream,
                owned.range,
                Some(owned.form),
                owned.size,
                None,
                None,
            )
        }
    };
    if source_page != request.target_page {
        context.validate_page_state(&target)?;
    }
    for scale in [(r[2] - r[0]) / size[0], (r[3] - r[1]) / size[1]] {
        if !scale.is_finite() || !(1e-9..=1e9).contains(&scale) {
            return Err(fail(
                "image fragment scale exceeds finite transformation bounds",
            ));
        }
    }
    let ocr = request
        .ocr
        .as_ref()
        .map(|selection| {
            if let Some(nested) = &nested {
                let target = selection.form_target.as_ref().ok_or_else(|| {
                    fail("nested Form OCR requires its exact Form occurrence target")
                })?;
                if target.page != source_page
                    || target.content_stream_index != stream
                    || target.invocation_path != nested.occurrence.invocation_path
                {
                    return Err(fail(
                        "nested Form OCR target does not match the selected image occurrence",
                    ));
                }
                ocr_carriers::capture_form(
                    engine,
                    selection,
                    &request.input_sha256,
                    context.tagged_story && story_owner.is_some(),
                )
            } else {
                ocr_carriers::capture(
                    input,
                    engine,
                    &source,
                    &source_buffers,
                    selection,
                    context.tagged_story && story_owner.is_some(),
                )
            }
        })
        .transpose()?;
    let (ocr_spans, ocr_source_text, owned_state_bytes) = if let Some(capture) = &ocr {
        (capture.spans, capture.text.clone(), 0)
    } else if let Some(id) = form {
        ocr::inspect(reader, id)?
    } else {
        (0, String::new(), 0)
    };
    let plan_sha256 = hash(&serde_json::to_vec(request).map_err(|e| fail(e.to_string()))?);
    let changed_pages = BTreeSet::from([source_page, request.target_page])
        .into_iter()
        .collect();
    let preview=ImageFragmentPreview{input_sha256:request.input_sha256.clone(),plan_sha256,key,source_page,source_rect:rect,target_page:request.target_page,target_rect:r,stack:request.stack,source_state_bytes:nested.as_ref().map_or_else(|| capsule.as_ref().map_or(0, |bytes| bytes.len()), |nested| nested.captured_bytes) + ocr.as_ref().map_or(0, |v|v.program.len()) + owned_state_bytes,ocr_spans,ocr_source_text,changed_pages,
        exact_limits:vec!["Explicit foreground/background placement changes painting order; surrounding-object collision checks are not automatic".into(),"Only explicitly selected invisible OCR operands move with an initial image; owned image/OCR groups move intact. Captions, links, annotations and other text remain in place".into(),"OCR selection is user-approved source ownership, not inferred association or new recognition; original font codes, state and complete direct ActualText scopes are retained".into(),"Page-owned images and occurrence-specific nested Form image/OCR paths are captured without changing shared Form definitions. Tagged nested Figures additionally require their coordinated semantic-owner validation; text clipping and page transparency groups remain separate migrations".into(),"Source clipping, resources and supported state are replayed; destination backdrop can change blend results".into(),"Incremental history and unused resources are retained; this is not redaction".into(),"Source-only implementation; no pixel comparison, standards or viewer qualification".into()],qualification:"source_implementation_only; vps_corpus_gate_pending".into()};
    Ok(Prepared {
        preview,
        source,
        target,
        source_buffers,
        stream,
        range,
        capsule,
        form,
        size,
        ocr,
        nested,
    })
}
pub fn preview_image_fragment_move(
    input: &[u8],
    request: &ImageFragmentMove,
) -> Result<ImageFragmentPreview> {
    Ok(prepare(input, request)?.preview)
}

struct Updates {
    next: u32,
    objects: Vec<IncrementalObject>,
}
impl Updates {
    fn add(&mut self, object: PdfObject) -> Result<Ref> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| fail("image fragment object space exhausted"))?;
        let r = (self.next, 0);
        self.put(r, object);
        Ok(r)
    }
    fn put(&mut self, r: Ref, object: PdfObject) {
        self.objects.push(IncrementalObject {
            number: r.0,
            generation: r.1,
            object,
        });
    }
    fn stream(&mut self, bytes: &[u8], mut dict: PdfDictionary) -> Result<Ref> {
        let raw = crate::filters::flate_encode_cancellable(bytes, 6)?;
        dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
        for key in ["DecodeParms", "DP", "DL", "F", "FFilter", "FDecodeParms"] {
            dict.remove(key);
        }
        self.add(PdfObject::Stream { dict, raw })
    }
}
/// Atomic private-buffer move, gated by the exact preview receipt. The source
/// paint occurrence is removed; shared source definitions remain unchanged.
pub fn apply_image_fragment_move(
    input: &[u8],
    request: &ImageFragmentMove,
    approved_plan_sha256: &str,
) -> Result<(Vec<u8>, ImageFragmentReport)> {
    let prepared = prepare(input, request)?;
    if prepared.preview.plan_sha256 != approved_plan_sha256 {
        return Err(fail("image fragment preview approval is stale"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut updates = Updates {
        next: reader.object_ids().iter().map(|v| v.0).max().unwrap_or(0),
        objects: Vec::new(),
    };
    if prepared.nested.is_some() {
        let form = nested_capsule_form(&mut updates, &prepared, reader)?;
        return apply_nested_prepared_move(request, prepared, reader, updates, form);
    }
    let form = capsule_form(&mut updates, &prepared)?;
    apply_prepared_move(request, prepared, reader, updates, form)
}

fn xobject_resources(reader: &PdfReader, resources: &PdfDictionary) -> Result<PdfDictionary> {
    dict(reader, resources.get("XObject"))
}

fn add_unique_xobject(
    reader: &PdfReader,
    resources: &PdfDictionary,
    prefix: &str,
    child: Ref,
) -> Result<(PdfDictionary, String)> {
    let mut resources = resources.clone();
    let mut xobjects = xobject_resources(reader, &resources)?;
    for suffix in 0..=1_000_000u32 {
        let name = if suffix == 0 {
            format!("{prefix}{}", child.0)
        } else {
            format!("{prefix}{}_{}", child.0, suffix)
        };
        if !xobjects.contains_key(&name) {
            xobjects.insert(name.clone(), reference(child));
            resources.insert("XObject", PdfObject::Dictionary(xobjects));
            return Ok((resources, name));
        }
    }
    Err(fail("nested image resource-name budget exceeded"))
}

fn replace_xobject(
    reader: &PdfReader,
    resources: &PdfDictionary,
    name: &str,
    child: Ref,
) -> Result<PdfDictionary> {
    let mut resources = resources.clone();
    let mut xobjects = xobject_resources(reader, &resources)?;
    if !xobjects.contains_key(name) {
        return Err(fail("nested image invocation resource is missing"));
    }
    xobjects.insert(name, reference(child));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    Ok(resources)
}

fn clone_stream_patch(
    updates: &mut Updates,
    reader: &PdfReader,
    owner: Ref,
    range: [usize; 2],
    replacement: &[u8],
    resources: Option<PdfDictionary>,
) -> Result<Ref> {
    clone_stream_patches(
        updates,
        reader,
        owner,
        vec![(range[0], range[1], replacement.to_vec())],
        resources,
    )
}

fn clone_stream_patches(
    updates: &mut Updates,
    reader: &PdfReader,
    owner: Ref,
    patches: Vec<(usize, usize, Vec<u8>)>,
    resources: Option<PdfDictionary>,
) -> Result<Ref> {
    let object = reader.get_object(owner.0, owner.1)?;
    let mut stream = object
        .as_stream()
        .ok_or_else(|| fail("nested image owner is not a stream"))?
        .0
        .clone();
    let bytes = decode(reader, owner)?;
    let output = ocr_carriers::apply_patches(&bytes, patches)?;
    if let Some(resources) = resources {
        stream.insert("Resources", PdfObject::Dictionary(resources));
    }
    updates.stream(&output, stream)
}

fn form_with_program(
    updates: &mut Updates,
    reader: &PdfReader,
    source: Ref,
    program: &[u8],
    resources: PdfDictionary,
    search: bool,
) -> Result<Ref> {
    let object = reader.get_object(source.0, source.1)?;
    let mut form = object
        .as_stream()
        .ok_or_else(|| fail("nested image capsule level is not a Form stream"))?
        .0
        .clone();
    if form.get_name("Subtype") != Some("Form") {
        return Err(fail("nested image capsule level is not a Form XObject"));
    }
    form.insert("Resources", PdfObject::Dictionary(resources));
    form.remove("StructParent");
    form.remove("StructParents");
    if search {
        form.remove("WFNestedImageCapsule");
        form.insert("WFNestedOcrSearch", PdfObject::Boolean(true));
    } else {
        form.remove("WFNestedOcrSearch");
        form.insert("WFNestedImageCapsule", PdfObject::Boolean(true));
    }
    updates.stream(program, form)
}

fn nested_capsule_form(
    updates: &mut Updates,
    prepared: &Prepared,
    reader: &PdfReader,
) -> Result<Ref> {
    let nested = prepared
        .nested
        .as_ref()
        .ok_or_else(|| fail("nested image capsule plan missing"))?;
    let path = &nested.occurrence.invocation_path;
    let resources = nested_resource_chain(reader, &prepared.source.resources, path)?;
    let leaf_source = (
        nested.occurrence.owner_stream_object,
        nested.occurrence.owner_stream_generation,
    );
    let mut child = form_with_program(
        updates,
        reader,
        leaf_source,
        &nested.leaf_program,
        resources
            .last()
            .cloned()
            .ok_or_else(|| fail("nested image leaf capsule resources missing"))?,
        false,
    )?;
    for index in (1..path.len()).rev() {
        let invocation = &path[index];
        let parent_source = (path[index - 1].form_object, path[index - 1].form_generation);
        let parent_resources = replace_xobject(
            reader,
            resources
                .get(index)
                .ok_or_else(|| fail("nested image parent capsule resources missing"))?,
            &invocation.resource_name,
            child,
        )?;
        child = form_with_program(
            updates,
            reader,
            parent_source,
            nested
                .parent_programs
                .get(index - 1)
                .ok_or_else(|| fail("nested image parent capsule program missing"))?,
            parent_resources,
            false,
        )?;
    }
    let page_resources = replace_xobject(
        reader,
        &prepared.source.resources,
        &path[0].resource_name,
        child,
    )?;
    let mut form = PdfDictionary::empty();
    form.insert("Type", PdfObject::Name("XObject".into()));
    form.insert("Subtype", PdfObject::Name("Form".into()));
    form.insert("FormType", PdfObject::Integer(1));
    form.insert("BBox", array(prepared.source.crop_box));
    form.insert(
        "Matrix",
        PdfObject::Array(vec![
            PdfObject::Integer(1),
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(1),
            PdfObject::Real(-prepared.preview.source_rect[0]),
            PdfObject::Real(-prepared.preview.source_rect[1]),
        ]),
    );
    form.insert("Resources", PdfObject::Dictionary(page_resources));
    form.insert("WFImageCapsule", PdfObject::Boolean(true));
    form.insert(
        "WFImageSize",
        PdfObject::Array(prepared.size.into_iter().map(PdfObject::Real).collect()),
    );
    let visual = updates.stream(
        prepared
            .capsule
            .as_ref()
            .ok_or_else(|| fail("nested image page capsule missing"))?,
        form.clone(),
    )?;
    let Some(capture) = &prepared.ocr else {
        return Ok(visual);
    };

    let mut search_child = form_with_program(
        updates,
        reader,
        leaf_source,
        &capture.program,
        resources
            .last()
            .cloned()
            .ok_or_else(|| fail("nested OCR leaf resources missing"))?,
        true,
    )?;
    for index in (1..path.len()).rev() {
        let invocation = &path[index];
        let parent_source = (path[index - 1].form_object, path[index - 1].form_generation);
        let parent_resources = replace_xobject(
            reader,
            resources
                .get(index)
                .ok_or_else(|| fail("nested OCR parent resources missing"))?,
            &invocation.resource_name,
            search_child,
        )?;
        search_child = form_with_program(
            updates,
            reader,
            parent_source,
            nested
                .parent_programs
                .get(index - 1)
                .ok_or_else(|| fail("nested OCR parent program missing"))?,
            parent_resources,
            true,
        )?;
    }
    let search_resources = replace_xobject(
        reader,
        &prepared.source.resources,
        &path[0].resource_name,
        search_child,
    )?;
    let mut search_form = form;
    search_form.remove("WFImageCapsule");
    search_form.remove("WFImageSize");
    search_form.insert("Resources", PdfObject::Dictionary(search_resources));
    search_form.insert("WFNestedOcrSearch", PdfObject::Boolean(true));
    let search = updates.stream(
        prepared
            .capsule
            .as_ref()
            .ok_or_else(|| fail("nested OCR page capsule missing"))?,
        search_form,
    )?;
    ocr::group_prebuilt(updates, prepared, visual, search, capture)
}

fn nested_source_clone(
    updates: &mut Updates,
    prepared: &Prepared,
    reader: &PdfReader,
) -> Result<(Ref, PdfDictionary)> {
    let nested = prepared
        .nested
        .as_ref()
        .ok_or_else(|| fail("nested image source-clone plan missing"))?;
    let path = &nested.occurrence.invocation_path;
    let resources = nested_resource_chain(reader, &prepared.source.resources, path)?;
    let mut leaf_patches = prepared
        .ocr
        .as_ref()
        .map(|capture| {
            if !capture.form_local || capture.source_edits.keys().any(|index| *index != 0) {
                return Err(fail(
                    "nested OCR edits are not bound to the selected leaf Form",
                ));
            }
            Ok(capture.source_edits.get(&0).cloned().unwrap_or_default())
        })
        .transpose()?
        .unwrap_or_default();
    leaf_patches.push((
        nested.occurrence.operation_byte_start,
        nested.occurrence.operation_byte_end,
        b"\n".to_vec(),
    ));
    let mut child = clone_stream_patches(
        updates,
        reader,
        (
            nested.occurrence.owner_stream_object,
            nested.occurrence.owner_stream_generation,
        ),
        leaf_patches,
        None,
    )?;
    for index in (1..path.len()).rev() {
        let invocation = &path[index];
        let (parent_resources, name) = add_unique_xobject(
            reader,
            resources
                .get(index)
                .ok_or_else(|| fail("nested image source parent resources missing"))?,
            "WFNIF",
            child,
        )?;
        child = clone_stream_patch(
            updates,
            reader,
            (
                invocation.owner_stream_object,
                invocation.owner_stream_generation,
            ),
            [
                invocation.owner_operation_byte_start,
                invocation.owner_operation_byte_end,
            ],
            format!("/{name} Do").as_bytes(),
            Some(parent_resources),
        )?;
    }
    let outer = &path[0];
    let (page_resources, name) =
        add_unique_xobject(reader, &prepared.source.resources, "WFNIP", child)?;
    let page_stream = clone_stream_patch(
        updates,
        reader,
        (outer.owner_stream_object, outer.owner_stream_generation),
        [
            outer.owner_operation_byte_start,
            outer.owner_operation_byte_end,
        ],
        format!("/{name} Do").as_bytes(),
        None,
    )?;
    Ok((page_stream, page_resources))
}

fn capsule_form(updates: &mut Updates, prepared: &Prepared) -> Result<Ref> {
    Ok(if let Some(form) = prepared.form {
        form
    } else {
        let mut d = PdfDictionary::empty();
        d.insert("Type", PdfObject::Name("XObject".into()));
        d.insert("Subtype", PdfObject::Name("Form".into()));
        d.insert("FormType", PdfObject::Integer(1));
        d.insert("BBox", array(prepared.source.crop_box));
        d.insert(
            "Matrix",
            PdfObject::Array(vec![
                PdfObject::Integer(1),
                PdfObject::Integer(0),
                PdfObject::Integer(0),
                PdfObject::Integer(1),
                PdfObject::Real(-prepared.preview.source_rect[0]),
                PdfObject::Real(-prepared.preview.source_rect[1]),
            ]),
        );
        d.insert(
            "Resources",
            PdfObject::Dictionary(prepared.source.resources.clone()),
        );
        d.insert("WFImageCapsule", PdfObject::Boolean(true));
        d.insert(
            "WFImageSize",
            PdfObject::Array(prepared.size.into_iter().map(PdfObject::Real).collect()),
        );
        let visual = updates.stream(
            prepared
                .capsule
                .as_ref()
                .ok_or_else(|| fail("image capsule missing"))?,
            d.clone(),
        )?;
        if let Some(capture) = &prepared.ocr {
            ocr::group(updates, prepared, visual, d, capture)?
        } else {
            visual
        }
    })
}

fn apply_nested_prepared_move(
    request: &ImageFragmentMove,
    prepared: Prepared,
    reader: &PdfReader,
    mut updates: Updates,
    form: Ref,
) -> Result<(Vec<u8>, ImageFragmentReport)> {
    if prepared.form.is_some() {
        return Err(fail("invalid nested image prepared state"));
    }
    let nested = prepared
        .nested
        .as_ref()
        .ok_or_else(|| fail("nested image apply plan missing"))?;
    let outer = nested
        .occurrence
        .invocation_path
        .first()
        .ok_or_else(|| fail("nested image outer invocation missing"))?;
    if prepared.source.contents.get(prepared.stream)
        != Some(&(outer.owner_stream_object, outer.owner_stream_generation))
    {
        return Err(fail("nested image page occurrence changed before apply"));
    }
    let (source_clone, source_resources) = nested_source_clone(&mut updates, &prepared, reader)?;
    let source_resource_copy = source_resources.clone();
    let mut pages = BTreeMap::new();
    for page in [&prepared.source, &prepared.target] {
        let dictionary = reader
            .get_object(page.object_number, page.generation_number)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("nested image page dictionary missing"))?;
        pages.entry(page.page_number).or_insert((
            page.clone(),
            dictionary,
            page.contents
                .iter()
                .copied()
                .map(reference)
                .collect::<Vec<_>>(),
        ));
    }
    let (_, source_dictionary, source_contents) = pages
        .get_mut(&prepared.source.page_number)
        .ok_or_else(|| fail("nested image source page disappeared"))?;
    let slot = source_contents
        .get_mut(prepared.stream)
        .ok_or_else(|| fail("nested image source content slot changed"))?;
    if slot.as_reference() != Some((outer.owner_stream_object, outer.owner_stream_generation)) {
        return Err(fail("nested image source content identity changed"));
    }
    *slot = reference(source_clone);
    source_dictionary.insert("Resources", PdfObject::Dictionary(source_resources.clone()));

    let (target, destination, contents) = pages
        .get_mut(&prepared.target.page_number)
        .ok_or_else(|| fail("nested image target page disappeared"))?;
    let name = format!("WFIF{}", prepared.preview.key);
    let mut resources = if prepared.target.page_number == prepared.source.page_number {
        source_resource_copy
    } else {
        target.resources.clone()
    };
    let mut xobjects = xobject_resources(reader, &resources)?;
    if let Some(old) = xobjects.get(&name) {
        if old.as_reference() != Some(form) {
            return Err(fail("nested image fragment resource name collision"));
        }
    }
    xobjects.insert(name, reference(form));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    destination.insert("Resources", PdfObject::Dictionary(resources));
    let paint = owned_bytes(&prepared.preview.key, request.target_rect, prepared.size);
    match request.stack {
        ImageFragmentStack::Background => {
            let mut background = paint.clone();
            background.push(b'\n');
            let stream = updates.stream(&background, PdfDictionary::empty())?;
            contents.insert(0, reference(stream));
        }
        ImageFragmentStack::Foreground => {
            let prefix = updates.stream(b"q\n", isolation_dict(&prepared.preview.key, "Prefix"))?;
            let mut tail = b"n\nQ\n".to_vec();
            tail.extend_from_slice(&paint);
            tail.push(b'\n');
            let suffix = updates.stream(&tail, isolation_dict(&prepared.preview.key, "Tail"))?;
            contents.insert(0, reference(prefix));
            contents.push(reference(suffix));
        }
    }
    for (_, (page, mut dictionary, contents)) in pages {
        dictionary.insert("Contents", PdfObject::Array(contents));
        updates.put(
            (page.object_number, page.generation_number),
            PdfObject::Dictionary(dictionary),
        );
    }
    crate::cancel::check_current_cancel("nested image fragment commit")?;
    let output = write_incremental_update(reader, updates.objects)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let source = reopened.document().get_page(prepared.source.page_number)?;
    if source.contents.get(prepared.stream) != Some(&source_clone) {
        return Err(fail("nested image source clone failed reopen verification"));
    }
    let binding = image_fragment_bindings(&output)?
        .into_iter()
        .find(|binding| binding.key == prepared.preview.key)
        .ok_or_else(|| fail("nested image fragment owner missing after save"))?;
    if binding.page != request.target_page
        || binding.rect != request.target_rect
        || binding.content_sha256 != hash(&paint)
    {
        return Err(fail(
            "nested image fragment placement failed reopen verification",
        ));
    }
    if ocr::info(reopened.document().reader(), form)?
        != (
            prepared.preview.ocr_spans,
            prepared.preview.ocr_source_text.clone(),
        )
    {
        return Err(fail(
            "nested image/OCR capsule failed reopen ownership verification",
        ));
    }
    let report = ImageFragmentReport {
        preview: prepared.preview,
        binding,
        output_sha256: hash(&output),
        source_occurrence_removed: true,
        historical_bytes_removed: false,
        output_reopened: true,
    };
    Ok((output, report))
}

fn apply_prepared_move(
    request: &ImageFragmentMove,
    prepared: Prepared,
    reader: &PdfReader,
    mut updates: Updates,
    form: Ref,
) -> Result<(Vec<u8>, ImageFragmentReport)> {
    // Bind every removal to the original revision, then rewrite each affected
    // /Contents occurrence once. Never rebase offsets after a partial mutation.
    let data = &prepared.source_buffers[prepared.stream];
    let [start, end] = prepared.range;
    let mut edits = prepared
        .ocr
        .as_ref()
        .map(|v| v.source_edits.clone())
        .unwrap_or_default();
    edits
        .entry(prepared.stream)
        .or_default()
        .push((start, end, b"\n".to_vec()));
    // Retire only complete, byte-verified wrappers owned by this capsule.
    // Otherwise preserve the q/Q scaffold: removing a suffix Q on its own
    // would change every later graphics-state operation on the page.
    let old_paint = owned_bytes(
        &prepared.preview.key,
        prepared.preview.source_rect,
        prepared.size,
    );
    let mut old_tail = b"n\nQ\n".to_vec();
    old_tail.extend_from_slice(&old_paint);
    old_tail.push(b'\n');
    let mut old_background = old_paint;
    old_background.push(b'\n');
    let trim_pair = prepared.form.is_some()
        && prepared.stream + 1 == prepared.source.contents.len()
        && prepared.stream > 0
        && data == &old_tail
        && prepared.source_buffers[0] == b"q\n"
        && owns_isolation(
            reader,
            prepared.source.contents[0],
            &prepared.preview.key,
            "Prefix",
        )?
        && owns_isolation(
            reader,
            prepared.source.contents[prepared.stream],
            &prepared.preview.key,
            "Tail",
        )?;
    let trim_single = prepared.form.is_some() && prepared.stream == 0 && data == &old_background;
    let mut source_clones = BTreeMap::new();
    if !trim_pair && !trim_single {
        let mut total = 0usize;
        for (index, patches) in edits {
            crate::cancel::check_current_cancel("native image/OCR batch removal")?;
            let source = prepared
                .source
                .contents
                .get(index)
                .ok_or_else(|| fail("OCR source stream index changed"))?;
            let bytes = ocr_carriers::apply_patches(&prepared.source_buffers[index], patches)?;
            total = total
                .checked_add(bytes.len())
                .ok_or_else(|| fail("OCR source byte budget overflow"))?;
            if total > MAX_TOTAL {
                return Err(fail("OCR source rewrite budget exceeded"));
            }
            let object = reader.get_object(source.0, source.1)?;
            let dict = object
                .as_stream()
                .ok_or_else(|| fail("OCR source is not a stream"))?
                .0
                .clone();
            let id = updates.stream(&bytes, dict)?;
            source_clones.insert(index, (id, bytes));
        }
    }
    let mut pages = BTreeMap::new();
    for p in [&prepared.source, &prepared.target] {
        let d = reader
            .get_object(p.object_number, p.generation_number)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("image source page dictionary missing"))?;
        pages.entry(p.page_number).or_insert((
            p.clone(),
            d,
            p.contents
                .iter()
                .copied()
                .map(reference)
                .collect::<Vec<_>>(),
        ));
    }
    let source_contents = &mut pages
        .get_mut(&prepared.source.page_number)
        .ok_or_else(|| fail("image source page disappeared"))?
        .2;
    if trim_pair {
        source_contents.remove(prepared.stream);
        source_contents.remove(0);
    } else if trim_single {
        source_contents.remove(prepared.stream);
    } else {
        for (index, (id, _)) in &source_clones {
            source_contents[*index] = reference(*id);
        }
    }
    let (target, destination, contents) = pages
        .get_mut(&prepared.target.page_number)
        .ok_or_else(|| fail("image target page disappeared"))?;
    let name = format!("WFIF{}", prepared.preview.key);
    let mut resources = target.resources.clone();
    let mut xobjects = dict(reader, resources.get("XObject"))?;
    if let Some(old) = xobjects.get(&name) {
        if old.as_reference() != Some(form) {
            return Err(fail("image fragment resource name collision"));
        }
    }
    xobjects.insert(name, reference(form));
    resources.insert("XObject", PdfObject::Dictionary(xobjects));
    destination.insert("Resources", PdfObject::Dictionary(resources));
    let paint = owned_bytes(&prepared.preview.key, request.target_rect, prepared.size);
    match request.stack {
        ImageFragmentStack::Background => {
            let mut background = paint.clone();
            background.push(b'\n');
            let id = updates.stream(&background, PdfDictionary::empty())?;
            contents.insert(0, reference(id));
        }
        ImageFragmentStack::Foreground => {
            // q alone does not reset state. Save the default state BEFORE the
            // original streams, then restore it before the moved native Form.
            let prefix = updates.stream(b"q\n", isolation_dict(&prepared.preview.key, "Prefix"))?;
            let mut tail = b"n\nQ\n".to_vec();
            tail.extend_from_slice(&paint);
            tail.push(b'\n');
            let suffix = updates.stream(&tail, isolation_dict(&prepared.preview.key, "Tail"))?;
            contents.insert(0, reference(prefix));
            contents.push(reference(suffix));
        }
    }
    for (_, (page, mut dict, contents)) in pages {
        dict.insert("Contents", PdfObject::Array(contents));
        updates.put(
            (page.object_number, page.generation_number),
            PdfObject::Dictionary(dict),
        );
    }
    crate::cancel::check_current_cancel("image fragment commit")?;
    let output = write_incremental_update(reader, updates.objects)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    let source = reopened.document().get_page(prepared.source.page_number)?;
    if !source_clones.is_empty() {
        for (id, bytes) in source_clones.values() {
            if !source.contents.contains(id) || decode(reopened.document().reader(), *id)? != *bytes
            {
                return Err(fail("image/OCR source removal failed reopen verification"));
            }
        }
    } else if source
        .contents
        .contains(&prepared.source.contents[prepared.stream])
    {
        return Err(fail(
            "retired image fragment paint remains reachable on its source page",
        ));
    }
    let binding = image_fragment_bindings(&output)?
        .into_iter()
        .find(|v| v.key == prepared.preview.key)
        .ok_or_else(|| fail("image fragment owner missing after save"))?;
    if binding.page != request.target_page
        || binding.rect != request.target_rect
        || binding.content_sha256 != hash(&paint)
    {
        return Err(fail("image fragment placement failed reopen verification"));
    }
    if ocr::info(reopened.document().reader(), form)?
        != (
            prepared.preview.ocr_spans,
            prepared.preview.ocr_source_text.clone(),
        )
    {
        return Err(fail("image/OCR native group failed reopen verification"));
    }
    let report = ImageFragmentReport {
        preview: prepared.preview,
        binding,
        output_sha256: hash(&output),
        source_occurrence_removed: true,
        historical_bytes_removed: false,
        output_reopened: true,
    };
    Ok((output, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("image_fragment_ocr_tests.rs");
    fn capture(bytes: &[u8], needle: &[u8]) -> Result<Vec<u8>> {
        let start = bytes
            .windows(needle.len())
            .position(|w| w == needle)
            .unwrap();
        let mut state = State::default();
        state.feed(bytes, Some([start, start + needle.len()]), true)?;
        state.finish()?;
        state.captured.ok_or_else(|| fail("test selection missing"))
    }
    #[test]
    fn capture_preserves_clip_and_color_but_not_preceding_paint_or_logical_text() {
        let bytes=b"q 1 0 0 1 5 7 cm /GS gs 1 0 0 rg 0 0 100 100 re W S /Span << /ActualText (OLD) /Reviewed true /Extra null >> BDC BT /F1 10 Tf (SECRET) Tj ET EMC 30 0 0 20 10 10 cm /Im Do Q";
        let capsule = capture(bytes, b"/Im Do").unwrap();
        let text = String::from_utf8(capsule).unwrap();
        assert!(text.contains("/GS gs\n"));
        assert!(text.contains("W\nn\n"));
        assert!(text.contains("1 0 0 rg"));
        assert!(text.contains("30 0 0 20 10 10 cm"));
        assert!(!text.contains("SECRET"));
        assert!(!text.contains("ActualText"));
        assert!(!text.contains("OLD"));
        assert!(text.ends_with("/Im Do\nQ\n"));
    }
    #[test]
    fn state_and_selection_are_carried_across_complete_content_streams() {
        let mut state = State::default();
        state.feed(b"q 0 0 80 80 re W n", None, true).unwrap();
        state.feed(b"/Im Do Q", Some([0, 6]), true).unwrap();
        state.finish().unwrap();
        assert!(state.captured.unwrap().ends_with(b"/Im Do\nQ\n"));
        assert!(capture(b"BT 7 Tr (clip) Tj ET /Im Do", b"/Im Do").is_err());
        assert!(capture(
            b"/Span << /ActualText (logical image) >> BDC /Im Do EMC",
            b"/Im Do"
        )
        .is_err());
        assert!(capture(b"Q /Im Do", b"/Im Do").is_err());
    }
    #[test]
    fn inline_image_payload_is_not_dispatched_as_operators() {
        let image = b"BI /W 3 /H 1 /CS /G /BPC 8 ID qQ/ EI";
        let mut bytes = b"q 10 0 0 10 5 5 cm ".to_vec();
        bytes.extend_from_slice(image);
        bytes.extend_from_slice(b" Q");
        let cap = capture(&bytes, image).unwrap();
        assert!(cap.windows(image.len()).any(|w| w == image));
        let mut operations_seen = Vec::new();
        operations(&bytes, |_, _, op, _| {
            operations_seen.push(op.operator.clone());
            Ok(())
        })
        .unwrap();
        assert_eq!(operations_seen, vec!["q", "cm", "BI", "Q"]);
    }
    fn fixture(repeat: bool) -> (Vec<u8>, Ref, Ref) {
        use crate::authoring::{PageSize, PdfBuilder};
        let mut b = PdfBuilder::new();
        b.add_page(PageSize::custom(200.0, 200.0));
        b.add_page(PageSize::custom(200.0, 200.0));
        let base = b.to_bytes().unwrap();
        let engine = ContentEngine::open_bytes(base).unwrap();
        let reader = engine.document().reader();
        let pages = engine.document().get_pages().unwrap();
        let mut updates = Updates {
            next: reader.object_ids().iter().map(|id| id.0).max().unwrap(),
            objects: Vec::new(),
        };
        let mut mask = PdfDictionary::empty();
        mask.insert("Type", PdfObject::Name("XObject".into()));
        mask.insert("Subtype", PdfObject::Name("Image".into()));
        mask.insert("Width", PdfObject::Integer(1));
        mask.insert("Height", PdfObject::Integer(1));
        mask.insert("BitsPerComponent", PdfObject::Integer(8));
        mask.insert("ColorSpace", PdfObject::Name("DeviceGray".into()));
        let mask_id = updates.stream(&[128], mask).unwrap();
        let mut image = PdfDictionary::empty();
        image.insert("Type", PdfObject::Name("XObject".into()));
        image.insert("Subtype", PdfObject::Name("Image".into()));
        image.insert("Width", PdfObject::Integer(1));
        image.insert("Height", PdfObject::Integer(1));
        image.insert("BitsPerComponent", PdfObject::Integer(8));
        image.insert("ColorSpace", PdfObject::Name("DeviceRGB".into()));
        image.insert("SMask", reference(mask_id));
        let image_id = updates.stream(&[255, 0, 32], image).unwrap();
        let mut xo = PdfDictionary::empty();
        xo.insert("Im", reference(image_id));
        let mut resources = PdfDictionary::empty();
        resources.insert("XObject", PdfObject::Dictionary(xo));
        let mut gs = PdfDictionary::empty();
        gs.insert("ca", PdfObject::Real(0.5));
        gs.insert("BM", PdfObject::Name("Multiply".into()));
        let mut states = PdfDictionary::empty();
        states.insert("GS", PdfObject::Dictionary(gs));
        resources.insert("ExtGState", PdfObject::Dictionary(states));
        let stream = updates
            .stream(
                b"q /GS gs 0 0 100 100 re W n 30 0 0 20 10 10 cm /Im Do Q",
                PdfDictionary::empty(),
            )
            .unwrap();
        for page in pages {
            let mut d = reader
                .get_object(page.object_number, page.generation_number)
                .unwrap()
                .as_dict()
                .unwrap()
                .clone();
            d.insert("Resources", PdfObject::Dictionary(resources.clone()));
            d.insert(
                "Contents",
                PdfObject::Array(if repeat && page.page_number == 1 {
                    vec![reference(stream), reference(stream)]
                } else {
                    vec![reference(stream)]
                }),
            );
            updates.put(
                (page.object_number, page.generation_number),
                PdfObject::Dictionary(d),
            );
        }
        (
            write_incremental_update(reader, updates.objects).unwrap(),
            image_id,
            stream,
        )
    }

    fn nested_fixture() -> (Vec<u8>, Ref, Ref, Ref, Ref) {
        use crate::authoring::{PageSize, PdfBuilder};

        let mut builder = PdfBuilder::new();
        builder.add_page(PageSize::custom(240.0, 200.0));
        builder.add_page(PageSize::custom(240.0, 200.0));
        let base = builder.to_bytes().unwrap();
        let engine = ContentEngine::open_bytes(base).unwrap();
        let reader = engine.document().reader();
        let pages = engine.document().get_pages().unwrap();
        let mut updates = Updates {
            next: reader.object_ids().iter().map(|id| id.0).max().unwrap(),
            objects: Vec::new(),
        };

        let mut image = PdfDictionary::empty();
        image.insert("Type", PdfObject::Name("XObject".into()));
        image.insert("Subtype", PdfObject::Name("Image".into()));
        image.insert("Width", PdfObject::Integer(1));
        image.insert("Height", PdfObject::Integer(1));
        image.insert("BitsPerComponent", PdfObject::Integer(8));
        image.insert("ColorSpace", PdfObject::Name("DeviceRGB".into()));
        let image_id = updates.stream(&[18, 96, 214], image).unwrap();

        let mut leaf_xobjects = PdfDictionary::empty();
        leaf_xobjects.insert("Im", reference(image_id));
        let mut leaf_resources = PdfDictionary::empty();
        leaf_resources.insert("XObject", PdfObject::Dictionary(leaf_xobjects));
        let mut leaf = PdfDictionary::empty();
        leaf.insert("Type", PdfObject::Name("XObject".into()));
        leaf.insert("Subtype", PdfObject::Name("Form".into()));
        leaf.insert("FormType", PdfObject::Integer(1));
        leaf.insert("BBox", array([0.0, 0.0, 40.0, 30.0]));
        leaf.insert("Resources", PdfObject::Dictionary(leaf_resources));
        let leaf_id = updates
            .stream(b"q 40 0 0 30 0 0 cm /Im Do Q", leaf)
            .unwrap();

        let mut outer_xobjects = PdfDictionary::empty();
        outer_xobjects.insert("Leaf", reference(leaf_id));
        let mut outer_resources = PdfDictionary::empty();
        outer_resources.insert("XObject", PdfObject::Dictionary(outer_xobjects));
        let mut outer = PdfDictionary::empty();
        outer.insert("Type", PdfObject::Name("XObject".into()));
        outer.insert("Subtype", PdfObject::Name("Form".into()));
        outer.insert("FormType", PdfObject::Integer(1));
        outer.insert("BBox", array([0.0, 0.0, 40.0, 30.0]));
        outer.insert("Resources", PdfObject::Dictionary(outer_resources));
        let outer_id = updates
            .stream(b"q 1 0 0 1 0 0 cm /Leaf Do Q", outer)
            .unwrap();

        let mut page_xobjects = PdfDictionary::empty();
        page_xobjects.insert("Outer", reference(outer_id));
        let mut page_resources = PdfDictionary::empty();
        page_resources.insert("XObject", PdfObject::Dictionary(page_xobjects));
        let page_stream = updates
            .stream(
                b"q 1 0 0 1 20 30 cm /Outer Do Q q 1 0 0 1 120 30 cm /Outer Do Q",
                PdfDictionary::empty(),
            )
            .unwrap();

        for page in pages {
            let mut dictionary = reader
                .get_object(page.object_number, page.generation_number)
                .unwrap()
                .as_dict()
                .unwrap()
                .clone();
            if page.page_number == 1 {
                dictionary.insert("Resources", PdfObject::Dictionary(page_resources.clone()));
                dictionary.insert("Contents", reference(page_stream));
            }
            updates.put(
                (page.object_number, page.generation_number),
                PdfObject::Dictionary(dictionary),
            );
        }

        (
            write_incremental_update(reader, updates.objects).unwrap(),
            image_id,
            leaf_id,
            outer_id,
            page_stream,
        )
    }
    fn request(input: &[u8], stream: usize) -> ImageFragmentMove {
        let image = crate::universal_editing::universal_image_occurrences_v2(input, &[1])
            .unwrap()
            .into_iter()
            .find(|v| v.content_stream_index == stream)
            .unwrap();
        ImageFragmentMove {
            input_sha256: hash(input),
            source: ImageFragmentSource::Occurrence {
                page: 1,
                content_stream_index: stream,
                occurrence_id: image.occurrence_id,
            },
            target_page: 2,
            target_rect: [40.0, 60.0, 100.0, 100.0],
            stack: ImageFragmentStack::Foreground,
            ocr: None,
            signature_policy_override: false,
        }
    }
    fn apply(input: &[u8], r: &ImageFragmentMove) -> (Vec<u8>, ImageFragmentReport) {
        let p = preview_image_fragment_move(input, r).unwrap();
        apply_image_fragment_move(input, r, &p.plan_sha256).unwrap()
    }
    #[test]
    fn capture_context_shares_page_buffers_and_binds_revision_and_policy() {
        let (input, _, _) = fixture(true);
        let context = CaptureContext::new(&input, false, false).unwrap();
        let inventory =
            crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
        let a = prepare_known(
            &context,
            &request(&input, 0),
            Some(&inventory),
            None,
            None,
            None,
        )
        .unwrap();
        let total = context.buffers.borrow().total;
        let b = prepare_known(
            &context,
            &request(&input, 1),
            Some(&inventory),
            None,
            None,
            None,
        )
        .unwrap();
        assert!(Arc::ptr_eq(&a.source_buffers, &b.source_buffers));
        assert_eq!(context.buffers.borrow().total, total);
        assert_eq!(context.buffers.borrow().pages.len(), 2);
        assert_ne!(a.preview.key, b.preview.key);
        let mut stale = request(&input, 0);
        stale.input_sha256 = "0".repeat(64);
        assert!(prepare_known(&context, &stale, Some(&inventory), None, None, None).is_err());
        stale.input_sha256 = hash(&input);
        stale.signature_policy_override = true;
        assert!(prepare_known(&context, &stale, Some(&inventory), None, None, None).is_err());
        let separate = CaptureContext::new(&input, false, false).unwrap();
        let c = prepare_known(
            &separate,
            &request(&input, 0),
            Some(&inventory),
            None,
            None,
            None,
        )
        .unwrap();
        assert!(!Arc::ptr_eq(&a.source_buffers, &c.source_buffers));
    }
    #[test]
    fn batch_capture_scans_each_page_once_and_reuses_exact_capsules() {
        let (input, _, _) = fixture(true);
        let context = CaptureContext::new(&input, false, false).unwrap();
        let inventory =
            crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
        let selections = inventory
            .iter()
            .map(|occurrence| {
                (
                    occurrence.page,
                    occurrence.content_stream_index,
                    [
                        occurrence.operation_byte_start,
                        occurrence.operation_byte_end,
                    ],
                )
            })
            .collect::<Vec<_>>();
        let capsules =
            capture_occurrence_capsules(&context, &selections, 128 * 1024 * 1024).unwrap();
        assert_eq!(capsules.len(), selections.len());
        assert!(context.validated_pages.borrow().contains(&1));
        let baseline = CaptureContext::new(&input, false, false).unwrap();

        for stream in 0..2 {
            let prepared = prepare_known(
                &context,
                &request(&input, stream),
                Some(&inventory),
                None,
                None,
                Some(&capsules),
            )
            .unwrap();
            let occurrence = inventory
                .iter()
                .find(|occurrence| occurrence.content_stream_index == stream)
                .unwrap();
            let cached = capsules
                .get(&(
                    occurrence.page,
                    occurrence.content_stream_index,
                    occurrence.operation_byte_start,
                    occurrence.operation_byte_end,
                ))
                .unwrap();
            assert!(Arc::ptr_eq(prepared.capsule.as_ref().unwrap(), cached));
            let standalone = prepare_known(
                &baseline,
                &request(&input, stream),
                Some(&inventory),
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(prepared.capsule, standalone.capsule);
        }
        assert_eq!(context.validated_pages.borrow().len(), 2);
    }
    #[test]
    fn exact_occurrence_moves_without_reencoding_shared_image_or_mask() {
        let (input, image, stream) = fixture(true);
        let r = request(&input, 1);
        let (output, report) = apply(&input, &r);
        let original = ContentEngine::open_bytes(input).unwrap();
        let reopened = ContentEngine::open_bytes(output.clone()).unwrap();
        let page = reopened.document().get_page(1).unwrap();
        assert_eq!(page.contents[0], stream);
        assert_ne!(page.contents[1], stream);
        assert!(!decode(reopened.document().reader(), page.contents[1])
            .unwrap()
            .windows(6)
            .any(|w| w == b"/Im Do"));
        assert_eq!(
            original
                .document()
                .reader()
                .get_object(image.0, image.1)
                .unwrap(),
            reopened
                .document()
                .reader()
                .get_object(image.0, image.1)
                .unwrap()
        );
        let targets =
            crate::universal_editing::universal_image_occurrences_v2(&output, &[2]).unwrap();
        let moved = targets
            .iter()
            .find(|v| !v.invocation_path.is_empty())
            .unwrap();
        assert_eq!(moved.bbox, r.target_rect);
        assert_eq!(moved.object_number, Some(image.0));
        assert!(report.source_occurrence_removed);
        assert!(!report.historical_bytes_removed);
        assert_eq!(
            image_fragment_bindings(&output).unwrap(),
            vec![report.binding]
        );
    }

    #[test]
    fn nested_form_move_clones_only_the_selected_invocation_chain() {
        let (input, image, leaf, outer, page_stream) = nested_fixture();
        let occurrences =
            crate::universal_editing::universal_image_occurrences_v2(&input, &[1]).unwrap();
        assert_eq!(occurrences.len(), 2);
        assert!(occurrences
            .iter()
            .all(|occurrence| occurrence.invocation_path.len() == 2));
        let selected = occurrences
            .iter()
            .min_by(|left, right| left.bbox[0].total_cmp(&right.bbox[0]))
            .unwrap();
        let request = ImageFragmentMove {
            input_sha256: hash(&input),
            source: ImageFragmentSource::Occurrence {
                page: 1,
                content_stream_index: selected.content_stream_index,
                occurrence_id: selected.occurrence_id.clone(),
            },
            target_page: 2,
            target_rect: [50.0, 80.0, 110.0, 125.0],
            stack: ImageFragmentStack::Foreground,
            ocr: None,
            signature_policy_override: false,
        };
        let preview = preview_image_fragment_move(&input, &request).unwrap();
        assert!(preview.source_state_bytes > 0);
        assert!(apply_image_fragment_move(&input, &request, &"0".repeat(64)).is_err());
        let (output, report) =
            apply_image_fragment_move(&input, &request, &preview.plan_sha256).unwrap();

        let before = ContentEngine::open_bytes(input).unwrap();
        let after = ContentEngine::open_bytes(output.clone()).unwrap();
        for object in [image, leaf, outer, page_stream] {
            assert_eq!(
                before
                    .document()
                    .reader()
                    .get_object(object.0, object.1)
                    .unwrap(),
                after
                    .document()
                    .reader()
                    .get_object(object.0, object.1)
                    .unwrap()
            );
        }
        let source_occurrences =
            crate::universal_editing::universal_image_occurrences_v2(&output, &[1]).unwrap();
        assert_eq!(source_occurrences.len(), 1);
        assert_eq!(source_occurrences[0].object_number, Some(image.0));
        assert!(source_occurrences[0].bbox[0] > selected.bbox[0]);
        let target_occurrences =
            crate::universal_editing::universal_image_occurrences_v2(&output, &[2]).unwrap();
        assert_eq!(
            target_occurrences
                .iter()
                .filter(|occurrence| occurrence.object_number == Some(image.0))
                .count(),
            1
        );
        assert_eq!(report.binding.page, 2);
        assert_eq!(report.binding.rect, request.target_rect);
        assert!(report.source_occurrence_removed);
        assert!(report.output_reopened);
        assert_eq!(
            image_fragment_bindings(&output).unwrap(),
            vec![report.binding]
        );
    }
    #[test]
    fn repeat_move_reuses_capsule_and_retires_only_its_verified_edge_wrappers() {
        let (input, _, _) = fixture(false);
        let (mut output, mut report) = apply(&input, &request(&input, 0));
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let page = engine.document().get_page(2).unwrap();
        let mut total = 0;
        let first = owned_on_page(
            engine.document().reader(),
            &page,
            &page_buffers(engine.document().reader(), &page, &mut total).unwrap(),
        )
        .unwrap()
        .remove(0)
        .form;
        let count = page.contents.len();
        for n in 0..4 {
            let r = ImageFragmentMove {
                input_sha256: hash(&output),
                source: ImageFragmentSource::Owned {
                    binding: report.binding.clone(),
                },
                target_page: 2,
                target_rect: [40.0 + n as f64, 60.0, 100.0 + n as f64, 100.0],
                stack: ImageFragmentStack::Foreground,
                ocr: None,
                signature_policy_override: false,
            };
            (output, report) = apply(&output, &r);
            let e = ContentEngine::open_bytes(output.clone()).unwrap();
            let page = e.document().get_page(2).unwrap();
            let mut budget = 0;
            let owners = owned_on_page(
                e.document().reader(),
                &page,
                &page_buffers(e.document().reader(), &page, &mut budget).unwrap(),
            )
            .unwrap();
            assert_eq!(owners[0].form, first);
            assert_eq!(page.contents.len(), count);
        }
        let r = ImageFragmentMove {
            input_sha256: hash(&output),
            source: ImageFragmentSource::Owned {
                binding: report.binding,
            },
            target_page: 1,
            target_rect: [20.0, 80.0, 80.0, 120.0],
            stack: ImageFragmentStack::Background,
            ocr: None,
            signature_policy_override: false,
        };
        let (output, report) = apply(&output, &r);
        assert_eq!(report.binding.page, 1);
        assert_eq!(
            ContentEngine::open_bytes(output)
                .unwrap()
                .document()
                .get_page(2)
                .unwrap()
                .contents
                .len(),
            1
        );
    }
    #[test]
    fn preview_receipt_binds_geometry_order_and_current_input() {
        let (input, _, _) = fixture(false);
        let mut r = request(&input, 0);
        let p = preview_image_fragment_move(&input, &r).unwrap();
        r.target_rect[0] += 1.0;
        assert!(apply_image_fragment_move(&input, &r, &p.plan_sha256).is_err());
        r = request(&input, 0);
        r.stack = ImageFragmentStack::Background;
        assert!(apply_image_fragment_move(&input, &r, &p.plan_sha256).is_err());
        r.input_sha256 = "0".repeat(64);
        assert!(preview_image_fragment_move(&input, &r).is_err());
    }

    #[test]
    fn named_logical_properties_do_not_bypass_image_semantic_ownership() {
        let mut state = State::default();
        state.logical_properties.insert("Semantic".into());
        let bytes = b"/Artifact /Semantic BDC /Im Do EMC";
        let start = bytes.windows(6).position(|w| w == b"/Im Do").unwrap();
        assert!(state.feed(bytes, Some([start, start + 6]), true).is_err());
    }

    #[test]
    fn signed_zero_rebinds_and_nonfinite_placement_is_rejected_at_preview() {
        let (input, _, _) = fixture(false);
        let mut r = request(&input, 0);
        r.target_rect = [-0.0, 50.0, 60.0, 90.0];
        let (_, report) = apply(&input, &r);
        assert_eq!(report.binding.rect, r.target_rect);
        r.target_rect = [0.0, 0.0, 1e-300, 1.0];
        assert!(preview_image_fragment_move(&input, &r).is_err());
        r.target_rect = [0.0, 0.0, f64::INFINITY, 10.0];
        assert!(preview_image_fragment_move(&input, &r).is_err());
    }

    #[test]
    fn universal_route_requires_exact_candidate_approval_and_dispatches_native_move() {
        use crate::universal_editing::*;
        let (input, _, _) = fixture(false);
        let request = UniversalEditRequestV2 {
            operation: UniversalEditOperationV2::ImageFragment {
                request: request(&input, 0),
            },
            policy: UniversalEditPolicyV2::default(),
        };
        let plan = plan_universal_edit_v2(&input, &request).unwrap();
        assert_eq!(plan.state, UniversalPlanStateV2::ApprovalRequired);
        let mut decision = UniversalApprovalDecisionV2 {
            selected_candidate_ids: Vec::new(),
            approved_font: None,
            mutation_mode: plan.policy.mutation_mode,
            accept_visual_change: true,
            accept_signature_invalidation: false,
        };
        assert!(create_universal_approval_token_v2(&plan, decision.clone()).is_err());
        decision.selected_candidate_ids = plan.selected_candidate_ids.clone();
        let approval = create_universal_approval_token_v2(&plan, decision).unwrap();
        let (output, report) = apply_universal_edit_v2(&input, &plan, Some(&approval)).unwrap();
        assert_ne!(input, output);
        assert_eq!(image_fragment_bindings(&output).unwrap().len(), 1);
        assert_eq!(report.operation_report["source_occurrence_removed"], true);
    }
}
