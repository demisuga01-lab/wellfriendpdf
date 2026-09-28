//! Private batch detachment/placement for linked-story transactions. Every
//! source range is bound on the same original revision. Temporary catalog
//! references keep capsules reachable through canonical page-tree insertion.
use super::*;
use crate::linked_stories::{LinkedStoryPreview, LinkedStoryRequest};
#[path = "story_ocr_ranges.rs"]
mod source_text;

const STAGED: &str = "WFStagedStoryFigures";
struct Inventory<'a> {
    capture: CaptureContext<'a>,
    owner: String,
    occurrences: Vec<crate::universal_editing::UniversalImageOccurrenceV2>,
    owners: Vec<ImageFragmentBinding>,
    occurrence_capsules: BTreeMap<(usize, usize, usize, usize), Arc<Vec<u8>>>,
}
impl<'a> Inventory<'a> {
    fn new(input: &'a [u8], request: &LinkedStoryRequest) -> Result<Self> {
        let pages = request
            .figures
            .iter()
            .filter_map(|f| match &f.source {
                ImageFragmentSource::Occurrence { page, .. } => Some(*page),
                _ => None,
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        // Linked-story validation has already proved exact Figure paint /
        // semantic scope ownership on these immutable input bytes.
        let capture = CaptureContext::new(
            input,
            request.signature_policy_override,
            request.source_tags.is_some(),
        )?;
        let occurrences = if pages.is_empty() {
            Vec::new()
        } else {
            crate::universal_editing::universal_image_occurrences_v2(input, &pages)?
        };
        let mut selections = Vec::new();
        for figure in &request.figures {
            let ImageFragmentSource::Occurrence {
                page,
                content_stream_index,
                occurrence_id,
            } = &figure.source
            else {
                continue;
            };
            let mut candidates = occurrences.iter().filter(|occurrence| {
                occurrence.page == *page
                    && occurrence.content_stream_index == *content_stream_index
                    && occurrence.occurrence_id == *occurrence_id
            });
            let selected = candidates
                .next()
                .ok_or_else(|| fail("story image occurrence is stale or absent"))?;
            if candidates.next().is_some() {
                return Err(fail("story image occurrence identity is ambiguous"));
            }
            if selected.invocation_path.is_empty() {
                selections.push((
                    *page,
                    *content_stream_index,
                    [selected.operation_byte_start, selected.operation_byte_end],
                ));
            }
        }
        let occurrence_capsules =
            capture_occurrence_capsules(&capture, &selections, 128 * 1024 * 1024)?;
        Ok(Self {
            capture,
            owner: request.story_id.clone(),
            occurrences,
            owners: if !request.figure_removals.is_empty()
                || request
                    .figures
                    .iter()
                    .any(|f| matches!(&f.source, ImageFragmentSource::Owned { .. }))
            {
                image_fragment_bindings(input)?
            } else {
                Vec::new()
            },
            occurrence_capsules,
        })
    }
    fn prepare(&self, request: &ImageFragmentMove) -> Result<Prepared> {
        let nested = match &request.source {
            ImageFragmentSource::Occurrence {
                page,
                content_stream_index,
                occurrence_id,
            } => self.occurrences.iter().any(|occurrence| {
                occurrence.page == *page
                    && occurrence.content_stream_index == *content_stream_index
                    && occurrence.occurrence_id == *occurrence_id
                    && !occurrence.invocation_path.is_empty()
            }),
            ImageFragmentSource::Owned { .. } => false,
        };
        prepare_known(
            &self.capture,
            request,
            Some(&self.occurrences),
            Some(&self.owners),
            Some(&self.owner),
            (!nested).then_some(&self.occurrence_capsules),
        )
    }
}
pub(crate) fn source_key(revision: &str, source: &ImageFragmentSource) -> String {
    match source {
        ImageFragmentSource::Occurrence {
            content_stream_index,
            occurrence_id,
            ..
        } => hash(
            format!("image-fragment:{revision}:{content_stream_index}:{occurrence_id}").as_bytes(),
        ),
        ImageFragmentSource::Owned { binding } => binding.key.clone(),
    }
}
fn request_for(
    engine: &ContentEngine,
    request: &LinkedStoryRequest,
    source: &ImageFragmentSource,
    ocr: Option<&OcrCarrierSelection>,
) -> Result<ImageFragmentMove> {
    let page = match source {
        ImageFragmentSource::Occurrence { page, .. } => *page,
        ImageFragmentSource::Owned { binding } => binding.page,
    };
    Ok(ImageFragmentMove {
        input_sha256: request.input_sha256.clone(),
        source: source.clone(),
        target_page: page,
        target_rect: match source {
            ImageFragmentSource::Owned { binding } => binding.rect,
            ImageFragmentSource::Occurrence { .. } => engine.document().get_page(page)?.crop_box,
        },
        stack: ImageFragmentStack::Foreground,
        ocr: ocr.cloned(),
        signature_policy_override: request.signature_policy_override,
    })
}
pub(crate) fn validate(input: &[u8], request: &LinkedStoryRequest) -> Result<()> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let inventory = Inventory::new(input, request)?;
    let mut size = 0usize;
    let mut removed_text = source_text::RemovedText::default();
    if engine.document().get_catalog()?.contains_key(STAGED) {
        return Err(fail("uncommitted image staging metadata exists"));
    }
    for figure in &request.figures {
        let prepared = inventory.prepare(&request_for(
            &engine,
            request,
            &figure.source,
            figure.ocr.as_ref(),
        )?)?;
        if let Some(capture) = &prepared.ocr {
            removed_text.add(prepared.source.page_number, capture)?;
        }
        size = size
            .saturating_add(prepared.preview.source_state_bytes)
            .saturating_add(prepared.preview.ocr_source_text.len());
        if size > 128 * 1024 * 1024 {
            return Err(fail("story image capsule budget exceeded"));
        }
        for scale in [
            figure.width / prepared.size[0],
            figure.height / prepared.size[1],
        ] {
            if !scale.is_finite() || !(1e-9..=1e9).contains(&scale) {
                return Err(fail("story image scale exceeds finite bounds"));
            }
        }
    }
    for removal in &request.figure_removals {
        let prepared = inventory.prepare(&request_for(
            &engine,
            request,
            &ImageFragmentSource::Owned {
                binding: removal.binding.clone(),
            },
            None,
        )?)?;
        size = size
            .saturating_add(prepared.preview.source_state_bytes)
            .saturating_add(prepared.preview.ocr_source_text.len());
        if size > 128 * 1024 * 1024 {
            return Err(fail("story image capsule budget exceeded"));
        }
    }
    removed_text.seal(&request.frames)?;
    Ok(())
}
#[derive(Default)]
pub(crate) struct Receipt {
    entries: BTreeMap<String, String>,
    removed: BTreeSet<String>,
    owner: String,
    ocr: BTreeMap<String, (usize, String)>,
    removed_text: source_text::RemovedText,
}
impl Receipt {
    pub(crate) fn rebind_source_frames(
        &self,
        input: &[u8],
        frames: &mut [crate::linked_stories::StoryFrameLayout],
    ) -> Result<()> {
        self.removed_text.rebind_frames(input, frames)
    }
}
struct Patch {
    range: [usize; 2],
    sha: String,
    replacement: Vec<u8>,
}
struct Stream {
    id: Ref,
    bytes: Vec<u8>,
    patches: Vec<Patch>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct NestedEdge {
    range: [usize; 2],
    resource_name: String,
    child: Ref,
}

struct NestedNode {
    source: Ref,
    resources: PdfDictionary,
    patches: BTreeMap<[usize; 2], Vec<u8>>,
    children: BTreeMap<NestedEdge, NestedNode>,
}

struct NestedRoot {
    source: Ref,
    children: BTreeMap<NestedEdge, NestedNode>,
}

#[derive(Default)]
struct NestedBatch {
    roots: BTreeMap<(usize, usize), NestedRoot>,
    nodes: usize,
}

impl NestedBatch {
    fn insert_path<'a>(
        children: &'a mut BTreeMap<NestedEdge, NestedNode>,
        path: &[crate::advanced_editing::VectorFormInvocation],
        resources: &[PdfDictionary],
        depth: usize,
        added: &mut usize,
    ) -> Result<&'a mut NestedNode> {
        let invocation = path
            .get(depth)
            .ok_or_else(|| fail("nested story image path ended early"))?;
        let edge = NestedEdge {
            range: [
                invocation.owner_operation_byte_start,
                invocation.owner_operation_byte_end,
            ],
            resource_name: invocation.resource_name.clone(),
            child: (invocation.form_object, invocation.form_generation),
        };
        let node = match children.entry(edge) {
            std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
            std::collections::btree_map::Entry::Vacant(entry) => {
                *added = added
                    .checked_add(1)
                    .ok_or_else(|| fail("nested story image tree size overflow"))?;
                entry.insert(NestedNode {
                    source: (invocation.form_object, invocation.form_generation),
                    resources: resources
                        .get(depth + 1)
                        .cloned()
                        .ok_or_else(|| fail("nested story image resources missing"))?,
                    patches: BTreeMap::new(),
                    children: BTreeMap::new(),
                })
            }
        };
        if node.source != (invocation.form_object, invocation.form_generation) {
            return Err(fail("nested story image prefix owner changed"));
        }
        if depth + 1 == path.len() {
            Ok(node)
        } else {
            Self::insert_path(&mut node.children, path, resources, depth + 1, added)
        }
    }

