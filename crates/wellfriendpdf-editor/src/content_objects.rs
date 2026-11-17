//! Every visible thing on a page, wherever it is drawn.
//!
//! The recognisers of `wellfriendpdf-edit` certify a few exact shapes
//! (`q cm /Im Do Q` without masks, `q [cm] [state] path paint Q`) on the
//! page's own content streams. This module reads the page content as a
//! renderer does (graphics-state stack, CTM, clipping, text state, nested
//! Form XObjects) and lists every image XObject placement, inline image
//! (`BI … EI`), path group (its construction operators up to the painting
//! operator), shading (`sh`), text run (QF-5211: the glyphs one line of a
//! text object shows, in any font kind including Type 3, with each glyph's
//! drawn box), group (`q … Q` drawing two or more objects) and optional
//! content (layer) span, with page-space bounds. An object is addressed by
//! its container (the page content, or a chain of Form XObject invocations)
//! and its operator range, checked by a digest of those operators.
//!
//! Edits never rewrite an object's own operators:
//! - move/resize wraps the range in `q X cm … Q` with `X = C·T·C⁻¹`, `C` the
//!   CTM where the range starts and `T` the page-space change, so the object
//!   lands where it was dragged whatever state it is drawn in;
//! - recolour wraps it in `q <colour> … Q` (colour operators inside a group
//!   or text run are left out of the wrapped copy);
//! - delete removes the range, and the resources nothing draws any more
//!   (QF-5212: fonts, images, forms, shadings) leave the file, so removed
//!   content is gone, not covered;
//! - bring to front / send to back moves the range to the end or start of
//!   its container, re-establishing the clip, colours and text state it was
//!   drawn with.
//!
//! A text run inside a longer text object (`BT … ET`) is cut out of it: the
//! object is closed before the run, the run is drawn in its own `q … BT …
//! ET Q` from the same text matrix, and the object resumes after it at the
//! pen position it had. A deleted run leaves a glyph-less `TJ` advance, so
//! the text after it on the line stays where it was.
//!
//! A path that paints and clips (`re W f`) is split: the painted copy is
//! edited and the clip (`re W n`) stays, so later content is clipped as
//! before. A form drawn by more than one placement (or reachable from more
//! than one place) is copied for the edited placement under a new resource
//! name (copy-on-write, as the Form XObject text edits do), and the form
//! boxes on the way grow to hold a moved object.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wellfriend_pdf_edit::{PdfMatrix, PdfObjectId};

use crate::error::EditorError;
use crate::fonts::{library_match, page_resources, FontSource, LibraryMatch};
use crate::ops::Rgb;
use crate::overlay;
use crate::text_fonts::{self, FontInfo, FontKind};

type M = [f64; 6];
const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// Nested Form XObjects followed when listing objects.
pub const MAX_FORM_DEPTH: usize = 12;
/// Objects listed per page (a page with more lists the first ones).
pub const MAX_PAGE_OBJECTS: usize = 5_000;
/// Hit-test boxes kept per object (glyphs, subpaths, group members).
pub const MAX_PARTS: usize = 300;
/// Clip paths and state operators a z-order move re-establishes.
const MAX_REPLAY_OPERATORS: usize = 4_000;

fn refused(message: impl Into<String>) -> EditorError {
    EditorError::new("EDITOR_OBJECT_REFUSED", message)
}

fn stale() -> EditorError {
    EditorError::new(
        "EDITOR_OBJECT_STALE",
        "the object changed since it was selected; select it again",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ContentObjectKind {
    /// An image XObject drawn with `Do`.
    Image,
    /// An inline image (`BI … ID … EI`).
    InlineImage,
    /// Path construction operators and their painting operator.
    Path,
    /// A shading drawn with `sh` (with the clip it fills, when it has one).
    Shading,
    /// The glyphs one line of a text object shows (or, for text that clips,
    /// the whole text object).
    Text,
    /// A `q … Q` block drawing two or more objects, or an optional content
    /// (layer) span `BDC … EMC`.
    Group,
}

/// One Form XObject invocation on the way to an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormStep {
    /// Index of the `Do` among its container's operators.
    pub invocation: usize,
    pub name: String,
    pub object: PdfObjectId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentObjectSelector {
    pub page_number: u32,
    /// Form XObjects from the page content down to the object's container.
    #[serde(default)]
    pub forms: Vec<FormStep>,
    pub kind: ContentObjectKind,
    /// Operator range `[start, end]` in the container (the page's content
    /// streams count as one sequence).
    pub start: usize,
    pub end: usize,
    /// SHA-256 of the range's operators as written.
    pub expected_sha256: String,
}

/// The font a text run is shown in, for the properties panel.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectFont {
    pub resource: String,
    pub base_font: String,
    pub subtype: String,
    pub subset: bool,
    pub embedded: bool,
    /// Font size times the text matrix scale (user space).
    pub size: f64,
    pub bold: bool,
    pub italic: bool,
    /// The library face used when the embedded font lacks a typed glyph.
    pub library_match: Option<LibraryMatch>,
}

/// How a text run is cut out of its text object (internal).
#[derive(Debug, Clone)]
pub(crate) struct ShowStep {
    index: usize,
    operator: String,
    /// The pen advance the operator makes, in text space.
    advance: f64,
    /// `Tfs × Th` when it is shown (a `TJ` number moves by `n/1000` of it).
    scale: f64,
    operands: Vec<Object>,
}

#[derive(Debug, Clone)]
pub(crate) struct TextRun {
    tlm_start: M,
    tm_start: M,
    tlm_end: M,
    tm_end: M,
    shows: Vec<ShowStep>,
    end_scale: f64,
    font: Arc<FontInfo>,
    font_object: Option<ObjectId>,
    size: f64,
    th: f64,
    ctm: M,
    /// Glyph height range in text space per unit size (for new bounds).
    ascent: f64,
    descent: f64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Replay {
    fill: Vec<Operation>,
    stroke: Vec<Operation>,
    misc: BTreeMap<String, Operation>,
    gs: Vec<Operation>,
    clips: Vec<(M, Vec<Operation>)>,
    size: usize,
    /// A clip the operators cannot re-establish (text clipping) or a state
    /// too large to repeat: the object cannot change its stacking order.
    incomplete: bool,
}

impl Replay {
    fn grow(&mut self, by: usize) {
        self.size += by;
        if self.size > MAX_REPLAY_OPERATORS {
            self.incomplete = true;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentObject {
    pub selector: ContentObjectSelector,
    /// Page-space bounds `[x0, y0, x1, y1]`.
    pub bounds: [f64; 4],
    /// Image space (the unit square) to page space, for images.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matrix: Option<[f64; 6]>,
    /// The painting operator of a path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paint: Option<String>,
    /// Approximate fill and stroke colours (RGB, 0 to 1) when they are plain
    /// colours; `None` for patterns and colour spaces that are not shown.
    pub fill: Option<[f64; 3]>,
    pub stroke: Option<[f64; 3]>,
    /// How the object is drawn: `inForm`, `inlineImage`, `softMask`,
    /// `stencilMask`, `clipped`, `clipAndPaint`, `nestedState`,
    /// `graphicsState`, `pattern`, `shading`; for text `type3`,
    /// `subsetFont`, `stroked`, `rotated`, `undecodable`, `colouredGlyphs`,
    /// `textClip`; `group`, `optionalContent`.
    pub traits: Vec<String>,
    /// The image XObject (images drawn with `Do`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_object: Option<PdfObjectId>,
    pub can_recolor: bool,
    /// The text a run shows (`None` when a glyph has no known Unicode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Page-space boxes of what is drawn (glyphs, subpaths, members), for
    /// hit-testing the drawn geometry rather than the bounds.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<[f64; 4]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<ObjectFont>,
    /// Text rendering mode (0 fill, 1 stroke, 2 fill and stroke, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub render_mode: Option<i64>,
    /// The optional content (layer) the object is drawn in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    /// Whether the object can move to the front or back of its container.
    pub can_reorder: bool,
    #[serde(skip)]
    pub(crate) insert_ctm: M,
    #[serde(skip)]
    pub(crate) clip_split: bool,
    #[serde(skip)]
    pub(crate) form_ctms: Vec<M>,
    /// Page-content objects: (stream, first and last operator in it).
    #[serde(skip)]
    pub(crate) span: Option<(ObjectId, usize, usize)>,
    #[serde(skip)]
    pub(crate) run: Option<Arc<TextRun>>,
    #[serde(skip)]
    pub(crate) replay: Arc<Replay>,
    #[serde(skip)]
    pub(crate) container_start_ctm: M,
    #[serde(skip)]
    pub(crate) container_end_ctm: Option<M>,
}

impl ContentObject {
    pub fn has(&self, name: &str) -> bool {
        self.traits.iter().any(|t| t == name)
    }

    /// Where the page content stream holds this object (page-level objects).
    pub fn page_span(&self) -> Option<(ObjectId, usize, usize)> {
        self.span
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageContentObjects {
    pub objects: Vec<ContentObject>,
    /// Image placements and paths that could not be read (a content stream
    /// or form that does not parse, forms nested deeper than the limit).
    pub unreadable_images: usize,
    pub unreadable_paths: usize,
    /// A page content stream does not parse: nothing on it is listed.
    pub content_unreadable: bool,
    pub truncated: bool,
}

/// What `editObject` does to the object.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ObjectEdit {
    /// Apply the page-space matrix `matrix` to the object (move, resize).
    Transform { matrix: PdfMatrix },
    Delete,
    /// Paint a path, group or text run in plain colours (either may be omitted).
    Recolor {
        #[serde(default)]
        fill: Option<Rgb>,
        #[serde(default)]
        stroke: Option<Rgb>,
    },
    /// Draw the object after everything else in its container.
    BringToFront,
    /// Draw the object before everything else in its container.
    SendToBack,
}

/// What an object edit does (`editObject`, `editObjectText`,
/// `replaceObjectWithText`).
pub enum ObjectAction<'a> {
    Edit(&'a ObjectEdit),
    /// Set the text a run shows, in its own font when that font has every
    /// glyph, else in `fallback` (or the closest library face), unless `font`
    /// names the library face to set it in.
    SetText {
        text: &'a str,
        expected: Option<&'a str>,
        font: Option<&'a str>,
        fallback: Option<&'a str>,
    },
    /// Replace any object with editable text in the library face `font` at
    /// its place and size.
    ReplaceWithText {
        text: &'a str,
        font: &'a str,
        size: Option<f64>,
        color: Option<&'a Rgb>,
    },
}

/// What an applied object edit did.
#[derive(Debug, Clone, Default)]
pub struct ObjectOutcome {
    pub bytes: Vec<u8>,
    pub changed: [f64; 4],
    pub notes: Vec<String>,
    pub fonts_added: Vec<String>,
}

// ---- matrices ----

/// `a` then `b` (PDF row-vector convention: `cm m` makes CTM = m × CTM).
pub(crate) fn mul(a: M, b: M) -> M {
    [
        a[0] * b[0] + a[1] * b[2],
        a[0] * b[1] + a[1] * b[3],
        a[2] * b[0] + a[3] * b[2],
        a[2] * b[1] + a[3] * b[3],
        a[4] * b[0] + a[5] * b[2] + b[4],
        a[4] * b[1] + a[5] * b[3] + b[5],
    ]
}

pub(crate) fn invert(m: M) -> Option<M> {
    let det = m[0] * m[3] - m[1] * m[2];
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
    Some([a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)])
}

fn apply(m: M, x: f64, y: f64) -> (f64, f64) {
    (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5])
}

pub(crate) fn transform_box(m: M, b: [f64; 4]) -> [f64; 4] {
    let mut out = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (x, y) in [(b[0], b[1]), (b[2], b[1]), (b[0], b[3]), (b[2], b[3])] {
        let (px, py) = apply(m, x, y);
        out = [out[0].min(px), out[1].min(py), out[2].max(px), out[3].max(py)];
    }
    out
}

fn translate(tx: f64, ty: f64) -> M {
    [1.0, 0.0, 0.0, 1.0, tx, ty]
}

fn near_identity(m: M) -> bool {
    m.iter().zip(IDENTITY.iter()).all(|(a, b)| (a - b).abs() < 1e-9)
}

/// The `cm` that takes the CTM from `from` to `to` (`None` when equal).
fn relative(from: M, to: M) -> Result<Option<M>, EditorError> {
    let inverse = invert(from).ok_or_else(|| refused("the object is drawn with a degenerate matrix"))?;
    let m = mul(to, inverse);
    Ok((!near_identity(m)).then_some(m))
}

fn number(object: &Object) -> Option<f64> {
    match object {
        Object::Integer(v) => Some(*v as f64),
        Object::Real(v) => Some(f64::from(*v)),
        _ => None,
    }
}

fn numbers(operands: &[Object]) -> Option<Vec<f64>> {
    operands.iter().map(number).collect()
}

fn matrix_of(operands: &[Object]) -> Option<M> {
    let v = numbers(operands)?;
    (v.len() == 6 && v.iter().all(|n| n.is_finite())).then(|| [v[0], v[1], v[2], v[3], v[4], v[5]])
}

fn matrix_object(m: M) -> Vec<Object> {
    m.iter().map(|v| real(*v)).collect()
}

fn real(value: f64) -> Object {
    let rounded = (value * 1e6).round() / 1e6;
    if rounded.fract() == 0.0 && rounded.abs() < 1e9 {
        Object::Integer(rounded as i64)
    } else {
        Object::Real(rounded as f32)
    }
}

fn intersect(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])],
    }
}

