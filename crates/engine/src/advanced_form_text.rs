//! Form-local text selection and mutation over the native source writer.
//! Coordinates are in the selected Form's content space, before /Matrix and
//! its caller's transforms. No page flattening or surrogate PDF is produced.
use super::*;
use crate::{PdfDictionary, PdfReader};

#[cfg(test)]
#[path = "advanced_form_text_tests.rs"]
mod tests;

#[path = "advanced_appearance_text.rs"]
pub mod appearance;

const MAX_FORM_DEPTH: usize = 8;
const MAX_FORM_OCCURRENCES: usize = 4096;
const MAX_TOTAL_DECODED: usize = 256 * 1024 * 1024;
const SCHEMA_VERSION: &str = "advanced_editing.form-text-occurrences.v1";
type ObjectRef = (u32, u16);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FormTextTarget {
    pub input_sha256: String,
    pub page: usize,
    pub content_stream_index: usize,
    pub invocation_path: Vec<VectorFormInvocation>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormTextOccurrence {
    pub target: FormTextTarget,
    pub text: MultiRunRangeModel,
    pub external_actual_text_owner: bool,
    pub coordinate_space: String,
    pub form_bbox: [f64; 4],
}

#[derive(Debug, Clone, Serialize)]
pub struct FormTextInventory {
    pub schema_version: String,
    pub input_sha256: String,
    pub page: usize,
    pub occurrences: Vec<FormTextOccurrence>,
    pub limits: Vec<String>,
}

pub(crate) struct OcrFormScope {
    pub source_page: crate::document::PdfPage,
    pub initial: ScannedTextTokenState,
    pub span_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormTextEditRequest {
    pub target: FormTextTarget,
    pub edit: MultiRunTextRangeRequest,
    pub shared_form_policy: SharedFormEditPolicy,
}

#[derive(Debug, Clone, Serialize)]
pub struct FormTextEditReport {
    pub schema_version: String,
    pub target_before: FormTextTarget,
    pub target_after: FormTextTarget,
    pub shared_form_policy: SharedFormEditPolicy,
    pub native_edit: MultiRunTextEditReport,
    pub direct_text_before: String,
    pub direct_text_after: String,
    pub whole_direct_text_verified: bool,
    pub source_form_retained: bool,
    pub limits: Vec<String>,
}

#[derive(Clone)]
pub(crate) struct Scope {
    pub(super) source_page: crate::document::PdfPage,
    pub(super) initial: ScannedTextTokenState,
    page: crate::document::PdfPage,
    target: ScopeTarget,
    owner_resources: Vec<PdfDictionary>,
    external_actual_text: bool,
    policy: SharedFormEditPolicy,
    annotation_update: Option<PdfDictionary>,
    tagged_clone: crate::tagged_structure::stream_clones::TaggedCloneOptions,
}

#[derive(Clone)]
enum ScopeTarget {
    PageForm(FormTextTarget),
    Appearance(appearance::AppearanceTextTarget),
}

impl ScopeTarget {
    fn path(&self) -> &[VectorFormInvocation] {
        match self {
            Self::PageForm(target) => &target.invocation_path,
            Self::Appearance(target) => &target.invocation_path,
        }
    }
    fn set_revision(&mut self, revision: String) {
        match self {
            Self::PageForm(target) => target.input_sha256 = revision,
            Self::Appearance(target) => target.input_sha256 = revision,
        }
    }
    fn as_form(&self) -> Result<&FormTextTarget> {
        match self {
            Self::PageForm(target) => Ok(target),
            _ => Err(fail("not a page Form target")),
        }
    }
}

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("Form text: {message}"))
}
fn reference(id: ObjectRef) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}
#[cfg(test)]
fn stream_ref(invocation: &VectorFormInvocation) -> ObjectRef {
    (invocation.form_object, invocation.form_generation)
}

fn occurrence_key(target: &ScopeTarget) -> String {
    let target = match target {
        ScopeTarget::Appearance(target) => return appearance::occurrence_key(target),
        ScopeTarget::PageForm(target) => target,
    };
    format!(
        "form:{}:{}:{}",
        target.page,
        target.content_stream_index,
        target
            .invocation_path
            .iter()
            .map(|item| item.owner_operation_byte_start.to_string())
            .collect::<Vec<_>>()
            .join("/")
    )
}
fn dictionary(reader: &PdfReader, value: Option<&PdfObject>) -> Result<Option<PdfDictionary>> {
    match value
        .map(|value| reader.resolve(value.clone()))
        .transpose()?
    {
        None | Some(PdfObject::Null) => Ok(None),
        Some(PdfObject::Dictionary(dictionary)) => Ok(Some(dictionary)),
        _ => Err(fail("invalid resource dictionary")),
    }
}

