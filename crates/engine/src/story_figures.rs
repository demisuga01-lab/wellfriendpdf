//! Native figure blocks in the canonical story paginator. Associations are
//! explicit; nearby images/words are never silently inferred to be a caption.
use super::*;
use crate::image_fragments::{ImageFragmentBinding, ImageFragmentSource, ImageFragmentStack};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigureRemoval {
    pub figure_id: String,
    /// Exact current owner returned by load_linked_stories. Removes the painted
    /// occurrence from the current revision, not historical bytes or resources.
    pub binding: ImageFragmentBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigureDetachment {
    pub figure_id: String,
    /// Exact current owner returned by `load_linked_stories`. Detachment keeps
    /// this paint occurrence byte-for-byte reachable at its current position,
    /// but releases it from the saved story so a later exact revision can move
    /// it independently or attach it to another story.
    pub binding: ImageFragmentBinding,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigureTransferRequest {
    pub input_sha256: String,
    pub source_story_id: String,
    pub source_figure_id: String,
    pub source_binding: ImageFragmentBinding,
    pub target_story_id: String,
    pub target_figure_id: String,
    pub target_caption_paragraph: String,
    #[serde(default)]
    pub signature_policy_override: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigureTransferPreview {
    pub input_sha256: String,
    pub plan_sha256: String,
    pub intermediate_sha256: String,
    pub source: Box<LinkedStoryPreview>,
    pub target: Box<LinkedStoryPreview>,
    pub exact_limits: Vec<String>,
    pub qualification: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigureTransferReport {
    pub preview: StoryFigureTransferPreview,
    pub output_sha256: String,
    pub source_owner_released: bool,
    pub target_owner_attached: bool,
    pub physical_owner_count: usize,
    pub tagged_structure_owner_transferred: Option<bool>,
    pub parent_tree_verified: Option<bool>,
    pub output_page_count: usize,
    pub historical_bytes_removed: bool,
    pub output_reopened: bool,
}

struct PreparedTransfer {
    preview: StoryFigureTransferPreview,
    intermediate: Vec<u8>,
    target_request: LinkedStoryRequest,
}

fn one_saved_story(input: &[u8], story_id: &str, role: &str) -> Result<LinkedStoryRequest> {
    let mut matches = load_linked_stories(input)?
        .into_iter()
        .filter(|saved| saved.request.story_id == story_id);
    let request = matches
        .next()
        .ok_or_else(|| fail(&format!("{role} story is not saved on this revision")))?
        .request;
    if matches.next().is_some() {
        return Err(fail(&format!("{role} story identity is ambiguous")));
    }
    Ok(request)
}

fn prepare_transfer(
    input: &[u8],
    request: &StoryFigureTransferRequest,
) -> Result<PreparedTransfer> {
    if hash(input) != request.input_sha256
        || request.source_story_id.is_empty()
        || request.target_story_id.is_empty()
        || request.source_story_id == request.target_story_id
        || request.source_figure_id.is_empty()
        || request.target_figure_id.is_empty()
        || request.target_caption_paragraph.is_empty()
        || request.source_binding.key.is_empty()
    {
        return Err(fail("invalid or stale story Figure transfer request"));
    }
    let mut source = one_saved_story(input, &request.source_story_id, "source")?;
    let mut target = one_saved_story(input, &request.target_story_id, "target")?;
    let tagged = match (source.source_tags.is_some(), target.source_tags.is_some()) {
        (false, false) => false,
        (true, true) => true,
        _ => {
            return Err(fail(
                "Figure transfer between tagged and untagged stories requires an explicit semantic conversion decision",
            ))
        }
    };
    if tagged && (source.table_layout.is_some() || target.table_layout.is_some()) {
        return Err(fail(
            "tagged table/Figure transfer requires mixed-object cell ownership migration",
        ));
    }
    if !source.figure_removals.is_empty()
        || !source.figure_detachments.is_empty()
        || !target.figure_removals.is_empty()
        || !target.figure_detachments.is_empty()
    {
        return Err(fail("saved story contains an unconsumed Figure command"));
    }
    let indices = source
        .figures
        .iter()
        .enumerate()
        .filter(|(_, figure)| figure.id == request.source_figure_id)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if indices.len() != 1 {
        return Err(fail("source Figure identity is absent or ambiguous"));
    }
    let source_figure = source.figures.remove(indices[0]);
    if !matches!(&source_figure.source, ImageFragmentSource::Owned { binding } if binding == &request.source_binding)
    {
        return Err(fail("source Figure transfer binding is stale"));
    }
    source.signature_policy_override = request.signature_policy_override;
    // Keep this exact private checkpoint so target planning and approved apply
    // use the same bytes rather than independently regenerating an intermediate.
    let (intermediate, source_preview, transferred_target_tags) = if tagged {
        // The source leg changes only saved ownership and the structure parent;
        // its frames and current paint stay byte-for-byte in place until the
        // target leg captures and relocates that paint.
        let mut preview_source = one_saved_story(input, &request.source_story_id, "source")?;
        preview_source.signature_policy_override = request.signature_policy_override;
        let mut preview = super::preview_linked_story(input, &preview_source)?;
        preview.figure_detachments.push(StoryFigureDetachment {
            figure_id: request.source_figure_id.clone(),
            binding: request.source_binding.clone(),
        });
        preview.exact_limits.push("Tagged transfer preserves the existing Figure leaf or approved bounded contentless subtree and semantic attributes, moves its root between exact approved sibling intervals, and rebuilds ParentTree before target placement".into());

        let (structured, source_tags, target_tags) =
            crate::tagged_structure::story::transfer_figure_owner(
                input,
                &preview_source,
                &target,
                &request.source_figure_id,
                &request.target_figure_id,
                &request.target_caption_paragraph,
            )?;
        source.source_tags = Some(source_tags.clone());
        source.figure_detachments.clear();
        let structured_engine = ContentEngine::open_bytes(structured.clone())?;
        let source_fonts = super::load_story_fonts(&structured_engine, &source.story_id)?;
        let intermediate = super::save_story_metadata(
            &structured,
            &source,
            &source.frames,
            &source_fonts,
            &source.annotation_anchors,
            Some(source_tags),
            None,
        )?;
        preview.output_sha256 = Some(hash(&intermediate));
        (intermediate, preview, Some(target_tags))
    } else {
        source.figure_detachments.push(StoryFigureDetachment {
            figure_id: request.source_figure_id.clone(),
            binding: request.source_binding.clone(),
        });
        let (intermediate, preview) = super::apply_linked_story(input, &source)?;
        (intermediate, preview, None)
    };
    let binding = crate::image_fragments::image_fragment_bindings(&intermediate)?
        .into_iter()
        .filter(|binding| binding.key == request.source_binding.key)
        .collect::<Vec<_>>();
    if binding.len() != 1 || binding[0] != request.source_binding {
        return Err(fail(
            "detached Figure paint changed before target-story attachment",
        ));
    }
    target = one_saved_story(&intermediate, &request.target_story_id, "target")?;
    if let Some(tags) = transferred_target_tags {
        target.source_tags = Some(tags);
    }
    target.signature_policy_override = request.signature_policy_override;
    if target
        .figures
        .iter()
        .any(|figure| figure.id == request.target_figure_id)
        || target
            .paragraphs
            .iter()
            .filter(|paragraph| paragraph.id == request.target_caption_paragraph)
            .count()
            != 1
    {
        return Err(fail(
            "target Figure identity or caption paragraph is not uniquely available",
        ));
    }
    let mut transferred = source_figure;
    transferred.id = request.target_figure_id.clone();
    transferred.caption_paragraph = request.target_caption_paragraph.clone();
    transferred.source = ImageFragmentSource::Owned {
        binding: binding[0].clone(),
    };
    transferred.ocr = None;
    transferred.ocr_unrelated = false;
    target.figures.push(transferred);
    let target_preview = super::preview_linked_story(&intermediate, &target)?;
    let intermediate_sha256 = hash(&intermediate);
    let plan_sha256 = super::value_hash(&(
        request,
        &source_preview,
        &target_preview,
        &intermediate_sha256,
    ))?;
    Ok(PreparedTransfer {
        preview: StoryFigureTransferPreview {
            input_sha256: request.input_sha256.clone(),
            plan_sha256,
            intermediate_sha256,
            source: Box::new(source_preview),
            target: Box::new(target_preview),
            exact_limits: vec![
                "Both stories must already be saved and distinct, must either both be untagged or both use supported Figure/tag ownership, and the target caption paragraph must already exist exactly once".into(),
                "The source Figure geometry/style is retained; only its story/figure/caption identities and final placement change".into(),
                "Tagged transfer moves one page-owned Figure leaf or explicitly preserved bounded contentless subtree with its existing semantic attributes; content-bearing/shared subtrees, mixed tagged/untagged transfer and tagged table-cell objects remain explicit refusals".into(),
                "Publication is all-or-nothing to the caller, but the final incremental PDF retains the private intermediate revision in history; this is not sanitizing redaction".into(),
            ],
            qualification: "source_implementation_only; vps_corpus_gate_pending".into(),
        },
        intermediate,
        target_request: target,
    })
}

pub fn preview_story_figure_transfer(
    input: &[u8],
    request: &StoryFigureTransferRequest,
) -> Result<StoryFigureTransferPreview> {
    Ok(prepare_transfer(input, request)?.preview)
}

pub fn apply_story_figure_transfer(
    input: &[u8],
    request: &StoryFigureTransferRequest,
    approved_plan_sha256: &str,
) -> Result<(Vec<u8>, StoryFigureTransferReport)> {
    let prepared = prepare_transfer(input, request)?;
    if prepared.preview.plan_sha256 != approved_plan_sha256 {
        return Err(fail("story Figure transfer approval is stale"));
    }
    let (output, target_preview) =
        super::apply_linked_story(&prepared.intermediate, &prepared.target_request)?;
    if target_preview.story_id != request.target_story_id {
        return Err(fail("target story changed during Figure transfer"));
    }
    let source = one_saved_story(&output, &request.source_story_id, "source")?;
    let target = one_saved_story(&output, &request.target_story_id, "target")?;
    let source_owner_released = !source.figures.iter().any(|figure| {
        figure.id == request.source_figure_id
            || matches!(&figure.source, ImageFragmentSource::Owned { binding } if binding.key == request.source_binding.key)
    });
    let target_owner_attached = target.figures.iter().any(|figure| {
        figure.id == request.target_figure_id
            && figure.caption_paragraph == request.target_caption_paragraph
            && matches!(&figure.source, ImageFragmentSource::Owned { binding } if binding.key == request.source_binding.key)
    });
    let physical_owner_count = crate::image_fragments::image_fragment_bindings(&output)?
        .into_iter()
        .filter(|binding| binding.key == request.source_binding.key)
        .count();
    let (tagged_structure_owner_transferred, parent_tree_verified) =
        match (&source.source_tags, &target.source_tags) {
            (None, None) => (None, None),
            (Some(source_tags), Some(target_tags)) => {
                let transferred = !source_tags.figures.contains_key(&request.source_figure_id)
                    && target_tags
                        .figures
                        .get(&request.target_figure_id)
                        .and_then(|binding| binding.source.as_ref())
                        .is_some();
                crate::tagged_structure::validate_parent_tree(&output)?;
                crate::tagged_structure::story::validate_selection(&output, &source)?;
                crate::tagged_structure::story::validate_selection(&output, &target)?;
                (Some(transferred), Some(true))
            }
            _ => return Err(fail("Figure transfer produced mixed tag state")),
        };
    let output_page_count = ContentEngine::open_bytes(output.clone())?.page_count()?;
    if !source_owner_released
        || !target_owner_attached
        || physical_owner_count != 1
        || tagged_structure_owner_transferred == Some(false)
    {
        return Err(fail("story Figure transfer postcondition failed"));
    }
    let output_sha256 = hash(&output);
    Ok((
        output,
        StoryFigureTransferReport {
            preview: prepared.preview,
            output_sha256,
            source_owner_released,
            target_owner_attached,
            physical_owner_count,
            tagged_structure_owner_transferred,
            parent_tree_verified,
            output_page_count,
            historical_bytes_removed: false,
            output_reopened: true,
        },
    ))
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FigureAlignment {
    #[default]
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigure {
    pub id: String,
    /// The image precedes this paragraph. The whole caption and image stay in
    /// one approved frame; an empty caption produces an image-only block.
    pub caption_paragraph: String,
    pub source: ImageFragmentSource,
    /// One-shot original-revision invisible source selection. Saved owned
    /// groups carry OCR themselves and must not persist/replay these offsets.
    #[serde(default)]
    pub ocr: Option<crate::advanced_editing::ocr_carriers::OcrCarrierSelection>,
    /// One-shot confirmation that invisible text on this page is unrelated to
    /// the initial image. Mutually exclusive with an OCR capture selection.
    #[serde(default)]
    pub ocr_unrelated: bool,
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub gap: f64,
    #[serde(default)]
    pub alignment: FigureAlignment,
    /// Explicit compositing choice; no silent stacking-order promise.
    pub stack: ImageFragmentStack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoryFigurePlacement {
    pub figure_id: String,
    pub rect: [f64; 4],
}
pub(crate) struct Reservation {
    pub placement: StoryFigurePlacement,
    pub caption_top: f64,
    pub caption_left: f64,
    pub width: f64,
}
/// Avoid pinned artwork by moving the entire image block below intersecting
/// exclusions, not by moving the caption alone or slicing the image.
pub(crate) fn reserve(
    frame: &StoryFrame,
    p: &StoryParagraph,
    y: f64,
    figure: &StoryFigure,
) -> Result<Option<Reservation>> {
    let width = frame.rect[2] - frame.rect[0];
    if figure.width > width + 1e-7 {
        return Ok(None);
    }
    let x = frame.rect[0]
        + match figure.alignment {
            FigureAlignment::Left => 0.0,
            FigureAlignment::Center => (width - figure.width) / 2.0,
            FigureAlignment::Right => width - figure.width,
        };
    let mut top = y - p.space_before;
    for _ in 0..=frame.exclusions.len() {
        crate::cancel::check_current_cancel("story figure exclusion layout")?;
        let rect = [x, top - figure.height, x + figure.width, top];
        if rect[1] < frame.rect[1] - 1e-7 {
            return Ok(None);
        }
        let next = frame
            .exclusions
            .iter()
            .filter(|r| overlaps(r, &rect))
            .map(|r| r[1])
            .reduce(f64::min);
        if let Some(bottom) = next {
            if bottom >= top {
                return Err(fail("figure exclusion made no layout progress"));
            }
            top = bottom;
            continue;
        }
        return Ok(Some(Reservation {
            placement: StoryFigurePlacement {
                figure_id: figure.id.clone(),
                rect,
            },
            caption_top: rect[1] - if p.text.is_empty() { 0.0 } else { figure.gap },
            caption_left: x,
            width: figure.width,
        }));
    }
    Err(fail("figure exclusion layout budget exceeded"))
}

pub(crate) fn validate(input: &[u8], request: &LinkedStoryRequest) -> Result<()> {
    if request.figures.is_empty()
        && request.figure_removals.is_empty()
        && request.figure_detachments.is_empty()
    {
        if load_linked_stories(input)?
            .iter()
            .any(|s| s.request.story_id == request.story_id && !s.request.figures.is_empty())
        {
            return Err(fail(
                "dropping saved figures requires explicit source removal or detachment",
            ));
        }
        return Ok(());
    }
    if request
        .figures
        .len()
        .saturating_add(request.figure_removals.len())
        .saturating_add(request.figure_detachments.len())
        > 1024
        || request.table_layout.is_some()
    {
        return Err(fail("figure blocks support at most 1024 ordinary-story figures; table-cell objects require table fragmentation integration"));
    }
    let mut ids = BTreeSet::new();
    let mut captions = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut paragraph_counts = BTreeMap::new();
    for p in &request.paragraphs {
        *paragraph_counts.entry(p.id.as_str()).or_insert(0usize) += 1;
    }
    for figure in &request.figures {
        if (figure.ocr.is_some() && figure.ocr_unrelated)
            || (matches!(&figure.source, ImageFragmentSource::Owned { .. })
                && (figure.ocr.is_some() || figure.ocr_unrelated))
        {
            return Err(fail("OCR capture and unrelated-text decisions are exclusive one-shot initial-image choices"));
        }
        if figure.id.is_empty()
            || figure.id.len() > 256
            || !ids.insert(figure.id.as_str())
            || !captions.insert(&figure.caption_paragraph)
            || paragraph_counts.get(figure.caption_paragraph.as_str()) != Some(&1)
            || [figure.width, figure.height]
                .iter()
                .any(|v| !v.is_finite() || *v <= 0.0 || *v > 1e6)
            || !figure.gap.is_finite()
            || !(0.0..=1e6).contains(&figure.gap)
        {
            return Err(fail(
                "invalid figure identity, caption ownership or dimensions",
            ));
        }
        let key =
            crate::image_fragments::stories::source_key(&request.input_sha256, &figure.source);
        if !sources.insert(key) {
            return Err(fail(
                "one image occurrence cannot belong to multiple story figures",
            ));
        }
    }
    let mut removals = BTreeMap::new();
    for removal in &request.figure_removals {
        if removal.figure_id.is_empty()
            || removal.figure_id.len() > 256
            || ids.contains(removal.figure_id.as_str())
            || removals
                .insert(removal.figure_id.as_str(), &removal.binding)
                .is_some()
            || !sources.insert(removal.binding.key.clone())
        {
            return Err(fail("duplicate, retained or invalid figure removal owner"));
        }
    }
    let mut detachments = BTreeMap::new();
    for detachment in &request.figure_detachments {
        if detachment.figure_id.is_empty()
            || detachment.figure_id.len() > 256
            || ids.contains(detachment.figure_id.as_str())
            || removals.contains_key(detachment.figure_id.as_str())
            || detachments
                .insert(detachment.figure_id.as_str(), &detachment.binding)
                .is_some()
            || !sources.insert(detachment.binding.key.clone())
        {
            return Err(fail(
                "duplicate, retained or invalid figure detachment owner",
            ));
        }
    }
    if !request.figure_detachments.is_empty() && request.source_tags.is_some() {
        return Err(fail(
            "tagged Figure detachment requires an explicit structure-owner transfer",
        ));
    }
    let mut validated_removals = BTreeSet::new();
    let mut validated_detachments = BTreeSet::new();
    for saved in load_linked_stories(input)? {
        for owned in &saved.request.figures {
            let key = crate::image_fragments::stories::source_key(
                &saved.request.input_sha256,
                &owned.source,
            );
            if saved.request.story_id != request.story_id && sources.contains(&key) {
                return Err(fail("figure source belongs to another saved story"));
            }
            if saved.request.story_id == request.story_id {
                if let Some(binding) = removals.get(owned.id.as_str()) {
                    if !matches!(&owned.source, ImageFragmentSource::Owned { binding: current } if current == *binding)
                    {
                        return Err(fail(
                            "figure removal binding is stale or belongs to another occurrence",
                        ));
                    }
                    validated_removals.insert(owned.id.as_str().to_owned());
                } else if let Some(binding) = detachments.get(owned.id.as_str()) {
                    if !matches!(&owned.source, ImageFragmentSource::Owned { binding: current } if current == *binding)
                    {
                        return Err(fail(
                            "figure detachment binding is stale or belongs to another occurrence",
                        ));
                    }
                    validated_detachments.insert(owned.id.as_str().to_owned());
                } else if !request.figures.iter().any(|f| {
                    f.id == owned.id
                        && crate::image_fragments::stories::source_key(
                            &request.input_sha256,
                            &f.source,
                        ) == key
                }) {
                    return Err(fail("dropping/replacing a saved figure needs an explicit source removal or detachment operation"));
                }
            }
        }
    }
    if validated_removals.len() != removals.len() {
        return Err(fail("figure removal is not owned by this saved story"));
    }
    if validated_detachments.len() != detachments.len() {
        return Err(fail("figure detachment is not owned by this saved story"));
    }
    // Initial scans need an explicit decision about which invisible carriers
    // belong to them. Owned groups already define their complete source scope;
    // unrelated OCR elsewhere on their page must not prevent repeat movement.
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let pages = request
        .figures
        .iter()
        .filter_map(|f| match (&f.source, &f.ocr) {
            (ImageFragmentSource::Occurrence { page, .. }, None) if !f.ocr_unrelated => Some(*page),
            _ => None,
        })
        .collect::<BTreeSet<_>>();
    for page in pages {
        if engine
            .collect_page_text_chunks(page)?
            .iter()
            .any(|c| c.is_invisible)
        {
            return Err(fail("initial figure source page contains invisible text; select its OCR carriers or explicitly confirm they are unrelated"));
        }
    }
    crate::image_fragments::stories::validate(input, request)
}

pub(crate) fn attach_preview(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &mut LinkedStoryPreview,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    let engine = if request.figures.is_empty()
        && request.figure_removals.is_empty()
        && request.figure_detachments.is_empty()
    {
        None
    } else {
        Some(ContentEngine::open_bytes(input.to_vec())?)
    };
    for frame in &preview.frames {
        for placement in &frame.figures {
            if !seen.insert(&placement.figure_id)
                || !request.figures.iter().any(|f| f.id == placement.figure_id)
            {
                return Err(fail("figure pagination duplicated or invented an owner"));
            }
            let figure = request
                .figures
                .iter()
                .find(|f| f.id == placement.figure_id)
                .ok_or_else(|| fail("figure owner missing"))?;
            let engine = engine
                .as_ref()
                .ok_or_else(|| fail("figure preview document missing"))?;
            let origin = match &figure.source {
                ImageFragmentSource::Occurrence { page, .. } => *page,
                ImageFragmentSource::Owned { binding } => binding.page,
            };
            let source = engine.document().get_page(origin)?;
            let target = engine.document().get_page(if frame.created {
                request
                    .frames
                    .last()
                    .ok_or_else(|| fail("figure continuation template missing"))?
                    .page
            } else {
                frame.frame.page
            })?;
            if source.rotate.rem_euclid(360) != target.rotate.rem_euclid(360)
                || (source.user_unit - target.user_unit).abs() > 1e-9
                || (!frame.created
                    && engine
                        .document()
                        .reader()
                        .get_object(target.object_number, target.generation_number)?
                        .as_dict()
                        .is_some_and(|d| d.contains_key("Group")))
            {
                return Err(fail(
                    "figure destination requires page compositing/rotation migration",
                ));
            }
        }
    }
    if seen.len() != request.figures.len() {
        return Err(fail("figure pagination omitted an owner"));
    }
    for page in request
        .figures
        .iter()
        .map(|figure| match &figure.source {
            ImageFragmentSource::Occurrence { page, .. } => *page,
            ImageFragmentSource::Owned { binding } => binding.page,
        })
        .chain(request.figure_removals.iter().map(|r| r.binding.page))
    {
        preview.changed_pages.push(page);
        let crop = engine
            .as_ref()
            .ok_or_else(|| fail("figure source document missing"))?
            .document()
            .get_page(page)?
            .crop_box;
        if !preview
            .full_page_invalidations
            .iter()
            .any(|(n, _)| *n == page)
        {
            preview.full_page_invalidations.push((page, crop));
        }
    }
    preview.changed_pages.sort_unstable();
    preview.changed_pages.dedup();
    preview.figure_removals = request.figure_removals.clone();
    preview.figure_detachments = request.figure_detachments.clone();
    if !request.figure_removals.is_empty() {
        preview.exact_limits.push("Explicit figure removal deletes only the selected painted owner from the current revision; historical bytes and shared image resources can remain. This is not sanitizing redaction".into());
    }
    if !request.figure_detachments.is_empty() {
        preview.exact_limits.push("Explicit figure detachment preserves the exact current painted owner in place and releases only saved story ownership. A cross-story transfer is a second revision-bound transaction; it is not an atomic two-story move".into());
    }
    if !request.figures.is_empty() {
        preview.exact_limits.push("Images and whole captions share one frame with explicit dimensions and stacking. Approved page-owned or exact nested-Form invisible OCR joins a native image/search group; source frame ranges rebind after removal. Tagged capture accepts page-owned Figure paint or a nested Form MCR; exact selected spans may remain with the Figure or be partitioned across explicitly selected content-only sibling P/Span owners. An explicitly approved bounded semantic-only Figure descendant tree is retained without flattening when every descendant is contentless; valid explicit descendant page bindings follow the Figure destination. A separate one-shot approval removes the complete contentless subtree only when no surviving relationship targets it. Reused-Form splitting can copy-on-write clone a separately approved contentless subtree with rebuilt parent links, rewritten internal Figure/subtree /Ref links and no duplicate IDs/page bindings; external subtree links use the same explicit outbound/incoming split decisions, while links between multiple trees split together follow one coordinated clone map. Reused Form occurrences otherwise require an explicit semantic split that preserves residual Figure and ordered selected OCR-owner leaves. Outbound /Ref ownership can move, remain or copy; incoming /Ref owners can follow, retarget or reference both through bounded direct/indirect array graphs, with topology-preserving copy-on-write for rewritten containers. Cyclic, excessive or non-reference relationship containers, content-bearing/shared reused Figure subtrees and mixed-object table cells remain separate migrations".into());
    }
    let captures = request
        .figures
        .iter()
        .filter_map(|f| f.ocr.as_ref())
        .collect::<Vec<_>>();
    if !captures.is_empty() {
        preview.exact_limits.push(format!("Explicit OCR grouping: {} source operands across {} initial image captures. Unselected invisible text stays in place; a carrier cannot also be selected as story paragraph text. Owned groups retain their search content on later moves and explicit deletion removes both current paints.",captures.iter().map(|s|s.span_ids.len()).sum::<usize>(),captures.len()));
    }
    let unrelated = request.figures.iter().filter(|f| f.ocr_unrelated).count();
    if unrelated > 0 {
        preview.exact_limits.push(format!("Explicit unrelated-OCR decisions for {unrelated} initial images: their page OCR remains in place, not inside the moved native group."));
    }
    Ok(())
}

pub(crate) fn rebind(
    input: &[u8],
    figures: &[StoryFigure],
    source_revision: &str,
    verify_old: bool,
) -> Result<Vec<StoryFigure>> {
    if figures.is_empty() {
        return Ok(Vec::new());
    }
    let inventory = crate::image_fragments::image_fragment_bindings(input)?
        .into_iter()
        .map(|b| (b.key.clone(), b))
        .collect::<BTreeMap<_, _>>();
    figures
        .iter()
        .map(|figure| {
            let key = crate::image_fragments::stories::source_key(source_revision, &figure.source);
            let binding = inventory
                .get(&key)
                .ok_or_else(|| fail("saved story figure owner is missing"))?;
            if verify_old {
                if figure.ocr.is_some() || figure.ocr_unrelated {
                    return Err(fail(
                        "saved figure contains an unconsumed original-revision OCR selection",
                    ));
                }
                match &figure.source {
                    ImageFragmentSource::Owned { binding: old }
                        if old.key == binding.key
                            && old.rect == binding.rect
                            && old.content_sha256 == binding.content_sha256 => {}
                    _ => {
                        return Err(fail(
                            "saved story figure changed outside its approved transaction",
                        ))
                    }
                }
            }
            let mut rebound = figure.clone();
            rebound.source = ImageFragmentSource::Owned {
                binding: binding.clone(),
            };
            rebound.ocr = None;
            rebound.ocr_unrelated = false;
            Ok(rebound)
        })
        .collect()
}