fn union(a: Option<[f64; 4]>, b: [f64; 4]) -> [f64; 4] {
    match a {
        None => b,
        Some(a) => [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])],
    }
}

fn union_all(boxes: &[[f64; 4]]) -> Option<[f64; 4]> {
    boxes.iter().fold(None, |acc, b| Some(union(acc, *b)))
}

fn area(b: [f64; 4]) -> f64 {
    (b[2] - b[0]).max(0.0) * (b[3] - b[1]).max(0.0)
}

/// At most `MAX_PARTS` boxes: neighbours merged when there are more.
fn capped(mut boxes: Vec<[f64; 4]>) -> Vec<[f64; 4]> {
    while boxes.len() > MAX_PARTS {
        boxes = boxes.chunks(2).map(|pair| union_all(pair).expect("a non-empty chunk")).collect();
    }
    boxes
}

// ---- reading ----

#[derive(Debug, Clone, PartialEq)]
enum Paint {
    Rgb([f64; 3]),
    Pattern,
    Other,
}

#[derive(Debug, Clone)]
struct TextParams {
    font: Option<Vec<u8>>,
    size: f64,
    tc: f64,
    tw: f64,
    tz: f64,
    tl: f64,
    ts: f64,
    tr: i64,
}

impl Default for TextParams {
    fn default() -> Self {
        Self {
            font: None,
            size: 0.0,
            tc: 0.0,
            tw: 0.0,
            tz: 100.0,
            tl: 0.0,
            ts: 0.0,
            tr: 0,
        }
    }
}

#[derive(Debug, Clone)]
struct State {
    ctm: M,
    fill: Paint,
    stroke: Paint,
    fill_pattern_space: bool,
    stroke_pattern_space: bool,
    /// Page-space bounds of the clip (clip paths and form boxes).
    clip: Option<[f64; 4]>,
    /// A clip path (`W n`) is in effect.
    clipped: bool,
    graphics_state: bool,
    text: TextParams,
    replay: Arc<Replay>,
}

impl State {
    fn new(ctm: M) -> Self {
        Self {
            ctm,
            fill: Paint::Rgb([0.0; 3]),
            stroke: Paint::Rgb([0.0; 3]),
            fill_pattern_space: false,
            stroke_pattern_space: false,
            clip: None,
            clipped: false,
            graphics_state: false,
            text: TextParams::default(),
            replay: Arc::new(Replay::default()),
        }
    }

    fn replay(&mut self) -> &mut Replay {
        Arc::make_mut(&mut self.replay)
    }
}

fn colour(values: &[f64]) -> Paint {
    match values.len() {
        1 => Paint::Rgb([values[0]; 3]),
        3 => Paint::Rgb([values[0], values[1], values[2]]),
        4 => {
            let k = values[3];
            Paint::Rgb([
                (1.0 - values[0]) * (1.0 - k),
                (1.0 - values[1]) * (1.0 - k),
                (1.0 - values[2]) * (1.0 - k),
            ])
        }
        _ => Paint::Other,
    }
}

fn rgb_of(paint: &Paint) -> Option<[f64; 3]> {
    match paint {
        Paint::Rgb(v) => Some(v.map(|c| c.clamp(0.0, 1.0))),
        _ => None,
    }
}

const PATH_CONSTRUCTION: [&str; 7] = ["m", "l", "c", "v", "y", "h", "re"];
const PAINT: [&str; 10] = ["S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "n"];
const DRAWING: [&str; 16] = [
    "S", "s", "f", "F", "f*", "B", "B*", "b", "b*", "Do", "BI", "sh", "Tj", "TJ", "'", "\"",
];
const SHOW: [&str; 4] = ["Tj", "TJ", "'", "\""];
const FILL_COLOUR: [&str; 6] = ["g", "rg", "k", "cs", "sc", "scn"];
const STROKE_COLOUR: [&str; 6] = ["G", "RG", "K", "CS", "SC", "SCN"];
/// State operators that persist after the operator range they occur in.
const STATE: [&str; 23] = [
    "w", "J", "j", "M", "d", "ri", "i", "gs", "g", "G", "rg", "RG", "k", "K", "cs", "CS", "sc", "SC", "scn", "SCN", "Tc", "Tw", "Tz",
];
const TEXT_STATE: [&str; 4] = ["TL", "Tf", "Tr", "Ts"];

fn is_state(op: &str) -> bool {
    STATE.contains(&op) || TEXT_STATE.contains(&op)
}

pub(crate) fn digest(operations: &[Operation]) -> String {
    let encoded = Content {
        operations: operations.to_vec(),
    }
    .encode()
    .unwrap_or_default();
    let hash = Sha256::digest(&encoded);
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

fn resolve<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        other => Some(other),
    }
}

fn resolve_dict<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object)? {
        Object::Dictionary(d) => Some(d),
        Object::Stream(s) => Some(&s.dict),
        _ => None,
    }
}

/// The XObject a resource name resolves to.
fn xobject(document: &Document, resources: Option<&Dictionary>, name: &[u8]) -> Option<ObjectId> {
    let resources = resources?;
    let xobjects = resolve_dict(document, resources.get(b"XObject").ok()?)?;
    xobjects.get(name).ok()?.as_reference().ok()
}

fn subtype(stream: &Stream) -> &[u8] {
    stream
        .dict
        .get(b"Subtype")
        .and_then(Object::as_name)
        .unwrap_or_default()
}

fn decode_stream(stream: &Stream) -> Option<Vec<Operation>> {
    let plain = if stream.dict.has(b"Filter") {
        stream.decompressed_content().ok()?
    } else {
        stream.content.clone()
    };
    Content::decode(&plain).ok().map(|c| c.operations)
}

/// Operators and, for each, its content stream and index in it.
pub(crate) type PageOperations = (Vec<Operation>, Vec<(ObjectId, usize)>);

/// The page's content streams as one operator sequence, with the stream and
/// local index of every operator. `None` when a stream does not parse.
pub(crate) fn page_operations(
    document: &Document,
    page: ObjectId,
) -> Option<PageOperations> {
    let mut operations = Vec::new();
    let mut origin = Vec::new();
    for id in document.get_page_contents(page) {
        let stream = document.get_object(id).ok()?.as_stream().ok()?;
        let ops = decode_stream(stream)?;
        origin.extend((0..ops.len()).map(|local| (id, local)));
        operations.extend(ops);
    }
    Some((operations, origin))
}

/// Text of a PDF text string (UTF-16BE with a byte order mark, else bytes).
fn text_string(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..].chunks(2).filter(|p| p.len() == 2).map(|p| u16::from_be_bytes([p[0], p[1]])).collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|b| char::from(*b)).collect()
    }
}

/// A text run being read.
struct RunBuilder {
    first: usize,
    last: usize,
    tlm_start: M,
    tm_start: M,
    shows: Vec<ShowStep>,
    boxes: Vec<[f64; 4]>,
    text: String,
    decodable: bool,
    font_name: Vec<u8>,
    font: Arc<FontInfo>,
    font_object: Option<ObjectId>,
    size: f64,
    th: f64,
    render_mode: i64,
    fill: Paint,
    stroke: Paint,
    type3: bool,
    stroked: bool,
    end_pen: (f64, f64),
    tlm_after: M,
    tm_after: M,
    state: State,
    layer: Option<String>,
    marked: usize,
}

/// One `BT … ET` being read.
struct TextBlock {
    start: usize,
    tm: M,
    tlm: M,
    runs: Vec<(RunBuilder, M, M, f64)>,
    current: Option<RunBuilder>,
    clip_modes: bool,
    clip_boxes: Vec<[f64; 4]>,
    visible_boxes: Vec<[f64; 4]>,
    paint_clip_visible: bool,
    state_at_start: State,
}

struct Walker<'a> {
    document: &'a Document,
    page: u32,
    media_box: [f64; 4],
    out: PageContentObjects,
    texts: Vec<ContentObject>,
    groups: Vec<ContentObject>,
    fonts: HashMap<ObjectId, Arc<FontInfo>>,
    /// Codes shown per font (a subset font surely has these glyphs).
    known_codes: HashMap<ObjectId, BTreeSet<u32>>,
    layers: Vec<Option<String>>,
}

struct Container<'a> {
    operations: &'a [Operation],
    resources: Option<Dictionary>,
    forms: Vec<FormStep>,
    form_ctms: Vec<M>,
    origin: Option<&'a [(ObjectId, usize)]>,
    depth: usize,
    start_ctm: M,
}

/// What the walk leaves at the end of a container.
struct Ending {
    ctm: M,
    in_text: bool,
}

