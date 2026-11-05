//! Every image placement and path group of a page, wherever it is drawn.
//!
//! The recognisers of `wellfriendpdf-edit` certify a few exact shapes
//! (`q cm /Im Do Q` without masks, `q [cm] [state] path paint Q`) on the
//! page's own content streams. This module reads the page content as a
//! renderer does (graphics-state stack, CTM, clipping, nested Form XObjects)
//! and lists every image XObject placement, inline image (`BI … EI`), path
//! group (its construction operators up to the painting operator) and
//! shading (`sh`) with its page-space bounds. An object is addressed by its
//! container (the page content, or a chain of Form XObject invocations) and
//! its operator range, checked by a digest of those operators.
//!
//! Edits never rewrite an object's own operators:
//! - move/resize wraps the range in `q X cm … Q` with `X = C·T·C⁻¹`, `C` the
//!   CTM where the range starts and `T` the page-space change, so the object
//!   lands where it was dragged whatever state it is drawn in;
//! - recolour wraps a path in `q <colour> … Q`;
//! - delete removes the range.
//!
//! A path that paints and clips (`re W f`) is split: the painted copy is
//! edited and the clip (`re W n`) stays, so later content is clipped as
//! before. A form drawn by more than one placement (or reachable from more
//! than one place) is copied for the edited placement under a new resource
//! name (copy-on-write, as the Form XObject text edits do), and the form
//! boxes on the way grow to hold a moved object.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use wellfriend_pdf_edit::{PdfMatrix, PdfObjectId};

use crate::error::EditorError;
use crate::fonts::page_resources;
use crate::ops::Rgb;
use crate::overlay;

type M = [f64; 6];
const IDENTITY: M = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
/// Nested Form XObjects followed when listing objects.
pub const MAX_FORM_DEPTH: usize = 12;
/// Objects listed per page (a page with more lists the first ones).
pub const MAX_PAGE_OBJECTS: usize = 5_000;

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
    /// `graphicsState`, `pattern`, `shading`.
    pub traits: Vec<String>,
    /// The image XObject (images drawn with `Do`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_object: Option<PdfObjectId>,
    pub can_recolor: bool,
    #[serde(skip)]
    pub(crate) insert_ctm: M,
    #[serde(skip)]
    pub(crate) clip_split: bool,
    #[serde(skip)]
    pub(crate) form_ctms: Vec<M>,
    /// Page-content objects: (stream, first and last operator in it).
    #[serde(skip)]
    pub(crate) span: Option<(ObjectId, usize, usize)>,
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
    /// Paint a path in plain RGB colours (either may be omitted).
    Recolor {
        #[serde(default)]
        fill: Option<Rgb>,
        #[serde(default)]
        stroke: Option<Rgb>,
    },
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

// ---- reading ----

#[derive(Debug, Clone, PartialEq)]
enum Paint {
    Rgb([f64; 3]),
    Pattern,
    Other,
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
        }
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

struct Walker<'a> {
    document: &'a Document,
    page: u32,
    media_box: [f64; 4],
    out: PageContentObjects,
}

struct Container<'a> {
    operations: &'a [Operation],
    resources: Option<Dictionary>,
    forms: Vec<FormStep>,
    form_ctms: Vec<M>,
    origin: Option<&'a [(ObjectId, usize)]>,
    depth: usize,
}