    fn add(&mut self, reader: &PdfReader, prepared: &Prepared) -> Result<()> {
        let nested = prepared
            .nested
            .as_ref()
            .ok_or_else(|| fail("nested story image plan missing"))?;
        let path = &nested.occurrence.invocation_path;
        let outer = path
            .first()
            .ok_or_else(|| fail("nested story image invocation path missing"))?;
        let source = prepared
            .source
            .contents
            .get(prepared.stream)
            .copied()
            .ok_or_else(|| fail("nested story image page stream missing"))?;
        if source != (outer.owner_stream_object, outer.owner_stream_generation) {
            return Err(fail("nested story image outer owner changed"));
        }
        let resources = nested_resource_chain(reader, &prepared.source.resources, path)?;
        let root = self
            .roots
            .entry((prepared.source.page_number, prepared.stream))
            .or_insert_with(|| NestedRoot {
                source,
                children: BTreeMap::new(),
            });
        if root.source != source {
            return Err(fail("nested story image page owner is ambiguous"));
        }
        let mut added = 0usize;
        let selected_node = Self::insert_path(&mut root.children, path, &resources, 0, &mut added)?;
        let image_range = [
            nested.occurrence.operation_byte_start,
            nested.occurrence.operation_byte_end,
        ];
        if selected_node
            .patches
            .insert(image_range, b"\n".to_vec())
            .is_some()
        {
            return Err(fail("nested story image occurrence is duplicated"));
        }
        if let Some(capture) = &prepared.ocr {
            if !capture.form_local || capture.source_edits.keys().any(|index| *index != 0) {
                return Err(fail(
                    "nested story OCR edits are not bound to the selected leaf Form",
                ));
            }
            for (start, end, replacement) in
                capture.source_edits.get(&0).cloned().unwrap_or_default()
            {
                if selected_node
                    .patches
                    .insert([start, end], replacement)
                    .is_some()
                {
                    return Err(fail("nested story OCR source patch is duplicated"));
                }
            }
        }
        self.nodes = self
            .nodes
            .checked_add(added)
            .ok_or_else(|| fail("nested story image tree size overflow"))?;
        if self.nodes > 65_536 {
            return Err(fail("nested story image tree node budget exceeded"));
        }
        Ok(())
    }