struct DecodedSources<'a> {
    reader: &'a PdfReader,
    streams: BTreeMap<ObjectRef, Arc<Vec<u8>>>,
    bytes: usize,
}
impl<'a> DecodedSources<'a> {
    fn new(reader: &'a PdfReader) -> Self {
        Self {
            reader,
            streams: BTreeMap::new(),
            bytes: 0,
        }
    }
    fn get(&mut self, id: ObjectRef) -> Result<Arc<Vec<u8>>> {
        if let Some(data) = self.streams.get(&id) {
            return Ok(Arc::clone(data));
        }
        if self.bytes >= MAX_TOTAL_DECODED {
            return Err(WellfriendError::ResourceLimit(
                "Form text decoded-byte budget exhausted".into(),
            ));
        }
        crate::cancel::check_current_cancel("Form text source decode")?;
        let decoded = decode_stream_lossless_with_limits(
            &self.reader.get_object(id.0, id.1)?,
            self.reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_TOTAL_DECODED.saturating_sub(self.bytes) as u64,
                ..Default::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(fail("source is not losslessly decodable"));
        }
        self.bytes = self
            .bytes
            .checked_add(decoded.data.len())
            .ok_or_else(|| fail("decode budget overflow"))?;
        if self.bytes > MAX_TOTAL_DECODED {
            return Err(WellfriendError::ResourceLimit(
                "Form text decoded-byte budget exceeded".into(),
            ));
        }
        let data = Arc::new(decoded.data);
        self.streams.insert(id, Arc::clone(&data));
        Ok(data)
    }
}

/// Bind inherited *objects*, not resource-name spellings. Child /F1 may be an
/// entirely different font than the already selected caller /F1.
fn bind_resource(
    reader: &PdfReader,
    parent: &PdfDictionary,
    child: &mut PdfDictionary,
    category: &str,
    name: &str,
) -> Result<String> {
    let source = dictionary(reader, parent.get(category))?
        .and_then(|dictionary| dictionary.get(name).cloned())
        .ok_or_else(|| fail("inherited resource cannot be resolved"))?;
    let mut target = dictionary(reader, child.get(category))?.unwrap_or_default();
    if target.get(name) == Some(&source) {
        return Ok(name.to_string());
    }
    if let Some((existing, _)) = target.iter().find(|(_, value)| **value == source) {
        return Ok(existing.clone());
    }
    let alias = (0..10_000)
        .map(|index| format!("WFInherited{category}{index}"))
        .find(|name| !target.contains_key(name))
        .ok_or_else(|| fail("inherited alias budget exhausted"))?;
    target.insert(&alias, source);
    child.insert(category, PdfObject::Dictionary(target));
    Ok(alias)
}

fn remap_paint(
    reader: &PdfReader,
    parent: &PdfDictionary,
    child: &mut PdfDictionary,
    command: &str,
) -> Result<String> {
    let tokens = lex_content(command.as_bytes())?;
    let mut operands = Vec::new();
    let mut changes = Vec::new();
    for token in tokens {
        let LexicalKind::Word(operator) = &token.kind else {
            operands.push(token);
            continue;
        };
        let category = match operator.as_str() {
            "cs" | "CS" => Some("ColorSpace"),
            "scn" | "SCN" => Some("Pattern"),
            _ => None,
        };
        if let Some(category) = category {
            for operand in &operands {
                if let LexicalKind::Name(name) = &operand.kind {
                    if category == "ColorSpace"
                        && matches!(
                            name.as_str(),
                            "DeviceGray" | "DeviceRGB" | "DeviceCMYK" | "Pattern"
                        )
                    {
                        continue;
                    }
                    let alias = bind_resource(reader, parent, child, category, name)?;
                    if alias != *name {
                        changes.push((operand.start, operand.end, format!("/{alias}")));
                    }
                }
            }
        }
        operands.clear();
    }
    let mut output = command.as_bytes().to_vec();
    for (start, end, replacement) in changes.into_iter().rev() {
        output.splice(start..end, replacement.bytes());
    }
    String::from_utf8(output).map_err(|_| fail("invalid inherited paint command"))
}

