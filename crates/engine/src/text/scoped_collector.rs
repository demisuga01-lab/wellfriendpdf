//! Fallible page-content extraction. Direct `collect` intentionally remains a
//! single-program API for source editing; this walker executes Form occurrences.
use super::{MarkedTextChunk, TextChunk, TextCollector};
use crate::cancel::CancelToken;
use crate::content::{ContentOperation, ContentParser, GraphicsState, Operand};
use crate::engine::PageResources;
use crate::error::{Result, WellfriendError};
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::fonts::{FontDecodeSource, FontResolver};
use crate::info::decode_pdf_text_string;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

type ObjectId = (u32, u16);
type FontCache = HashMap<String, Arc<FontResolver>>;

/// One occurrence, not merely a shared Form object. The root operator index is
/// in the concatenated page program; nested indexes are local to their Form.
/// These are extraction provenance, not revision-bound edit target IDs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFormInvocation {
    pub object: ObjectId,
    pub resource_name: String,
    pub operator_index: usize,
}

/// Source normal appearance occurrence. Annotation array position is preserved
/// separately from nested Do indexes; neither is a revision-bound edit target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextAppearanceInvocation {
    pub annotation: ObjectId,
    pub annotation_index: usize,
    pub stream: ObjectId,
}

#[derive(Debug, Clone)]
pub struct ScopedTextChunk {
    pub chunk: TextChunk,
    pub form_path: Vec<TextFormInvocation>,
    pub appearance: Option<TextAppearanceInvocation>,
    pub mcid: Option<i64>,
    /// None denotes page ownership. A Form MCID is never a page MCID merely
    /// because its integer value is equal.
    pub mcid_owner: Option<ObjectId>,
    pub mcid_stream_owner: Option<ObjectId>,
}

impl ScopedTextChunk {
    /// Preserve logical ownership for the stream-aware semantic/tag bridge.
    pub fn into_marked(self) -> MarkedTextChunk {
        MarkedTextChunk {
            chunk: self.chunk,
            mcid: self.mcid,
            mcid_owner: self.mcid_owner,
            mcid_stream_owner: self.mcid_stream_owner,
        }
    }