impl Walker<'_> {
    fn total(&self) -> usize {
        self.out.objects.len() + self.texts.len() + self.groups.len()
    }

    fn push(&mut self, object: ContentObject) {
        if self.total() >= MAX_PAGE_OBJECTS {
            self.out.truncated = true;
            return;
        }
        match object.selector.kind {
            ContentObjectKind::Text => self.texts.push(object),
            ContentObjectKind::Group => self.groups.push(object),
            _ => self.out.objects.push(object),
        }
    }

    fn mark(&self) -> (usize, usize, usize) {
        (self.out.objects.len(), self.texts.len(), self.groups.len())
    }

    /// Objects pushed since `mark`.
    fn since(&self, mark: (usize, usize, usize)) -> Vec<&ContentObject> {
        self.out.objects[mark.0.min(self.out.objects.len())..]
            .iter()
            .chain(self.texts[mark.1.min(self.texts.len())..].iter())
            .chain(self.groups[mark.2.min(self.groups.len())..].iter())
            .collect()
    }

    fn layer(&self) -> Option<String> {
        self.layers.iter().rev().find_map(|l| l.clone())
    }

    #[allow(clippy::too_many_arguments)]
    fn object(
        &mut self,
        container: &Container<'_>,
        kind: ContentObjectKind,
        start: usize,
        end: usize,
        bounds: [f64; 4],
        state: &State,
        insert_ctm: M,
        mut traits: Vec<&'static str>,
        nested: bool,
    ) -> ContentObject {
        if !container.forms.is_empty() {
            traits.push("inForm");
        }
        if state.clipped {
            traits.push("clipped");
        }
        if nested {
            traits.push("nestedState");
        }
        if state.graphics_state {
            traits.push("graphicsState");
        }
        let layer = self.layer();
        if self.layers.iter().any(Option::is_some) {
            traits.push("optionalContent");
        }
        let span = container.origin.and_then(|origin| {
            let (first, local_start) = origin.get(start).copied()?;
            let (last, local_end) = origin.get(end).copied()?;
            (first == last).then_some((first, local_start, local_end))
        });
        ContentObject {
            selector: ContentObjectSelector {
                page_number: self.page,
                forms: container.forms.clone(),
                kind,
                start,
                end,
                expected_sha256: digest(&container.operations[start..=end]),
            },
            bounds,
            matrix: None,
            paint: None,
            fill: None,
            stroke: None,
            traits: traits.into_iter().map(str::to_owned).collect(),
            image_object: None,
            can_recolor: false,
            text: None,
            parts: Vec::new(),
            font: None,
            render_mode: None,
            layer,
            can_reorder: !state.replay.incomplete,
            insert_ctm,
            clip_split: false,
            form_ctms: container.form_ctms.clone(),
            span,
            run: None,
            replay: state.replay.clone(),
            container_start_ctm: container.start_ctm,
            container_end_ctm: None,
        }
    }

    fn font(&mut self, resources: Option<&Dictionary>, name: &[u8]) -> Option<(Arc<FontInfo>, Option<ObjectId>)> {
        let fonts = resolve_dict(self.document, resources?.get(b"Font").ok()?)?;
        let entry = fonts.get(name).ok()?;
        match entry {
            Object::Reference(id) => {
                if let Some(info) = self.fonts.get(id) {
                    return Some((info.clone(), Some(*id)));
                }
                let dict = resolve_dict(self.document, entry)?;
                let info = Arc::new(text_fonts::load(self.document, dict));
                self.fonts.insert(*id, info.clone());
                Some((info, Some(*id)))
            }
            Object::Dictionary(dict) => Some((Arc::new(text_fonts::load(self.document, dict)), None)),
            _ => None,
        }
    }

    /// The name of the optional content group a `BDC /OC` names.
    fn layer_name(&self, resources: Option<&Dictionary>, operand: Option<&Object>) -> String {
        let dict = match operand {
            Some(Object::Name(name)) => resources
                .and_then(|r| r.get(b"Properties").ok())
                .and_then(|p| resolve_dict(self.document, p))
                .and_then(|p| p.get(name).ok())
                .and_then(|o| resolve_dict(self.document, o)),
            Some(other) => resolve_dict(self.document, other),
            None => None,
        };
        dict.and_then(|d| d.get(b"Name").ok())
            .and_then(|n| n.as_str().ok())
            .map(text_string)
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| "Layer".to_owned())
    }

    fn walk(&mut self, container: &Container<'_>, initial: State) -> Ending {
        let operations = container.operations;
        let mut state = initial;
        let mut stack: Vec<(State, usize, (usize, usize, usize))> = Vec::new();
        let mut path_start: Option<usize> = None;
        let mut path_points: Vec<(f64, f64)> = Vec::new();
        let mut subpaths: Vec<[f64; 4]> = Vec::new();
        let mut path_clip = false;
        let mut block: Option<TextBlock> = None;
        // Text objects that only clip (render mode 7), by operator range.
        let mut clip_only_text: Vec<(usize, usize)> = Vec::new();
        // Marked content: (index, layer, mark, q depth, in text, state).
        let mut marked: Vec<(usize, Option<String>, (usize, usize, usize), usize, bool, State)> = Vec::new();
        for (index, operation) in operations.iter().enumerate() {
            let op = operation.operator.as_str();
            let operands = &operation.operands;
            let in_text = block.is_some();
            match op {
                "q" => stack.push((state.clone(), index, self.mark())),
                "Q" => {
                    if let Some((saved, q_index, mark)) = stack.pop() {
                        state = saved.clone();
                        if !in_text {
                            self.group(container, q_index, index, mark, &saved, stack.len());
                        }
                    }
                }
                "cm" => {
                    if let Some(m) = matrix_of(operands) {
                        state.ctm = mul(m, state.ctm);
                    }
                }
                "BT" => {
                    block = Some(TextBlock {
                        start: index,
                        tm: IDENTITY,
                        tlm: IDENTITY,
                        runs: Vec::new(),
                        current: None,
                        clip_modes: false,
                        clip_boxes: Vec::new(),
                        visible_boxes: Vec::new(),
                        paint_clip_visible: false,
                        state_at_start: state.clone(),
                    });
                }
                "ET" => {
                    if let Some(mut text) = block.take() {
                        self.close_run(&mut text);
                        if let Some(range) = self.finish_text(container, text, index, &mut state, stack.len()) {
                            clip_only_text.push(range);
                        }
                    }
                }
                "BDC" | "BMC" => {
                    let oc = op == "BDC" && operands.first().and_then(|o| o.as_name().ok()) == Some(b"OC".as_slice());
                    let layer = oc.then(|| self.layer_name(container.resources.as_ref(), operands.get(1)));
                    self.layers.push(layer.clone());
                    marked.push((index, layer, self.mark(), stack.len(), in_text, state.clone()));
                }
                "EMC" => {
                    self.layers.pop();
                    if let Some((start, Some(layer), mark, depth, started_in_text, at_start)) = marked.pop() {
                        if !started_in_text && !in_text && depth == stack.len() {
                            self.layer_group(container, start, index, mark, &at_start, layer);
                        }
                    }
                }
                "gs" => {
                    state.graphics_state = true;
                    state.replay().gs.push(operation.clone());
                    state.replay().grow(1);
                }
                "g" | "rg" | "k" => {
                    state.fill = numbers(operands).map(|v| colour(&v)).unwrap_or(Paint::Other);
                    state.fill_pattern_space = false;
                    state.replay().fill = vec![operation.clone()];
                }
                "G" | "RG" | "K" => {
                    state.stroke = numbers(operands).map(|v| colour(&v)).unwrap_or(Paint::Other);
                    state.stroke_pattern_space = false;
                    state.replay().stroke = vec![operation.clone()];
                }
                "cs" | "CS" => {
                    let pattern = operands.first().and_then(|o| o.as_name().ok()) == Some(b"Pattern".as_slice())
                        || operands
                            .first()
                            .and_then(|o| o.as_name().ok())
                            .and_then(|name| colour_space_family(self.document, container.resources.as_ref(), name))
                            .is_some_and(|family| family == b"Pattern");
                    let initial = if pattern { Paint::Pattern } else { Paint::Rgb([0.0; 3]) };
                    if op == "cs" {
                        state.fill = initial;
                        state.fill_pattern_space = pattern;
                        state.replay().fill = vec![operation.clone()];
                    } else {
                        state.stroke = initial;
                        state.stroke_pattern_space = pattern;
                        state.replay().stroke = vec![operation.clone()];
                    }
                }
                "sc" | "scn" | "SC" | "SCN" => {
                    let fill = op.starts_with('s');
                    let pattern_space = if fill { state.fill_pattern_space } else { state.stroke_pattern_space };
                    let paint = if pattern_space || operands.last().is_some_and(|o| o.as_name().is_ok()) {
                        Paint::Pattern
                    } else {
                        numbers(operands).map(|v| colour(&v)).unwrap_or(Paint::Other)
                    };
                    let replay = state.replay();
                    let list = if fill { &mut replay.fill } else { &mut replay.stroke };
                    list.retain(|o| o.operator == "cs" || o.operator == "CS");
                    list.push(operation.clone());
                    if fill {
                        state.fill = paint;
                    } else {
                        state.stroke = paint;
                    }
                }
                "w" | "J" | "j" | "M" | "d" | "ri" | "i" => {
                    state.replay().misc.insert(op.to_owned(), operation.clone());
                }
                "Tf" => {
                    state.text.font = operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec);
                    state.text.size = operands.get(1).and_then(number).unwrap_or(0.0);
                    state.replay().misc.insert(op.to_owned(), operation.clone());
                }
                "Tc" | "Tw" | "Tz" | "TL" | "Ts" | "Tr" => {
                    let value = operands.first().and_then(number).unwrap_or(0.0);
                    match op {
                        "Tc" => state.text.tc = value,
                        "Tw" => state.text.tw = value,
                        "Tz" => state.text.tz = value,
                        "TL" => state.text.tl = value,
                        "Ts" => state.text.ts = value,
                        _ => state.text.tr = value as i64,
                    }
                    state.replay().misc.insert(op.to_owned(), operation.clone());
                }
                "Td" | "TD" | "Tm" | "T*" if in_text => {
                    let text = block.as_mut().expect("in a text object");
                    match op {
                        "Tm" => {
                            if let Some(m) = matrix_of(operands) {
                                text.tlm = m;
                                text.tm = m;
                            }
                        }
                        "T*" => {
                            text.tlm = mul(translate(0.0, -state.text.tl), text.tlm);
                            text.tm = text.tlm;
                        }
                        _ => {
                            let v = numbers(operands).unwrap_or_default();
                            if v.len() == 2 {
                                if op == "TD" {
                                    state.text.tl = -v[1];
                                }
                                text.tlm = mul(translate(v[0], v[1]), text.tlm);
                                text.tm = text.tlm;
                            }
                        }
                    }
                }
                _ if SHOW.contains(&op) && in_text => {
                    let mut text = block.take().expect("in a text object");
                    self.show(container, &mut state, &mut text, index, operation);
                    block = Some(text);
                }
                _ if PATH_CONSTRUCTION.contains(&op) && !in_text => {
                    path_start.get_or_insert(index);
                    if let Some(v) = numbers(operands) {
                        if op == "re" && v.len() == 4 {
                            let corners = [(v[0], v[1]), (v[0] + v[2], v[1]), (v[0], v[1] + v[3]), (v[0] + v[2], v[1] + v[3])];
                            let mut sub: Option<[f64; 4]> = None;
                            for (x, y) in corners {
                                let p = apply(state.ctm, x, y);
                                path_points.push(p);
                                sub = Some(union(sub, [p.0, p.1, p.0, p.1]));
                            }
                            subpaths.extend(sub);
                        } else {
                            for pair in v.as_chunks::<2>().0 {
                                let p = apply(state.ctm, pair[0], pair[1]);
                                path_points.push(p);
                                if op == "m" || subpaths.is_empty() {
                                    subpaths.push([p.0, p.1, p.0, p.1]);
                                } else if let Some(last) = subpaths.last_mut() {
                                    *last = union(Some(*last), [p.0, p.1, p.0, p.1]);
                                }
                            }
                        }
                    }
                }
                "W" | "W*" if path_start.is_some() => path_clip = true,
                _ if PAINT.contains(&op) => {
                    let Some(start) = path_start.take() else {
                        continue;
                    };
                    let points = std::mem::take(&mut path_points);
                    let parts = std::mem::take(&mut subpaths);
                    let clip = std::mem::replace(&mut path_clip, false);
                    let bounds = points.iter().fold(
                        [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                        |b, (x, y)| [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)],
                    );
                    let finite = bounds.iter().all(|v| v.is_finite());
                    if clip {
                        // The clip this path sets, as operators that set it again.
                        let mut ops: Vec<Operation> = operations[start..index]
                            .iter()
                            .filter(|o| PATH_CONSTRUCTION.contains(&o.operator.as_str()))
                            .cloned()
                            .collect();
                        let clip_op = operations[start..index]
                            .iter()
                            .find(|o| o.operator == "W" || o.operator == "W*")
                            .map(|o| o.operator.clone())
                            .unwrap_or_else(|| "W".into());
                        ops.push(Operation::new(&clip_op, vec![]));
                        ops.push(Operation::new("n", vec![]));
                        let count = ops.len();
                        let ctm = state.ctm;
                        let replay = state.replay();
                        replay.clips.push((ctm, ops));
                        replay.grow(count);
                    }
                    if op == "n" {
                        if clip && finite {
                            state.clip = Some(intersect(state.clip, bounds));
                            state.clipped = true;
                        }
                        continue;
                    }
                    if !finite {
                        self.out.unreadable_paths += 1;
                        continue;
                    }
                    let stroking = matches!(op, "S" | "s" | "B" | "B*" | "b" | "b*");
                    let filling = op != "S" && op != "s";
                    let mut traits = Vec::new();
                    if clip {
                        traits.push("clipAndPaint");
                    }
                    if (filling && state.fill == Paint::Pattern) || (stroking && state.stroke == Paint::Pattern) {
                        traits.push("pattern");
                    }
                    let mut object = self.object(
                        container,
                        ContentObjectKind::Path,
                        start,
                        index,
                        bounds,
                        &state,
                        state.ctm,
                        traits,
                        stack.len() > 1,
                    );
                    object.paint = Some(op.to_owned());
                    object.fill = filling.then(|| rgb_of(&state.fill)).flatten();
                    object.stroke = stroking.then(|| rgb_of(&state.stroke)).flatten();
                    object.clip_split = clip;
                    object.can_recolor = true;
                    if parts.len() > 1 {
                        object.parts = capped(parts);
                    }
                    if clip {
                        state.clip = Some(intersect(state.clip, bounds));
                        state.clipped = true;
                    }
                    self.push(object);
                }
                "BI" => {
                    let Some(Object::Stream(image)) = operands.first() else {
                        self.out.unreadable_images += 1;
                        continue;
                    };
                    let mut traits = vec!["inlineImage"];
                    if image
                        .dict
                        .get(b"IM")
                        .or_else(|_| image.dict.get(b"ImageMask"))
                        .and_then(Object::as_bool)
                        .unwrap_or(false)
                    {
                        traits.push("stencilMask");
                    }
                    let bounds = transform_box(state.ctm, [0.0, 0.0, 1.0, 1.0]);
                    let mut object = self.object(
                        container,
                        ContentObjectKind::InlineImage,
                        index,
                        index,
                        bounds,
                        &state,
                        state.ctm,
                        traits,
                        stack.len() > 1,
                    );
                    object.matrix = Some(state.ctm);
                    self.push(object);
                }
                "sh" => {
                    // A shading inside `q … Q` that only sets state and clips
                    // (with paths, or text that only clips) before it, with the
                    // `Q` right after, is one object with its clip; otherwise
                    // the `sh` alone.
                    let block_range = stack.last().and_then(|(saved, q, _)| {
                        let clip_text = |i: usize| clip_only_text.iter().any(|(s, e)| *s <= i && i <= *e);
                        let clean = operations.get(index + 1).is_some_and(|o| o.operator == "Q")
                            && operations[q + 1..index].iter().enumerate().all(|(offset, o)| {
                                let i = q + 1 + offset;
                                clip_text(i)
                                    || (!DRAWING.contains(&o.operator.as_str()) && o.operator != "q" && o.operator != "Q" && o.operator != "BT")
                            });
                        clean.then_some((q + 1, saved.ctm, saved.replay.clone()))
                    });
                    let (start, insert_ctm, replay) = block_range.unwrap_or((index, state.ctm, state.replay.clone()));
                    let bbox = operands
                        .first()
                        .and_then(|o| o.as_name().ok())
                        .and_then(|name| shading_bbox(self.document, container.resources.as_ref(), name))
                        .map(|b| transform_box(state.ctm, b));
                    let page = self.media_box;
                    let mut bounds = state.clip.unwrap_or(page);
                    if let Some(b) = bbox {
                        bounds = intersect(Some(bounds), b);
                    }
                    bounds = intersect(Some(bounds), page);
                    if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
                        bounds = page;
                    }
                    let mut object = self.object(
                        container,
                        ContentObjectKind::Shading,
                        start,
                        index,
                        bounds,
                        &state,
                        insert_ctm,
                        vec!["shading"],
                        stack.len() > 1,
                    );
                    object.replay = replay.clone();
                    object.can_reorder = !replay.incomplete;
                    self.push(object);
                }
                "Do" => {
                    let Some(name) = operands.first().and_then(|o| o.as_name().ok()) else {
                        continue;
                    };
                    let Some(id) = xobject(self.document, container.resources.as_ref(), name) else {
                        continue;
                    };
                    let Ok(stream) = self.document.get_object(id).and_then(Object::as_stream) else {
                        continue;
                    };
                    match subtype(stream) {
                        b"Image" => {
                            let mut traits = Vec::new();
                            if stream.dict.has(b"SMask") || stream.dict.has(b"Mask") {
                                traits.push("softMask");
                            }
                            if stream.dict.get(b"ImageMask").and_then(Object::as_bool).unwrap_or(false) {
                                traits.push("stencilMask");
                            }
                            let bounds = transform_box(state.ctm, [0.0, 0.0, 1.0, 1.0]);
                            let mut object = self.object(
                                container,
                                ContentObjectKind::Image,
                                index,
                                index,
                                bounds,
                                &state,
                                state.ctm,
                                traits,
                                stack.len() > 1,
                            );
                            object.matrix = Some(state.ctm);
                            object.image_object = Some(id.into());
                            self.push(object);
                        }
                        b"Form" => self.form(container, &state, index, name, id, stream),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        Ending {
            ctm: state.ctm,
            in_text: block.is_some(),
        }
    }

    /// A `q … Q` that drew two or more objects becomes a group (not the
    /// block that wraps a whole page, nor one that covers nearly all of it).
    fn group(&mut self, container: &Container<'_>, q: usize, end: usize, mark: (usize, usize, usize), saved: &State, depth: usize) {
        let members = self.since(mark);
        if members.len() < 2 {
            return;
        }
        let whole = container.forms.is_empty() && q == 0 && end + 1 == container.operations.len();
        let bounds = members.iter().fold(None, |acc, m| Some(union(acc, m.bounds)));
        let Some(bounds) = bounds else { return };
        let page = self.media_box;
        if whole || area(bounds) >= 0.85 * area(page) {
            return;
        }
        let can_recolor = members.iter().any(|m| m.can_recolor);
        let parts: Vec<[f64; 4]> = members.iter().filter(|m| m.selector.kind != ContentObjectKind::Group).map(|m| m.bounds).collect();
        let mut object = self.object(container, ContentObjectKind::Group, q, end, bounds, saved, saved.ctm, vec!["group"], depth > 0);
        object.can_recolor = can_recolor;
        object.parts = capped(parts);
        self.push(object);
    }

    fn layer_group(&mut self, container: &Container<'_>, start: usize, end: usize, mark: (usize, usize, usize), at_start: &State, layer: String) {
        let members = self.since(mark);
        if members.is_empty() {
            return;
        }
        let Some(bounds) = members.iter().fold(None, |acc, m| Some(union(acc, m.bounds))) else { return };
        let can_recolor = members.iter().any(|m| m.can_recolor);
        let parts: Vec<[f64; 4]> = members.iter().filter(|m| m.selector.kind != ContentObjectKind::Group).map(|m| m.bounds).collect();
        // The span's own BDC is still open while its members are read; the
        // group itself is in the layer.
        let mut object = self.object(container, ContentObjectKind::Group, start, end, bounds, at_start, at_start.ctm, vec!["group", "optionalContent"], false);
        object.layer = Some(layer);
        object.can_recolor = can_recolor;
        object.parts = capped(parts);
        self.push(object);
    }

    /// Glyph boxes, text and pen advances of one text-showing operator.
    fn show(&mut self, container: &Container<'_>, state: &mut State, text: &mut TextBlock, index: usize, operation: &Operation) {
        let op = operation.operator.as_str();
        let operands = &operation.operands;
        let tlm_before = text.tlm;
        let tm_before = text.tm;
        // `'` and `"` move to the next line first (and `"` sets spacing).
        if op == "\"" {
            if let (Some(aw), Some(ac)) = (operands.first().and_then(number), operands.get(1).and_then(number)) {
                state.text.tw = aw;
                state.text.tc = ac;
            }
        }
        if op == "'" || op == "\"" {
            text.tlm = mul(translate(0.0, -state.text.tl), text.tlm);
            text.tm = text.tlm;
        }
        let Some(font_name) = state.text.font.clone() else { return };
        let Some((font, font_object)) = self.font(container.resources.as_ref(), &font_name) else {
            return;
        };
        let tfs = state.text.size;
        let th = state.text.tz / 100.0;
        let mode = state.text.tr;
        let visible = !matches!(mode, 3 | 7);
        if (4..=7).contains(&mode) {
            text.clip_modes = true;
        }
        let pen_page = apply(mul(text.tm, state.ctm), 0.0, 0.0);
        // A new line, a new column or a jump back starts a new run.
        let (continuous, spaced) = match &text.current {
            Some(run) if op != "'" && op != "\"" => {
                let frame = mul(run.tm_start, state.ctm);
                match invert(frame) {
                    Some(inverse) => {
                        let q = apply(inverse, pen_page.0, pen_page.1);
                        let e = apply(inverse, run.end_pen.0, run.end_pen.1);
                        let size = run.size.abs().max(tfs.abs()).max(1e-3);
                        let same_line = q.1.abs() <= 0.35 * size && q.0 >= e.0 - 0.6 * size && q.0 <= e.0 + 2.5 * size;
                        (same_line, q.0 - e.0 > 0.2 * size)
                    }
                    None => (false, false),
                }
            }
            _ => (false, false),
        };
        if !continuous {
            self.close_run(text);
            text.current = Some(RunBuilder {
                first: index,
                last: index,
                tlm_start: tlm_before,
                tm_start: tm_before,
                shows: Vec::new(),
                boxes: Vec::new(),
                text: String::new(),
                decodable: true,
                font_name: font_name.clone(),
                font: font.clone(),
                font_object,
                size: tfs,
                th,
                render_mode: mode,
                fill: state.fill.clone(),
                stroke: state.stroke.clone(),
                type3: false,
                stroked: false,
                end_pen: pen_page,
                tlm_after: tlm_before,
                tm_after: tm_before,
                state: state.clone(),
                layer: self.layer(),
                marked: self.layers.len(),
            });
        }
        let ctm = state.ctm;
        let strings: Vec<&Object> = match op {
            "TJ" => operands.first().and_then(|o| o.as_array().ok()).map(|a| a.iter().collect()).unwrap_or_default(),
            "\"" => operands.get(2).into_iter().collect(),
            _ => operands.first().into_iter().collect(),
        };
        let start_x = text.tm;
        let mut boxes = Vec::new();
        let mut decoded = String::new();
        let mut decodable = true;
        for item in strings {
            match item {
                Object::String(bytes, _) => {
                    for code in font.codes(bytes) {
                        if let Some(id) = font_object {
                            self.known_codes.entry(id).or_default().insert(code);
                        }
                        let w0 = font.width(code);
                        let glyph = [0.0, font.descent * tfs + state.text.ts, w0 * tfs * th, font.ascent * tfs + state.text.ts];
                        let page_box = transform_box(mul(text.tm, ctm), glyph);
                        if visible && page_box.iter().all(|v| v.is_finite()) && (glyph[2] - glyph[0]).abs() > 0.0 {
                            boxes.push(page_box);
                        }
                        if (4..=7).contains(&mode) && page_box.iter().all(|v| v.is_finite()) {
                            text.clip_boxes.push(page_box);
                        }
                        match font.unicode(code) {
                            Some(t) => decoded.push_str(t),
                            None => decodable = false,
                        }
                        let tx = (w0 * tfs + state.text.tc + if font.word_space(code) { state.text.tw } else { 0.0 }) * th;
                        text.tm = mul(translate(tx, 0.0), text.tm);
                    }
                }
                other => {
                    if let Some(n) = number(other) {
                        let tx = -n / 1000.0 * tfs * th;
                        text.tm = mul(translate(tx, 0.0), text.tm);
                        // A wide gap inside a TJ array reads as a word space.
                        if n < -250.0 && !decoded.ends_with(' ') {
                            decoded.push(' ');
                        }
                    }
                }
            }
        }
        // The pen advance in text space (along the line).
        let advance = {
            let line = text.tlm;
            let (dx, dy) = (text.tm[4] - start_x[4], text.tm[5] - start_x[5]);
            if line[0].abs() >= line[1].abs() && line[0].abs() > 1e-12 {
                dx / line[0]
            } else if line[1].abs() > 1e-12 {
                dy / line[1]
            } else {
                0.0
            }
        };
        let end_pen = apply(mul(text.tm, ctm), 0.0, 0.0);
        if visible {
            text.visible_boxes.extend(boxes.iter().copied());
            if (4..=6).contains(&mode) {
                text.paint_clip_visible = true;
            }
        }
        let run = text.current.as_mut().expect("a run was started");
        run.last = index;
        run.boxes.extend(boxes);
        if spaced && !run.text.is_empty() && !run.text.ends_with(' ') && !decoded.starts_with(' ') {
            run.text.push(' ');
        }
        run.text.push_str(&decoded);
        run.decodable &= decodable;
        run.type3 |= font.kind == FontKind::Type3;
        run.stroked |= matches!(mode, 1 | 2 | 5 | 6);
        run.end_pen = end_pen;
        run.tlm_after = text.tlm;
        run.tm_after = text.tm;
        run.shows.push(ShowStep {
            index,
            operator: op.to_owned(),
            advance,
            scale: tfs * th,
            operands: operands.clone(),
        });
    }

    fn close_run(&mut self, text: &mut TextBlock) {
        if let Some(run) = text.current.take() {
            let scale = run.shows.last().map(|s| s.scale).unwrap_or(0.0);
            let (tlm, tm) = (run.tlm_after, run.tm_after);
            text.runs.push((run, tlm, tm, scale));
        }
    }

    /// The runs of a finished text object become objects (or the whole
    /// object, when it clips). Returns the range of a text object that only
    /// clips.
    fn finish_text(&mut self, container: &Container<'_>, text: TextBlock, end: usize, state: &mut State, depth: usize) -> Option<(usize, usize)> {
        if text.clip_modes {
            // Text clipping applies at ET: the clip is the glyph area.
            if let Some(bounds) = union_all(&text.clip_boxes) {
                state.clip = Some(intersect(state.clip, bounds));
                state.clipped = true;
            }
            state.replay().incomplete = true;
            if !text.paint_clip_visible {
                return Some((text.start, end));
            }
            let Some(bounds) = union_all(&text.visible_boxes) else { return None };
            let at_start = text.state_at_start.clone();
            let mut object = self.object(container, ContentObjectKind::Text, text.start, end, bounds, &at_start, at_start.ctm, vec!["textClip"], depth > 1);
            object.parts = capped(text.visible_boxes.clone());
            object.can_reorder = false;
            let mut texts = String::new();
            let mut decodable = true;
            for (run, _, _, _) in &text.runs {
                texts.push_str(&run.text);
                decodable &= run.decodable;
            }
            object.text = decodable.then_some(texts);
            self.push(object);
            return None;
        }
        for (run, tlm_end, tm_end, end_scale) in text.runs {
            let Some(bounds) = union_all(&run.boxes) else { continue };
            let mut traits = Vec::new();
            if run.type3 {
                traits.push("type3");
            }
            if run.font.subset {
                traits.push("subsetFont");
            }
            if run.stroked {
                traits.push("stroked");
            }
            if !run.decodable {
                traits.push("undecodable");
            }
            if run.font.coloured_glyphs {
                traits.push("colouredGlyphs");
            }
            let filling = matches!(run.render_mode, 0 | 2 | 4 | 6);
            if filling && run.fill == Paint::Pattern {
                traits.push("pattern");
            }
            let frame = mul(run.tm_start, run.state.ctm);
            if frame[1].abs() > 1e-6 || frame[2].abs() > 1e-6 {
                traits.push("rotated");
            }
            let saved_layers = std::mem::take(&mut self.layers);
            self.layers = saved_layers[..run.marked.min(saved_layers.len())].to_vec();
            let mut object = self.object(container, ContentObjectKind::Text, run.first, run.last, bounds, &run.state, run.state.ctm, traits, depth > 1);
            self.layers = saved_layers;
            object.layer = run.layer.clone();
            object.text = run.decodable.then(|| run.text.clone());
            object.parts = capped(run.boxes.clone());
            object.render_mode = Some(run.render_mode);
            object.fill = filling.then(|| rgb_of(&run.fill)).flatten();
            object.stroke = run.stroked.then(|| rgb_of(&run.stroke)).flatten();
            object.can_recolor = !run.font.coloured_glyphs;
            let scale = (frame[0] * frame[3] - frame[1] * frame[2]).abs().sqrt();
            let resource = String::from_utf8_lossy(&run.font_name).into_owned();
            let family = crate::fonts::family_of(run.font.base_font.get(7..).filter(|_| run.font.subset).unwrap_or(&run.font.base_font));
            object.font = Some(ObjectFont {
                resource,
                base_font: run.font.base_font.clone(),
                subtype: run.font.subtype.clone(),
                subset: run.font.subset,
                embedded: run.font.embedded,
                size: (run.size * scale * 100.0).round() / 100.0,
                bold: run.font.bold,
                italic: run.font.italic,
                library_match: library_match(&family, run.font.bold, run.font.italic, run.font.flags),
            });
            object.run = Some(Arc::new(TextRun {
                tlm_start: run.tlm_start,
                tm_start: run.tm_start,
                tlm_end,
                tm_end,
                shows: run.shows,
                end_scale,
                ascent: run.font.ascent,
                descent: run.font.descent,
                font: run.font,
                font_object: run.font_object,
                size: run.size,
                th: run.th,
                ctm: run.state.ctm,
            }));
            self.push(object);
        }
        None
    }

    fn form(&mut self, parent: &Container<'_>, state: &State, index: usize, name: &[u8], id: ObjectId, stream: &Stream) {
        if parent.depth >= MAX_FORM_DEPTH || parent.forms.iter().any(|step| ObjectId::from((step.object.number, step.object.generation)) == id) {
            return;
        }
        let Some(operations) = decode_stream(stream) else {
            // Count what the unreadable form would have drawn as unreadable.
            if let Ok(plain) = stream.decompressed_content() {
                let text = String::from_utf8_lossy(&plain);
                self.out.unreadable_images += text.matches(" Do").count() + text.matches("BI").count();
                self.out.unreadable_paths += text.matches(" f").count() + text.matches(" S").count();
            }
            return;
        };
        let Ok(name) = String::from_utf8(name.to_vec()) else {
            return;
        };
        let matrix = stream
            .dict
            .get(b"Matrix")
            .ok()
            .and_then(|m| resolve(self.document, m))
            .and_then(|m| m.as_array().ok())
            .and_then(|a| matrix_of(a))
            .unwrap_or(IDENTITY);
        let ctm = mul(matrix, state.ctm);
        let resources = stream
            .dict
            .get(b"Resources")
            .ok()
            .and_then(|r| resolve_dict(self.document, r))
            .cloned()
            .or_else(|| parent.resources.clone());
        let mut inner = state.clone();
        inner.ctm = ctm;
        if let Some(bbox) = stream
            .dict
            .get(b"BBox")
            .ok()
            .and_then(|b| resolve(self.document, b))
            .and_then(|b| b.as_array().ok())
            .and_then(|a| numbers(a))
            .filter(|v| v.len() == 4)
        {
            let b = [bbox[0].min(bbox[2]), bbox[1].min(bbox[3]), bbox[0].max(bbox[2]), bbox[1].max(bbox[3])];
            inner.clip = Some(intersect(state.clip, transform_box(ctm, b)));
        }
        let mut forms = parent.forms.clone();
        forms.push(FormStep {
            invocation: index,
            name,
            object: id.into(),
        });
        let mut form_ctms = parent.form_ctms.clone();
        form_ctms.push(ctm);
        let container = Container {
            operations: &operations,
            resources,
            forms,
            form_ctms,
            origin: None,
            depth: parent.depth + 1,
            start_ctm: ctm,
        };
        // Form content starts with its own state stack.
        let mark = self.mark();
        let ending = self.walk(&container, inner);
        self.finish_container(&container, mark, &ending);
    }

    /// Objects of a container learn the state its content ends in (where
    /// "bring to front" draws them).
    fn finish_container(&mut self, container: &Container<'_>, mark: (usize, usize, usize), ending: &Ending) {
        let end = (!ending.in_text).then_some(ending.ctm);
        let forms = &container.forms;
        let (objects, texts, groups) = (self.out.objects.len(), self.texts.len(), self.groups.len());
        for object in self.out.objects[mark.0.min(objects)..]
            .iter_mut()
            .chain(self.texts[mark.1.min(texts)..].iter_mut())
            .chain(self.groups[mark.2.min(groups)..].iter_mut())
        {
            if object.selector.forms == *forms {
                object.container_end_ctm = end;
            }
        }
    }
}

fn colour_space_family(document: &Document, resources: Option<&Dictionary>, name: &[u8]) -> Option<Vec<u8>> {
    let spaces = resolve_dict(document, resources?.get(b"ColorSpace").ok()?)?;
    match resolve(document, spaces.get(name).ok()?)? {
        Object::Name(n) => Some(n.clone()),
        Object::Array(items) => items.first()?.as_name().ok().map(<[u8]>::to_vec),
        _ => None,
    }
}

fn shading_bbox(document: &Document, resources: Option<&Dictionary>, name: &[u8]) -> Option<[f64; 4]> {
    let shadings = resolve_dict(document, resources?.get(b"Shading").ok()?)?;
    let shading = resolve_dict(document, shadings.get(name).ok()?)?;
    let v = numbers(resolve(document, shading.get(b"BBox").ok()?)?.as_array().ok()?)?;
    (v.len() == 4).then(|| [v[0].min(v[2]), v[1].min(v[3]), v[0].max(v[2]), v[1].max(v[3])])
}

/// Every visible object of `page`: images, inline images, paths, shadings,
/// then text runs, then groups and layers.
pub fn page_content_objects(document: &Document, page: u32) -> Result<PageContentObjects, EditorError> {
    read_page(document, page).map(|(objects, _)| objects)
}

fn read_page(document: &Document, page: u32) -> Result<(PageContentObjects, HashMap<ObjectId, BTreeSet<u32>>), EditorError> {
    let page_id = overlay::page_id(document, page)?;
    let media_box = overlay::media_box(document, page_id)?;
    let mut walker = Walker {
        document,
        page,
        media_box,
        out: PageContentObjects::default(),
        texts: Vec::new(),
        groups: Vec::new(),
        fonts: HashMap::new(),
        known_codes: HashMap::new(),
        layers: Vec::new(),
    };
    let Some((operations, origin)) = page_operations(document, page_id) else {
        walker.out.content_unreadable = true;
        return Ok((walker.out, HashMap::new()));
    };
    let container = Container {
        operations: &operations,
        resources: page_resources(document, page_id),
        forms: Vec::new(),
        form_ctms: Vec::new(),
        origin: Some(&origin),
        depth: 0,
        start_ctm: IDENTITY,
    };
    let mark = walker.mark();
    let ending = walker.walk(&container, State::new(IDENTITY));
    walker.finish_container(&container, mark, &ending);
    let mut out = walker.out;
    out.objects.extend(walker.texts);
    out.objects.extend(walker.groups);
    Ok((out, walker.known_codes))
}

// ---- editing ----

/// One container on the way to the object: its operators and where they live.
struct Level {
    operations: Vec<Operation>,
    /// The page (content streams with their operator counts) or a form.
    source: LevelSource,
    dirty: bool,
}

enum LevelSource {
    Page { streams: Vec<(ObjectId, usize)> },
    Form(ObjectId),
}

fn write_failed(error: impl std::fmt::Display) -> EditorError {
    EditorError::new("EDITOR_WRITE_FAILED", error.to_string())
}

fn count_references(object: &Object, id: ObjectId, count: &mut usize) {
    match object {
        Object::Reference(r) if *r == id => *count += 1,
        Object::Array(items) => items.iter().for_each(|item| count_references(item, id, count)),
        Object::Dictionary(dict) => dict.iter().for_each(|(_, value)| count_references(value, id, count)),
        Object::Stream(stream) => stream.dict.iter().for_each(|(_, value)| count_references(value, id, count)),
        _ => {}
    }
}

/// Objects holding a reference to `id`, with how many references in total.
fn holders(document: &Document, id: ObjectId) -> (Vec<ObjectId>, usize) {
    let mut found = Vec::new();
    let mut total = 0;
    for (holder, object) in &document.objects {
        let mut count = 0;
        count_references(object, id, &mut count);
        if count > 0 {
            found.push(*holder);
            total += count;
        }
    }
    (found, total)
}

/// Whether anything but the one `Do` at `invocation` of the container
/// `container` can draw `form`: another reference to it, a shared resource
/// dictionary on the way to it, resources the page inherits, or another
/// `Do` of the same name in the container.
fn form_shared(document: &Document, form: ObjectId, container: ObjectId, operations: &[Operation], name: &[u8]) -> bool {
    let invocations = operations
        .iter()
        .filter(|o| o.operator == "Do" && o.operands.first().and_then(|n| n.as_name().ok()) == Some(name))
        .count();
    if invocations != 1 {
        return true;
    }
    let mut target = form;
    for _ in 0..4 {
        let (found, total) = holders(document, target);
        if total != 1 {
            return true;
        }
        let holder = found[0];
        if holder == container {
            return false;
        }
        target = holder;
    }
    true
}

fn stream_ops(document: &Document, id: ObjectId) -> Result<Vec<Operation>, EditorError> {
    let stream = document.get_object(id).and_then(Object::as_stream).map_err(|_| stale())?;
    decode_stream(stream).ok_or_else(stale)
}

fn encode(operations: &[Operation]) -> Result<Vec<u8>, EditorError> {
    Content {
        operations: operations.to_vec(),
    }
    .encode()
    .map_err(write_failed)
}

/// A fresh content stream body (Flate) keeping the other dictionary entries.
fn set_stream_content(stream: &mut Stream, operations: &[Operation]) -> Result<(), EditorError> {
    let content = encode(operations)?;
    stream.dict.remove(b"Filter");
    stream.dict.remove(b"DecodeParms");
    stream.set_plain_content(content);
    let _ = stream.compress();
    Ok(())
}

/// The resources dictionary a container uses, made the container's own.
fn own_resources(document: &mut Document, container: ObjectId, is_page: bool) -> Result<Dictionary, EditorError> {
    let resources = if is_page {
        page_resources(document, container).unwrap_or_default()
    } else {
        let stream = document.get_object(container).and_then(Object::as_stream).map_err(write_failed)?;
        match stream.dict.get(b"Resources") {
            Ok(r) => resolve_dict(document, r).cloned().unwrap_or_default(),
            Err(_) => Dictionary::new(),
        }
    };
    Ok(resources)
}

fn set_resources(document: &mut Document, container: ObjectId, resources: Dictionary) -> Result<(), EditorError> {
    match document.get_object_mut(container).map_err(write_failed)? {
        Object::Dictionary(dict) => dict.set("Resources", Object::Dictionary(resources)),
        Object::Stream(stream) => stream.dict.set("Resources", Object::Dictionary(resources)),
        _ => return Err(refused("the container is not a page or form")),
    }
    Ok(())
}

/// Copy `form` for the placement at `levels[parent].operations[invocation]`
/// under a new resource name in the parent's own resources.
#[allow(clippy::too_many_arguments)]
fn copy_form(
    document: &mut Document,
    levels: &mut [Level],
    parent: usize,
    parent_id: ObjectId,
    invocation: usize,
    name: &str,
    form: ObjectId,
    parent_resources_fallback: Option<Dictionary>,
) -> Result<ObjectId, EditorError> {
    let is_page = matches!(levels[parent].source, LevelSource::Page { .. });
    let copy = document.get_object(form).map_err(write_failed)?.clone();
    let copy_id = document.add_object(copy);
    let mut resources = own_resources(document, parent_id, is_page)?;
    if !is_page && resources.is_empty() {
        resources = parent_resources_fallback.unwrap_or_default();
    }
    let mut xobjects = resources
        .get(b"XObject")
        .ok()
        .and_then(|o| resolve_dict(document, o))
        .cloned()
        .unwrap_or_default();
    let fresh = (1..)
        .map(|n| format!("{name}c{n}"))
        .find(|candidate| !xobjects.has(candidate.as_bytes()))
        .expect("an unused resource name");
    xobjects.set(fresh.as_bytes(), Object::Reference(copy_id));
    resources.set("XObject", Object::Dictionary(xobjects));
    set_resources(document, parent_id, resources)?;
    let operation = levels[parent].operations.get_mut(invocation).ok_or_else(stale)?;
    operation.operands = vec![Object::Name(fresh.into_bytes())];
    levels[parent].dirty = true;
    Ok(copy_id)
}

fn grow_box(document: &mut Document, form: ObjectId, drawn: [f64; 4]) {
    let Ok(stream) = document.get_object_mut(form).and_then(Object::as_stream_mut) else {
        return;
    };
    let current: Vec<f64> = stream
        .dict
        .get(b"BBox")
        .and_then(Object::as_array)
        .map(|items| items.iter().filter_map(number).collect())
        .unwrap_or_default();
    let [a, b, c, d] = current[..] else { return };
    let (x0, y0, x1, y1) = (a.min(c), b.min(d), a.max(c), b.max(d));
    if drawn[0] >= x0 && drawn[1] >= y0 && drawn[2] <= x1 && drawn[3] <= y1 {
        return;
    }
    let grown = [x0.min(drawn[0] - 1.0), y0.min(drawn[1] - 1.0), x1.max(drawn[2] + 1.0), y1.max(drawn[3] + 1.0)];
    stream.dict.set("BBox", Object::Array(grown.iter().map(|v| real(*v)).collect()));
}

/// Write the page level back: only the stream the change falls in, or one
/// merged stream when it spans streams. A stream another page shares is
/// replaced by a copy for this page.
fn write_page(document: &mut Document, page_id: ObjectId, level: &Level, changed: (usize, usize), delta: isize) -> Result<(), EditorError> {
    let LevelSource::Page { streams } = &level.source else {
        return Ok(());
    };
    // Which stream holds the changed range (in original indices)?
    let mut offset = 0usize;
    let mut holder = None;
    for (position, (id, count)) in streams.iter().enumerate() {
        let range = offset..offset + count;
        if range.contains(&changed.0) && range.contains(&changed.1) {
            holder = Some((position, *id, offset, *count));
        }
        offset += count;
    }
    let contents_single = |document: &mut Document, operations: &[Operation]| -> Result<(), EditorError> {
        let mut stream = Stream::new(Dictionary::new(), Vec::new());
        set_stream_content(&mut stream, operations)?;
        let id = document.add_object(stream);
        let page = document.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(write_failed)?;
        page.set("Contents", Object::Reference(id));
        Ok(())
    };
    let Some((position, id, offset, count)) = holder else {
        return contents_single(document, &level.operations);
    };
    let new_count = (count as isize + delta) as usize;
    let operations = &level.operations[offset..offset + new_count];
    let (_, references) = holders(document, id);
    if references > 1 {
        // Shared with another page: this page gets its own copy.
        let mut stream = document.get_object(id).and_then(Object::as_stream).map_err(write_failed)?.clone();
        set_stream_content(&mut stream, operations)?;
        let copy = document.add_object(stream);
        let page = document.get_object_mut(page_id).and_then(Object::as_dict_mut).map_err(write_failed)?;
        match page.get_mut(b"Contents").map_err(write_failed)? {
            Object::Array(items) => {
                if let Some(item) = items.get_mut(position) {
                    *item = Object::Reference(copy);
                }
            }
            other => *other = Object::Reference(copy),
        }
        return Ok(());
    }
    let stream = document.get_object_mut(id).and_then(Object::as_stream_mut).map_err(write_failed)?;
    set_stream_content(stream, operations)
}

/// Replace `[start, end)` of a container's operators (an insertion when
/// `start == end`).
struct Splice {
    start: usize,
    end: usize,
    operations: Vec<Operation>,
}

fn op(operator: &str, operands: Vec<Object>) -> Operation {
    Operation::new(operator, operands)
}

/// A glyph-less `TJ` moving the pen by `advance` text-space units.
fn advance_op(advance: f64, scale: f64) -> Result<Option<Operation>, EditorError> {
    if advance.abs() < 1e-9 {
        return Ok(None);
    }
    if scale.abs() < 1e-9 {
        return Err(refused("this text is shown at size zero and cannot be cut out of its line"));
    }
    Ok(Some(op("TJ", vec![Object::Array(vec![real(-advance * 1000.0 / scale)])])))
}

/// The pen offset of `tm` from the line start `tlm`, along the line.
fn pen_offset(tlm: M, tm: M) -> f64 {
    let (dx, dy) = (tm[4] - tlm[4], tm[5] - tlm[5]);
    if tlm[0].abs() >= tlm[1].abs() && tlm[0].abs() > 1e-12 {
        dx / tlm[0]
    } else if tlm[1].abs() > 1e-12 {
        dy / tlm[1]
    } else {
        0.0
    }
}

/// What a show operator becomes when its glyphs are removed: the same pen
/// movement (and line change, and spacing) without glyphs.
fn without_glyphs(step: &ShowStep) -> Result<Vec<Operation>, EditorError> {
    let mut out = Vec::new();
    if step.operator == "\"" {
        if let (Some(aw), Some(ac)) = (step.operands.first(), step.operands.get(1)) {
            out.push(op("Tw", vec![aw.clone()]));
            out.push(op("Tc", vec![ac.clone()]));
        }
    }
    if step.operator == "'" || step.operator == "\"" {
        out.push(op("T*", vec![]));
    }
    out.extend(advance_op(step.advance, step.scale)?);
    Ok(out)
}

fn strip_colours(operations: &[Operation], fill: bool, stroke: bool) -> Vec<Operation> {
    operations
        .iter()
        .filter(|o| {
            let name = o.operator.as_str();
            !(fill && FILL_COLOUR.contains(&name)) && !(stroke && STROKE_COLOUR.contains(&name))
        })
        .cloned()
        .collect()
}

/// A run drawn on its own: the text object is closed before it, the run is
/// drawn in `q prefix BT … ET Q` from its text matrix, the state the run's
/// operators set is set again, and the text object resumes at the pen
/// position it had after the run.
fn isolated_run(run: &TextRun, range: &[Operation], prefix: Vec<Operation>, body: Vec<Operation>) -> Result<Vec<Operation>, EditorError> {
    let mut out = vec![op("ET", vec![]), op("q", vec![])];
    out.extend(prefix);
    out.push(op("BT", vec![]));
    out.push(op("Tm", matrix_object(run.tlm_start)));
    let first_scale = run.shows.first().map(|s| s.scale).unwrap_or(run.size * run.th);
    out.extend(advance_op(pen_offset(run.tlm_start, run.tm_start), first_scale)?);
    out.extend(body);
    out.push(op("ET", vec![]));
    out.push(op("Q", vec![]));
    out.extend(range.iter().filter(|o| is_state(&o.operator)).cloned());
    out.push(op("BT", vec![]));
    out.push(op("Tm", matrix_object(run.tlm_end)));
    out.extend(advance_op(pen_offset(run.tlm_end, run.tm_end), run.end_scale)?);
    Ok(out)
}

/// Operators that re-establish the state an object was drawn in (its clip
/// paths, extended graphics states, line and text state and colours) from a
/// place whose CTM is `base`, ending with the CTM the object starts in.
fn replay_ops(object: &ContentObject, base: M) -> Result<Vec<Operation>, EditorError> {
    let replay = &object.replay;
    if replay.incomplete || !object.can_reorder {
        return Err(refused("this object is drawn under a clip that cannot be repeated elsewhere, so it keeps its place in the stacking order"));
    }
    let mut out = Vec::new();
    let mut current = base;
    for (ctm, ops) in &replay.clips {
        if let Some(m) = relative(current, *ctm)? {
            out.push(op("cm", matrix_object(m)));
        }
        current = *ctm;
        out.extend(ops.iter().cloned());
    }
    out.extend(replay.gs.iter().cloned());
    out.extend(replay.misc.values().cloned());
    out.extend(replay.fill.iter().cloned());
    out.extend(replay.stroke.iter().cloned());
    if let Some(m) = relative(current, object.insert_ctm)? {
        out.push(op("cm", matrix_object(m)));
    }
    Ok(out)
}

fn colour_ops(fill: Option<&Rgb>, stroke: Option<&Rgb>) -> Result<Vec<Operation>, EditorError> {
    let mut prefix = Vec::new();
    for (paint, stroking) in [(fill, false), (stroke, true)] {
        let Some(paint) = paint else { continue };
        let components = paint.components();
        let operator = match (paint, stroking) {
            (Rgb::Rgb { .. }, false) => "rg",
            (Rgb::Rgb { .. }, true) => "RG",
            (Rgb::Cmyk { .. }, false) => "k",
            (Rgb::Cmyk { .. }, true) => "K",
            (Rgb::Spot { .. }, _) => return Err(refused("recolour takes RGB or CMYK colours")),
        };
        prefix.push(op(operator, components.into_iter().map(real).collect()));
    }
    Ok(prefix)
}

/// A fresh resource name of `category` in `resources`.
fn fresh_name(document: &Document, resources: &Dictionary, category: &[u8], stem: &str) -> String {
    let existing = resources.get(category).ok().and_then(|o| resolve_dict(document, o)).cloned().unwrap_or_default();
    (1..).map(|n| format!("{stem}{n}")).find(|candidate| !existing.has(candidate.as_bytes())).expect("an unused name")
}

fn add_resource(resources: &mut Dictionary, document: &Document, category: &[u8], name: &str, target: ObjectId) {
    let mut entries = resources.get(category).ok().and_then(|o| resolve_dict(document, o)).cloned().unwrap_or_default();
    entries.set(name.as_bytes(), Object::Reference(target));
    resources.set(category, Object::Dictionary(entries));
}

/// Fonts a container's operators draw glyphs with (a `Tf` whose font only
/// glyph-less `TJ` advances follow draws nothing).
fn glyph_fonts(operations: &[Operation]) -> BTreeSet<Vec<u8>> {
    let mut out = BTreeSet::new();
    let mut current: Option<Vec<u8>> = None;
    let mut stack: Vec<Option<Vec<u8>>> = Vec::new();
    for operation in operations {
        let operands = &operation.operands;
        match operation.operator.as_str() {
            "q" => stack.push(current.clone()),
            "Q" => current = stack.pop().unwrap_or(None),
            "Tf" => current = operands.first().and_then(|o| o.as_name().ok()).map(<[u8]>::to_vec),
            "Tj" | "'" | "TJ" | "\"" => {
                let shown = match operation.operator.as_str() {
                    "TJ" => operands
                        .first()
                        .and_then(|o| o.as_array().ok())
                        .is_some_and(|items| items.iter().any(|item| matches!(item, Object::String(bytes, _) if !bytes.is_empty()))),
                    "\"" => matches!(operands.get(2), Some(Object::String(bytes, _)) if !bytes.is_empty()),
                    _ => matches!(operands.first(), Some(Object::String(bytes, _)) if !bytes.is_empty()),
                };
                if shown {
                    out.extend(current.clone());
                }
            }
            _ => {}
        }
    }
    out
}

/// A font named only by `Tf` operators that draw no glyph (the pen
/// advances a removed run leaves): a standard font with no glyph data, so
/// the removed font's program leaves the file and the name stays valid.
fn placeholder_font() -> Object {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    Object::Dictionary(font)
}

/// Resource names a container's operators use, by category.
fn used_names(operations: &[Operation]) -> HashMap<&'static str, BTreeSet<Vec<u8>>> {
    let mut used: HashMap<&'static str, BTreeSet<Vec<u8>>> = HashMap::new();
    let mut add = |category: &'static str, object: Option<&Object>| {
        if let Some(name) = object.and_then(|o| o.as_name().ok()) {
            used.entry(category).or_default().insert(name.to_vec());
        }
    };
    for operation in operations {
        let operands = &operation.operands;
        match operation.operator.as_str() {
            "Tf" => add("Font", operands.first()),
            "Do" => add("XObject", operands.first()),
            "gs" => add("ExtGState", operands.first()),
            "sh" => add("Shading", operands.first()),
            "cs" | "CS" => add("ColorSpace", operands.first()),
            "scn" | "SCN" => add("Pattern", operands.last()),
            "BDC" | "DP" => add("Properties", operands.get(1)),
            "BI" => {
                if let Some(Object::Stream(image)) = operands.first() {
                    add("ColorSpace", image.dict.get(b"CS").or_else(|_| image.dict.get(b"ColorSpace")).ok());
                }
            }
            _ => {}
        }
    }
    used
}

