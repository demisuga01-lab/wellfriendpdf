//! Native source text edits in one selected annotation appearance. The stream
//! and every selected nested Form are copied, then the annotation's exact normal
//! appearance slot is rebound. No page overlay or surrogate PDF is used.
use super::*;

#[path = "advanced_widget_text.rs"]
pub mod widgets;

const SCHEMA: &str = "advanced_editing.appearance-text-occurrences.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceTextTarget {
    pub input_sha256: String,
    pub page: usize,
    pub annotation_index: usize,
    pub annotation: ObjectRef,
    pub appearance_stream: ObjectRef,
    /// None means AP N directly names a stream; Some selects a state dictionary slot.
    pub normal_state: Option<String>,
    pub invocation_path: Vec<VectorFormInvocation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppearanceMetadataPolicy {
    /// Contents is often an annotation comment, not painted text. Explicitly
    /// retain all annotation metadata while editing the source appearance only.
    PreserveAnnotationMetadata,
    /// Root FreeText only. Require old Contents to equal the whole appearance
    /// text; update Contents and discard RC under explicit plain-text approval.
    SynchronizeFreeTextPlainText,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceTextEditRequest {
    pub target: AppearanceTextTarget,
    pub edit: MultiRunTextRangeRequest,
    pub metadata_policy: AppearanceMetadataPolicy,
    #[serde(default)]
    pub tagged_clone: crate::tagged_structure::stream_clones::TaggedCloneOptions,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppearanceTextOccurrence {
    pub target: AppearanceTextTarget,
    pub text: MultiRunRangeModel,
    pub external_actual_text_owner: bool,
    pub coordinate_space: String,
    pub source_bbox: [f64; 4],
    pub annotation_rect: [f64; 4],
    pub annotation_subtype: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppearanceTextInventory {
    pub schema_version: String,
    pub input_sha256: String,
    pub page: usize,
    pub occurrences: Vec<AppearanceTextOccurrence>,
    pub limits: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AppearanceTextEditReport {
    pub schema_version: String,
    pub target_before: AppearanceTextTarget,
    pub target_after: AppearanceTextTarget,
    pub metadata_policy: AppearanceMetadataPolicy,
    pub tagged_clone: crate::tagged_structure::stream_clones::TaggedCloneOptions,
    pub tagged_ownership: Option<crate::tagged_structure::ParentTreeReport>,
    pub native_edit: MultiRunTextEditReport,
    pub direct_text_before: String,
    pub direct_text_after: String,
    pub whole_direct_text_verified: bool,
    pub source_programs_retained: bool,
    pub annotation_contents_updated: bool,
    pub rich_text_discarded: bool,
    pub limits: Vec<String>,
}

pub(super) fn occurrence_key(target: &AppearanceTextTarget) -> String {
    format!(
        "appearance:{}:{}:{}-{}:{}",
        target.page,
        target.annotation_index,
        target.annotation.0,
        target.annotation.1,
        target
            .invocation_path
            .iter()
            .map(|step| step.owner_operation_byte_start.to_string())
            .collect::<Vec<_>>()
            .join("/")
    )
}

fn target(scope: &Scope) -> Result<&AppearanceTextTarget> {
    match &scope.target {
        ScopeTarget::Appearance(target) => Ok(target),
        _ => Err(fail("not an appearance target")),
    }
}

fn annotation(reader: &PdfReader, id: ObjectRef) -> Result<PdfDictionary> {
    reader
        .get_object(id.0, id.1)?
        .as_dict()
        .cloned()
        .ok_or_else(|| fail("annotation owner is not a dictionary"))
}

fn source_page(
    page: &crate::document::PdfPage,
    id: ObjectRef,
    dict: &PdfDictionary,
    reader: &PdfReader,
) -> Result<crate::document::PdfPage> {
    let bbox = crate::annotation_appearance::rectangle(dict, "BBox", reader)?;
    if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
        return Err(fail("appearance has empty source geometry"));
    }
    let mut source = page.clone();
    source.media_box = bbox;
    source.crop_box = bbox;
    source.contents = vec![id];
    source.resources =
        dictionary(reader, dict.get("Resources"))?.unwrap_or_else(|| page.resources.clone());
    Ok(source)
}

pub(super) fn discover_scopes(
    engine: &ContentEngine,
    page_number: usize,
    revision: &str,
) -> Result<Vec<Scope>> {
    let page = engine.document().get_page(page_number)?;
    let reader = engine.document().reader();
    let object = reader.get_object(page.object_number, page.generation_number)?;
    let dict = object
        .as_dict()
        .ok_or_else(|| fail("invalid annotation page"))?;
    let annotations = match dict
        .get("Annots")
        .map(|value| reader.resolve(value.clone()))
        .transpose()?
    {
        None | Some(PdfObject::Null) => return Ok(Vec::new()),
        Some(PdfObject::Array(items)) => items,
        _ => return Err(fail("invalid Annots array")),
    };
    if annotations.len() > MAX_FORM_OCCURRENCES {
        return Err(WellfriendError::ResourceLimit(
            "appearance occurrence budget exceeded".into(),
        ));
    }
    let mut cache = DecodedSources::new(reader);
    let mut output = Vec::new();
    for (index, value) in annotations.iter().enumerate() {
        crate::cancel::check_current_cancel("appearance text discovery")?;
        let object = reader.resolve(value.clone())?;
        let dict = object
            .as_dict()
            .ok_or_else(|| fail("annotation is not a dictionary"))?;
        let Some(selected) = crate::annotation_appearance::select_normal(dict, reader)? else {
            continue;
        };
        let annotation = value.as_reference().ok_or_else(|| {
            fail("promote the direct annotation before selecting appearance text")
        })?;
        let stream = selected
            .stream
            .ok_or_else(|| fail("appearance stream has no indirect source identity"))?;
        if selected.dict.get_name("Subtype") != Some("Form") {
            return Err(fail("appearance is not a Form stream"));
        }
        if output.len() >= MAX_FORM_OCCURRENCES {
            return Err(WellfriendError::ResourceLimit(
                "appearance occurrence budget exceeded".into(),
            ));
        }
        let source = source_page(&page, stream, &selected.dict, reader)?;
        let root = AppearanceTextTarget {
            input_sha256: revision.into(),
            page: page_number,
            annotation_index: index,
            annotation,
            appearance_stream: stream,
            normal_state: selected.state,
            invocation_path: Vec::new(),
        };
        let initial = ScannedTextTokenState::default();
        output.push(Scope {
            source_page: source.clone(),
            initial: initial.clone(),
            page: page.clone(),
            target: ScopeTarget::Appearance(root.clone()),
            owner_resources: Vec::new(),
            external_actual_text: false,
            policy: SharedFormEditPolicy::CloneEditOneInstance,
            annotation_update: None,
            tagged_clone: Default::default(),
        });
        let data = cache.get(stream)?;
        let resources = PageResources::from_dict(&source.resources, reader);
        let mut metrics = inline_text::Metrics::new(&resources, reader);
        let mut state = initial;
        let mut calls = Vec::new();
        scan_text_program(
            &data,
            &mut state,
            Some(stream),
            Some(&mut metrics),
            Some(&mut calls),
        )?;
        visit_calls(
            &page,
            revision,
            0,
            stream,
            &source.resources,
            &[],
            &[],
            Some(&root),
            calls,
            &mut vec![stream],
            &mut cache,
            &mut output,
        )?;
    }
    Ok(output)
}

pub fn analyze_appearance_text(input: &[u8], page: usize) -> Result<AppearanceTextInventory> {
    let revision = format!("{:x}", Sha256::digest(input));
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let mut occurrences = Vec::new();
    for scope in discover_scopes(&engine, page, &revision)? {
        let target = target(&scope)?.clone();
        let dict = annotation(engine.document().reader(), target.annotation)?;
        let mut text =
            analyze_multi_run_source(&engine, &scope.source_page, scope.initial.clone())?;
        text.paragraph_block_id = occurrence_key(&target);
        for span in &mut text.source_spans {
            span.span_id = format!("{}:{}", text.paragraph_block_id, span.span_id);
        }
        text.exact_limits = vec!["direct source text in this appearance/Form occurrence; descendant Forms have separate targets".into(),
            "source-local coordinates precede all appearance placement and Form matrices".into()];
        occurrences.push(AppearanceTextOccurrence {
            target,
            text,
            external_actual_text_owner: scope.external_actual_text,
            coordinate_space: "appearance_or_form_content_before_matrix".into(),
            source_bbox: scope.source_page.media_box,
            annotation_rect: crate::annotation_appearance::rectangle(
                &dict,
                "Rect",
                engine.document().reader(),
            )?,
            annotation_subtype: dict.get_name("Subtype").unwrap_or("").into(),
        });
    }
    Ok(AppearanceTextInventory { schema_version: SCHEMA.into(), input_sha256: revision, page, occurrences,
        limits: vec!["existing selected normal appearances only; no synthesized programs or implicit field-value editing".into(),
            "tagged clones require an explicit ownership policy and affected structural ActualText decisions; widgets, externally owned ActualText and ambiguous/direct source ownership require separate transactions".into(),
            "revision-bound annotation slot and nested source path; source-local coordinates, not page hit-testing".into()] })
}

pub(super) fn output_scope(engine: &ContentEngine, before: &AppearanceTextTarget) -> Result<Scope> {
    let mut found = discover_scopes(engine, before.page, "")?
        .into_iter()
        .filter(|scope| {
            let Ok(after) = target(scope) else {
                return false;
            };
            after.annotation_index == before.annotation_index
                && after.annotation == before.annotation
                && after.normal_state == before.normal_state
                && after.invocation_path.len() == before.invocation_path.len()
                && after
                    .invocation_path
                    .iter()
                    .zip(&before.invocation_path)
                    .all(|(a, b)| a.owner_operation_byte_start == b.owner_operation_byte_start)
        });
    let selected = found
        .next()
        .ok_or_else(|| fail("saved appearance occurrence was lost"))?;
    if found.next().is_some() {
        return Err(fail("saved appearance occurrence is ambiguous"));
    }
    Ok(selected)
}

pub fn edit_appearance_text(
    input: &[u8],
    request: &AppearanceTextEditRequest,
    font_bytes: Option<&[u8]>,
) -> Result<(Vec<u8>, AppearanceTextEditReport)> {
    edit_appearance_text_inner(input, request, font_bytes, false)
}

// Only the child field-wide coordinator can enable widget writes. The public
// appearance API retains its refusal: a standalone AP is not a field edit.
fn edit_appearance_text_inner(
    input: &[u8],
    request: &AppearanceTextEditRequest,
    font_bytes: Option<&[u8]>,
    coordinated_widget: bool,
) -> Result<(Vec<u8>, AppearanceTextEditReport)> {
    let revision = format!("{:x}", Sha256::digest(input));
    if revision != request.target.input_sha256 || request.target.page != request.edit.page {
        return Err(fail("stale appearance revision or conflicting page"));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    // A single indirect annotation cannot be rebound occurrence-locally when it
    // has duplicate page ownership. Reuse the canonical identity invariant.
    crate::annotation_identity::index(engine.document(), 100_000)?;
    let key =
        serde_json::to_vec(&request.target).map_err(|_| fail("appearance target serialization"))?;
    let mut scope = discover_scopes(&engine, request.target.page, &revision)?
        .into_iter()
        .find(|scope| {
            target(scope)
                .ok()
                .and_then(|target| serde_json::to_vec(target).ok())
                .as_ref()
                == Some(&key)
        })
        .ok_or_else(|| fail("appearance target does not match the selected source occurrence"))?;
    scope.policy = SharedFormEditPolicy::CloneEditOneInstance;
    scope.tagged_clone = request.tagged_clone.clone();
    if scope.tagged_clone.policy
        == crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
        && !scope.tagged_clone.actual_text_updates.is_empty()
    {
        return Err(fail(
            "structural ActualText updates require a tagged clone policy",
        ));
    }
    if scope.tagged_clone.policy
        == crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
    {
        crate::tagged_structure::stream_clones::check_annotation_clone_ownership(
            &engine,
            request.target.annotation,
        )?;
    }
    if scope.external_actual_text {
        return Err(WellfriendError::UnsupportedFeature("appearance descendant is owned by caller ActualText; migrate that complete owner atomically".into()));
    }
    let reader = engine.document().reader();
    let mut annotation = annotation(reader, request.target.annotation)?;
    if annotation.get_name("Subtype") == Some("Widget") && !coordinated_widget {
        return Err(WellfriendError::UnsupportedFeature(
            "widget appearance text requires coordinated field value/default appearance mutation"
                .into(),
        ));
    }
    if coordinated_widget
        && (annotation.get_name("Subtype") != Some("Widget")
            || request.metadata_policy != AppearanceMetadataPolicy::PreserveAnnotationMetadata)
    {
        return Err(fail(
            "coordinated field edits require widget appearances with preserved annotation metadata",
        ));
    }
    let model = analyze_multi_run_source(&engine, &scope.source_page, scope.initial.clone())?;
    if request.edit.logical_start > request.edit.logical_end
        || request.edit.logical_end > model.logical_text.chars().count()
    {
        return Err(fail("appearance text range is out of bounds"));
    }
    let before = direct_text(&engine, &scope)?;
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
    let mut contents_updated = false;
    let mut rich_text_discarded = false;
    if request.metadata_policy == AppearanceMetadataPolicy::SynchronizeFreeTextPlainText {
        if annotation.get_name("Subtype") != Some("FreeText")
            || !request.target.invocation_path.is_empty()
        {
            return Err(fail(
                "plain-text synchronization requires a root FreeText appearance",
            ));
        }
        let contents = annotation
            .get("Contents")
            .map(|value| reader.resolve(value.clone()))
            .transpose()?;
        let contents = match contents {
            Some(PdfObject::String(bytes)) => crate::info::decode_pdf_text_string(&bytes),
            None | Some(PdfObject::Null) => String::new(),
            _ => return Err(fail("FreeText Contents is not a text string")),
        };
        let full = engine
            .collect_page_scoped_text_chunks_including_appearances(
                request.target.page,
                &crate::text::TextTraversalLimits::default(),
                &crate::cancel::current_cancel_token(),
            )?
            .into_iter()
            .filter(|chunk| {
                chunk
                    .appearance
                    .as_ref()
                    .is_some_and(|appearance| appearance.annotation == request.target.annotation)
            })
            .map(|chunk| chunk.chunk.text)
            .collect::<String>();
        if contents != before || full != before || before != model.logical_text {
            return Err(fail(
                "FreeText metadata/appearance ownership is not an exact direct-text mapping",
            ));
        }
        annotation.insert(
            "Contents",
            crate::annotation_identity::text_string(&expected),
        );
        rich_text_discarded = annotation.get("RC").is_some_and(|value| !value.is_null());
        annotation.remove("RC");
        scope.annotation_update = Some(annotation.clone());
        contents_updated = true;
    }
    let (output, mut native_edit) = edit_multi_run_text_range_in_scope(
        input,
        &request.edit,
        font_bytes,
        Some(&scope),
        None,
        false,
        false,
    )?;
    let saved = ContentEngine::open_bytes(output.clone())?;
    let mut after_scope = output_scope(&saved, &request.target)?;
    after_scope
        .target
        .set_revision(format!("{:x}", Sha256::digest(&output)));
    let after = direct_text(&saved, &after_scope)?;
    let whole = before == model.logical_text;
    if whole && after != expected {
        return Err(fail(
            "saved appearance direct text disagrees with the requested edit",
        ));
    }
    let mut source_ids = BTreeSet::from([
        request.target.appearance_stream,
        scope.source_page.contents[0],
    ]);
    source_ids.extend(
        request
            .target
            .invocation_path
            .iter()
            .map(|step| (step.owner_stream_object, step.owner_stream_generation)),
    );
    for id in source_ids {
        if reader.get_object(id.0, id.1)? != saved.document().reader().get_object(id.0, id.1)? {
            return Err(fail("appearance clone changed an original program"));
        }
    }
    let after_annotation = self::annotation(saved.document().reader(), request.target.annotation)?;
    let mut expected_annotation = annotation;
    let expected_ap = rebound_ap(
        reader,
        &expected_annotation,
        &request.target,
        target(&after_scope)?.appearance_stream,
    )?;
    expected_annotation.insert("AP", PdfObject::Dictionary(expected_ap));
    if after_annotation != expected_annotation {
        return Err(fail(
            "appearance edit changed unrelated annotation metadata",
        ));
    }
    let prefix = occurrence_key(&request.target);
    for span in &mut native_edit.selected_source_spans {
        span.span_id = format!("{prefix}:{}", span.span_id);
    }
    native_edit.exact_limits.push(
        "source-local appearance occurrence, not page-logical text; historical bytes remain".into(),
    );
    let tagged_ownership = if request.tagged_clone.policy
        != crate::tagged_structure::stream_clones::TaggedClonePolicy::Reject
        && engine
            .document()
            .get_catalog()?
            .contains_key("StructTreeRoot")
    {
        Some(crate::tagged_structure::validate_parent_tree(&output)?)
    } else {
        None
    };
    Ok((output, AppearanceTextEditReport { schema_version: SCHEMA.into(), target_before: request.target.clone(), target_after: target(&after_scope)?.clone(),
        metadata_policy: request.metadata_policy, tagged_clone: request.tagged_clone.clone(), tagged_ownership, native_edit, direct_text_before: before, direct_text_after: after, whole_direct_text_verified: whole,
        source_programs_retained: true, annotation_contents_updated: contents_updated, rich_text_discarded,
        limits: vec!["copy-on-write of the selected normal state and nested Forms; other states and original programs retained".into(),
            "tagged ownership validation is not PDF/UA certification; widget field-value coordination and external content-stream ActualText ownership remain separate work".into(),
            "incremental source editing is not permanent redaction or visual-fidelity qualification".into()] }))
}

pub(super) fn stage_root(
    reader: &PdfReader,
    target: &AppearanceTextTarget,
    metadata: Option<&PdfDictionary>,
    child: ObjectRef,
    changes: &mut Vec<IncrementalObject>,
) -> Result<()> {
    let source = annotation(reader, target.annotation)?;
    let selected = crate::annotation_appearance::select_normal(&source, reader)?
        .ok_or_else(|| fail("appearance selection disappeared before staging"))?;
    if selected.stream != Some(target.appearance_stream) || selected.state != target.normal_state {
        return Err(fail("appearance source slot changed before staging"));
    }
    let ap = rebound_ap(reader, &source, target, child)?;
    let mut updated = metadata.cloned().unwrap_or(source);
    updated.insert("AP", PdfObject::Dictionary(ap));
    if changes
        .iter()
        .any(|change| (change.number, change.generation) == target.annotation)
    {
        return Err(fail("duplicate appearance annotation update"));
    }
    changes.push(IncrementalObject {
        number: target.annotation.0,
        generation: target.annotation.1,
        object: PdfObject::Dictionary(updated),
    });
    Ok(())
}

fn rebound_ap(
    reader: &PdfReader,
    source: &PdfDictionary,
    target: &AppearanceTextTarget,
    child: ObjectRef,
) -> Result<PdfDictionary> {
    let mut ap = dictionary(reader, source.get("AP"))?
        .ok_or_else(|| fail("appearance dictionary disappeared"))?;
    if let Some(state) = &target.normal_state {
        let mut states = dictionary(reader, ap.get("N"))?
            .ok_or_else(|| fail("normal appearance state dictionary disappeared"))?;
        states.insert(state, reference(child));
        ap.insert("N", PdfObject::Dictionary(states));
    } else {
        ap.insert("N", reference(child));
    }
    Ok(ap)
}

#[cfg(test)]
#[path = "advanced_appearance_text_tests.rs"]
mod tests;