    /// Explicitly lossy projection for a consumer that only supports page IDs.
    /// The engine's semantic bridge uses `into_marked`, not this projection.
    pub fn into_page_marked(self) -> MarkedTextChunk {
        MarkedTextChunk {
            chunk: self.chunk,
            mcid_owner: None,
            mcid_stream_owner: None,
            mcid: if self.mcid_owner.is_none() {
                self.mcid
            } else {
                None
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct TextTraversalLimits {
    pub decode: DecodeLimits,
    pub max_form_depth: usize,
    pub max_form_invocations: usize,
    pub max_form_decoded_bytes: u64,
    pub max_operations: usize,
    pub max_marked_depth: usize,
    pub max_graphics_depth: usize,
    pub max_chunks: usize,
    pub max_text_bytes: usize,
}

impl Default for TextTraversalLimits {
    fn default() -> Self {
        Self {
            decode: DecodeLimits::default(),
            max_form_depth: 64,
            // Form-heavy technical PDFs commonly reuse tiny glyph, symbol, or
            // layout Forms many thousands of times on one page. Invocation
            // count alone is not a useful memory bound: decoded source bytes,
            // total operations, emitted chunks, recursion depth, and output
            // text are all bounded separately below. Keep a finite cycle/work
            // guard, but do not reject otherwise valid pages at 4,096 uses.
            max_form_invocations: 65_536,
            max_form_decoded_bytes: 256 * 1024 * 1024,
            max_operations: 1_000_000,
            max_marked_depth: 128,
            max_graphics_depth: 256,
            max_chunks: 1_000_000,
            max_text_bytes: 64 * 1024 * 1024,
        }
    }
}

struct FormProgram {
    operations: Vec<ContentOperation>,
    resources: Option<Arc<PageResources>>,
    matrix: [f64; 6],
}

struct Marker {
    actual_text: Option<String>,
    emitted: bool,
    mcid: Option<i64>,
}

struct Walk<'a> {
    reader: &'a PdfReader,
    page_resources: Arc<PageResources>,
    limits: &'a TextTraversalLimits,
    cancel: &'a CancelToken,
    programs: HashMap<ObjectId, Option<Arc<FormProgram>>>,
    fonts: HashMap<ObjectId, FontCache>,
    active: HashSet<ObjectId>,
    path: Vec<TextFormInvocation>,
    appearance: Option<TextAppearanceInvocation>,
    invocations: usize,
    decoded_bytes: u64,
    operations: usize,
    chunks: usize,
    text_bytes: usize,
}

fn malformed(message: impl Into<String>) -> WellfriendError {
    WellfriendError::MalformedPdf(format!("text extraction: {}", message.into()))
}

fn scoped_marked_content_operand_refusal(op: &ContentOperation) -> Option<String> {
    if op.operator == "BDC" && op.operands.len() >= 2 {
        // A material amount of real-world TeX output prefixes BDC with a
        // stray name (for example `/S /Span <<...>> BDC`). PDF processors in
        // the field recover this by treating the final name/dictionary pair as
        // the BDC operands. Do the same for logical extraction while retaining
        // strict validation for every other malformed marked-content shape.
        let tag = &op.operands[op.operands.len() - 2];
        let properties = &op.operands[op.operands.len() - 1];
        if tag.as_name().is_some()
            && matches!(properties, Operand::Name(_) | Operand::Dictionary(_))
        {
            return None;
        }
    }
    crate::render::plan::marked_content_operand_refusal(op)
}

fn charge(current: &mut usize, amount: usize, maximum: usize, what: &str) -> Result<()> {
    *current = current.checked_add(amount).ok_or_else(|| {
        WellfriendError::ResourceLimit(format!("text extraction {what} overflow"))
    })?;
    if *current > maximum {
        return Err(WellfriendError::ResourceLimit(format!(
            "text extraction {what} exceeds {maximum}"
        )));
    }
    Ok(())
}

impl<'a> TextCollector<'a> {
    /// Extract a whole page's Form occurrences with bounded work, strict stream
    /// decoding and explicit errors. Includes invisible OCR just like `collect`.
    /// Does not execute glyph CharProcs, patterns, masks or annotation appearances
    /// as independent logical text (doing so would duplicate glyphs/mask content).
    pub fn collect_scoped(
        &mut self,
        operations: &[ContentOperation],
        limits: &TextTraversalLimits,
        cancel: &CancelToken,
    ) -> Result<Vec<ScopedTextChunk>> {
        self.collect_scoped_with_appearances(operations, &[], limits, cancel)
    }

    /// Logical source extraction including the selected normal appearance of
    /// each annotation. This does not apply screen/print visibility, optional
    /// content, clipping or synthesize missing appearances. It is not a visible
    /// pixel or form-value extractor. All programs share one traversal budget.
    pub fn collect_scoped_with_appearances(
        &mut self,
        operations: &[ContentOperation],
        annotations: &[PdfObject],
        limits: &TextTraversalLimits,
        cancel: &CancelToken,
    ) -> Result<Vec<ScopedTextChunk>> {
        let reader = self
            .reader
            .ok_or_else(|| malformed("Form traversal requires a PDF reader"))?;
        self.gs = GraphicsState::new();
        self.selected_font = None;
        self.selected_font_stack.clear();
        self.font_resolvers.clear();
        let mut walk = Walk {
            reader,
            page_resources: self.resources.clone(),
            limits,
            cancel,
            programs: HashMap::new(),
            fonts: HashMap::new(),
            active: HashSet::new(),
            path: Vec::new(),
            appearance: None,
            invocations: 0,
            decoded_bytes: 0,
            operations: 0,
            chunks: 0,
            text_bytes: 0,
        };
        // The synchronous decoder and font resolver observe the same token.
        cancel.scope(|| {
            let mut chunks = walk.program(self, operations, false)?;
            for (index, annotation) in annotations.iter().enumerate() {
                cancel.check("annotation text extraction")?;
                charge(
                    &mut walk.invocations,
                    1,
                    limits.max_form_invocations,
                    "Form/appearance invocations",
                )?;
                chunks.extend(walk.appearance(self, annotation, index)?);
            }
            Ok(chunks)
        })
    }
}

impl Walk<'_> {
    fn appearance(
        &mut self,
        collector: &mut TextCollector<'_>,
        annotation: &PdfObject,
        index: usize,
    ) -> Result<Vec<ScopedTextChunk>> {
        let object = self.reader.resolve(annotation.clone())?;
        let dict = object
            .as_dict()
            .ok_or_else(|| malformed("annotation is not a dictionary"))?;
        let Some(selected) = crate::annotation_appearance::select_normal(dict, self.reader)? else {
            return Ok(Vec::new());
        };
        if self.limits.max_form_depth == 0 {
            return Err(WellfriendError::ResourceLimit(
                "text appearance depth exceeded".into(),
            ));
        }
        let annotation = annotation
            .as_reference()
            .ok_or_else(|| malformed("source appearance annotation must be indirect"))?;
        let stream = selected
            .stream
            .ok_or_else(|| malformed("source appearance stream must be indirect"))?;
        let rect = crate::annotation_appearance::rectangle(dict, "Rect", self.reader)?;
        let bbox = crate::annotation_appearance::rectangle(&selected.dict, "BBox", self.reader)?;
        let Some(program) = self.load(stream)? else {
            return Err(malformed("annotation appearance must be a Form"));
        };
        let Some(placement) = crate::annotation_appearance::placement(rect, bbox, program.matrix)
        else {
            return Ok(Vec::new());
        };
        let saved_resources = std::mem::replace(
            &mut collector.resources,
            program
                .resources
                .clone()
                .unwrap_or_else(|| self.page_resources.clone()),
        );
        collector.gs = GraphicsState::new();
        collector.gs.ctm =
            crate::content::state::concat_matrix(&program.matrix, &placement.to_array());
        collector.selected_font = None;
        collector.selected_font_stack.clear();
        collector.font_resolvers.clear();
        self.appearance = Some(TextAppearanceInvocation {
            annotation,
            annotation_index: index,
            stream,
        });
        self.active.insert(stream);
        let result = self.program(collector, &program.operations, false);
        self.active.remove(&stream);
        self.appearance = None;
        collector.resources = saved_resources;
        result
    }

    fn load(&mut self, id: ObjectId) -> Result<Option<Arc<FormProgram>>> {
        if let Some(program) = self.programs.get(&id) {
            return Ok(program.clone());
        }
        self.cancel.check("text Form loading")?;
        let stream = self.reader.get_object(id.0, id.1)?;
        let (dict, _) = stream
            .as_stream()
            .ok_or_else(|| malformed("XObject is not a stream"))?;
        match dict.get_name("Subtype") {
            Some("Image") => {
                self.programs.insert(id, None);
                return Ok(None);
            }
            Some("Form") => {}
            _ => return Err(malformed("unknown XObject subtype")),
        }
        let resources = PageResources::from_content_owner(dict, self.reader)?.map(Arc::new);
        let matrix = matrix(dict, self.reader)?;
        let remaining = self
            .limits
            .max_form_decoded_bytes
            .saturating_sub(self.decoded_bytes);
        let mut decode = self.limits.decode.clone();
        decode.max_decoded_bytes_per_stream = decode.max_decoded_bytes_per_stream.min(remaining);
        let decoded = decode_stream_lossless_with_limits(&stream, self.reader, &decode)?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(malformed("Form content ended at an image-only filter"));
        }
        self.decoded_bytes = self
            .decoded_bytes
            .checked_add(decoded.data.len() as u64)
            .ok_or_else(|| malformed("decoded Form byte counter overflow"))?;
        if self.decoded_bytes > self.limits.max_form_decoded_bytes {
            return Err(WellfriendError::ResourceLimit(
                "text Form decode budget exceeded".into(),
            ));
        }
        self.cancel.check("text Form parsing")?;
        let program = Arc::new(FormProgram {
            operations: ContentParser::parse_cancellable(&decoded.data, self.cancel)?,
            resources,
            matrix,
        });
        self.programs.insert(id, Some(program.clone()));
        Ok(Some(program))
    }

    fn invoke(
        &mut self,
        collector: &mut TextCollector<'_>,
        name: &str,
        operator_index: usize,
        ancestor_actual_text: bool,
    ) -> Result<Vec<ScopedTextChunk>> {
        let id = collector
            .resources
            .xobjects
            .get(name)
            .copied()
            .ok_or_else(|| malformed(format!("missing XObject /{name} in current scope")))?;
        let Some(program) = self.load(id)? else {
            return Ok(Vec::new());
        };
        charge(
            &mut self.invocations,
            1,
            self.limits.max_form_invocations,
            "Form invocations",
        )?;
        let depth = self.path.len() + usize::from(self.appearance.is_some());
        if depth >= self.limits.max_form_depth || depth >= 128 {
            return Err(WellfriendError::ResourceLimit(
                "text Form depth exceeded".into(),
            ));
        }
        if !self.active.insert(id) {
            return Err(malformed("recursive Form invocation"));
        }
        let resources = program
            .resources
            .clone()
            .unwrap_or_else(|| self.page_resources.clone());
        let saved_resources = std::mem::replace(&mut collector.resources, resources);
        let saved_fonts = std::mem::replace(
            &mut collector.font_resolvers,
            self.fonts.remove(&id).unwrap_or_default(),
        );
        let saved_gs = collector.gs.clone();
        let saved_font = collector.selected_font.clone();
        let saved_font_stack = std::mem::take(&mut collector.selected_font_stack);
        // q/Q inside this Form cannot consume the caller's saved state. Retain
        // the selected font object independently of the Form's font-name map.
        collector.gs.clear_saved_states();
        collector.gs.ctm = crate::content::state::concat_matrix(&program.matrix, &collector.gs.ctm);
        self.path.push(TextFormInvocation {
            object: id,
            resource_name: name.to_owned(),
            operator_index,
        });
        let result = self.program(collector, &program.operations, ancestor_actual_text);
        self.path.pop();
        self.active.remove(&id);
        collector.resources = saved_resources;
        self.fonts.insert(
            id,
            std::mem::replace(&mut collector.font_resolvers, saved_fonts),
        );
        collector.gs = saved_gs;
        collector.selected_font = saved_font;
        collector.selected_font_stack = saved_font_stack;
        result
    }

    fn program(
        &mut self,
        collector: &mut TextCollector<'_>,
        operations: &[ContentOperation],
        ancestor_actual_text: bool,
    ) -> Result<Vec<ScopedTextChunk>> {
        let mut output = Vec::new();
        let mut markers: Vec<Marker> = Vec::new();
        let mut in_text = false;
        for (index, op) in operations.iter().enumerate() {
            self.cancel.check("text content traversal")?;
            charge(
                &mut self.operations,
                1,
                self.limits.max_operations,
                "operations",
            )?;
            if let Some(reason) = crate::render::plan::text_operand_refusal(op)
                .or_else(|| crate::render::plan::graphics_state_operand_refusal(op))
                .or_else(|| scoped_marked_content_operand_refusal(op))
                .or_else(|| crate::render::plan::resource_invocation_operand_refusal(op))
            {
                return Err(malformed(reason));
            }
            match op.operator.as_str() {
                "BMC" | "BDC" => {
                    if markers.len() >= self.limits.max_marked_depth {
                        return Err(WellfriendError::ResourceLimit(
                            "text marked-content depth exceeded".into(),
                        ));
                    }
                    markers.push(self.marker(collector, op)?);
                    continue;
                }
                "EMC" => {
                    markers.pop().ok_or_else(|| malformed("unmatched EMC"))?;
                    continue;
                }
                "BT" => {
                    if in_text {
                        return Err(malformed("nested text object"));
                    }
                    in_text = true;
                }
                "ET" => {
                    if !in_text {
                        return Err(malformed("unmatched ET"));
                    }
                    in_text = false;
                }
                "Q" if collector.gs.stack_depth() == 0 => return Err(malformed("unmatched Q")),
                "q" if collector.gs.stack_depth() >= self.limits.max_graphics_depth => {
                    return Err(WellfriendError::ResourceLimit(
                        "text graphics-state depth exceeded".into(),
                    ));
                }
                "Tj" | "TJ" | "'" | "\"" if !in_text => {
                    return Err(malformed("text shown outside a text object"))
                }
                _ => {}
            }
            let active_mcid = markers.iter().rev().find_map(|marker| marker.mcid);
            let owner = self
                .path
                .last()
                .map(|entry| entry.object)
                .or_else(|| self.appearance.as_ref().map(|entry| entry.stream));
            let stream_owner = if self.path.is_empty() {
                self.appearance.as_ref().map(|entry| entry.annotation)
            } else {
                None
            };
            let mut emitted = if op.operator == "Do" {
                if in_text {
                    return Err(malformed("Form/image invocation inside a text object"));
                }
                let name = op
                    .name(0)
                    .ok_or_else(|| malformed("Do operand is not a resource name"))?;
                self.invoke(
                    collector,
                    name,
                    index,
                    ancestor_actual_text
                        || markers.iter().any(|marker| marker.actual_text.is_some()),
                )?
            } else {
                self.validate_text_state(collector, op)?;
                let mut chunks = Vec::new();
                collector.process_op_checked(
                    op,
                    &mut chunks,
                    Some(&mut super::TextEmissionBudget {
                        remaining_bytes: self.limits.max_text_bytes.saturating_sub(self.text_bytes),
                        remaining_chunks: self.limits.max_chunks.saturating_sub(self.chunks),
                        cancel: self.cancel,
                    }),
                )?;
                for chunk in &chunks {
                    if [chunk.x, chunk.y, chunk.width, chunk.font_size]
                        .iter()
                        .any(|value| !value.is_finite())
                    {
                        return Err(malformed("nonfinite text geometry after transformation"));
                    }
                    charge(&mut self.chunks, 1, self.limits.max_chunks, "chunks")?;
                    charge(
                        &mut self.text_bytes,
                        chunk.text.len(),
                        self.limits.max_text_bytes,
                        "text bytes",
                    )?;
                }
                chunks
                    .into_iter()
                    .map(|chunk| ScopedTextChunk {
                        chunk,
                        form_path: self.path.clone(),
                        appearance: self.appearance.clone(),
                        mcid: active_mcid,
                        mcid_owner: owner,
                        mcid_stream_owner: stream_owner,
                    })
                    .collect::<Vec<_>>()
            };
            // Parent ActualText owns the logical replacement of the invocation,
            // not just the direct glyphs. Inner marker scopes cannot pop it.
            if !emitted.is_empty() {
                if let Some(marker) = markers
                    .iter_mut()
                    .find(|marker| !ancestor_actual_text && marker.actual_text.is_some())
                {
                    if marker.emitted || marker.actual_text.as_deref() == Some("") {
                        emitted.clear();
                    } else if let Some(text) = &marker.actual_text {
                        charge(
                            &mut self.text_bytes,
                            text.len(),
                            self.limits.max_text_bytes,
                            "ActualText bytes",
                        )?;
                        emitted.truncate(1);
                        let first = &mut emitted[0];
                        first.chunk.text.clone_from(text);
                        first.chunk.is_actual_text = true;
                        first.chunk.is_rtl = super::is_rtl_dominant(text);
                        first.chunk.mapping_sources =
                            vec![FontDecodeSource::ActualText; text.chars().count()];
                        first.mcid = active_mcid;
                        first.mcid_owner = owner;
                        first.mcid_stream_owner = stream_owner;
                    }
                    marker.emitted = true;
                } else if let Some(mcid) = active_mcid {
                    for chunk in &mut emitted {
                        if chunk.mcid.is_none() {
                            chunk.mcid = Some(mcid);
                            chunk.mcid_owner = owner;
                            chunk.mcid_stream_owner = stream_owner;
                        }
                    }
                }
            }
            output.extend(emitted);
        }
        if in_text || !markers.is_empty() || collector.gs.stack_depth() != 0 {
            return Err(malformed(
                "unbalanced text, marked-content or graphics-state scope",
            ));
        }
        Ok(output)
    }

    fn validate_text_state(
        &self,
        collector: &TextCollector<'_>,
        op: &ContentOperation,
    ) -> Result<()> {
        match op.operator.as_str() {
            "Tf" => {
                let name = op
                    .name(0)
                    .ok_or_else(|| malformed("invalid Tf font name"))?;
                if !collector.resources.fonts.contains_key(name) {
                    return Err(malformed(format!("missing font /{name} in current scope")));
                }
            }
            "gs" => {
                let name = op.name(0).ok_or_else(|| malformed("invalid gs name"))?;
                let dict = collector
                    .resources
                    .ext_g_states
                    .get(name)
                    .ok_or_else(|| malformed(format!("missing ExtGState /{name}")))?;
                let mut gs = collector.gs.clone();
                gs.try_apply_ext_g_state_for_text(dict, name)
                    .map_err(malformed)?;
                if dict.get("Font").is_some()
                    && !collector.resources.fonts.contains_key(&gs.text.font_name)
                {
                    return Err(malformed("ExtGState selected an unavailable font"));
                }
            }
            "Tj" | "TJ" | "'" | "\"" if collector.selected_font.is_none() => {
                return Err(malformed("text shown without a resolved font"));
            }
            _ => {}
        }
        Ok(())
    }

    fn marker(&self, collector: &TextCollector<'_>, op: &ContentOperation) -> Result<Marker> {
        let property_operand = op.operands.last();
        let (actual_text, mcid) = if op.operator == "BMC" {
            (None, None)
        } else if let Some(Operand::Name(name)) = property_operand {
            let object = collector
                .resources
                .properties
                .get(name)
                .ok_or_else(|| malformed(format!("missing property list /{name}")))?;
            let object = self.reader.resolve(object.clone())?;
            let dict = object
                .as_dict()
                .ok_or_else(|| malformed("property list is not a dictionary"))?;
            let text = dict
                .get("ActualText")
                .map(|value| self.reader.resolve(value.clone()))
                .transpose()?;
            let text = match text {
                Some(PdfObject::String(bytes)) => Some(decode_pdf_text_string(&bytes)),
                Some(PdfObject::Null) | None => None,
                _ => return Err(malformed("ActualText is not a text string")),
            };
            let mcid = dict
                .get("MCID")
                .map(|value| self.reader.resolve(value.clone()))
                .transpose()?;
            let mcid = match mcid {
                Some(PdfObject::Integer(value)) if value >= 0 => Some(value),
                Some(PdfObject::Null) | None => None,
                _ => return Err(malformed("MCID is not a nonnegative integer")),
            };
            (text, mcid)
        } else if let Some(Operand::Dictionary(entries)) = property_operand {
            let text = match entries
                .iter()
                .find(|(key, _)| key == "ActualText")
                .map(|(_, value)| value)
            {
                Some(Operand::String(bytes)) => Some(decode_pdf_text_string(bytes)),
                Some(Operand::Null) | None => None,
                _ => return Err(malformed("ActualText is not a text string")),
            };
            let mcid = match entries
                .iter()
                .find(|(key, _)| key == "MCID")
                .map(|(_, value)| value)
            {
                Some(Operand::Integer(value)) if *value >= 0 => Some(*value),
                Some(Operand::Null) | None => None,
                _ => return Err(malformed("MCID is not a nonnegative integer")),
            };
            (text, mcid)
        } else {
            return Err(malformed("BDC requires a property list"));
        };
        Ok(Marker {
            actual_text,
            emitted: false,
            mcid,
        })
    }
}

fn matrix(dict: &PdfDictionary, reader: &PdfReader) -> Result<[f64; 6]> {
    let Some(value) = dict.get("Matrix") else {
        return Ok(crate::content::state::IDENTITY_MATRIX);
    };
    let value = reader.resolve(value.clone())?;
    if matches!(value, PdfObject::Null) {
        return Ok(crate::content::state::IDENTITY_MATRIX);
    }
    let entries = value
        .as_array()
        .filter(|items| items.len() == 6)
        .ok_or_else(|| malformed("invalid Form Matrix"))?;
    let mut matrix = [0.0; 6];
    for (slot, value) in matrix.iter_mut().zip(entries) {
        *slot = reader
            .resolve(value.clone())?
            .as_number()
            .filter(|v| v.is_finite())
            .ok_or_else(|| malformed("nonfinite/non-numeric Form Matrix"))?;
    }
    Ok(matrix)
}

#[cfg(test)]
#[path = "scoped_collector_tests.rs"]
mod tests;