/// Drop the resources a container no longer uses (when the container owns
/// its resources and every form it draws has its own), so deleted content's
/// fonts, images and forms leave the file on save (QF-5212).
fn prune_unused_resources(document: &mut Document, container: ObjectId, operations: &[Operation]) {
    let used = used_names(operations);
    let holder = match document.get_object(container) {
        Ok(Object::Dictionary(d)) => d.get(b"Resources").ok().cloned(),
        Ok(Object::Stream(s)) => s.dict.get(b"Resources").ok().cloned(),
        _ => None,
    };
    let Some(holder) = holder else { return };
    // A resources dictionary shared with other containers is left alone.
    let resources_id = match &holder {
        Object::Reference(id) => {
            if holders(document, *id).1 > 1 {
                return;
            }
            Some(*id)
        }
        _ => None,
    };
    let Some(mut resources) = resolve_dict(document, &holder).cloned() else { return };
    // Forms drawn here without resources of their own, and Type 3 fonts
    // without them, use these resources by name: keep everything.
    let xobjects = resources.get(b"XObject").ok().and_then(|o| resolve_dict(document, o)).cloned().unwrap_or_default();
    for name in used.get("XObject").into_iter().flatten() {
        if let Some(Ok(stream)) = xobjects.get(name).ok().and_then(|o| resolve(document, o)).map(Object::as_stream) {
            if subtype(stream) == b"Form" && !stream.dict.has(b"Resources") {
                return;
            }
        }
    }
    let fonts = resources.get(b"Font").ok().and_then(|o| resolve_dict(document, o)).cloned().unwrap_or_default();
    for (_, font) in fonts.iter() {
        if let Some(dict) = resolve_dict(document, font) {
            if dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type3".as_slice()) && !dict.has(b"Resources") {
                return;
            }
        }
    }
    let drawn = glyph_fonts(operations);
    let mut changed = false;
    for category in ["Font", "XObject", "ExtGState", "Shading", "Pattern", "Properties", "ColorSpace"] {
        let Some(entries) = resources.get(category.as_bytes()).ok().and_then(|o| resolve_dict(document, o)).cloned() else {
            continue;
        };
        let keep = used.get(category);
        let mut pruned = Dictionary::new();
        let mut dropped = false;
        for (name, value) in entries.iter() {
            // DefaultGray/DefaultRGB/DefaultCMYK apply without being named.
            let implicit = category == "ColorSpace" && name.starts_with(b"Default");
            let named = keep.is_some_and(|k| k.contains(name));
            if category == "Font" && named && !drawn.contains(name) {
                // Only glyph-less advances name it: its program goes.
                let standard = resolve_dict(document, value).is_some_and(|d| {
                    d.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Type1".as_slice())
                        && !d.has(b"FontDescriptor")
                });
                if standard {
                    pruned.set(name.clone(), value.clone());
                } else {
                    pruned.set(name.clone(), placeholder_font());
                    dropped = true;
                }
            } else if implicit || named {
                pruned.set(name.clone(), value.clone());
            } else {
                dropped = true;
            }
        }
        if dropped {
            changed = true;
            if pruned.is_empty() {
                resources.remove(category.as_bytes());
            } else {
                resources.set(category, Object::Dictionary(pruned));
            }
        }
    }
    if !changed {
        return;
    }
    match resources_id {
        Some(id) => {
            if let Ok(object) = document.get_object_mut(id) {
                *object = Object::Dictionary(resources);
            }
        }
        None => {
            let _ = set_resources(document, container, resources);
        }
    }
}