impl Walker<'_> {
    fn push(&mut self, object: ContentObject) {
        if self.out.objects.len() >= MAX_PAGE_OBJECTS {
            self.out.truncated = true;
        } else {
            self.out.objects.push(object);
        }
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
            insert_ctm,
            clip_split: false,
            form_ctms: container.form_ctms.clone(),
            span,
        }
    }

    fn walk(&mut self, container: &Container<'_>, initial: State) {
        let operations = container.operations;
        let mut state = initial;
        let mut stack: Vec<(State, usize)> = Vec::new();
        let mut path_start: Option<usize> = None;
        let mut path_points: Vec<(f64, f64)> = Vec::new();
        let mut path_clip = false;
        let mut in_text = false;
        for (index, operation) in operations.iter().enumerate() {
            let op = operation.operator.as_str();
            let operands = &operation.operands;
            match op {
                "q" => stack.push((state.clone(), index)),
                "Q" => {
                    if let Some((saved, _)) = stack.pop() {
                        state = saved;
                    }
                }
                "cm" => {
                    if let Some(m) = matrix_of(operands) {
                        state.ctm = mul(m, state.ctm);
                    }
                }
                "BT" => in_text = true,
                "ET" => in_text = false,
                "gs" => state.graphics_state = true,
                "g" | "rg" | "k" => {
                    state.fill = numbers(operands).map(|v| colour(&v)).unwrap_or(Paint::Other);
                    state.fill_pattern_space = false;
                }
                "G" | "RG" | "K" => {
                    state.stroke = numbers(operands).map(|v| colour(&v)).unwrap_or(Paint::Other);
                    state.stroke_pattern_space = false;
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
                    } else {
                        state.stroke = initial;
                        state.stroke_pattern_space = pattern;
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
                    if fill {
                        state.fill = paint;
                    } else {
                        state.stroke = paint;
                    }
                }
                _ if PATH_CONSTRUCTION.contains(&op) && !in_text => {
                    path_start.get_or_insert(index);
                    if let Some(v) = numbers(operands) {
                        if op == "re" && v.len() == 4 {
                            for (x, y) in [(v[0], v[1]), (v[0] + v[2], v[1]), (v[0], v[1] + v[3]), (v[0] + v[2], v[1] + v[3])] {
                                path_points.push(apply(state.ctm, x, y));
                            }
                        } else {
                            for pair in v.as_chunks::<2>().0 {
                                path_points.push(apply(state.ctm, pair[0], pair[1]));
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
                    let clip = std::mem::replace(&mut path_clip, false);
                    let bounds = points.iter().fold(
                        [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY],
                        |b, (x, y)| [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)],
                    );
                    let finite = bounds.iter().all(|v| v.is_finite());
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
                    // before it, with the `Q` right after, is one object with
                    // its clip; otherwise the `sh` alone.
                    let block = stack.last().and_then(|(saved, q)| {
                        let clean = operations.get(index + 1).is_some_and(|o| o.operator == "Q")
                            && operations[q + 1..index]
                                .iter()
                                .all(|o| !DRAWING.contains(&o.operator.as_str()) && o.operator != "q" && o.operator != "Q" && o.operator != "BT");
                        clean.then_some((q + 1, saved.ctm))
                    });
                    let (start, insert_ctm) = block.unwrap_or((index, state.ctm));
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
                    let object = self.object(
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
        };
        // Form content starts with its own state stack.
        self.walk(&container, inner);
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

/// Every image placement, inline image, path group and shading of `page`.
pub fn page_content_objects(document: &Document, page: u32) -> Result<PageContentObjects, EditorError> {
    let page_id = overlay::page_id(document, page)?;
    let media_box = overlay::media_box(document, page_id)?;
    let mut walker = Walker {
        document,
        page,
        media_box,
        out: PageContentObjects::default(),
    };
    let Some((operations, origin)) = page_operations(document, page_id) else {
        walker.out.content_unreadable = true;
        return Ok(walker.out);
    };
    let container = Container {
        operations: &operations,
        resources: page_resources(document, page_id),
        forms: Vec::new(),
        form_ctms: Vec::new(),
        origin: Some(&origin),
        depth: 0,
    };
    walker.walk(&container, State::new(IDENTITY));
    Ok(walker.out)
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

/// Apply `edit` to the object `selector` names. Returns the new bytes and
/// the page-space region that changed.
pub fn edit_object(input: &[u8], selector: &ContentObjectSelector, edit: &ObjectEdit) -> Result<(Vec<u8>, [f64; 4]), EditorError> {
    let mut document = overlay::load(input)?;
    let page_id = overlay::page_id(&document, selector.page_number)?;
    let listed = page_content_objects(&document, selector.page_number)?;
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
    let start = selector.start;
    let end = selector.end;
    let range = levels[last].operations.get(start..=end).ok_or_else(stale)?.to_vec();
    if digest(&range) != selector.expected_sha256 {
        return Err(stale());
    }
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
        out.push(clip_operator.clone().unwrap_or_else(|| Operation::new("W", vec![])));
        out.push(Operation::new("n", vec![]));
        out
    };
    let painted = |prefix: Vec<Operation>| -> Vec<Operation> {
        let mut out = vec![Operation::new("q", vec![])];
        out.extend(prefix);
        if object.clip_split {
            out.extend(path.iter().cloned());
            out.push(paint.clone().expect("split paths keep their paint"));
        } else {
            out.extend(range.iter().cloned());
        }
        out.push(Operation::new("Q", vec![]));
        if object.clip_split {
            out.extend(clip_only());
        }
        out
    };
    let (replacement, changed) = match edit {
        ObjectEdit::Transform { matrix } => {
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
            let changed = [
                object.bounds[0].min(moved[0]),
                object.bounds[1].min(moved[1]),
                object.bounds[2].max(moved[2]),
                object.bounds[3].max(moved[3]),
            ];
            (painted(vec![Operation::new("cm", matrix_object(x))]), changed)
        }
        ObjectEdit::Delete => (if object.clip_split { clip_only() } else { Vec::new() }, object.bounds),
        ObjectEdit::Recolor { fill, stroke } => {
            if !object.can_recolor {
                return Err(refused("only paths can be recoloured"));
            }
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
                prefix.push(Operation::new(operator, components.into_iter().map(real).collect()));
            }
            if prefix.is_empty() {
                return Err(refused("choose a fill or stroke colour"));
            }
            (painted(prefix), object.bounds)
        }
    };
    let delta = replacement.len() as isize - (end - start + 1) as isize;
    let level = &mut levels[last];
    level.operations.splice(start..=end, replacement);
    level.dirty = true;

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
                    ((start, end), delta)
                } else {
                    let invocation = selector.forms[0].invocation;
                    ((invocation, invocation), 0)
                };
                write_page(&mut document, page_id, level, range, shift)?;
            }
        }
    }
    Ok((overlay::save(&mut document)?, changed))
}