fn enter_form(
    reader: &PdfReader,
    parent: &PdfDictionary,
    page_resources: &PdfDictionary,
    form: &PdfDictionary,
    mut state: ScannedTextTokenState,
) -> Result<(PdfDictionary, ScannedTextTokenState, bool)> {
    let mut parent = parent.clone();
    let parsed_parent = PageResources::from_dict(&parent, reader);
    crate::ext_gstate_fonts::materialize(&parsed_parent, &mut parent, reader)?;
    // Legacy omitted Resources falls back to the PAGE, not the enclosing
    // Form. Already selected graphics-state objects still belong to parent.
    let mut resources =
        dictionary(reader, form.get("Resources"))?.unwrap_or_else(|| page_resources.clone());
    // Replaying inherited `rg` under a different DefaultRGB would select a
    // different colour space. Keep the source inline state, but do not claim
    // that style reconstruction can reproduce that paint through this command.
    // A subsequent local colour operator makes its own resource scope explicit.
    let parent_spaces = dictionary(reader, parent.get("ColorSpace"))?.unwrap_or_default();
    let child_spaces = dictionary(reader, resources.get("ColorSpace"))?.unwrap_or_default();
    let changed_default = |command: &str| -> Result<bool> {
        let tokens = lex_content(command.as_bytes())?;
        let key = match tokens.last().map(|token| &token.kind) {
            Some(LexicalKind::Word(operator)) => match operator.as_str() {
                "g" | "G" => Some("DefaultGray"),
                "rg" | "RG" => Some("DefaultRGB"),
                "k" | "K" => Some("DefaultCMYK"),
                _ => None,
            },
            _ => None,
        };
        Ok(key.is_some_and(|key| parent_spaces.get(key) != child_spaces.get(key)))
    };
    state.unsupported_fill_paint_state |= changed_default(&state.fill_color_command)?;
    state.unsupported_stroke_paint_state |= changed_default(&state.stroke_color_command)?;
    let mut external = state.actual_text_stack.iter().any(Option::is_some)
        || state.actual_text_conflict_stack.iter().any(|value| *value);
    let properties = dictionary(reader, parent.get("Properties"))?.unwrap_or_default();
    for name in state.named_property_stack.iter().flatten() {
        let property = dictionary(reader, properties.get(name))?
            .ok_or_else(|| fail("unresolved inherited marked-content property"))?;
        external |= property
            .get("ActualText")
            .is_some_and(|value| !matches!(value, PdfObject::Null));
    }
    if !state.font_name.is_empty() {
        state.font_name = bind_resource(reader, &parent, &mut resources, "Font", &state.font_name)?;
    }
    state.fill_color_command =
        remap_paint(reader, &parent, &mut resources, &state.fill_color_command)?;
    state.stroke_color_command =
        remap_paint(reader, &parent, &mut resources, &state.stroke_color_command)?;
    state.fill_color_space_command = remap_paint(
        reader,
        &parent,
        &mut resources,
        &state.fill_color_space_command,
    )?;
    state.stroke_color_space_command = remap_paint(
        reader,
        &parent,
        &mut resources,
        &state.stroke_color_space_command,
    )?;
    for name in state.named_property_stack.iter_mut().flatten() {
        *name = bind_resource(reader, &parent, &mut resources, "Properties", name)?;
    }
    state.graphics_stack.clear();
    state.position = state.position.form_entry()?;
    Ok((resources, state, external))
}