    fn emit_node_with_receipt(
        updates: &mut Updates,
        reader: &PdfReader,
        node: &NestedNode,
        mappings: &mut Vec<(Ref, Ref)>,
        tagged: bool,
    ) -> Result<Ref> {
        let mut resources = node.resources.clone();
        let mut patches = node
            .patches
            .iter()
            .map(|(range, replacement)| (range[0], range[1], replacement.clone()))
            .collect::<Vec<_>>();
        for (edge, child) in &node.children {
            if xobject_resources(reader, &resources)?
                .get(&edge.resource_name)
                .and_then(PdfObject::as_reference)
                != Some(edge.child)
            {
                return Err(fail("nested story image child resource changed"));
            }
            let child = Self::emit_node_with_receipt(updates, reader, child, mappings, tagged)?;
            let (next_resources, name) = add_unique_xobject(reader, &resources, "WFNIS", child)?;
            resources = next_resources;
            patches.push((
                edge.range[0],
                edge.range[1],
                format!("/{name} Do").into_bytes(),
            ));
        }
        let bytes = decode(reader, node.source)?;
        let output = ocr_carriers::apply_patches(&bytes, patches)?;
        let object = reader.get_object(node.source.0, node.source.1)?;
        let mut dictionary = object
            .as_stream()
            .ok_or_else(|| fail("nested story image node is not a stream"))?
            .0
            .clone();
        if dictionary.get_name("Subtype") != Some("Form") {
            return Err(fail("nested story image node is not a Form"));
        }
        dictionary.insert("Resources", PdfObject::Dictionary(resources));
        if tagged {
            dictionary.insert("WFNestedImageSource", reference(node.source));
        }
        let clone = updates.stream(&output, dictionary)?;
        if tagged {
            mappings.push((clone, node.source));
        }
        Ok(clone)
    }