/// Apply `edit` to the object `selector` names. Returns the new bytes and
/// the page-space region that changed.
pub fn edit_object(input: &[u8], selector: &ContentObjectSelector, edit: &ObjectEdit) -> Result<(Vec<u8>, [f64; 4]), EditorError> {
    let fonts = crate::fonts::MemoryFonts::new();
    edit_object_with(input, selector, &ObjectAction::Edit(edit), &fonts).map(|outcome| (outcome.bytes, outcome.changed))
}

/// Apply an object action (`editObject`, `editObjectText`,
/// `replaceObjectWithText`) to the object `selector` names.
pub fn edit_object_with(input: &[u8], selector: &ContentObjectSelector, action: &ObjectAction<'_>, fonts: &dyn FontSource) -> Result<ObjectOutcome, EditorError> {
    let mut document = overlay::load(input)?;
    let page_id = overlay::page_id(&document, selector.page_number)?;
    let (listed, known_codes) = read_page(&document, selector.page_number)?;
    let object = listed
        .objects
        .into_iter()
        .find(|candidate| candidate.selector == *selector)
        .ok_or_else(stale)?;

    // Containers from the page down to the object's.
    let (page_ops, origin) = page_operations(&document, page_id).ok_or_else(stale)?;
    // Every content stream in /Contents order with its operator count
    // (empty streams included, so positions match the /Contents array).
    let streams: Vec<(ObjectId, usize)> = document
        .get_page_contents(page_id)
        .into_iter()
        .map(|id| (id, origin.iter().filter(|(s, _)| *s == id).count()))
        .collect();
    let mut levels = vec![Level {
        operations: page_ops,
        source: LevelSource::Page { streams },
        dirty: false,
    }];
    let mut container_ids = vec![page_id];
    let mut fallback_resources = page_resources(&document, page_id);
    for (depth, step) in selector.forms.iter().enumerate() {
        let declared: ObjectId = (step.object.number, step.object.generation);
        let parent_ops = &levels[depth].operations;
        let invoked = parent_ops.get(step.invocation).ok_or_else(stale)?;
        if invoked.operator != "Do" || invoked.operands.first().and_then(|o| o.as_name().ok()) != Some(step.name.as_bytes()) {
            return Err(stale());
        }
        let parent_id = container_ids[depth];
        let shared = form_shared(&document, declared, parent_id, parent_ops, step.name.as_bytes());
        let form_id = if shared {
            copy_form(
                &mut document,
                &mut levels,
                depth,
                parent_id,
                step.invocation,
                &step.name,
                declared,
                fallback_resources.clone(),
            )?
        } else {
            declared
        };
        let form_resources = document
            .get_object(form_id)
            .and_then(Object::as_stream)
            .ok()
            .and_then(|s| s.dict.get(b"Resources").ok().and_then(|r| resolve_dict(&document, r)).cloned());
        if form_resources.is_some() {
            fallback_resources = form_resources;
        }
        levels.push(Level {
            operations: stream_ops(&document, form_id)?,
            source: LevelSource::Form(form_id),
            dirty: false,
        });
        container_ids.push(form_id);
    }

    let last = levels.len() - 1;
    let last_id = container_ids[last];
    let last_is_page = last == 0;
    let start = selector.start;
    let end = selector.end;
    let range = levels[last].operations.get(start..=end).ok_or_else(stale)?.to_vec();
    if digest(&range) != selector.expected_sha256 {
        return Err(stale());
    }
    let container_len = levels[last].operations.len();
    let mut outcome = ObjectOutcome::default();
    let mut prune = false;

    // The painted path without its clip operator and paint, for split paths.
    let (path, paint) = if object.clip_split {
        let paint = range.last().cloned().ok_or_else(stale)?;
        let path: Vec<Operation> = range[..range.len() - 1]
            .iter()
            .filter(|o| o.operator != "W" && o.operator != "W*")
            .cloned()
            .collect();
        (path, Some(paint))
    } else {
        (Vec::new(), None)
    };
    let clip_operator = range.iter().find(|o| o.operator == "W" || o.operator == "W*").cloned();
    let clip_only = || -> Vec<Operation> {
        let mut out = path.clone();
        out.push(clip_operator.clone().unwrap_or_else(|| op("W", vec![])));
        out.push(op("n", vec![]));
        out
    };
    // The object's own drawing operators (a split path without its clip).
    let drawn = || -> Vec<Operation> {
        if object.clip_split {
            let mut out = path.clone();
            out.push(paint.clone().expect("split paths keep their paint"));
            out
        } else {
            range.clone()
        }
    };
    let painted = |prefix: Vec<Operation>, body: Vec<Operation>| -> Vec<Operation> {
        let mut out = vec![op("q", vec![])];
        out.extend(prefix);
        out.extend(body);
        out.push(op("Q", vec![]));
        if object.clip_split {
            out.extend(clip_only());
        }
        out
    };
    // What is left where the object was when it is removed.
    let removal = |run: Option<&TextRun>| -> Result<Vec<Splice>, EditorError> {
        match run {
            Some(run) => run
                .shows
                .iter()
                .map(|step| {
                    Ok(Splice {
                        start: step.index,
                        end: step.index + 1,
                        operations: without_glyphs(step)?,
                    })
                })
                .collect(),
            None => Ok(vec![Splice {
                start,
                end: end + 1,
                operations: if object.clip_split { clip_only() } else { Vec::new() },
            }]),
        }
    };
    if object.has("textClip") && !matches!(action, ObjectAction::Edit(ObjectEdit::Recolor { .. })) {
        return Err(refused(
            "this text clips the content drawn after it (a gradient or image shows through the letters); select that content to edit them together",
        ));
    }
    let run = object.run.as_deref();

    let splices: Vec<Splice> = match action {
        ObjectAction::Edit(ObjectEdit::Transform { matrix }) => {
            let t = [matrix.a, matrix.b, matrix.c, matrix.d, matrix.e, matrix.f];
            let c = object.insert_ctm;
            let inverse = invert(c).ok_or_else(|| refused("the object is drawn with a degenerate matrix and cannot be moved"))?;
            invert(t).ok_or_else(|| refused("the new size must not be zero"))?;
            let x = mul(mul(c, t), inverse);
            let moved = transform_box(t, object.bounds);
            // Forms on the way clip to their boxes: grow them to hold it.
            for (index, form_ctm) in object.form_ctms.iter().enumerate() {
                if let (Some(inverse), LevelSource::Form(id)) = (invert(*form_ctm), &levels[index + 1].source) {
                    let id = *id;
                    grow_box(&mut document, id, transform_box(inverse, moved));
                }
            }
            outcome.changed = union(Some(object.bounds), moved);
            let prefix = vec![op("cm", matrix_object(x))];
            let replacement = match run {
                Some(run) => isolated_run(run, &range, prefix, range.clone())?,
                None => painted(prefix, drawn()),
            };
            vec![Splice { start, end: end + 1, operations: replacement }]
        }
        ObjectAction::Edit(ObjectEdit::Delete) => {
            outcome.changed = object.bounds;
            prune = true;
            outcome.notes.push("The object was removed from the page content, not covered.".into());
            removal(run)?
        }
        ObjectAction::Edit(ObjectEdit::Recolor { fill, stroke }) => {
            if !object.can_recolor || object.has("textClip") {
                return Err(refused(match object.selector.kind {
                    ContentObjectKind::Text => "this text draws its own colours (Type 3 glyphs), so it cannot be recoloured; replace it with editable text instead",
                    _ => "only paths, text and groups that draw them can be recoloured",
                }));
            }
            let prefix = colour_ops(fill.as_ref(), stroke.as_ref())?;
            if prefix.is_empty() {
                return Err(refused("choose a fill or stroke colour"));
            }
            outcome.changed = object.bounds;
            let replacement = match (run, object.selector.kind) {
                (Some(run), _) => isolated_run(run, &range, prefix, strip_colours(&range, fill.is_some(), stroke.is_some()))?,
                (None, ContentObjectKind::Group) => painted(prefix, strip_colours(&range, fill.is_some(), stroke.is_some())),
                _ => painted(prefix, drawn()),
            };
            vec![Splice { start, end: end + 1, operations: replacement }]
        }
        ObjectAction::Edit(edit @ (ObjectEdit::BringToFront | ObjectEdit::SendToBack)) => {
            let front = matches!(edit, ObjectEdit::BringToFront);
            let base = if front {
                object.container_end_ctm.ok_or_else(|| refused("the content here ends inside a text object, so nothing can be drawn after it"))?
            } else {
                object.container_start_ctm
            };
            let mut copy = vec![op("q", vec![])];
            copy.extend(replay_ops(&object, base)?);
            match run {
                Some(run) => {
                    copy.push(op("BT", vec![]));
                    copy.push(op("Tm", matrix_object(run.tlm_start)));
                    let first_scale = run.shows.first().map(|s| s.scale).unwrap_or(run.size * run.th);
                    copy.extend(advance_op(pen_offset(run.tlm_start, run.tm_start), first_scale)?);
                    copy.extend(range.iter().cloned());
                    copy.push(op("ET", vec![]));
                }
                None => copy.extend(drawn()),
            }
            copy.push(op("Q", vec![]));
            outcome.changed = object.bounds;
            let mut splices = removal(run)?;
            let at = if front { container_len } else { 0 };
            splices.push(Splice { start: at, end: at, operations: copy });
            splices
        }
        ObjectAction::SetText { text, expected, font, fallback } => {
            let run = run.ok_or_else(|| refused("only text can be edited as text; use Replace with editable text for this object"))?;
            if let Some(expected) = expected {
                if object.text.as_deref() != Some(*expected) {
                    return Err(stale());
                }
            }
            if text.trim().is_empty() {
                outcome.changed = object.bounds;
                prune = true;
                removal(Some(run))?
            } else {
                let source_known = run.font_object.and_then(|id| known_codes.get(&id)).cloned().unwrap_or_default();
                let kept = if font.is_none() { run.font.encode(text, &source_known) } else { None };
                let (body, advance) = match kept {
                    Some(bytes) => {
                        let advance: f64 = run.font.codes(&bytes).iter().map(|c| run.font.width(*c)).sum();
                        (vec![op("Tj", vec![run.font.string(bytes)])], advance)
                    }
                    None => {
                        let missing: String = text
                            .chars()
                            .filter(|c| !c.is_whitespace() && run.font.encode(&c.to_string(), &source_known).is_none())
                            .collect::<BTreeSet<char>>()
                            .into_iter()
                            .collect();
                        let family = crate::fonts::family_of(run.font.base_font.get(7..).filter(|_| run.font.subset).unwrap_or(&run.font.base_font));
                        let matched = library_match(&family, run.font.bold, run.font.italic, run.font.flags).map(|m| m.font);
                        let id = font.map(str::to_owned).or_else(|| fallback.map(str::to_owned)).or(matched).ok_or_else(|| {
                            EditorError::new("EDITOR_FONT_UNAVAILABLE", "choose a font for the new text")
                        })?;
                        let bytes = fonts
                            .font_bytes(&id)
                            .ok_or_else(|| EditorError::new("EDITOR_FONT_UNAVAILABLE", format!("font {id} is not in the font library")))?;
                        let embedded = text_fonts::embed(&mut document, &bytes, &id, text)?;
                        let (codes, advance) = embedded.encode(text).map_err(|missing| {
                            EditorError::new("EDITOR_FONT_GLYPHS_MISSING", format!("font {id} has no glyph for: {}", missing.iter().take(12).collect::<String>()))
                        })?;
                        let mut resources = if last_is_page {
                            own_resources(&mut document, last_id, true)?
                        } else {
                            let own = own_resources(&mut document, last_id, false)?;
                            if own.is_empty() { fallback_resources.clone().unwrap_or_default() } else { own }
                        };
                        let name = fresh_name(&document, &resources, b"Font", "WFed");
                        add_resource(&mut resources, &document, b"Font", &name, embedded.font);
                        set_resources(&mut document, last_id, resources)?;
                        outcome.fonts_added.push(id.clone());
                        outcome.notes.push(match font {
                            Some(_) => format!("The text is now set in {id} from the font library, at the same position, size and colour."),
                            None if run.font.kind == FontKind::Type3 || object.text.is_none() => format!(
                                "This text was drawn with glyphs that cannot be typed with (a Type 3 or unmapped font); the edited text uses {id}, the closest library font."
                            ),
                            None => format!(
                                "The embedded font {} is a subset without {}; the edited text uses {id}, the closest library font, at the same size, colour and position.",
                                run.font.base_font.get(7..).filter(|_| run.font.subset).unwrap_or(&run.font.base_font),
                                if missing.is_empty() { "some of these letters".to_owned() } else { format!("the letters {missing}") }
                            ),
                        });
                        prune = true;
                        (
                            vec![
                                op("Tf", vec![Object::Name(name.into_bytes()), real(run.size)]),
                                op("Tj", vec![Object::String(codes, StringFormat::Hexadecimal)]),
                            ],
                            advance,
                        )
                    }
                };
                let width = advance * run.size * run.th;
                let estimate = transform_box(mul(run.tm_start, run.ctm), [0.0, run.descent * run.size, width, run.ascent * run.size]);
                outcome.changed = union(Some(object.bounds), estimate);
                vec![Splice { start, end: end + 1, operations: isolated_run(run, &range, Vec::new(), body)? }]
            }
        }
        ObjectAction::ReplaceWithText { text, font, size, color } => {
            if text.trim().is_empty() {
                return Err(refused("type the text that replaces the object"));
            }
            let bytes = fonts
                .font_bytes(font)
                .ok_or_else(|| EditorError::new("EDITOR_FONT_UNAVAILABLE", format!("font {font} is not in the font library")))?;
            let embedded = text_fonts::embed(&mut document, &bytes, font, text)?;
            let (codes, advance) = embedded
                .encode(text)
                .map_err(|missing| EditorError::new("EDITOR_FONT_GLYPHS_MISSING", format!("font {font} has no glyph for: {}", missing.iter().take(12).collect::<String>())))?;
            let mut resources = if last_is_page {
                own_resources(&mut document, last_id, true)?
            } else {
                let own = own_resources(&mut document, last_id, false)?;
                if own.is_empty() { fallback_resources.clone().unwrap_or_default() } else { own }
            };
            let name = fresh_name(&document, &resources, b"Font", "WFed");
            add_resource(&mut resources, &document, b"Font", &name, embedded.font);
            set_resources(&mut document, last_id, resources)?;
            outcome.fonts_added.push((*font).to_owned());
            prune = true;
            let colour = color.cloned().or_else(|| object.fill.map(|[r, g, b]| Rgb::rgb(r, g, b))).unwrap_or(Rgb::BLACK);
            let colour_prefix = colour_ops(Some(&colour), None)?;
            let font_op = |size: f64| op("Tf", vec![Object::Name(name.clone().into_bytes()), real(size)]);
            let show = op("Tj", vec![Object::String(codes, StringFormat::Hexadecimal)]);
            match run {
                Some(run) => {
                    // Text keeps its matrix and state; only the font changes.
                    let size = size.unwrap_or(run.size);
                    let mut body = vec![font_op(size), show];
                    if color.is_none() {
                        body.insert(0, op("Tr", vec![Object::Integer(object.render_mode.unwrap_or(0))]));
                    }
                    let prefix = if color.is_some() { colour_prefix } else { Vec::new() };
                    outcome.changed = object.bounds;
                    outcome.notes.push(format!("The text is now editable, set in {font} at the same position and size."));
                    vec![Splice { start, end: end + 1, operations: isolated_run(run, &range, prefix, body)? }]
                }
                None => {
                    // Placed on the object's box: cap height to its height,
                    // horizontally scaled to its width.
                    let [x0, y0, x1, y1] = object.bounds;
                    let (width, height) = ((x1 - x0).max(1.0), (y1 - y0).max(1.0));
                    let chosen = size.unwrap_or(height / embedded.cap_height.max(0.3)).clamp(1.0, 500.0);
                    let natural = advance * chosen;
                    let scale = if size.is_none() && natural > 0.0 { (width / natural * 100.0).clamp(50.0, 200.0) } else { 100.0 };
                    let mut ops = vec![op("q", vec![])];
                    if let Some(m) = relative(object.insert_ctm, IDENTITY)? {
                        ops.push(op("cm", matrix_object(m)));
                    }
                    ops.extend(colour_prefix);
                    ops.push(op("BT", vec![]));
                    ops.push(font_op(chosen));
                    if (scale - 100.0).abs() > 0.5 {
                        ops.push(op("Tz", vec![real(scale)]));
                    }
                    ops.push(op("Tm", matrix_object([1.0, 0.0, 0.0, 1.0, x0, y0])));
                    ops.push(show);
                    ops.push(op("ET", vec![]));
                    ops.push(op("Q", vec![]));
                    if object.clip_split {
                        ops.extend(clip_only());
                    }
                    outcome.changed = object.bounds;
                    outcome.notes.push(format!("The object was replaced with editable text in {font} at its place and size."));
                    vec![Splice { start, end: end + 1, operations: ops }]
                }
            }
        }
    };

    // Apply the splices from the last to the first.
    let mut ordered = splices;
    ordered.sort_by(|a, b| b.start.cmp(&a.start).then(b.end.cmp(&a.end)));
    let first_changed = ordered.iter().map(|s| s.start).min().unwrap_or(start);
    let last_changed = ordered.iter().map(|s| s.end.max(s.start + 1) - 1).max().unwrap_or(end);
    let mut delta = 0isize;
    {
        let level = &mut levels[last];
        for splice in ordered {
            delta += splice.operations.len() as isize - (splice.end - splice.start) as isize;
            level.operations.splice(splice.start..splice.end, splice.operations);
        }
        level.dirty = true;
    }

    // Write the containers back, deepest first.
    for (index, level) in levels.iter().enumerate().rev() {
        if !level.dirty {
            continue;
        }
        match &level.source {
            LevelSource::Form(id) => {
                let stream = document.get_object_mut(*id).and_then(Object::as_stream_mut).map_err(write_failed)?;
                set_stream_content(stream, &level.operations)?;
            }
            LevelSource::Page { .. } => {
                let (range, shift) = if index == last {
                    ((first_changed, last_changed.min(container_len.saturating_sub(1)).max(first_changed)), delta)
                } else {
                    let invocation = selector.forms[0].invocation;
                    ((invocation, invocation), 0)
                };
                // An insertion past the end spans no single stream: one merged stream.
                let range = if first_changed >= container_len { (usize::MAX - 1, usize::MAX - 1) } else { range };
                write_page(&mut document, page_id, level, range, shift)?;
            }
        }
    }
    if prune {
        prune_unused_resources(&mut document, last_id, &levels[last].operations);
        document.prune_objects();
    }
    outcome.bytes = overlay::save(&mut document)?;
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pen_offset_reads_along_rotated_lines() {
        let line = [0.0, 1.0, -1.0, 0.0, 100.0, 100.0];
        let pen = mul(translate(12.0, 0.0), line);
        assert!((pen_offset(line, pen) - 12.0).abs() < 1e-9);
    }

    #[test]
    fn removed_glyphs_keep_the_pen_movement() {
        let step = ShowStep { index: 0, operator: "'".into(), advance: 30.0, scale: 12.0, operands: vec![] };
        let ops = without_glyphs(&step).unwrap();
        assert_eq!(ops.iter().map(|o| o.operator.as_str()).collect::<Vec<_>>(), ["T*", "TJ"]);
        assert_eq!(ops[1].operands, vec![Object::Array(vec![real(-2500.0)])]);
    }
}