fn discover_scopes(
    engine: &ContentEngine,
    page_number: usize,
    revision: &str,
) -> Result<Vec<Scope>> {
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let mut cache = DecodedSources::new(reader);
    let mut output = Vec::new();
    let mut state = ScannedTextTokenState::default();
    let resources = PageResources::from_dict(&page.resources, reader);
    let mut metrics = inline_text::Metrics::new(&resources, reader);
    for (index, id) in page.contents.iter().copied().enumerate() {
        let data = cache.get(id)?;
        let mut calls = Vec::new();
        scan_text_program(
            &data,
            &mut state,
            Some(id),
            Some(&mut metrics),
            Some(&mut calls),
        )?;
        visit_calls(
            &page,
            revision,
            index,
            id,
            &page.resources,
            &[],
            &[],
            None,
            calls,
            &mut Vec::new(),
            &mut cache,
            &mut output,
        )?;
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn visit_calls(
    page: &crate::document::PdfPage,
    revision: &str,
    index: usize,
    owner: ObjectRef,
    resources: &PdfDictionary,
    path: &[VectorFormInvocation],
    owner_resources: &[PdfDictionary],
    appearance_root: Option<&appearance::AppearanceTextTarget>,
    calls: Vec<TextFormInvocationState>,
    active: &mut Vec<ObjectRef>,
    cache: &mut DecodedSources<'_>,
    output: &mut Vec<Scope>,
) -> Result<()> {
    let reader = cache.reader;
    let xobjects = dictionary(reader, resources.get("XObject"))?.unwrap_or_default();
    for call in calls {
        crate::cancel::check_current_cancel("Form text occurrence discovery")?;
        let Some(id) = xobjects.get(&call.name).and_then(PdfObject::as_reference) else {
            continue;
        };
        let PdfObject::Stream { dict, .. } = reader.get_object(id.0, id.1)? else {
            continue;
        };
        if dict.get_name("Subtype") != Some("Form") {
            continue;
        }
        if path.len() + usize::from(appearance_root.is_some()) >= MAX_FORM_DEPTH
            || output.len() >= MAX_FORM_OCCURRENCES
        {
            return Err(WellfriendError::ResourceLimit(
                "Form text occurrence/depth budget exceeded".into(),
            ));
        }
        if active.contains(&id) {
            return Err(fail("cyclic Form invocation graph"));
        }
        let (child_resources, initial, external) =
            enter_form(reader, resources, &page.resources, &dict, call.state)?;
        let mut path = path.to_vec();
        path.push(VectorFormInvocation {
            resource_name: call.name,
            owner_stream_object: owner.0,
            owner_stream_generation: owner.1,
            owner_operation_byte_start: call.start,
            owner_operation_byte_end: call.end,
            form_object: id.0,
            form_generation: id.1,
            depth: path.len() + 1,
        });
        let mut owners = owner_resources.to_vec();
        owners.push(resources.clone());
        let mut source_page = page.clone();
        let bbox = reader.resolve(
            dict.get("BBox")
                .cloned()
                .ok_or_else(|| fail("Form has no BBox"))?,
        )?;
        let bbox = bbox
            .as_array()
            .filter(|items| items.len() == 4)
            .and_then(|items| {
                items
                    .iter()
                    .map(PdfObject::as_number)
                    .collect::<Option<Vec<_>>>()
            })
            .and_then(|values| <[f64; 4]>::try_from(values).ok())
            .ok_or_else(|| fail("invalid Form BBox"))?;
        if bbox.iter().any(|value| !value.is_finite()) || bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            return Err(fail("empty or invalid Form content geometry"));
        }
        source_page.media_box = bbox;
        source_page.crop_box = bbox;
        source_page.contents = vec![id];
        source_page.resources = child_resources.clone();
        output.push(Scope {
            source_page,
            initial: initial.clone(),
            page: page.clone(),
            target: if let Some(root) = appearance_root {
                let mut target = root.clone();
                target.invocation_path = path.clone();
                ScopeTarget::Appearance(target)
            } else {
                ScopeTarget::PageForm(FormTextTarget {
                    input_sha256: revision.to_string(),
                    page: page.page_number,
                    content_stream_index: index,
                    invocation_path: path.clone(),
                })
            },
            owner_resources: owners.clone(),
            external_actual_text: external,
            policy: SharedFormEditPolicy::Reject,
            annotation_update: None,
            tagged_clone: Default::default(),
        });
        let data = cache.get(id)?;
        let parsed = PageResources::from_dict(&child_resources, reader);
        let mut metrics = inline_text::Metrics::new(&parsed, reader);
        let mut state = initial;
        let mut nested = Vec::new();
        scan_text_program(
            &data,
            &mut state,
            Some(id),
            Some(&mut metrics),
            Some(&mut nested),
        )?;
        active.push(id);
        visit_calls(
            page,
            revision,
            index,
            id,
            &child_resources,
            &path,
            &owners,
            appearance_root,
            nested,
            active,
            cache,
            output,
        )?;
        active.pop();
    }
    Ok(())
}

pub fn analyze_form_text(input: &[u8], page: usize) -> Result<FormTextInventory> {
    let revision = format!("{:x}", Sha256::digest(input));
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut occurrences = Vec::new();
    for scope in discover_scopes(&engine, page, &revision)? {
        let mut text =
            analyze_multi_run_source(&engine, &scope.source_page, scope.initial.clone())?;
        text.paragraph_block_id = occurrence_key(&scope.target);
        for span in &mut text.source_spans {
            span.span_id = format!("{}:{}", text.paragraph_block_id, span.span_id);
        }
        text.exact_limits = vec!["logical offsets select direct text operands in this exact Form occurrence; nested Forms have separate targets".into(),
            "positions are Form-local; original Matrix, BBox, group, clipping and paint order remain on the invocation chain".into()];
        occurrences.push(FormTextOccurrence {
            target: scope.target.as_form()?.clone(),
            text,
            external_actual_text_owner: scope.external_actual_text,
            coordinate_space: "form_content_before_matrix".into(),
            form_bbox: scope.source_page.media_box,
        });
    }
    Ok(FormTextInventory {
        schema_version: SCHEMA_VERSION.into(),
        input_sha256: revision,
        page,
        occurrences,
        limits: vec![
            "targets are bound to exact input bytes and ordered page-to-Form invocation identities"
                .into(),
            "maximum depth 8, 4096 occurrences per page, and 256 MiB unique decoded source bytes"
                .into(),
            "annotation appearances and pattern/shading programs are not page Form text targets"
                .into(),
        ],
    })
}

fn same_location(a: &FormTextTarget, b: &FormTextTarget) -> bool {
    a.page == b.page
        && a.content_stream_index == b.content_stream_index
        && a.invocation_path.len() == b.invocation_path.len()
        && a.invocation_path
            .iter()
            .zip(&b.invocation_path)
            .all(|(a, b)| a.owner_operation_byte_start == b.owner_operation_byte_start)
}

pub(crate) fn ocr_capture_scope(
    engine: &ContentEngine,
    target: &FormTextTarget,
    revision: &str,
) -> Result<OcrFormScope> {
    if target.input_sha256 != revision {
        return Err(fail(
            "OCR Form target is bound to a different input revision",
        ));
    }
    let mut matches = discover_scopes(engine, target.page, revision)?
        .into_iter()
        .filter(|scope| {
            scope
                .target
                .as_form()
                .is_ok_and(|candidate| candidate == target)
        });
    let scope = matches
        .next()
        .ok_or_else(|| fail("OCR Form target does not match a reachable occurrence"))?;
    if matches.next().is_some() {
        return Err(fail("OCR Form target resolves ambiguously"));
    }
    if scope.external_actual_text {
        return Err(WellfriendError::UnsupportedFeature(
            "Form OCR is owned by caller ActualText; migrate that outer logical owner atomically"
                .into(),
        ));
    }
    if scope.source_page.contents.len() != 1 {
        return Err(fail("OCR Form scope is not one exact source stream"));
    }
    Ok(OcrFormScope {
        span_prefix: occurrence_key(&scope.target),
        source_page: scope.source_page,
        initial: scope.initial,
    })
}

fn output_scope(engine: &ContentEngine, before: &Scope) -> Result<Scope> {
    if let ScopeTarget::Appearance(target) = &before.target {
        return appearance::output_scope(engine, target);
    }
    let before_target = before.target.as_form()?;
    let mut matches = discover_scopes(engine, before_target.page, "")?
        .into_iter()
        .filter(|scope| {
            scope
                .target
                .as_form()
                .is_ok_and(|target| same_location(target, before_target))
        });
    let scope = matches
        .next()
        .ok_or_else(|| fail("saved Form occurrence was lost"))?;
    if matches.next().is_some() {
        return Err(fail("saved Form occurrence is ambiguous"));
    }
    Ok(scope)
}

/// Page-Form compatibility checks cannot certify the same source program when
/// used as an annotation appearance, tiling pattern or Type3 glyph program.
/// Traverse those resource closures without decoding pixels or following
/// annotation /P back into the page tree. References are conservatively treated
/// as uses, even when a resource is present but not painted by that program.
fn check_other_uses(engine: &ContentEngine, source: ObjectRef, scopes: &[Scope]) -> Result<()> {
    let reader = engine.document().reader();
    let mut roots = Vec::new();
    {
        let mut collect_resources = |resources: &PdfDictionary| -> Result<()> {
            if let Some(patterns) = resources.get("Pattern") {
                roots.push(patterns.clone());
            }
            if let Some(fonts) = dictionary(reader, resources.get("Font"))? {
                for (_, value) in fonts.iter() {
                    if let Some(font) = dictionary(reader, Some(value))? {
                        if font.get_name("Subtype") == Some("Type3") {
                            if let Some(programs) = font.get("CharProcs") {
                                roots.push(programs.clone());
                            }
                        }
                    }
                }
            }
            Ok(())
        };
        for scope in scopes {
            collect_resources(&scope.source_page.resources)?;
        }
        for page_number in 1..=engine.document().page_count()? {
            let page = engine.document().get_page(page_number)?;
            collect_resources(&page.resources)?;
        }
    }
    for page_number in 1..=engine.document().page_count()? {
        let page = engine.document().get_page(page_number)?;
        let object = reader.get_object(page.object_number, page.generation_number)?;
        if let Some(annotations) = object.as_dict().and_then(|dict| dict.get("Annots")) {
            let annotations = reader.resolve(annotations.clone())?;
            let annotations = annotations
                .as_array()
                .ok_or_else(|| fail("invalid annotation owners during edit-all"))?;
            for annotation in annotations {
                if let Some(annotation) = dictionary(reader, Some(annotation))? {
                    if let Some(appearance) = annotation.get("AP") {
                        roots.push(appearance.clone());
                    }
                }
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut visits = 0usize;
    while let Some(value) = roots.pop() {
        crate::cancel::check_current_cancel("Form edit-all non-page resource ownership")?;
        visits += 1;
        if visits > 100_000 || roots.len() > 100_000 {
            return Err(WellfriendError::ResourceLimit(
                "Form edit-all resource ownership budget exceeded".into(),
            ));
        }
        match value {
            PdfObject::Reference { number, generation } => {
                if (number, generation) == source {
                    return Err(WellfriendError::UnsupportedFeature("Form edit-all source is also referenced by a non-page program; use clone-one or an owner-specific transaction".into()));
                }
                if seen.insert((number, generation)) {
                    roots.push(reader.get_object(number, generation)?);
                }
            }
            PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
                roots.extend(dict.iter().map(|(_, value)| value.clone()))
            }
            PdfObject::Array(items) => roots.extend(items),
            _ => {}
        }
    }
    Ok(())
}

fn direct_text(engine: &ContentEngine, scope: &Scope) -> Result<String> {
    let reader = engine.document().reader();
    let id = scope.source_page.contents[0];
    let mut cache = DecodedSources::new(reader);
    let data = cache.get(id)?;
    let resources = PageResources::from_dict(&scope.source_page.resources, reader);
    // Initialize inherited text parameters for extraction only. No PDF is
    // written, and no synthetic visible text is introduced.
    let mut name = Vec::new();
    crate::writer::serialize_object(&PdfObject::Name(scope.initial.font_name.clone()), &mut name);
    let prefix = if scope.initial.font_name.is_empty() {
        String::new()
    } else {
        format!(
            "BT {} {} Tf {} Tc {} Tw {} Tz {} Ts {} Tr ET\n",
            String::from_utf8(name).map_err(|_| fail("font name serialization"))?,
            fmt_num(scope.initial.font_size),
            fmt_num(scope.initial.character_spacing),
            fmt_num(scope.initial.word_spacing),
            fmt_num(scope.initial.horizontal_scaling),
            fmt_num(scope.initial.text_rise),
            scope.initial.render_mode
        )
    };
    let mut operations = crate::content::ContentParser::parse(prefix.as_bytes())?;
    operations.extend(crate::content::ContentParser::parse(&data)?);
    // Direct text only: descendant Forms are independently occurrence-bound.
    operations.retain(|operation| operation.operator != "Do");
    let scoped_font_names = resources.fonts.keys().cloned().collect::<Vec<_>>();
    let mut collector = crate::text::TextCollector::new(resources, reader);
    let chunks = collector
        .collect_scoped(
            &operations,
            &crate::text::TextTraversalLimits::default(),
            &crate::cancel::current_cancel_token(),
        )
        .map_err(|error| {
            WellfriendError::MalformedPdf(format!(
                "Form text scoped extraction failed with fonts [{}]: {error}",
                scoped_font_names.join(", ")
            ))
        })?;
    Ok(chunks.into_iter().map(|item| item.chunk.text).collect())
}

pub(super) fn extract_scope(
    engine: &ContentEngine,
    page: usize,
    scope: Option<&Scope>,
) -> Result<String> {
    match scope {
        Some(scope) => direct_text(engine, &output_scope(engine, scope)?),
        None => engine.get_page_text(page),
    }
}

/// Rebind the exact edited program after occurrence-specific cloning. Used by
/// source postconditions that must not be satisfied by a duplicate elsewhere.
pub(super) fn output_page(
    engine: &ContentEngine,
    page: usize,
    scope: Option<&Scope>,
) -> Result<crate::document::PdfPage> {
    match scope {
        Some(scope) => Ok(output_scope(engine, scope)?.source_page),
        None => engine.document().get_page(page),
    }
}

pub fn edit_form_text(
    input: &[u8],
    request: &FormTextEditRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, FormTextEditReport)> {
    let revision = format!("{:x}", Sha256::digest(input));
    if request.target.input_sha256 != revision || request.target.page != request.edit.page {
        return Err(fail("stale revision or conflicting target page"));
    }
    if request.shared_form_policy == SharedFormEditPolicy::Reject {
        return Err(fail("select clone-one or edit-all explicitly"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let key = serde_json::to_vec(&request.target).map_err(|_| fail("target serialization"))?;
    let mut scope = discover_scopes(&engine, request.target.page, &revision)?
        .into_iter()
        .find(|scope| {
            scope
                .target
                .as_form()
                .ok()
                .and_then(|target| serde_json::to_vec(target).ok())
                .as_ref()
                == Some(&key)
        })
        .ok_or_else(|| fail("target does not match a reachable Form occurrence"))?;
    if scope.external_actual_text {
        return Err(WellfriendError::UnsupportedFeature(
        "Form text is owned by caller ActualText; select and migrate that logical owner atomically".into()));
    }
    scope.policy = request.shared_form_policy;
    let source_ref = scope.source_page.contents[0];
    if scope.policy == SharedFormEditPolicy::EditAllUses {
        let signature =
            |scope: &Scope| format!("{:?}:{:?}", scope.initial, scope.source_page.resources);
        let expected = signature(&scope);
        let mut all_scopes = Vec::new();
        // Every reachable page occurrence must agree, not only copies on the
        // selected page. Different inherited fonts/paint cannot be flattened.
        for page in 1..=engine.document().page_count()? {
            for candidate in discover_scopes(&engine, page, &revision)? {
                if all_scopes.len() >= MAX_FORM_OCCURRENCES {
                    return Err(WellfriendError::ResourceLimit(
                        "Form edit-all document occurrence budget exceeded".into(),
                    ));
                }
                if candidate.source_page.contents[0] == source_ref
                    && (candidate.external_actual_text || signature(&candidate) != expected)
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "Form edit-all has incompatible inherited occurrence contexts".into(),
                    ));
                }
                all_scopes.push(candidate);
            }
        }
        check_other_uses(&engine, source_ref, &all_scopes)?;
    }
    let model = analyze_multi_run_source(&engine, &scope.source_page, scope.initial.clone())?;
    let before_text = direct_text(&engine, &scope)?;
    let (output, mut native_edit) = edit_multi_run_text_range_in_scope(
        input,
        &request.edit,
        font_bytes,
        Some(&scope),
        None,
        false,
        false,
    )?;
    native_edit.exact_limits.push("this native edit was applied in the returned Form occurrence scope, not the page-logical text model; positions and font names are Form-local".into());
    let prefix = occurrence_key(&scope.target);
    for span in &mut native_edit.selected_source_spans {
        span.span_id = format!("{prefix}:{}", span.span_id);
    }
    let saved = ContentEngine::open_bytes(output.clone())?;
    let mut after_scope = output_scope(&saved, &scope)?;
    after_scope
        .target
        .set_revision(format!("{:x}", Sha256::digest(&output)));
    let after_text = direct_text(&saved, &after_scope)?;
    let expected = model
        .logical_text
        .chars()
        .take(request.edit.logical_start)
        .collect::<String>()
        + &request.edit.replacement_text
        + &model
            .logical_text
            .chars()
            .skip(request.edit.logical_end)
            .collect::<String>();
    let whole_direct_text_verified = before_text == model.logical_text;
    if whole_direct_text_verified && after_text != expected {
        return Err(fail(
            "saved direct Form text does not match the exact requested logical edit",
        ));
    }
    let source_form_retained = saved
        .document()
        .reader()
        .get_object(source_ref.0, source_ref.1)?
        == engine
            .document()
            .reader()
            .get_object(source_ref.0, source_ref.1)?;
    if scope.policy == SharedFormEditPolicy::CloneEditOneInstance && !source_form_retained {
        return Err(fail("clone-one changed the original Form object"));
    }
    Ok((output, FormTextEditReport { schema_version: SCHEMA_VERSION.into(), target_before: request.target.clone(), target_after: after_scope.target.as_form()?.clone(),
        shared_form_policy: scope.policy, native_edit, direct_text_before: before_text, direct_text_after: after_text,
        whole_direct_text_verified, source_form_retained, limits: vec![
            "source-local Form editing uses the existing shaping, CMap-boundary, ActualText and displacement writer".into(),
            "non-isomorphic pre-existing ActualText retains the native complete-owner selection checks; whole_direct_text_verified reports whether a full exact-text postcondition was available".into(),
            "stream-owned tagged clone migration, external ActualText ownership and annotation/pattern programs remain separate boundaries".into(),
            "incremental save retains historical bytes and is not sanitizing redaction; no pixel-fidelity certification is implied".into(),
        ] }))
}

pub(super) fn write_scope(
    reader: &PdfReader,
    page: &crate::document::PdfPage,
    parsed: &PageResources,
    changes: Vec<IncrementalObject>,
    scope: Option<&Scope>,
) -> Result<Vec<u8>> {
    write_scope_with_protected_fonts(reader, page, parsed, changes, scope, &BTreeSet::new())
}

pub(super) fn write_scope_with_protected_fonts(
    reader: &PdfReader,
    page: &crate::document::PdfPage,
    parsed: &PageResources,
    mut changes: Vec<IncrementalObject>,
    scope: Option<&Scope>,
    protected_fonts: &BTreeSet<String>,
) -> Result<Vec<u8>> {
    let Some(scope) = scope else {
        return text_resources::write_with_protected_fonts(
            reader,
            page,
            parsed,
            changes,
            protected_fonts,
        );
    };
    let leaf = scope.source_page.contents[0];
    let mut resources = scope.source_page.resources.clone();
    // The native writer stages its font registry on a page dictionary. For a
    // Form source scope those resources belong on the edited Form, not the page.
    let mut page_updates = 0usize;
    for update in &changes {
        if (update.number, update.generation)
            == (scope.page.object_number, scope.page.generation_number)
        {
            page_updates += 1;
            let dict = update
                .object
                .as_dict()
                .ok_or_else(|| fail("invalid staged resource owner"))?;
            if let Some(updated) = dictionary(reader, dict.get("Resources"))? {
                // The native page writer stages newly generated font names on
                // a page dictionary. A Form edit must import those additions,
                // but must not replace the Form-local resource scope with the
                // caller page's `/Resources`: doing so drops object-bound
                // inherited aliases such as `/WFInheritedFont0` and can also
                // rebind a child `/F1` to the unrelated page `/F1`.
                let mut scoped_fonts =
                    dictionary(reader, resources.get("Font"))?.unwrap_or_default();
                if let Some(updated_fonts) = dictionary(reader, updated.get("Font"))? {
                    for (name, value) in updated_fonts.iter() {
                        if !scoped_fonts.contains_key(name) {
                            scoped_fonts.insert(name, value.clone());
                        }
                    }
                }
                resources.insert("Font", PdfObject::Dictionary(scoped_fonts));
            }
        }
    }
    if page_updates > 1 {
        return Err(fail("duplicate staged resource owners"));
    }
    changes.retain(|update| {
        (update.number, update.generation)
            != (scope.page.object_number, scope.page.generation_number)
    });
    crate::ext_gstate_fonts::materialize(parsed, &mut resources, reader)?;
    let content_roots = scope
        .source_page
        .contents
        .iter()
        .map(|&(number, generation)| PdfObject::Reference { number, generation })
        .collect::<Vec<_>>();
    retire_unreferenced_generated_fonts(
        reader,
        &content_roots,
        &mut resources,
        None,
        &changes,
        protected_fonts,
    )?;
    let source_ids = reader.object_ids().into_iter().collect::<BTreeSet<_>>();
    if changes.iter().any(|update| {
        source_ids.contains(&(update.number, update.generation))
            && (update.number, update.generation) != leaf
    }) {
        return Err(fail(
            "Form-local edit attempted an unrelated source-object mutation",
        ));
    }
    let mut leaf_updates = changes
        .iter_mut()
        .filter(|update| (update.number, update.generation) == leaf);
    let update = leaf_updates
        .next()
        .ok_or_else(|| fail("native writer produced no Form update"))?;
    let PdfObject::Stream { dict, .. } = &mut update.object else {
        return Err(fail("Form update is not a stream"));
    };
    dict.insert("Resources", PdfObject::Dictionary(resources));
    if leaf_updates.next().is_some() {
        return Err(fail("duplicate Form source mutation"));
    }
    if scope.policy == SharedFormEditPolicy::EditAllUses {
        return write_incremental_update(reader, changes);
    }
    let clones = clone_path(reader, scope, &mut changes)?;
    let output = write_incremental_update(reader, changes)?;
    if let ScopeTarget::Appearance(target) = &scope.target {
        if scope.tagged_clone.policy
            != crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
        {
            return crate::tagged_structure::stream_clones::finish(
                reader.file_bytes(),
                output,
                target.annotation,
                (scope.page.object_number, scope.page.generation_number),
                &clones,
                &scope.tagged_clone,
            );
        }
    }
    Ok(output)
}

fn allocate(reader: &PdfReader, changes: &[IncrementalObject]) -> Result<u32> {
    reader
        .object_ids()
        .into_iter()
        .map(|id| id.0)
        .chain(changes.iter().map(|update| update.number))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| WellfriendError::ResourceLimit("Form text object space exhausted".into()))
}

fn clone_path(
    reader: &PdfReader,
    scope: &Scope,
    changes: &mut Vec<IncrementalObject>,
) -> Result<BTreeMap<ObjectRef, ObjectRef>> {
    let leaf = scope.source_page.contents[0];
    let mut sources = BTreeSet::from([leaf]);
    sources.extend(
        scope
            .target
            .path()
            .iter()
            .map(|step| (step.owner_stream_object, step.owner_stream_generation)),
    );
    if !matches!(&scope.target, ScopeTarget::Appearance(_))
        || scope.tagged_clone.policy
            == crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
    {
        vector_occurrence::check_clone_ownership(reader, &sources)?;
    }
    let child = allocate(reader, changes)?;
    let mut clones = BTreeMap::from([(leaf, (child, 0))]);
    let update = changes
        .iter_mut()
        .find(|update| (update.number, update.generation) == leaf)
        .ok_or_else(|| fail("missing leaf update"))?;
    update.number = child;
    update.generation = 0;
    let mut child = child;
    let mut cache = DecodedSources::new(reader);
    for (index, step) in scope.target.path().iter().enumerate().rev() {
        let owner = (step.owner_stream_object, step.owner_stream_generation);
        let mut resources = scope.owner_resources[index].clone();
        let mut xobjects = dictionary(reader, resources.get("XObject"))?.unwrap_or_default();
        let name = (0..10_000)
            .map(|suffix| format!("WFTextForm{child}_{suffix}"))
            .find(|name| !xobjects.contains_key(name))
            .ok_or_else(|| fail("Form resource-name budget exhausted"))?;
        xobjects.insert(&name, reference((child, 0)));
        resources.insert("XObject", PdfObject::Dictionary(xobjects));
        let data = cache.get(owner)?;
        let range = step.owner_operation_byte_start..step.owner_operation_byte_end;
        if range.start >= range.end || range.end > data.len() {
            return Err(fail("invalid source invocation range"));
        }
        let mut data = data.as_ref().clone();
        data.splice(range, format!("/{name} Do").bytes());
        let PdfObject::Stream { mut dict, .. } = reader.get_object(owner.0, owner.1)? else {
            return Err(fail("Form owner is not a stream"));
        };
        let raw = flate_encode_cancellable(&data, 6)?;
        dict.insert("Filter", PdfObject::Name("FlateDecode".into()));
        dict.remove("DecodeParms");
        dict.insert("Length", PdfObject::Integer(raw.len() as i64));
        if index == 0 && matches!(&scope.target, ScopeTarget::PageForm(_)) {
            changes.push(IncrementalObject {
                number: owner.0,
                generation: owner.1,
                object: PdfObject::Stream { dict, raw },
            });
            let mut page = reader
                .get_object(scope.page.object_number, scope.page.generation_number)?
                .as_dict()
                .cloned()
                .ok_or_else(|| fail("invalid page owner"))?;
            page.insert("Resources", PdfObject::Dictionary(resources));
            changes.push(IncrementalObject {
                number: scope.page.object_number,
                generation: scope.page.generation_number,
                object: PdfObject::Dictionary(page),
            });
            vector_occurrence::stage(
                reader,
                &scope.page,
                scope.target.as_form()?.content_stream_index,
                changes,
            )?;
        } else {
            dict.insert("Resources", PdfObject::Dictionary(resources));
            child = allocate(reader, changes)?;
            if clones.insert(owner, (child, 0)).is_some() {
                return Err(fail("cyclic clone source path"));
            }
            changes.push(IncrementalObject {
                number: child,
                generation: 0,
                object: PdfObject::Stream { dict, raw },
            });
        }
    }
    if let ScopeTarget::Appearance(target) = &scope.target {
        appearance::stage_root(
            reader,
            target,
            scope.annotation_update.as_ref(),
            (child, 0),
            changes,
        )?;
    }
    Ok(clones)
}