    fn emit(
        &self,
        updates: &mut Updates,
        reader: &PdfReader,
        pages: &BTreeMap<usize, crate::document::PdfPage>,
        tagged: bool,
    ) -> Result<(
        BTreeMap<(usize, usize), Vec<(usize, usize, Vec<u8>)>>,
        BTreeMap<usize, PdfDictionary>,
        Vec<(Ref, Ref)>,
    )> {
        let mut patches = BTreeMap::<(usize, usize), Vec<(usize, usize, Vec<u8>)>>::new();
        let mut page_resources = BTreeMap::<usize, PdfDictionary>::new();
        let mut mappings = Vec::new();
        for ((page_number, stream), root) in &self.roots {
            crate::cancel::check_current_cancel("nested story image clone tree")?;
            let page = pages
                .get(page_number)
                .ok_or_else(|| fail("nested story image source page disappeared"))?;
            if page.contents.get(*stream) != Some(&root.source) {
                return Err(fail("nested story image source stream changed"));
            }
            let resources = page_resources
                .entry(*page_number)
                .or_insert_with(|| page.resources.clone());
            for (edge, child) in &root.children {
                if xobject_resources(reader, resources)?
                    .get(&edge.resource_name)
                    .and_then(PdfObject::as_reference)
                    != Some(edge.child)
                {
                    return Err(fail("nested story image page resource changed"));
                }
                let child =
                    Self::emit_node_with_receipt(updates, reader, child, &mut mappings, tagged)?;
                let (next_resources, name) = add_unique_xobject(reader, resources, "WFNIP", child)?;
                *resources = next_resources;
                patches.entry((*page_number, *stream)).or_default().push((
                    edge.range[0],
                    edge.range[1],
                    format!("/{name} Do").into_bytes(),
                ));
            }
        }
        Ok((patches, page_resources, mappings))
    }
}

pub(crate) fn stage(input: &[u8], request: &LinkedStoryRequest) -> Result<(Vec<u8>, Receipt)> {
    if request.figures.is_empty() && request.figure_removals.is_empty() {
        return Ok((input.to_vec(), Receipt::default()));
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let inventory = Inventory::new(input, request)?;
    let mut catalog = engine.document().get_catalog()?;
    if catalog.contains_key(STAGED) {
        return Err(fail("image staging metadata already exists"));
    }
    let root = reader
        .root_reference()
        .ok_or_else(|| fail("image stage catalog missing"))?;
    let mut updates = Updates {
        next: reader.object_ids().iter().map(|r| r.0).max().unwrap_or(0),
        objects: Vec::new(),
    };
    let mut streams = BTreeMap::<(usize, usize), Stream>::new();
    let mut pages = BTreeMap::new();
    let mut entries = PdfDictionary::empty();
    let mut receipt = Receipt {
        owner: hash(request.story_id.as_bytes()),
        entries: BTreeMap::new(),
        removed: BTreeSet::new(),
        ocr: BTreeMap::new(),
        removed_text: source_text::RemovedText::default(),
    };
    let mut decoded = 0usize;
    let mut capsules = 0usize;
    let mut source_keys = BTreeSet::new();
    let mut nested_batch = NestedBatch::default();
    for (figure_id, source, selection, remove) in request
        .figures
        .iter()
        .map(|f| (&f.id, f.source.clone(), f.ocr.as_ref(), false))
        .chain(request.figure_removals.iter().map(|r| {
            (
                &r.figure_id,
                ImageFragmentSource::Owned {
                    binding: r.binding.clone(),
                },
                None,
                true,
            )
        }))
    {
        crate::cancel::check_current_cancel("story image batch capture")?;
        let prepared = inventory.prepare(&request_for(&engine, request, &source, selection)?)?;
        capsules = capsules
            .saturating_add(prepared.preview.source_state_bytes)
            .saturating_add(prepared.preview.ocr_source_text.len());
        if capsules > 128 * 1024 * 1024 || !source_keys.insert(prepared.preview.key.clone()) {
            return Err(fail(
                "duplicate story image source or capsule budget exceeded",
            ));
        }
        if prepared.nested.is_some() {
            nested_batch.add(reader, &prepared)?;
        }
        if remove {
            receipt.removed.insert(prepared.preview.key.clone());
        } else {
            receipt.ocr.insert(
                prepared.preview.key.clone(),
                (
                    prepared.preview.ocr_spans,
                    prepared.preview.ocr_source_text.clone(),
                ),
            );
            let form = if prepared.nested.is_some() {
                nested_capsule_form(&mut updates, &prepared, reader)?
            } else {
                capsule_form(&mut updates, &prepared)?
            };
            let mut entry = PdfDictionary::empty();
            entry.insert("Form", reference(form));
            entry.insert(
                "Key",
                PdfObject::String(prepared.preview.key.as_bytes().to_vec()),
            );
            entry.insert(
                "Size",
                PdfObject::Array(prepared.size.into_iter().map(PdfObject::Real).collect()),
            );
            entry.insert(
                "Origin",
                reference((
                    prepared.source.object_number,
                    prepared.source.generation_number,
                )),
            );
            entries.insert(hash(figure_id.as_bytes()), PdfObject::Dictionary(entry));
            receipt
                .entries
                .insert(figure_id.clone(), prepared.preview.key.clone());
        }
        let mut edits = if let Some(capture) = &prepared.ocr {
            receipt
                .removed_text
                .add(prepared.source.page_number, capture)?;
            if capture.form_local {
                BTreeMap::new()
            } else {
                capture.source_edits.clone()
            }
        } else {
            BTreeMap::new()
        };
        if prepared.nested.is_none() {
            edits.entry(prepared.stream).or_default().push((
                prepared.range[0],
                prepared.range[1],
                b"\n".to_vec(),
            ));
        }
        for (index, patches) in edits {
            let slot = (prepared.source.page_number, index);
            let data = prepared
                .source_buffers
                .get(index)
                .ok_or_else(|| fail("OCR batch stream index invalid"))?;
            if let std::collections::btree_map::Entry::Vacant(e) = streams.entry(slot) {
                decoded = decoded.saturating_add(data.len());
                if decoded > MAX_TOTAL {
                    return Err(fail("story image source buffer budget exceeded"));
                }
                e.insert(Stream {
                    id: prepared.source.contents[index],
                    bytes: data.clone(),
                    patches: Vec::new(),
                });
            }
            for (start, end, replacement) in patches {
                streams
                    .get_mut(&slot)
                    .ok_or_else(|| fail("image batch stream missing"))?
                    .patches
                    .push(Patch {
                        range: [start, end],
                        sha: hash(
                            data.get(start..end)
                                .ok_or_else(|| fail("OCR batch patch leaves source bounds"))?,
                        ),
                        replacement,
                    });
            }
        }
        pages.insert(prepared.source.page_number, prepared.source);
    }
    let (nested_patches, mut nested_page_resources, nested_sources) =
        nested_batch.emit(&mut updates, reader, &pages, request.source_tags.is_some())?;
    for ((page_number, index), patches) in nested_patches {
        let page = pages
            .get(&page_number)
            .ok_or_else(|| fail("nested story image page disappeared before rewrite"))?;
        let id = page
            .contents
            .get(index)
            .copied()
            .ok_or_else(|| fail("nested story image page stream disappeared"))?;
        if let std::collections::btree_map::Entry::Vacant(e) = streams.entry((page_number, index)) {
            let bytes = decode(reader, id)?;
            decoded = decoded.saturating_add(bytes.len());
            if decoded > MAX_TOTAL {
                return Err(fail("story image source buffer budget exceeded"));
            }
            e.insert(Stream {
                id,
                bytes,
                patches: Vec::new(),
            });
        }
        let stream = streams
            .get_mut(&(page_number, index))
            .ok_or_else(|| fail("nested story image rewrite stream missing"))?;
        for (start, end, replacement) in patches {
            stream.patches.push(Patch {
                range: [start, end],
                sha: hash(
                    stream
                        .bytes
                        .get(start..end)
                        .ok_or_else(|| fail("nested story image patch leaves source bounds"))?,
                ),
                replacement,
            });
        }
    }
    receipt.removed_text.seal(&request.frames)?;
    let mut rewritten = BTreeMap::new();
    for (number, page) in pages {
        let mut contents = page.contents.clone();
        let mut retired = BTreeSet::new();
        // Peel only complete, selected native batches at document edges. Other
        // edits interleaved with an old batch retain their enclosing q/Q state.
        let mut begin = 0usize;
        let mut end = contents.len();
        while begin < end {
            if let Some(s) = streams.get(&(number, begin)) {
                if fully_owned_paint(&s.bytes, &s.patches, false)? {
                    retired.insert(begin);
                    begin += 1;
                    continue;
                }
            }
            if end > begin + 1 {
                if let Some(s) = streams.get(&(number, end - 1)) {
                    let first = decode(reader, contents[begin])?;
                    if first == b"q\n" && fully_owned_paint(&s.bytes, &s.patches, true)? {
                        let pd = reader.get_object(contents[begin].0, contents[begin].1)?;
                        let td = reader.get_object(contents[end - 1].0, contents[end - 1].1)?;
                        if isolation_pair(pd.as_stream().map(|v| v.0), td.as_stream().map(|v| v.0))
                        {
                            retired.insert(begin);
                            retired.insert(end - 1);
                            begin += 1;
                            end -= 1;
                            continue;
                        }
                    }
                }
            }
            break;
        }
        for (index, id) in contents.iter_mut().enumerate() {
            if retired.contains(&index) {
                continue;
            }
            let Some(stream) = streams.remove(&(number, index)) else {
                continue;
            };
            if stream.id != *id {
                return Err(fail("image batch occurrence identity changed"));
            }
            for patch in &stream.patches {
                let [start, end] = patch.range;
                if stream
                    .bytes
                    .get(start..end)
                    .is_none_or(|b| hash(b) != patch.sha)
                {
                    return Err(fail("image batch ranges overlap or source changed"));
                }
            }
            let bytes = ocr_carriers::apply_patches(
                &stream.bytes,
                stream
                    .patches
                    .into_iter()
                    .map(|p| (p.range[0], p.range[1], p.replacement))
                    .collect(),
            )?;
            let original = reader.get_object(id.0, id.1)?;
            let dict = original
                .as_stream()
                .ok_or_else(|| fail("image batch stream is not a stream"))?
                .0
                .clone();
            *id = updates.stream(&bytes, dict)?;
            rewritten.insert((number, *id), hash(&bytes));
        }
        let mut dict = reader
            .get_object(page.object_number, page.generation_number)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("image batch page missing"))?;
        if let Some(resources) = nested_page_resources.remove(&number) {
            dict.insert("Resources", PdfObject::Dictionary(resources));
        }
        dict.insert(
            "Contents",
            PdfObject::Array(
                contents
                    .into_iter()
                    .enumerate()
                    .filter(|(i, _)| !retired.contains(i))
                    .map(|(_, r)| reference(r))
                    .collect(),
            ),
        );
        updates.put(
            (page.object_number, page.generation_number),
            PdfObject::Dictionary(dict),
        );
    }
    let mut staged = PdfDictionary::empty();
    staged.insert(
        "Owner",
        PdfObject::String(receipt.owner.as_bytes().to_vec()),
    );
    staged.insert("Entries", PdfObject::Dictionary(entries));
    staged.insert(
        "Removed",
        PdfObject::Array(
            receipt
                .removed
                .iter()
                .map(|key| PdfObject::String(key.as_bytes().to_vec()))
                .collect(),
        ),
    );
    staged.insert(
        "NestedSources",
        PdfObject::Array(
            nested_sources
                .iter()
                .map(|(clone, source)| {
                    let mut mapping = PdfDictionary::empty();
                    mapping.insert("Clone", reference(*clone));
                    mapping.insert("Source", reference(*source));
                    PdfObject::Dictionary(mapping)
                })
                .collect(),
        ),
    );
    catalog.insert(STAGED, PdfObject::Dictionary(staged));
    updates.put(root, PdfObject::Dictionary(catalog));
    let output = write_incremental_update(reader, updates.objects)?;
    let reopened = ContentEngine::open_bytes(output.clone())?;
    for ((page, id), expected) in rewritten {
        crate::cancel::check_current_cancel("story image/OCR batch readback")?;
        if !reopened.document().get_page(page)?.contents.contains(&id)
            || hash(&decode(reopened.document().reader(), id)?) != expected
        {
            return Err(fail("story image/OCR source detachment failed readback"));
        }
    }
    Ok((output, receipt))
}

pub(crate) fn staged_nested_sources(input: &[u8], story_id: &str) -> Result<Vec<(Ref, Ref)>> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let catalog = engine.document().get_catalog()?;
    let stage = catalog
        .get(STAGED)
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("story image staging metadata is missing"))?;
    let owner = hash(story_id.as_bytes());
    if !matches!(stage.get("Owner"), Some(PdfObject::String(value)) if value == owner.as_bytes()) {
        return Err(fail("story image staging owner changed"));
    }
    let mappings = stage
        .get("NestedSources")
        .and_then(PdfObject::as_array)
        .ok_or_else(|| fail("nested story image staging map is missing"))?;
    if mappings.len() > 65_536 {
        return Err(fail("nested story image staging map budget exceeded"));
    }
    let mut clones = BTreeSet::new();
    let mut result = Vec::with_capacity(mappings.len());
    for mapping in mappings {
        let mapping = mapping
            .as_dict()
            .ok_or_else(|| fail("invalid nested story image staging entry"))?;
        let clone = mapping
            .get_reference("Clone")
            .ok_or_else(|| fail("nested story image clone is missing"))?;
        let source = mapping
            .get_reference("Source")
            .ok_or_else(|| fail("nested story image source is missing"))?;
        let clone_object = reader.get_object(clone.0, clone.1)?;
        let dictionary = clone_object
            .as_stream()
            .ok_or_else(|| fail("nested story image clone is not a stream"))?
            .0;
        if dictionary.get_name("Subtype") != Some("Form")
            || dictionary.get_reference("WFNestedImageSource") != Some(source)
            || !clones.insert(clone)
        {
            return Err(fail("nested story image staging receipt changed"));
        }
        result.push((clone, source));
    }
    Ok(result)
}
fn isolation_pair(prefix: Option<&PdfDictionary>, tail: Option<&PdfDictionary>) -> bool {
    let (Some(p), Some(t)) = (prefix, tail) else {
        return false;
    };
    for key in ["WFImageIsolationKey", "WFImageBatchKey"] {
        if matches!(p.get(key), Some(PdfObject::String(_)))
            && p.get(key) == t.get(key)
            && p.get_name("WFImageIsolationRole") == Some("Prefix")
            && t.get_name("WFImageIsolationRole") == Some("Tail")
        {
            return true;
        }
    }
    false
}
fn fully_owned_paint(bytes: &[u8], patches: &[Patch], tail: bool) -> Result<bool> {
    let mut ranges = patches.iter().collect::<Vec<_>>();
    ranges.sort_by_key(|p| p.range);
    let prefix = if tail {
        b"n\nQ\n".as_slice()
    } else {
        b"".as_slice()
    };
    if !bytes.starts_with(prefix) {
        return Ok(false);
    }
    let mut cursor = prefix.len();
    for patch in ranges {
        let [start, end] = patch.range;
        if patch.replacement != b"\n"
            || start < cursor
            || bytes
                .get(cursor..start)
                .is_none_or(|b| b.iter().any(|v| !v.is_ascii_whitespace()))
            || bytes
                .get(start..end)
                .is_none_or(|b| hash(b) != patch.sha || !b.starts_with(b"/WFImageFragment "))
        {
            return Ok(false);
        }
        cursor = end;
    }
    Ok(!patches.is_empty()
        && bytes
            .get(cursor..)
            .is_some_and(|b| b.iter().all(u8::is_ascii_whitespace)))
}
fn batch_dict(owner: &str, role: &str) -> PdfDictionary {
    let mut d = PdfDictionary::empty();
    d.insert(
        "WFImageBatchKey",
        PdfObject::String(owner.as_bytes().to_vec()),
    );
    d.insert("WFImageIsolationRole", PdfObject::Name(role.into()));
    d
}

pub(crate) fn finish(
    input: &[u8],
    request: &LinkedStoryRequest,
    preview: &LinkedStoryPreview,
    receipt: &Receipt,
) -> Result<Vec<u8>> {
    if request.figures.is_empty() && request.figure_removals.is_empty() {
        return Ok(input.to_vec());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    let mut catalog = engine.document().get_catalog()?;
    let stage = catalog
        .get(STAGED)
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("story image staging was lost during page insertion"))?;
    if !matches!(stage.get("Owner"),Some(PdfObject::String(v)) if v==receipt.owner.as_bytes()) {
        return Err(fail("story image staging owner changed"));
    }
    let entries = stage
        .get("Entries")
        .and_then(PdfObject::as_dict)
        .ok_or_else(|| fail("story image staging entries missing"))?;
    if entries.len() != receipt.entries.len() || entries.len() != request.figures.len() {
        return Err(fail("story image staging cardinality changed"));
    }
    let removed = PdfObject::Array(
        receipt
            .removed
            .iter()
            .map(|key| PdfObject::String(key.as_bytes().to_vec()))
            .collect(),
    );
    if stage.get("Removed") != Some(&removed)
        || receipt.removed
            != request
                .figure_removals
                .iter()
                .map(|r| r.binding.key.clone())
                .collect()
    {
        return Err(fail("story image deletion receipt changed"));
    }
    let mut updates = Updates {
        next: reader.object_ids().iter().map(|r| r.0).max().unwrap_or(0),
        objects: Vec::new(),
    };
    let mut pages = BTreeMap::<usize, (Vec<Vec<u8>>, Vec<Vec<u8>>, PdfDictionary)>::new();
    let mut expected = BTreeMap::new();
    let all_pages = engine.document().get_pages()?;
    let source_pages = all_pages
        .iter()
        .map(|p| ((p.object_number, p.generation_number), p))
        .collect::<BTreeMap<_, _>>();
    for frame in &preview.frames {
        for placement in &frame.figures {
            crate::cancel::check_current_cancel("story image batch placement")?;
            let figure = request
                .figures
                .iter()
                .find(|f| f.id == placement.figure_id)
                .ok_or_else(|| fail("image placement owner is absent"))?;
            let key = receipt
                .entries
                .get(&figure.id)
                .ok_or_else(|| fail("image placement receipt missing"))?;
            let entry = entries
                .get(&hash(figure.id.as_bytes()))
                .and_then(PdfObject::as_dict)
                .ok_or_else(|| fail("image staged entry missing"))?;
            if !matches!(entry.get("Key"),Some(PdfObject::String(v)) if v==key.as_bytes()) {
                return Err(fail("staged image key changed"));
            }
            let form = entry
                .get_reference("Form")
                .ok_or_else(|| fail("staged image Form reference missing"))?;
            if receipt.ocr.get(key) != Some(&ocr::info(reader, form)?) {
                return Err(fail(
                    "staged native OCR group changed during story pagination",
                ));
            }
            let size = entry
                .get_array("Size")
                .ok_or_else(|| fail("staged image dimensions missing"))?;
            if size.len() != 2 {
                return Err(fail("staged image dimensions invalid"));
            }
            let size = [
                size[0].as_number().unwrap_or(f64::NAN),
                size[1].as_number().unwrap_or(f64::NAN),
            ];
            let origin = entry
                .get_reference("Origin")
                .ok_or_else(|| fail("staged image origin page missing"))?;
            let source = source_pages
                .get(&origin)
                .ok_or_else(|| fail("staged image origin page lost"))?;
            let target = engine.document().get_page(frame.frame.page)?;
            if source.rotate.rem_euclid(360) != target.rotate.rem_euclid(360)
                || (source.user_unit - target.user_unit).abs() > 1e-9
                || reader
                    .get_object(target.object_number, target.generation_number)?
                    .as_dict()
                    .is_some_and(|d| d.contains_key("Group"))
            {
                return Err(fail(
                    "story image destination requires page compositing/rotation migration",
                ));
            }
            let r = placement.rect;
            let crop = target.crop_box;
            if !valid_rect(r)
                || r[0] < crop[0] - 1e-7
                || r[1] < crop[1] - 1e-7
                || r[2] > crop[2] + 1e-7
                || r[3] > crop[3] + 1e-7
                || size.iter().any(|v| !v.is_finite() || *v <= 0.0)
            {
                return Err(fail("story image output geometry invalid"));
            }
            for scale in [(r[2] - r[0]) / size[0], (r[3] - r[1]) / size[1]] {
                if !scale.is_finite() || !(1e-9..=1e9).contains(&scale) {
                    return Err(fail("story image placement scale invalid"));
                }
            }
            if let std::collections::btree_map::Entry::Vacant(e) = pages.entry(target.page_number) {
                e.insert((Vec::new(), Vec::new(), target.resources.clone()));
            }
            let (back, front, resources) = pages
                .get_mut(&target.page_number)
                .ok_or_else(|| fail("story image destination vanished"))?;
            let mut xo = dict(reader, resources.get("XObject"))?;
            let name = format!("WFIF{key}");
            if xo
                .get(&name)
                .is_some_and(|v| v.as_reference() != Some(form))
            {
                return Err(fail("story image resource collision"));
            }
            xo.insert(name, reference(form));
            resources.insert("XObject", PdfObject::Dictionary(xo));
            let paint = owned_bytes(key, r, size);
            if expected
                .insert(key.clone(), (target.page_number, r, hash(&paint)))
                .is_some()
            {
                return Err(fail("story image owner painted more than once"));
            }
            let paint =
                crate::tagged_structure::story::figures::wrap_paint(request, &figure.id, paint)?;
            match figure.stack {
                ImageFragmentStack::Background => back.push(paint),
                ImageFragmentStack::Foreground => front.push(paint),
            };
        }
    }
    if expected.len() != receipt.entries.len() {
        return Err(fail("story image output omitted an owner"));
    }
    let mut total = 0usize;
    for (number, (back, front, resources)) in pages {
        let page = engine.document().get_page(number)?;
        let mut state = State::default();
        for bytes in page_buffers(reader, &page, &mut total)? {
            state.feed(&bytes, None, false)?;
        }
        state.finish()?;
        let mut contents = page
            .contents
            .iter()
            .copied()
            .map(reference)
            .collect::<Vec<_>>();
        if !back.is_empty() {
            let bytes = join_paints(&back);
            contents.insert(
                0,
                reference(updates.stream(&bytes, batch_dict(&receipt.owner, "Background"))?),
            );
        }
        if !front.is_empty() {
            let prefix = updates.stream(b"q\n", batch_dict(&receipt.owner, "Prefix"))?;
            let mut bytes = b"n\nQ\n".to_vec();
            bytes.extend_from_slice(&join_paints(&front));
            let tail = updates.stream(&bytes, batch_dict(&receipt.owner, "Tail"))?;
            contents.insert(0, reference(prefix));
            contents.push(reference(tail));
        }
        let mut pd = reader
            .get_object(page.object_number, page.generation_number)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("story image destination page invalid"))?;
        pd.insert("Resources", PdfObject::Dictionary(resources));
        pd.insert("Contents", PdfObject::Array(contents));
        updates.put(
            (page.object_number, page.generation_number),
            PdfObject::Dictionary(pd),
        );
    }
    catalog.remove(STAGED);
    updates.put(
        reader
            .root_reference()
            .ok_or_else(|| fail("image stage catalog missing"))?,
        PdfObject::Dictionary(catalog),
    );
    let output = write_incremental_update(reader, updates.objects)?;
    let bindings = image_fragment_bindings(&output)?;
    if bindings.iter().any(|b| receipt.removed.contains(&b.key)) {
        return Err(fail("deleted story figure is still painted in the output"));
    }
    for (key, (page, rect, sha)) in expected {
        if !bindings
            .iter()
            .any(|b| b.key == key && b.page == page && b.rect == rect && b.content_sha256 == sha)
        {
            return Err(fail("story image output ownership postcondition failed"));
        }
    }
    Ok(output)
}
fn join_paints(paints: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for p in paints {
        out.extend_from_slice(p);
        out.push(b'\n');
    }
    out
}
