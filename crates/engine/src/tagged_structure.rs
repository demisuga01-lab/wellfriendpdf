//! Structural owner indexing and ParentTree construction from the actual /K
//! graph. MCIDs are indexes in a container-specific parent array, never indexes
//! in an arbitrary list of all structure elements. No reading order is inferred.
use crate::content::operation::Operand;
use crate::content::parser::ContentParser;
use crate::filters::{decode_stream_lossless_with_limits, DecodeLimits, StreamDecodeStatus};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{ContentEngine, PdfDictionary, PdfObject, PdfReader, Result, WellfriendError};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[path = "tagged_annotation_deletion.rs"]
mod annotation_deletion;
#[path = "tagged_attributes.rs"]
mod attributes;
#[path = "tagged_story.rs"]
pub mod story;
#[path = "tagged_stream_clones.rs"]
pub mod stream_clones;

pub(crate) type ObjectRef = (u32, u16);
const MAX_NODES: usize = 100_000;
const MAX_MCID: usize = 262_143;
const MAX_PARENT_SLOTS: usize = 1_048_576;
const MAX_STREAM: u64 = 64 * 1024 * 1024;
const MAX_DECODED: usize = 512 * 1024 * 1024;
fn fail(message: impl Into<String>) -> WellfriendError {
    WellfriendError::invalid_input(message)
}
fn reference(id: ObjectRef) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ParentTreeReport {
    pub structure_elements: usize,
    pub marked_content_containers: usize,
    pub marked_content_items: usize,
    pub object_reference_items: usize,
    pub parent_tree_entries: usize,
    pub id_tree_entries: usize,
    pub materialized_direct_elements: usize,
    pub materialized_direct_roots: usize,
    pub repaired_parent_pointers: usize,
    pub assigned_or_changed_keys: usize,
    pub output_reopened: bool,
    pub ownership_verified: bool,
    pub conformance_certified: bool,
    pub limits: Vec<String>,
}

/// Shadow objects allow direct structure dictionaries to be made indirect,
/// preserving arbitrary metadata/children and using the normal atomic writer.
struct Store<'a> {
    reader: &'a PdfReader,
    updates: BTreeMap<ObjectRef, PdfObject>,
    next: u32,
}
impl<'a> Store<'a> {
    fn new(reader: &'a PdfReader) -> Self {
        Self {
            reader,
            updates: BTreeMap::new(),
            next: reader
                .object_ids()
                .iter()
                .map(|(n, _)| *n)
                .max()
                .unwrap_or(0),
        }
    }
    fn get(&self, id: ObjectRef) -> Result<PdfObject> {
        self.updates
            .get(&id)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| self.reader.get_object(id.0, id.1))
    }
    fn resolve(&self, value: &PdfObject) -> Result<PdfObject> {
        Ok(self.resolve_identity(value)?.0)
    }
    fn resolve_identity(&self, value: &PdfObject) -> Result<(PdfObject, Option<ObjectRef>)> {
        let mut value = value.clone();
        let mut seen = BTreeSet::new();
        let mut identity = None;
        while let Some(id) = value.as_reference() {
            if seen.len() >= 128 || !seen.insert(id) {
                return Err(fail("cyclic/overlong structural reference"));
            }
            identity = Some(id);
            value = self.get(id)?;
        }
        Ok((value, identity))
    }
    fn add(&mut self, value: PdfObject) -> Result<ObjectRef> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or_else(|| fail("structure object space exhausted"))?;
        let id = (self.next, 0);
        self.updates.insert(id, value);
        Ok(id)
    }
    fn dict(&self, id: ObjectRef) -> Result<PdfDictionary> {
        let object = self.get(id)?;
        dictionary(&object)
            .cloned()
            .ok_or_else(|| fail("structural target is not a dictionary/stream"))
    }
    fn replace_dict(&mut self, id: ObjectRef, dict: PdfDictionary) -> Result<()> {
        let object = match self.get(id)? {
            PdfObject::Dictionary(_) => PdfObject::Dictionary(dict),
            PdfObject::Stream { raw, .. } => PdfObject::Stream { dict, raw },
            _ => return Err(fail("invalid structural dictionary target")),
        };
        self.updates.insert(id, object);
        Ok(())
    }
}
fn dictionary(object: &PdfObject) -> Option<&PdfDictionary> {
    match object {
        PdfObject::Dictionary(d) | PdfObject::Stream { dict: d, .. } => Some(d),
        _ => None,
    }
}

fn appearance_streams(
    store: &Store<'_>,
    value: &PdfObject,
    out: &mut BTreeSet<ObjectRef>,
    active: &mut BTreeSet<ObjectRef>,
    visits: &mut usize,
    depth: usize,
) -> Result<()> {
    crate::cancel::check_current_cancel("tagged annotation appearance ownership")?;
    *visits += 1;
    if *visits > MAX_NODES || depth > 64 {
        return Err(fail("annotation appearance ownership budget exceeded"));
    }
    let (object, id) = store.resolve_identity(value)?;
    if let Some(id) = id {
        if !active.insert(id) {
            return Err(fail("cyclic annotation appearance graph"));
        }
    }
    match object {
        PdfObject::Stream { .. } => {
            out.insert(id.ok_or_else(|| fail("annotation appearance stream must be indirect"))?);
        }
        PdfObject::Dictionary(dict) => {
            if depth > 1 {
                return Err(fail(
                    "appearance state must name a stream, not a dictionary",
                ));
            }
            for (name, child) in dict.iter() {
                // AP extension metadata is not an appearance program. Only
                // N/R/D values name streams or one level of state dictionaries.
                if depth == 0 && !matches!(name.as_str(), "N" | "R" | "D") {
                    continue;
                }
                appearance_streams(store, child, out, active, visits, depth + 1)?;
            }
        }
        PdfObject::Null => {}
        _ => return Err(fail("invalid annotation appearance graph")),
    }
    if let Some(id) = id {
        active.remove(&id);
    }
    Ok(())
}

struct StructureIndex {
    nodes: BTreeSet<ObjectRef>,
    marked: BTreeMap<ObjectRef, BTreeMap<usize, ObjectRef>>,
    objects: BTreeMap<ObjectRef, ObjectRef>,
    object_pages: BTreeMap<ObjectRef, ObjectRef>,
    stream_pages: BTreeMap<ObjectRef, ObjectRef>,
    stream_owners: BTreeMap<ObjectRef, ObjectRef>,
    report: ParentTreeReport,
    visits: usize,
}
impl StructureIndex {
    fn new() -> Self {
        Self {
            nodes: BTreeSet::new(),
            marked: BTreeMap::new(),
            objects: BTreeMap::new(),
            object_pages: BTreeMap::new(),
            stream_pages: BTreeMap::new(),
            stream_owners: BTreeMap::new(),
            report: ParentTreeReport::default(),
            visits: 0,
        }
    }
    fn marked(&mut self, container: ObjectRef, mcid: i64, parent: ObjectRef) -> Result<()> {
        let mcid = usize::try_from(mcid).map_err(|_| fail("negative structure MCID"))?;
        if mcid > MAX_MCID {
            return Err(fail("structure MCID exceeds bounded parent-array capacity"));
        }
        if self.objects.contains_key(&container) {
            return Err(fail(
                "one object cannot have both StructParent and StructParents ownership",
            ));
        }
        if self
            .marked
            .entry(container)
            .or_default()
            .insert(mcid, parent)
            .is_some()
        {
            return Err(fail(
                "a marked-content item has duplicate structure ownership",
            ));
        }
        Ok(())
    }
    fn kids(
        &mut self,
        store: &mut Store<'_>,
        value: &PdfObject,
        parent: ObjectRef,
        page: Option<ObjectRef>,
        pages: &BTreeSet<ObjectRef>,
        depth: usize,
    ) -> Result<PdfObject> {
        crate::cancel::check_current_cancel("structure ownership traversal")?;
        self.visits += 1;
        if depth > 128 || self.visits > MAX_NODES {
            return Err(fail("structure traversal budget exceeded"));
        }
        let (object, id) = store.resolve_identity(value)?;
        match object {
            PdfObject::Null => Ok(PdfObject::Null),
            PdfObject::Array(items) => {
                let mut rewritten = Vec::with_capacity(items.len());
                for item in items {
                    rewritten.push(self.kids(store, &item, parent, page, pages, depth + 1)?);
                }
                Ok(PdfObject::Array(rewritten))
            }
            PdfObject::Integer(mcid) => {
                let owner = page.ok_or_else(|| fail("integer MCID has no inherited page"))?;
                self.marked(owner, mcid, parent)?;
                Ok(PdfObject::Integer(mcid))
            }
            PdfObject::Dictionary(mut dict) => {
                let here = match dict.get("Pg") {
                    Some(v) => {
                        let p = v.as_reference().ok_or_else(|| {
                            fail("structure Pg must be an indirect page reference")
                        })?;
                        if !pages.contains(&p) {
                            return Err(fail("structure Pg is not a page in this document"));
                        }
                        Some(p)
                    }
                    None => page,
                };
                if dict.get_name("Type") == Some("MCR")
                    || (dict.get_name("S").is_none() && dict.contains_key("MCID"))
                {
                    let mcid = dict
                        .get_integer("MCID")
                        .ok_or_else(|| fail("MCR is missing an integer MCID"))?;
                    let container = if let Some(stm) = dict.get("Stm") {
                        let stream = stm
                            .as_reference()
                            .ok_or_else(|| fail("MCR Stm must be indirect"))?;
                        if store.get(stream)?.as_stream().is_none() {
                            return Err(fail("MCR Stm is not a stream"));
                        }
                        if let Some(owner) = dict.get("StmOwn") {
                            let owner = owner
                                .as_reference()
                                .ok_or_else(|| fail("MCR StmOwn must be indirect"))?;
                            if dictionary(&store.get(owner)?).is_none() {
                                return Err(fail(
                                    "MCR StmOwn must reference a dictionary or stream",
                                ));
                            }
                            if self
                                .stream_owners
                                .insert(stream, owner)
                                .is_some_and(|old| old != owner)
                            {
                                return Err(fail("MCR stream has conflicting StmOwn owners"));
                            }
                        }
                        if let Some(page) = here {
                            if self
                                .stream_pages
                                .insert(stream, page)
                                .is_some_and(|old| old != page)
                            {
                                return Err(fail(
                                    "shared tagged stream has ambiguous page ownership",
                                ));
                            }
                        }
                        stream
                    } else {
                        if dict.contains_key("StmOwn") {
                            return Err(fail("MCR StmOwn requires a Stm"));
                        }
                        here.ok_or_else(|| fail("page MCR is missing Pg"))?
                    };
                    self.marked(container, mcid, parent)?;
                    Ok(PdfObject::Dictionary(dict))
                } else if dict.get_name("Type") == Some("OBJR") {
                    let item = dict
                        .get_reference("Obj")
                        .ok_or_else(|| fail("OBJR requires an indirect Obj"))?;
                    if dictionary(&store.get(item)?).is_none() {
                        return Err(fail("OBJR target is not a dictionary or stream"));
                    }
                    if self.marked.contains_key(&item)
                        || self.objects.insert(item, parent).is_some()
                    {
                        return Err(fail("object reference has conflicting structure owners"));
                    }
                    if let Some(page) = here {
                        self.object_pages.insert(item, page);
                    }
                    Ok(PdfObject::Dictionary(dict))
                } else {
                    if dict
                        .get_name("Type")
                        .is_some_and(|kind| kind != "StructElem")
                    {
                        return Err(fail("invalid structure element Type"));
                    }
                    if dict.get_name("S").is_none() {
                        return Err(fail(
                            "structure child has neither a role nor a content-item type",
                        ));
                    }
                    let node = if let Some(id) = id {
                        id
                    } else {
                        self.report.materialized_direct_elements += 1;
                        store.add(PdfObject::Dictionary(dict.clone()))?
                    };
                    if !self.nodes.insert(node) {
                        return Err(fail("cyclic/shared structure element requires an explicit ownership decision"));
                    }
                    if dict.get_reference("P") != Some(parent) {
                        self.report.repaired_parent_pointers += 1;
                        dict.insert("P", reference(parent));
                    }
                    if let Some(kids) = dict.get("K").cloned() {
                        dict.insert("K", self.kids(store, &kids, node, here, pages, depth + 1)?);
                    }
                    store.updates.insert(node, PdfObject::Dictionary(dict));
                    Ok(reference(node))
                }
            }
            _ => Err(fail("invalid structure K item")),
        }
    }
}

/// Collect actual MCID namespaces using the canonical content parser. Inline
/// image payloads and dictionary literals are never interpreted as operators.
struct ContentScopes<'a> {
    engine: &'a ContentEngine,
    actual: BTreeMap<ObjectRef, BTreeSet<usize>>,
    active: BTreeSet<ObjectRef>,
    decoded: usize,
    visits: usize,
    calls: BTreeMap<ObjectRef, BTreeSet<ObjectRef>>,
    page_resources: PdfDictionary,
}
impl<'a> ContentScopes<'a> {
    fn new(engine: &'a ContentEngine) -> Self {
        Self {
            engine,
            actual: BTreeMap::new(),
            active: BTreeSet::new(),
            decoded: 0,
            visits: 0,
            calls: BTreeMap::new(),
            page_resources: PdfDictionary::empty(),
        }
    }
    fn stream(&mut self, id: ObjectRef) -> Result<Vec<u8>> {
        let reader = self.engine.document().reader();
        let object = reader.get_object(id.0, id.1)?;
        let decoded = decode_stream_lossless_with_limits(
            &object,
            reader,
            &DecodeLimits {
                max_decoded_bytes_per_stream: MAX_STREAM,
                ..Default::default()
            },
        )?;
        if decoded.status != StreamDecodeStatus::Complete {
            return Err(fail("opaque tagged content stream"));
        }
        self.decoded = self
            .decoded
            .checked_add(decoded.data.len())
            .ok_or_else(|| fail("tagged decode budget overflow"))?;
        if self.decoded > MAX_DECODED {
            return Err(fail("tagged decode budget exceeded"));
        }
        Ok(decoded.data)
    }
    fn resource_dict(&self, resources: &PdfDictionary, key: &str) -> Result<PdfDictionary> {
        match resources.get(key) {
            None => Ok(PdfDictionary::empty()),
            Some(v) => self
                .engine
                .document()
                .reader()
                .resolve(v.clone())?
                .as_dict()
                .cloned()
                .ok_or_else(|| fail(format!("invalid tagged resource {key}"))),
        }
    }
    fn program_resources(&self, dict: &PdfDictionary) -> Result<PdfDictionary> {
        match dict
            .get("Resources")
            .map(|value| self.engine.document().reader().resolve(value.clone()))
            .transpose()?
        {
            None | Some(PdfObject::Null) => Ok(self.page_resources.clone()),
            Some(PdfObject::Dictionary(resources)) => Ok(resources),
            _ => Err(fail("invalid content-owner Resources")),
        }
    }
    fn scan(
        &mut self,
        id: ObjectRef,
        data: &[u8],
        resources: &PdfDictionary,
        depth: usize,
    ) -> Result<()> {
        self.visits += 1;
        if depth > 64 || self.visits > MAX_NODES || !self.active.insert(id) {
            return Err(fail("cyclic or excessive tagged content invocation"));
        }
        let result = self.scan_inner(id, data, resources, depth);
        self.active.remove(&id);
        result
    }
    fn scan_inner(
        &mut self,
        id: ObjectRef,
        data: &[u8],
        resources: &PdfDictionary,
        depth: usize,
    ) -> Result<()> {
        let engine = self.engine;
        let reader = engine.document().reader();
        let properties = self.resource_dict(resources, "Properties")?;
        let operations =
            ContentParser::parse_cancellable(data, &crate::cancel::current_cancel_token())?;
        let mut mcids = BTreeSet::new();
        let mut marked_depth = 0usize;
        for operation in operations {
            crate::cancel::check_current_cancel("marked-content namespace inventory")?;
            match operation.operator.as_str() {
                "BDC" => {
                    marked_depth += 1;
                    if marked_depth > 4096 {
                        return Err(fail("marked-content depth limit exceeded"));
                    }
                    let value = match operation.operands.get(1) {
                        Some(Operand::Dictionary(entries)) => {
                            let items = entries
                                .iter()
                                .filter(|(key, _)| key == "MCID")
                                .collect::<Vec<_>>();
                            if items.len() > 1 {
                                return Err(fail("duplicate MCID property"));
                            }
                            match items.first() {
                                None => None,
                                Some((_, Operand::Null)) => None,
                                Some((_, v)) => Some(
                                    v.as_integer()
                                        .ok_or_else(|| fail("noninteger MCID property"))?,
                                ),
                            }
                        }
                        Some(Operand::Name(name)) => {
                            let property = properties
                                .get(name)
                                .ok_or_else(|| fail("missing marked-content property resource"))?;
                            let resolved = reader.resolve(property.clone())?;
                            let dict = resolved.as_dict().ok_or_else(|| {
                                fail("marked-content property is not a dictionary")
                            })?;
                            match dict.get("MCID") {
                                None | Some(PdfObject::Null) => None,
                                Some(v) => Some(
                                    reader
                                        .resolve(v.clone())?
                                        .as_integer()
                                        .ok_or_else(|| fail("noninteger named MCID"))?,
                                ),
                            }
                        }
                        _ => return Err(fail("invalid BDC property operand")),
                    };
                    if let Some(mcid) = value {
                        let mcid =
                            usize::try_from(mcid).map_err(|_| fail("negative content MCID"))?;
                        if mcid > MAX_MCID || !mcids.insert(mcid) {
                            return Err(fail(
                                "duplicate/out-of-budget MCID in one content namespace",
                            ));
                        }
                    }
                }
                "BMC" => {
                    marked_depth += 1;
                    if marked_depth > 4096 {
                        return Err(fail("marked-content depth limit exceeded"));
                    }
                }
                "EMC" => {
                    marked_depth = marked_depth
                        .checked_sub(1)
                        .ok_or_else(|| fail("unbalanced tagged marked content"))?;
                }
                "Do" => {
                    let name = operation
                        .operands
                        .first()
                        .and_then(Operand::as_name)
                        .ok_or_else(|| fail("invalid XObject invocation"))?;
                    let xobjects = self.resource_dict(resources, "XObject")?;
                    let target = xobjects
                        .get(name)
                        .ok_or_else(|| fail("missing tagged XObject resource"))?;
                    let object = reader.resolve(target.clone())?;
                    if let Some((dict, _)) = object.as_stream() {
                        if dict.get_name("Subtype") == Some("Form") {
                            let stream = target
                                .as_reference()
                                .ok_or_else(|| fail("Form XObject must be indirect"))?;
                            let local = self.program_resources(dict)?;
                            let bytes = self.stream(stream)?;
                            self.calls.entry(id).or_default().insert(stream);
                            self.scan(stream, &bytes, &local, depth + 1)?;
                        }
                    }
                }
                _ => {}
            }
        }
        if marked_depth != 0 {
            return Err(fail("unterminated marked content in tagged namespace"));
        }
        if self
            .actual
            .insert(id, mcids.clone())
            .is_some_and(|old| old != mcids)
        {
            return Err(fail(
                "a shared content stream resolves different MCIDs in different resource contexts",
            ));
        }
        Ok(())
    }
    fn descendants(&self, root: ObjectRef) -> Result<BTreeSet<ObjectRef>> {
        let mut out = BTreeSet::new();
        let mut pending = vec![root];
        while let Some(scope) = pending.pop() {
            crate::cancel::check_current_cancel("tagged invocation ownership closure")?;
            if out.insert(scope) {
                if out.len() > MAX_NODES {
                    return Err(fail("tagged invocation closure budget exceeded"));
                }
                if let Some(children) = self.calls.get(&scope) {
                    pending.extend(children.iter().copied());
                }
            }
        }
        Ok(out)
    }
    fn collect(&mut self, index: &StructureIndex) -> Result<()> {
        self.collect_inner(index, true)
    }
    /// Physical page/appearance reachability only, without treating old MCR
    /// references as roots. Used while a staged clone has not been retagged yet.
    fn collect_live(&mut self) -> Result<()> {
        self.collect_inner(&StructureIndex::new(), false)
    }
    fn collect_inner(&mut self, index: &StructureIndex, verify_ownership: bool) -> Result<()> {
        let engine = self.engine;
        let appearance_store = Store::new(engine.document().reader());
        let pages = self.engine.document().get_pages()?;
        let page_indices = pages
            .iter()
            .enumerate()
            .map(|(i, p)| ((p.object_number, p.generation_number), i))
            .collect::<BTreeMap<_, _>>();
        let mut annotation_pages = BTreeMap::new();
        let mut annotation_count = 0usize;
        let mut appearance_visits = 0usize;
        let mut appearances = BTreeMap::<ObjectRef, BTreeSet<(ObjectRef, ObjectRef)>>::new();
        for page in &pages {
            crate::cancel::check_current_cancel("tagged page ownership inspection")?;
            self.page_resources = page.resources.clone();
            let reader = self.engine.document().reader();
            let page_id = (page.object_number, page.generation_number);
            let page_object = reader.get_object(page_id.0, page_id.1)?;
            let page_dict = page_object
                .as_dict()
                .ok_or_else(|| fail("invalid page dictionary"))?;
            if let Some(annotations) = page_dict.get("Annots") {
                let annotations = reader.resolve(annotations.clone())?;
                let annotations = annotations
                    .as_array()
                    .ok_or_else(|| fail("invalid page Annots"))?;
                annotation_count = annotation_count.saturating_add(annotations.len());
                if annotation_count > MAX_NODES {
                    return Err(fail("tagged annotation inspection budget exceeded"));
                }
                for annotation in annotations {
                    let object = reader.resolve(annotation.clone())?;
                    let dict = object
                        .as_dict()
                        .ok_or_else(|| fail("invalid annotation dictionary"))?;
                    if let Some(id) = annotation.as_reference() {
                        if annotation_pages
                            .insert(id, page_id)
                            .is_some_and(|old| old != page_id)
                        {
                            return Err(fail(
                                "annotation shared by pages has ambiguous structural ownership",
                            ));
                        }
                        if index
                            .object_pages
                            .get(&id)
                            .is_some_and(|owner_page| *owner_page != page_id)
                        {
                            return Err(fail("annotation OBJR page disagrees with page Annots"));
                        }
                        if index.objects.contains_key(&id)
                            && dict.get_reference("P").is_some_and(|p| p != page_id)
                        {
                            return Err(fail("tagged annotation P disagrees with page Annots"));
                        }
                    }
                    if verify_ownership
                        && dict.contains_key("StructParent")
                        && annotation
                            .as_reference()
                            .is_none_or(|id| !index.objects.contains_key(&id))
                    {
                        return Err(fail("annotation StructParent has no reachable OBJR owner"));
                    }
                    if dict.contains_key("StructParents") {
                        return Err(fail(
                            "annotation uses a content-stream StructParents marker",
                        ));
                    }
                    if let Some(ap) = dict.get("AP") {
                        let id = annotation.as_reference().ok_or_else(|| {
                            fail("appearance-bearing annotation must be indirect")
                        })?;
                        let mut streams = BTreeSet::new();
                        appearance_streams(
                            &appearance_store,
                            ap,
                            &mut streams,
                            &mut BTreeSet::new(),
                            &mut appearance_visits,
                            0,
                        )?;
                        for stream in streams {
                            appearances.entry(stream).or_default().insert((id, page_id));
                        }
                    }
                }
            }
            let mut data = Vec::new();
            for &stream in &page.contents {
                let bytes = self.stream(stream)?;
                if data.len().saturating_add(bytes.len()).saturating_add(1) > MAX_STREAM as usize {
                    return Err(fail("tagged page content budget exceeded"));
                }
                data.extend_from_slice(&bytes);
                data.push(b'\n');
            }
            self.scan(
                (page.object_number, page.generation_number),
                &data,
                &page.resources,
                0,
            )?;
        }
        let mut expanded_appearances =
            BTreeMap::<ObjectRef, BTreeSet<(ObjectRef, ObjectRef)>>::new();
        for (&stream, owners) in &appearances {
            let object = appearance_store.get(stream)?;
            let dict = object
                .as_stream()
                .ok_or_else(|| fail("invalid appearance stream"))?
                .0;
            for &(owner, page_id) in owners {
                if index
                    .stream_owners
                    .get(&stream)
                    .is_some_and(|id| *id != owner)
                {
                    return Err(fail(
                        "MCR StmOwn disagrees with actual annotation appearance owner",
                    ));
                }
                if index
                    .stream_pages
                    .get(&stream)
                    .is_some_and(|id| *id != page_id)
                {
                    return Err(fail("MCR Pg disagrees with annotation appearance page"));
                }
                let page = &pages[*page_indices
                    .get(&page_id)
                    .ok_or_else(|| fail("missing appearance owner page"))?];
                self.page_resources = page.resources.clone();
                let resources = self.program_resources(dict)?;
                let bytes = self.stream(stream)?;
                self.scan(stream, &bytes, &resources, 0)?;
                for child in self.descendants(stream)? {
                    if index
                        .stream_owners
                        .get(&child)
                        .is_some_and(|id| *id != owner)
                        || index
                            .stream_pages
                            .get(&child)
                            .is_some_and(|id| *id != page_id)
                    {
                        return Err(fail(
                            "nested appearance MCR disagrees with its annotation owner/page",
                        ));
                    }
                    expanded_appearances
                        .entry(child)
                        .or_default()
                        .insert((owner, page_id));
                }
            }
        }
        for (&stream, &owner) in &index.stream_owners {
            if annotation_pages.contains_key(&owner)
                && expanded_appearances
                    .get(&stream)
                    .is_none_or(|owners| !owners.iter().any(|(id, _)| *id == owner))
            {
                return Err(fail(
                    "MCR StmOwn points to an annotation that no longer owns the appearance",
                ));
            }
        }
        for page in &pages {
            let page_id = (page.object_number, page.generation_number);
            for stream in self.descendants(page_id)? {
                if index
                    .stream_pages
                    .get(&stream)
                    .is_some_and(|id| *id != page_id)
                {
                    return Err(fail("MCR Pg disagrees with a page invoking its Form"));
                }
            }
        }
        for &object in index.objects.keys() {
            let value = self
                .engine
                .document()
                .reader()
                .get_object(object.0, object.1)?;
            if dictionary(&value).is_some_and(|d| d.get_name("Type") == Some("Annot"))
                && !annotation_pages.contains_key(&object)
            {
                return Err(fail("OBJR annotation is absent from document page Annots"));
            }
        }
        for &scope in index.marked.keys() {
            if !self.actual.contains_key(&scope) {
                let object = self
                    .engine
                    .document()
                    .reader()
                    .get_object(scope.0, scope.1)?;
                let (dict, _) = object
                    .as_stream()
                    .ok_or_else(|| fail("marked-content owner is not a page or stream"))?;
                self.page_resources = index
                    .stream_pages
                    .get(&scope)
                    .and_then(|id| page_indices.get(id).map(|index| &pages[*index]))
                    .map(|p| p.resources.clone())
                    .unwrap_or_default();
                let resources = self.program_resources(dict)?;
                let data = self.stream(scope)?;
                self.scan(scope, &data, &resources, 0)?;
            }
        }
        if !verify_ownership {
            return Ok(());
        }
        for (&scope, actual) in &self.actual {
            let value = self
                .engine
                .document()
                .reader()
                .get_object(scope.0, scope.1)?;
            if dictionary(&value).is_some_and(|d| d.contains_key("StructParent"))
                && !index.objects.contains_key(&scope)
            {
                return Err(fail(
                    "content object StructParent has no reachable OBJR owner",
                ));
            }
            let expected = index
                .marked
                .get(&scope)
                .map(|m| m.keys().copied().collect())
                .unwrap_or_default();
            if *actual != expected {
                return Err(fail(format!(
                    "structure/content MCID mismatch for {} {}: actual {:?}, expected {:?}; explicit owner repair required",
                    scope.0, scope.1, actual, expected
                )));
            }
        }
        Ok(())
    }
}

fn index_document<'a>(
    engine: &'a ContentEngine,
    language: Option<&str>,
) -> Result<(Store<'a>, ObjectRef, StructureIndex)> {
    let reader = engine.document().reader();
    let mut store = Store::new(reader);
    let mut index = StructureIndex::new();
    let catalog_id = reader
        .root_reference()
        .ok_or_else(|| fail("missing catalog reference"))?;
    let mut catalog = engine.document().get_catalog()?;
    if let Some(lang) = language {
        catalog.insert("Lang", PdfObject::String(lang.as_bytes().to_vec()));
    }
    let root = match catalog.get("StructTreeRoot") {
        Some(v) => {
            let (object, id) = store.resolve_identity(v)?;
            let dict = object
                .as_dict()
                .cloned()
                .ok_or_else(|| fail("invalid StructTreeRoot"))?;
            if dict.get_name("Type") != Some("StructTreeRoot") {
                return Err(fail("invalid StructTreeRoot Type"));
            }
            if let Some(id) = id {
                id
            } else {
                index.report.materialized_direct_roots += 1;
                store.add(PdfObject::Dictionary(dict))?
            }
        }
        None => {
            let mut root = PdfDictionary::empty();
            root.insert("Type", PdfObject::Name("StructTreeRoot".into()));
            let id = store.add(PdfObject::Dictionary(root))?;
            let mut doc = PdfDictionary::empty();
            doc.insert("Type", PdfObject::Name("StructElem".into()));
            doc.insert("S", PdfObject::Name("Document".into()));
            doc.insert("P", reference(id));
            doc.insert("K", PdfObject::Array(Vec::new()));
            let document = store.add(PdfObject::Dictionary(doc))?;
            let mut root = store.dict(id)?;
            root.insert("K", reference(document));
            store.replace_dict(id, root)?;
            id
        }
    };
    catalog.insert("StructTreeRoot", reference(root));
    let mut mark = match catalog.get("MarkInfo") {
        None => PdfDictionary::empty(),
        Some(v) => store
            .resolve(v)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("invalid MarkInfo"))?,
    };
    mark.insert("Marked", PdfObject::Boolean(true));
    catalog.insert("MarkInfo", PdfObject::Dictionary(mark));
    store
        .updates
        .insert(catalog_id, PdfObject::Dictionary(catalog));
    let pages = engine
        .document()
        .get_pages()?
        .iter()
        .map(|p| (p.object_number, p.generation_number))
        .collect::<BTreeSet<_>>();
    let mut dict = store.dict(root)?;
    if let Some(kids) = dict.get("K").cloned() {
        dict.insert("K", index.kids(&mut store, &kids, root, None, &pages, 0)?);
    }
    if index
        .marked
        .values()
        .flat_map(|m| m.values())
        .chain(index.objects.values())
        .any(|owner| !index.nodes.contains(owner))
    {
        return Err(fail(
            "a structural content item must belong to a StructElem, not directly to StructTreeRoot",
        ));
    }
    store.replace_dict(root, dict)?;
    Ok((store, root, index))
}

fn parent_values(
    store: &mut Store<'_>,
    index: &mut StructureIndex,
) -> Result<BTreeMap<i64, PdfObject>> {
    let mut owners = index
        .marked
        .keys()
        .chain(index.objects.keys())
        .copied()
        .collect::<Vec<_>>();
    owners.sort_unstable();
    owners.dedup();
    let mut assigned = BTreeMap::new();
    let mut used = BTreeSet::new();
    for &id in &owners {
        let dict = store.dict(id)?;
        let key = dict.get_integer(if index.marked.contains_key(&id) {
            "StructParents"
        } else {
            "StructParent"
        });
        if let Some(key) = key.filter(|key| *key >= 0 && *key < i64::MAX) {
            if used.insert(key) {
                assigned.insert(id, key);
            }
        }
    }
    let mut free = 0i64;
    let mut values = BTreeMap::new();
    let mut slots = 0usize;
    for id in owners {
        let key = match assigned.get(&id) {
            Some(key) => *key,
            None => {
                while used.contains(&free) {
                    free = free
                        .checked_add(1)
                        .ok_or_else(|| fail("ParentTree key space exhausted"))?;
                }
                used.insert(free);
                free
            }
        };
        let mut dict = store.dict(id)?;
        let value = if let Some(mcids) = index.marked.get(&id) {
            let len = mcids.keys().last().copied().unwrap_or(0) + 1;
            slots = slots.saturating_add(len);
            if slots > MAX_PARENT_SLOTS {
                return Err(fail("ParentTree aggregate array budget exceeded"));
            }
            let mut array = vec![PdfObject::Null; len];
            for (&mcid, &parent) in mcids {
                array[mcid] = reference(parent);
            }
            if dict.get_integer("StructParents") != Some(key) || dict.contains_key("StructParent") {
                index.report.assigned_or_changed_keys += 1;
            }
            dict.remove("StructParent");
            dict.insert("StructParents", PdfObject::Integer(key));
            PdfObject::Array(array)
        } else {
            if dict.get_integer("StructParent") != Some(key) || dict.contains_key("StructParents") {
                index.report.assigned_or_changed_keys += 1;
            }
            dict.remove("StructParents");
            dict.insert("StructParent", PdfObject::Integer(key));
            reference(index.objects[&id])
        };
        store.replace_dict(id, dict)?;
        values.insert(key, value);
    }
    Ok(values)
}
fn lookup_tree<K: Ord + Clone>(
    store: &mut Store<'_>,
    values: &BTreeMap<K, PdfObject>,
    entry_name: &str,
    encode: fn(&K) -> PdfObject,
) -> Result<ObjectRef> {
    if values.is_empty() {
        let mut d = PdfDictionary::empty();
        d.insert(entry_name, PdfObject::Array(Vec::new()));
        return store.add(PdfObject::Dictionary(d));
    }
    let entries = values.iter().collect::<Vec<_>>();
    let mut layer = Vec::new();
    for chunk in entries.chunks(64) {
        crate::cancel::check_current_cancel("structural lookup tree construction")?;
        let low = (*chunk.first().unwrap().0).clone();
        let high = (*chunk.last().unwrap().0).clone();
        let mut d = PdfDictionary::empty();
        d.insert(
            "Limits",
            PdfObject::Array(vec![encode(&low), encode(&high)]),
        );
        let nums = chunk
            .iter()
            .flat_map(|(k, v)| [encode(k), (*v).clone()])
            .collect();
        d.insert(entry_name, PdfObject::Array(nums));
        layer.push((low, high, store.add(PdfObject::Dictionary(d))?));
    }
    while layer.len() > 1 {
        let mut next = Vec::new();
        for chunk in layer.chunks(64) {
            let low = chunk.first().unwrap().0.clone();
            let high = chunk.last().unwrap().1.clone();
            let mut d = PdfDictionary::empty();
            d.insert(
                "Limits",
                PdfObject::Array(vec![encode(&low), encode(&high)]),
            );
            d.insert(
                "Kids",
                PdfObject::Array(chunk.iter().map(|(_, _, id)| reference(*id)).collect()),
            );
            next.push((low, high, store.add(PdfObject::Dictionary(d))?));
        }
        layer = next;
    }
    Ok(layer[0].2)
}

fn structure_ids(
    store: &Store<'_>,
    index: &StructureIndex,
) -> Result<BTreeMap<Vec<u8>, PdfObject>> {
    let mut ids = BTreeMap::new();
    let mut bytes = 0usize;
    for &node in &index.nodes {
        crate::cancel::check_current_cancel("structure ID indexing")?;
        let dict = store.dict(node)?;
        if let Some(value) = dict.get("ID") {
            let value = store.resolve(value)?;
            let id = value
                .as_string()
                .ok_or_else(|| fail("structure ID must be a byte string"))?;
            bytes = bytes.saturating_add(id.len());
            if bytes > 16 * 1024 * 1024 {
                return Err(fail("structure ID byte budget exceeded"));
            }
            if ids.insert(id.to_vec(), reference(node)).is_some() {
                return Err(fail(
                    "duplicate structure ID requires an explicit identity decision",
                ));
            }
        }
    }
    Ok(ids)
}

pub fn rebuild_parent_tree(input: &[u8], language: &str) -> Result<(Vec<u8>, ParentTreeReport)> {
    if language.is_empty()
        || language.len() > 128
        || !language
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(fail("invalid document language"));
    }
    rebuild_owner_trees(input, Some(language))
}

fn rebuild_owner_trees(
    input: &[u8],
    language: Option<&str>,
) -> Result<(Vec<u8>, ParentTreeReport)> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let (mut store, root, mut index) = index_document(&engine, language)?;
    let mut content = ContentScopes::new(&engine);
    content.collect(&index)?;
    let values = parent_values(&mut store, &mut index)?;
    let parent_tree = lookup_tree(&mut store, &values, "Nums", |key| PdfObject::Integer(*key))?;
    let ids = structure_ids(&store, &index)?;
    let mut dict = store.dict(root)?;
    if ids.is_empty() {
        dict.remove("IDTree");
    } else {
        let id_tree = lookup_tree(&mut store, &ids, "Names", |key| {
            PdfObject::String(key.clone())
        })?;
        dict.insert("IDTree", reference(id_tree));
    }
    dict.insert("ParentTree", reference(parent_tree));
    dict.insert(
        "ParentTreeNextKey",
        PdfObject::Integer(values.keys().last().copied().unwrap_or(-1) + 1),
    );
    store.replace_dict(root, dict)?;
    // Empty scopes may retain bogus markers from an older repair. Clear only
    // scopes actually inspected and proven to own no MCID/OBJR; no global guess.
    for (&scope, mcids) in &content.actual {
        if mcids.is_empty() && !index.objects.contains_key(&scope) {
            let mut d = store.dict(scope)?;
            if d.remove("StructParents").is_some() {
                store.replace_dict(scope, d)?;
            }
        }
    }
    index.report.structure_elements = index.nodes.len();
    index.report.marked_content_containers = index.marked.len();
    index.report.marked_content_items = index.marked.values().map(|v| v.len()).sum();
    index.report.object_reference_items = index.objects.len();
    index.report.parent_tree_entries = values.len();
    index.report.id_tree_entries = ids.len();
    let updates = store
        .updates
        .into_iter()
        .map(|((number, generation), object)| IncrementalObject {
            number,
            generation,
            object,
        })
        .collect();
    let output = write_incremental_update(engine.document().reader(), updates)?;
    validate_parent_tree(&output)?;
    index.report.output_reopened = true;
    index.report.ownership_verified = true;
    index.report.limits=vec!["No semantic roles, reading order or alternative text are inferred".into(),"Page contents, invoked Forms, annotation appearance graphs and explicitly referenced MCR streams are checked; not full PDF/UA validation".into(),"Patterns, Type3 charprocs, semantic role rules and extension-specific ownership require separate qualification".into(),"Incremental history remains; this is not sanitization".into()];
    Ok((output, index.report))
}

#[derive(Default)]
struct LookupReadBudget {
    seen: BTreeSet<ObjectRef>,
    visits: usize,
    key_bytes: usize,
}

fn read_lookup_tree<K: Ord + Clone>(
    reader: &PdfReader,
    value: &PdfObject,
    out: &mut BTreeMap<K, PdfObject>,
    budget: &mut LookupReadBudget,
    depth: usize,
    entry_name: &str,
    key: fn(&PdfObject) -> Result<K>,
) -> Result<Option<(K, K)>> {
    crate::cancel::check_current_cancel("structural lookup tree validation")?;
    budget.visits += 1;
    if depth > 64 || budget.visits > MAX_NODES || out.len() > MAX_NODES {
        return Err(fail("structural lookup tree budget exceeded"));
    }
    let mut object = value.clone();
    let mut references = 0usize;
    while let Some(id) = object.as_reference() {
        references += 1;
        if references > 128 || !budget.seen.insert(id) {
            return Err(fail("cyclic/shared structural lookup tree node"));
        }
        object = reader.get_object(id.0, id.1)?;
    }
    let dict = object
        .as_dict()
        .ok_or_else(|| fail("structural lookup tree node is not a dictionary"))?;
    if dict.contains_key(entry_name) && dict.contains_key("Kids") {
        return Err(fail("structural lookup tree node has both values and Kids"));
    }
    let mut bounds: Option<(K, K)> = None;
    if let Some(nums) = dict.get(entry_name) {
        let nums = reader.resolve(nums.clone())?;
        let nums = nums
            .as_array()
            .ok_or_else(|| fail("invalid structural lookup tree values"))?;
        if nums.len() % 2 != 0 || out.len().saturating_add(nums.len() / 2) > MAX_NODES {
            return Err(fail("invalid/oversized structural lookup tree values"));
        }
        for pair in nums.chunks_exact(2) {
            budget.key_bytes = budget
                .key_bytes
                .saturating_add(pair[0].as_string().map_or(8, |s| s.len()));
            if budget.key_bytes > 16 * 1024 * 1024 {
                return Err(fail("structural lookup tree key budget exceeded"));
            }
            let k = key(&pair[0])?;
            if bounds.as_ref().is_some_and(|(_, high)| high >= &k)
                || out.insert(k.clone(), pair[1].clone()).is_some()
            {
                return Err(fail("unsorted/duplicate structural lookup tree key"));
            }
            match &mut bounds {
                Some((_, high)) => *high = k,
                None => bounds = Some((k.clone(), k)),
            }
        }
    } else if let Some(kids) = dict.get("Kids") {
        let kids = reader.resolve(kids.clone())?;
        let kids = kids
            .as_array()
            .ok_or_else(|| fail("invalid structural lookup tree Kids"))?;
        if kids.is_empty() || kids.len() > MAX_NODES {
            return Err(fail("invalid structural lookup tree child count"));
        }
        for kid in kids {
            if kid.as_reference().is_none() {
                return Err(fail("structural lookup tree Kids must be indirect"));
            }
            let (low, high) =
                read_lookup_tree(reader, kid, out, budget, depth + 1, entry_name, key)?
                    .ok_or_else(|| fail("empty structural lookup subtree"))?;
            if bounds.as_ref().is_some_and(|(_, old)| old >= &low) {
                return Err(fail("unordered/overlapping structural lookup subtrees"));
            }
            match &mut bounds {
                Some((_, old)) => *old = high,
                None => bounds = Some((low, high)),
            }
        }
    } else {
        return Err(fail("structural lookup tree node has no entries"));
    }
    match (dict.get("Limits"), &bounds) {
        (Some(limits), Some((low, high))) => {
            let limits = reader.resolve(limits.clone())?;
            let limits = limits
                .as_array()
                .ok_or_else(|| fail("invalid lookup tree Limits"))?;
            if limits.len() != 2 || key(&limits[0])? != *low || key(&limits[1])? != *high {
                return Err(fail("structural lookup tree Limits disagree with entries"));
            }
        }
        (None, Some(_)) if depth == 0 => {}
        (None, None) if depth == 0 => {}
        _ => return Err(fail("missing/inapplicable structural lookup tree Limits")),
    }
    Ok(bounds)
}

pub fn validate_parent_tree(input: &[u8]) -> Result<ParentTreeReport> {
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    let reader = engine.document().reader();
    if !engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        return Err(fail("document is not tagged"));
    }
    let (mut store, root, mut index) = index_document(&engine, None)?;
    if index.report.materialized_direct_elements != 0
        || index.report.materialized_direct_roots != 0
        || index.report.repaired_parent_pointers != 0
    {
        return Err(fail("structure elements need indirect-owner/parent repair"));
    }
    let mut content = ContentScopes::new(&engine);
    content.collect(&index)?;
    for (&scope, mcids) in &content.actual {
        if mcids.is_empty()
            && !index.objects.contains_key(&scope)
            && store.dict(scope)?.contains_key("StructParents")
        {
            return Err(fail(
                "empty content namespace has a dangling StructParents marker",
            ));
        }
    }
    let expected = parent_values(&mut store, &mut index)?;
    if index.report.assigned_or_changed_keys != 0 {
        return Err(fail("missing or conflicting structural parent keys"));
    }
    let ids = structure_ids(&store, &index)?;
    let root = reader.get_object(root.0, root.1)?;
    let root = root
        .as_dict()
        .ok_or_else(|| fail("invalid structure root"))?;
    let tree = root
        .get("ParentTree")
        .ok_or_else(|| fail("missing ParentTree"))?;
    let mut actual = BTreeMap::new();
    read_lookup_tree(
        reader,
        tree,
        &mut actual,
        &mut LookupReadBudget::default(),
        0,
        "Nums",
        |value| {
            value
                .as_integer()
                .filter(|key| *key >= 0)
                .ok_or_else(|| fail("invalid ParentTree key"))
        },
    )?;
    if actual.len() != expected.len() {
        return Err(fail("ParentTree has missing or unowned entries"));
    }
    for (key, value) in &expected {
        let actual = actual
            .get(key)
            .ok_or_else(|| fail("missing ParentTree owner"))?;
        let actual = if matches!(value, PdfObject::Array(_)) {
            reader.resolve(actual.clone())?
        } else {
            actual.clone()
        };
        if actual != *value {
            return Err(fail(
                "ParentTree disagrees with structural content ownership",
            ));
        }
    }
    if root
        .get_integer("ParentTreeNextKey")
        .is_none_or(|key| key <= expected.keys().last().copied().unwrap_or(-1))
    {
        return Err(fail("invalid ParentTreeNextKey"));
    }
    let mut actual_ids = BTreeMap::new();
    if let Some(tree) = root.get("IDTree") {
        read_lookup_tree(
            reader,
            tree,
            &mut actual_ids,
            &mut LookupReadBudget::default(),
            0,
            "Names",
            |value| {
                value
                    .as_string()
                    .map(|s| s.to_vec())
                    .ok_or_else(|| fail("invalid structure IDTree key"))
            },
        )?;
    }
    if actual_ids != ids {
        return Err(fail(
            "IDTree disagrees with reachable structure element IDs",
        ));
    }
    Ok(ParentTreeReport {
        structure_elements: index.nodes.len(),
        marked_content_containers: index.marked.len(),
        marked_content_items: index.marked.values().map(|m| m.len()).sum(),
        object_reference_items: index.objects.len(),
        parent_tree_entries: expected.len(),
        id_tree_entries: ids.len(),
        output_reopened: true,
        ownership_verified: true,
        ..Default::default()
    })
}

/// Read ownership declarations without first requiring indirect OBJR targets.
/// Annotation promotion uses this only to locate an exact source carrier, then
/// runs the full ownership validator on the complete unpublished candidate.
pub(crate) fn annotation_parent_entries(
    document: &crate::PdfDocument,
) -> Result<BTreeMap<i64, PdfObject>> {
    let reader = document.reader();
    let catalog = document.get_catalog()?;
    let root = reader.resolve(
        catalog
            .get("StructTreeRoot")
            .cloned()
            .ok_or_else(|| fail("tagged annotation has no StructTreeRoot"))?,
    )?;
    let root = root
        .as_dict()
        .ok_or_else(|| fail("invalid StructTreeRoot"))?;
    let tree = root
        .get("ParentTree")
        .ok_or_else(|| fail("tagged annotation has no ParentTree"))?;
    let mut entries = BTreeMap::new();
    read_lookup_tree(
        reader,
        tree,
        &mut entries,
        &mut LookupReadBudget::default(),
        0,
        "Nums",
        |value| {
            value
                .as_integer()
                .filter(|key| *key >= 0)
                .ok_or_else(|| fail("invalid annotation ParentTree key"))
        },
    )?;
    Ok(entries)
}

/// Stage only structural changes for an annotation-page transaction. The caller
/// must apply these alongside /P and both /Annots arrays, then validate the
/// complete candidate before publication. No intermediate PDF is returned.
/// Logical /K order stays unchanged: a geometric move does not imply a reading-
/// order change. Explicit MCR /Pg wins over a structure ancestor's inherited Pg.
pub(crate) fn annotation_page_migration(
    input: &[u8],
    moves: &BTreeMap<ObjectRef, ObjectRef>,
) -> Result<Vec<IncrementalObject>> {
    if moves.is_empty() {
        return Ok(Vec::new());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if !engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        return Ok(Vec::new());
    }
    validate_parent_tree(input)?;
    let (store, _, index) = index_document(&engine, None)?;
    let mut content = ContentScopes::new(&engine);
    content.collect(&index)?;
    let pages = engine.document().get_pages()?;
    let page_ids = pages
        .iter()
        .map(|p| (p.object_number, p.generation_number))
        .collect::<BTreeSet<_>>();
    if moves.values().any(|p| !page_ids.contains(p)) {
        return Err(fail("annotation migration target is not a document page"));
    }

    let mut appearance_owners = BTreeMap::<ObjectRef, BTreeSet<ObjectRef>>::new();
    let mut annotations = BTreeSet::new();
    let mut visits = 0;
    for page in &pages {
        let dict = store.dict((page.object_number, page.generation_number))?;
        if let Some(annots) = dict.get("Annots") {
            let annots = store.resolve(annots)?;
            for annotation in annots.as_array().ok_or_else(|| fail("invalid Annots"))? {
                let Some(id) = annotation.as_reference() else {
                    continue;
                };
                annotations.insert(id);
                let annotation = store.dict(id)?;
                if let Some(ap) = annotation.get("AP") {
                    let mut streams = BTreeSet::new();
                    appearance_streams(
                        &store,
                        ap,
                        &mut streams,
                        &mut BTreeSet::new(),
                        &mut visits,
                        0,
                    )?;
                    for stream in streams {
                        for scope in content.descendants(stream)? {
                            appearance_owners.entry(scope).or_default().insert(id);
                        }
                    }
                }
            }
        }
    }
    if moves.keys().any(|id| !annotations.contains(id)) {
        return Err(fail("annotation migration source is not in page Annots"));
    }
    let mut stream_destinations = BTreeMap::new();
    for (&stream, owners) in &appearance_owners {
        if !index.marked.contains_key(&stream)
            || !owners.iter().any(|owner| moves.contains_key(owner))
        {
            continue;
        }
        if owners.len() != 1 {
            return Err(fail(
                "shared tagged appearance requires occurrence cloning before annotation migration",
            ));
        }
        let owner = *owners.iter().next().unwrap();
        stream_destinations.insert(stream, (owner, moves[&owner]));
    }
    fn relocate(
        value: &PdfObject,
        moves: &BTreeMap<ObjectRef, ObjectRef>,
        streams: &BTreeMap<ObjectRef, (ObjectRef, ObjectRef)>,
        depth: usize,
    ) -> Result<(PdfObject, bool)> {
        if depth > 128 {
            return Err(fail("annotation tag child depth exceeded"));
        }
        match value {
            PdfObject::Array(items) => {
                let mut changed = false;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let (item, did_change) = relocate(item, moves, streams, depth + 1)?;
                    changed |= did_change;
                    out.push(item);
                }
                Ok((PdfObject::Array(out), changed))
            }
            PdfObject::Dictionary(dict) if dict.get_name("Type") == Some("OBJR") => {
                if let Some(page) = dict.get_reference("Obj").and_then(|id| moves.get(&id)) {
                    let mut dict = dict.clone();
                    dict.insert("Pg", reference(*page));
                    Ok((PdfObject::Dictionary(dict), true))
                } else {
                    Ok((value.clone(), false))
                }
            }
            PdfObject::Dictionary(dict)
                if dict.get_name("Type") == Some("MCR") || dict.contains_key("MCID") =>
            {
                let Some((owner, page)) = dict.get_reference("Stm").and_then(|id| streams.get(&id))
                else {
                    if dict
                        .get_reference("StmOwn")
                        .is_some_and(|id| moves.contains_key(&id))
                    {
                        return Err(fail(
                            "moved annotation MCR is not bound to its appearance graph",
                        ));
                    }
                    return Ok((value.clone(), false));
                };
                if dict.get_reference("StmOwn").is_some_and(|id| id != *owner) {
                    return Err(fail(
                        "MCR StmOwn conflicts with annotation appearance ownership",
                    ));
                }
                let mut dict = dict.clone();
                dict.insert("Pg", reference(*page));
                dict.insert("StmOwn", reference(*owner));
                Ok((PdfObject::Dictionary(dict), true))
            }
            _ => Ok((value.clone(), false)),
        }
    }
    let mut updates = Vec::new();
    for &node in &index.nodes {
        crate::cancel::check_current_cancel("annotation structure page migration")?;
        let mut dict = store.dict(node)?;
        if let Some(kids) = dict.get("K") {
            let (kids, changed) = relocate(kids, moves, &stream_destinations, 0)?;
            if changed {
                dict.insert("K", kids);
                updates.push(IncrementalObject {
                    number: node.0,
                    generation: node.1,
                    object: PdfObject::Dictionary(dict),
                });
            }
        }
    }
    // The index normalization is read-only here. Valid source ownership was
    // required above, so only nodes with changed content-item Pg are emitted.
    Ok(updates)
}

pub(crate) fn annotation_transaction_updates(
    input: &[u8],
    moves: &BTreeMap<ObjectRef, ObjectRef>,
    deleted: &BTreeSet<ObjectRef>,
) -> Result<Vec<IncrementalObject>> {
    annotation_deletion::stage(input, moves, deleted)
}

/// Finalize an unpublished candidate after its exact annotation carriers and
/// Annots entries were removed. Rebuild against actual surviving ownership.
pub(crate) fn finalize_annotation_deletion(input: &[u8]) -> Result<Vec<u8>> {
    rebuild_owner_trees(input, None).map(|(output, _)| output)
}

/// Appearance regeneration cannot discard a tagged paint program while leaving
/// its MCRs reachable. This guard covers nested invoked Forms, not only /AP/N.
/// A future semantic regeneration transaction can replace this guard for the
/// cases where it supplies explicit replacement MCID ownership.
pub(crate) fn check_annotation_appearance_replacements(
    input: &[u8],
    annotations: &BTreeSet<ObjectRef>,
) -> Result<()> {
    if annotations.is_empty() {
        return Ok(());
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if !engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        return Ok(());
    }
    let (store, _, index) = index_document(&engine, None)?;
    let mut content = ContentScopes::new(&engine);
    content.collect(&index)?;
    let mut visits = 0;
    for &annotation in annotations {
        let dict = store.dict(annotation)?;
        if let Some(ap) = dict.get("AP") {
            let mut roots = BTreeSet::new();
            appearance_streams(&store, ap, &mut roots, &mut BTreeSet::new(), &mut visits, 0)?;
            for root in roots {
                if content.descendants(root)?.iter().any(|stream| {
                    index.marked.contains_key(stream) || index.objects.contains_key(stream)
                }) {
                    return Err(fail("tagged appearance regeneration requires replacement semantic ownership; preserve its appearance or provide a semantic transaction"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writer::{OutputObject, PdfWriter};

    fn dict(entries: Vec<(&str, PdfObject)>) -> PdfDictionary {
        let mut result = PdfDictionary::empty();
        for (key, value) in entries {
            result.insert(key, value);
        }
        result
    }
    fn name(value: &str) -> PdfObject {
        PdfObject::Name(value.into())
    }
    fn r(number: u32) -> PdfObject {
        reference((number, 0))
    }
    fn array(values: &[i64]) -> PdfObject {
        PdfObject::Array(values.iter().copied().map(PdfObject::Integer).collect())
    }
    fn stream(data: &[u8], mut dictionary: PdfDictionary) -> PdfObject {
        dictionary.insert("Length", PdfObject::Integer(data.len() as i64));
        PdfObject::Stream {
            dict: dictionary,
            raw: data.to_vec(),
        }
    }
    fn element(id: &str, page: u32, kids: PdfObject) -> PdfObject {
        PdfObject::Dictionary(dict(vec![
            ("Type", name("StructElem")),
            ("S", name("P")),
            ("P", r(8)),
            ("Pg", r(page)),
            ("K", kids),
            ("ID", PdfObject::String(id.as_bytes().to_vec())),
        ]))
    }
    // Two pages reuse a broken old parent key. MCID 0 on page 1 and in its
    // invoked Form are different namespaces; page 2 deliberately uses MCID 3.
    // The annotation is a whole-object reference, not an MCID-indexed array.
    fn fixture_objects() -> Vec<OutputObject> {
        let resources = PdfObject::Dictionary(dict(vec![(
            "XObject",
            PdfObject::Dictionary(dict(vec![("Fx", r(11))])),
        )]));
        let objects = vec![
            PdfObject::Dictionary(dict(vec![
                ("Type", name("Catalog")),
                ("Pages", r(2)),
                ("StructTreeRoot", r(7)),
            ])),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("Pages")),
                ("Kids", PdfObject::Array(vec![r(3), r(4)])),
                ("Count", PdfObject::Integer(2)),
            ])),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("Page")),
                ("Parent", r(2)),
                ("MediaBox", array(&[0, 0, 200, 200])),
                ("Resources", resources),
                ("Contents", r(5)),
                ("StructParents", PdfObject::Integer(0)),
            ])),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("Page")),
                ("Parent", r(2)),
                ("MediaBox", array(&[0, 0, 200, 200])),
                ("Resources", PdfObject::Dictionary(PdfDictionary::empty())),
                ("Contents", r(6)),
                ("Annots", PdfObject::Array(vec![r(13)])),
                ("StructParents", PdfObject::Integer(0)),
            ])),
            stream(
                b"/P << /MCID 0 >> BDC 0 0 10 10 re f EMC /Fx Do",
                PdfDictionary::empty(),
            ),
            stream(
                b"/P << /MCID 3 >> BDC 20 20 10 10 re f EMC",
                PdfDictionary::empty(),
            ),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("StructTreeRoot")),
                ("K", r(8)),
                ("ParentTree", r(15)),
                ("ParentTreeNextKey", PdfObject::Integer(1)),
            ])),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("StructElem")),
                ("S", name("Document")),
                ("P", r(7)),
                ("K", PdfObject::Array(vec![r(9), r(10), r(12), r(14)])),
            ])),
            element("first", 3, PdfObject::Integer(0)),
            element("second", 4, PdfObject::Integer(3)),
            stream(
                b"/Span << /MCID 0 /WFReviewed true >> BDC 5 5 2 2 re f EMC",
                dict(vec![
                    ("Type", name("XObject")),
                    ("Subtype", name("Form")),
                    ("BBox", array(&[0, 0, 50, 50])),
                    ("Resources", PdfObject::Dictionary(PdfDictionary::empty())),
                    ("StructParents", PdfObject::Integer(0)),
                ]),
            ),
            element(
                "form",
                3,
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("MCR")),
                    ("Stm", r(11)),
                    ("Pg", r(3)),
                    ("MCID", PdfObject::Integer(0)),
                ])),
            ),
            PdfObject::Dictionary(dict(vec![
                ("Type", name("Annot")),
                ("Subtype", name("Link")),
                ("Rect", array(&[0, 0, 20, 20])),
                ("P", r(4)),
                ("StructParent", PdfObject::Integer(0)),
            ])),
            element(
                "annotation",
                4,
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("OBJR")),
                    ("Obj", r(13)),
                    ("Pg", r(4)),
                ])),
            ),
            PdfObject::Dictionary(dict(vec![(
                "Nums",
                PdfObject::Array(vec![
                    PdfObject::Integer(0),
                    PdfObject::Array(vec![r(9), r(10), r(12), r(14)]),
                ]),
            )])),
        ];
        objects
            .into_iter()
            .enumerate()
            .map(|(i, object)| OutputObject {
                number: i as u32 + 1,
                object,
            })
            .collect()
    }
    fn fixture() -> Vec<u8> {
        PdfWriter::new(fixture_objects(), 1).write().unwrap()
    }
    fn number_key(value: &PdfObject) -> Result<i64> {
        value
            .as_integer()
            .filter(|k| *k >= 0)
            .ok_or_else(|| fail("bad key"))
    }

    #[test]
    fn repairs_namespace_owners_sparse_mcids_and_annotation_object_references() {
        let input = fixture();
        assert!(validate_parent_tree(&input).is_err());
        let (output, report) = rebuild_parent_tree(&input, "en-US").unwrap();
        assert_eq!(report.marked_content_containers, 3);
        assert_eq!(report.marked_content_items, 3);
        assert_eq!(report.object_reference_items, 1);
        assert_eq!(report.parent_tree_entries, 4);
        assert_eq!(report.id_tree_entries, 4);
        assert!(report.output_reopened && report.ownership_verified);
        assert!(!report.conformance_certified);
        let engine = ContentEngine::open_bytes(output.clone()).unwrap();
        let (store, root, index) = index_document(&engine, None).unwrap();
        let ids = structure_ids(&store, &index).unwrap();
        let mut actual = BTreeMap::new();
        read_lookup_tree(
            store.reader,
            store.dict(root).unwrap().get("ParentTree").unwrap(),
            &mut actual,
            &mut LookupReadBudget::default(),
            0,
            "Nums",
            number_key,
        )
        .unwrap();
        let pages = engine.document().get_pages().unwrap();
        let first_key = store
            .dict((pages[0].object_number, pages[0].generation_number))
            .unwrap()
            .get_integer("StructParents")
            .unwrap();
        let second_key = store
            .dict((pages[1].object_number, pages[1].generation_number))
            .unwrap()
            .get_integer("StructParents")
            .unwrap();
        assert_ne!(first_key, second_key);
        assert_eq!(
            actual[&first_key],
            PdfObject::Array(vec![ids[b"first".as_slice()].clone()])
        );
        assert_eq!(
            actual[&second_key],
            PdfObject::Array(vec![
                PdfObject::Null,
                PdfObject::Null,
                PdfObject::Null,
                ids[b"second".as_slice()].clone()
            ])
        );
        let (&object, _) = index.objects.iter().next().unwrap();
        let annotation_key = store
            .dict(object)
            .unwrap()
            .get_integer("StructParent")
            .unwrap();
        assert_eq!(actual[&annotation_key], ids[b"annotation".as_slice()]);
        let (again, _) = rebuild_parent_tree(&output, "en-US").unwrap();
        assert_eq!(validate_parent_tree(&again).unwrap().parent_tree_entries, 4);
        let original = ContentEngine::open_bytes(input).unwrap();
        for (before, after) in original.document().get_pages().unwrap().iter().zip(pages) {
            for (a, b) in before.contents.iter().zip(after.contents) {
                let a = original.document().reader().get_object(a.0, a.1).unwrap();
                let b = engine.document().reader().get_object(b.0, b.1).unwrap();
                assert_eq!(a.as_stream().unwrap().1, b.as_stream().unwrap().1);
            }
        }
    }

    #[test]
    fn promotes_direct_structure_element_and_rebuilds_its_id_mapping() {
        let mut objects = fixture_objects();
        let direct = objects[8].object.clone();
        objects[7]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("K", PdfObject::Array(vec![direct, r(10), r(12), r(14)]));
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (output, report) = rebuild_parent_tree(&input, "en").unwrap();
        assert_eq!(report.materialized_direct_elements, 1);
        assert_eq!(validate_parent_tree(&output).unwrap().id_tree_entries, 4);
    }

    #[test]
    fn duplicate_ids_shared_nodes_and_dangling_mcids_are_rejected() {
        for variant in 0..4 {
            let mut objects = fixture_objects();
            match variant {
                0 => {
                    objects[9]
                        .object
                        .as_dict_mut()
                        .unwrap()
                        .insert("ID", PdfObject::String(b"first".to_vec()));
                }
                1 => {
                    objects[7]
                        .object
                        .as_dict_mut()
                        .unwrap()
                        .insert("K", PdfObject::Array(vec![r(9), r(9), r(10), r(12), r(14)]));
                }
                2 => {
                    objects[8]
                        .object
                        .as_dict_mut()
                        .unwrap()
                        .insert("K", PdfObject::Integer(7));
                }
                _ => {
                    objects[7]
                        .object
                        .as_dict_mut()
                        .unwrap()
                        .insert("K", PdfObject::Array(vec![r(9), r(10), r(12)]));
                }
            }
            let input = PdfWriter::new(objects, 1).write().unwrap();
            assert!(
                rebuild_parent_tree(&input, "en").is_err(),
                "variant {variant}"
            );
        }
    }

    #[test]
    fn lookup_validator_rejects_incorrect_limits_and_child_order() {
        let engine = ContentEngine::open_bytes(fixture()).unwrap();
        let reader = engine.document().reader();
        let wrong_limits = PdfObject::Dictionary(dict(vec![
            (
                "Nums",
                PdfObject::Array(vec![PdfObject::Integer(3), PdfObject::Null]),
            ),
            ("Limits", array(&[0, 3])),
        ]));
        assert!(read_lookup_tree(
            reader,
            &wrong_limits,
            &mut BTreeMap::new(),
            &mut LookupReadBudget::default(),
            0,
            "Nums",
            number_key
        )
        .is_err());
        let mut store = Store::new(reader);
        let values = (0..130).map(|i| (i, PdfObject::Null)).collect();
        let tree =
            lookup_tree(&mut store, &values, "Nums", |key| PdfObject::Integer(*key)).unwrap();
        let mut root = store.dict(tree).unwrap();
        let mut kids = root.get("Kids").unwrap().as_array().unwrap().to_vec();
        kids.reverse();
        root.insert("Kids", PdfObject::Array(kids));
        store.replace_dict(tree, root).unwrap();
        let updates = store
            .updates
            .into_iter()
            .map(|((number, generation), object)| IncrementalObject {
                number,
                generation,
                object,
            })
            .collect();
        let bytes = write_incremental_update(reader, updates).unwrap();
        let reopened = ContentEngine::open_bytes(bytes).unwrap();
        assert!(read_lookup_tree(
            reopened.document().reader(),
            &reference(tree),
            &mut BTreeMap::new(),
            &mut LookupReadBudget::default(),
            0,
            "Nums",
            number_key
        )
        .is_err());
    }

    #[test]
    fn cross_page_annotation_move_updates_objr_and_appearance_mcr_together() {
        let mut objects = fixture_objects();
        let annotation = objects[12].object.as_dict_mut().unwrap();
        annotation.insert("NM", PdfObject::String(b"tagged-link".to_vec()));
        annotation.insert("AP", PdfObject::Dictionary(dict(vec![("N", r(16))])));
        objects.push(OutputObject {
            number: 16,
            object: stream(
                b"/Span << /MCID 5 >> BDC 0 0 20 20 re f EMC",
                dict(vec![
                    ("Type", name("XObject")),
                    ("Subtype", name("Form")),
                    ("BBox", array(&[0, 0, 20, 20])),
                    ("Resources", PdfObject::Dictionary(PdfDictionary::empty())),
                ]),
            ),
        });
        let owner = objects[13].object.as_dict_mut().unwrap();
        let objr = owner.get("K").unwrap().clone();
        owner.insert(
            "K",
            PdfObject::Array(vec![
                objr,
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("MCR")),
                    ("Stm", r(16)),
                    ("StmOwn", r(13)),
                    ("Pg", r(4)),
                    ("MCID", PdfObject::Integer(5)),
                ])),
            ]),
        );
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (input, _) = rebuild_parent_tree(&input, "en").unwrap();
        let (output, report) = crate::annotation_media_redaction::move_resize_annotation_pdf(
            &input,
            "tagged-link",
            1,
            [30.0, 40.0, 50.0, 60.0],
        )
        .unwrap();
        assert_eq!(report.source_page, 2);
        assert_eq!(report.output_page, 1);
        assert_eq!(
            validate_parent_tree(&output).unwrap().parent_tree_entries,
            5
        );
        let engine = ContentEngine::open_bytes(output).unwrap();
        let (store, _, index) = index_document(&engine, None).unwrap();
        let ids = structure_ids(&store, &index).unwrap();
        let owner = store
            .dict(ids[b"annotation".as_slice()].as_reference().unwrap())
            .unwrap();
        let page = engine.document().get_page(1).unwrap();
        let target = (page.object_number, page.generation_number);
        for kid in owner.get("K").unwrap().as_array().unwrap() {
            assert_eq!(kid.as_dict().unwrap().get_reference("Pg"), Some(target));
        }
        let other = store
            .dict(ids[b"second".as_slice()].as_reference().unwrap())
            .unwrap();
        let second = engine.document().get_page(2).unwrap();
        assert_eq!(
            other.get_reference("Pg"),
            Some((second.object_number, second.generation_number))
        );
    }

    #[test]
    fn xfdf_deletion_and_migration_compose_in_one_structure_owner_and_rebuild_parent_tree() {
        use crate::annotation_media_redaction::{
            export_annotation_xfdf, import_annotation_xfdf_pdf, AnnotationAppearancePolicy,
            AnnotationDeletePolicy, AnnotationXfdfImportOptions,
        };
        let mut objects = fixture_objects();
        let annotation = objects[12].object.as_dict_mut().unwrap();
        annotation.insert("NM", PdfObject::String(b"delete-me".to_vec()));
        annotation.insert("AP", PdfObject::Dictionary(dict(vec![("N", r(16))])));
        objects.push(OutputObject {
            number: 16,
            object: stream(
                b"/Span << /MCID 5 >> BDC 0 0 20 20 re f EMC",
                dict(vec![
                    ("Type", name("XObject")),
                    ("Subtype", name("Form")),
                    ("BBox", array(&[0, 0, 20, 20])),
                    ("Resources", PdfObject::Dictionary(PdfDictionary::empty())),
                ]),
            ),
        });
        objects.push(OutputObject {
            number: 17,
            object: PdfObject::Dictionary(dict(vec![
                ("Type", name("Annot")),
                ("Subtype", name("Link")),
                ("NM", PdfObject::String(b"move-me".to_vec())),
                ("P", r(4)),
                ("Rect", array(&[30, 40, 50, 60])),
            ])),
        });
        objects[3]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("Annots", PdfObject::Array(vec![r(13), r(17)]));
        let owner = objects[13].object.as_dict_mut().unwrap();
        let deleted_objr = owner.get("K").unwrap().clone();
        owner.insert(
            "K",
            PdfObject::Array(vec![
                deleted_objr,
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("MCR")),
                    ("Stm", r(16)),
                    ("StmOwn", r(13)),
                    ("Pg", r(4)),
                    ("MCID", PdfObject::Integer(5)),
                ])),
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("OBJR")),
                    ("Obj", r(17)),
                    ("Pg", r(4)),
                ])),
            ]),
        );
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (input, _) = rebuild_parent_tree(&input, "en").unwrap();
        assert_eq!(validate_parent_tree(&input).unwrap().parent_tree_entries, 6);
        let engine = ContentEngine::open_bytes(input.clone()).unwrap();
        let (source_xfdf, _) = export_annotation_xfdf(&engine).unwrap();
        let exported =
            crate::annotation_media_redaction::parse_annotation_xfdf(&source_xfdf).unwrap();
        // External XFDF updates only the surviving annotation; deletion of its
        // sibling has an explicit ID policy. Both edit the same owner's K.
        let xfdf="<xfdf xmlns='http://ns.adobe.com/xfdf/'><annots><link name='move-me' page='0' rect='30,40,50,60'/></annots></xfdf>";
        assert!(exported.annotations.iter().any(|r| r.id == "delete-me"));
        let (output, report) = import_annotation_xfdf_pdf(
            &input,
            xfdf.as_bytes(),
            &AnnotationXfdfImportOptions {
                delete_policy: AnnotationDeletePolicy::ExplicitIds,
                delete_ids: vec!["delete-me".into()],
                appearance_policy: AnnotationAppearancePolicy::PreserveValid,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(report.relationship_transaction.output_graph_verified);
        assert!(report.relationship_transaction.tagged_ownership_rebuilt);
        assert_eq!(
            validate_parent_tree(&output).unwrap().parent_tree_entries,
            4
        );
        let engine = ContentEngine::open_bytes(output).unwrap();
        let (store, _, index) = index_document(&engine, None).unwrap();
        let ids = structure_ids(&store, &index).unwrap();
        let owner = store
            .dict(ids[b"annotation".as_slice()].as_reference().unwrap())
            .unwrap();
        let kids = owner.get("K").unwrap().as_array().unwrap();
        assert_eq!(kids.len(), 1);
        let remaining = kids[0].as_dict().unwrap();
        assert_eq!(remaining.get_name("Type"), Some("OBJR"));
        let first = engine.document().get_page(1).unwrap();
        assert_eq!(
            remaining.get_reference("Pg"),
            Some((first.object_number, first.generation_number))
        );
        let (xfdf, _) = export_annotation_xfdf(&engine).unwrap();
        let after = crate::annotation_media_redaction::parse_annotation_xfdf(&xfdf).unwrap();
        assert!(!after.annotations.iter().any(|r| r.id == "delete-me"));
        assert_eq!(
            after
                .annotations
                .iter()
                .find(|r| r.id == "move-me")
                .unwrap()
                .page,
            1
        );
    }

    #[test]
    fn story_popup_reply_group_moves_both_objr_owners_in_one_transaction() {
        let mut objects = fixture_objects();
        let root = objects[12].object.as_dict_mut().unwrap();
        root.insert("Subtype", name("Text"));
        root.insert("NM", PdfObject::String(b"thread-root".to_vec()));
        root.insert("Popup", r(16));
        objects[3]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("Annots", PdfObject::Array(vec![r(13), r(16), r(17)]));
        objects[7].object.as_dict_mut().unwrap().insert(
            "K",
            PdfObject::Array(vec![r(9), r(10), r(12), r(14), r(18)]),
        );
        objects.push(OutputObject {
            number: 16,
            object: PdfObject::Dictionary(dict(vec![
                ("Type", name("Annot")),
                ("Subtype", name("Popup")),
                ("Parent", r(13)),
                ("P", r(4)),
                ("Rect", array(&[30, 0, 50, 20])),
            ])),
        });
        objects.push(OutputObject {
            number: 17,
            object: PdfObject::Dictionary(dict(vec![
                ("Type", name("Annot")),
                ("Subtype", name("Text")),
                ("IRT", r(13)),
                ("P", r(4)),
                ("Rect", array(&[0, 30, 20, 50])),
            ])),
        });
        objects.push(OutputObject {
            number: 18,
            object: element(
                "reply",
                4,
                PdfObject::Dictionary(dict(vec![
                    ("Type", name("OBJR")),
                    ("Obj", r(17)),
                    ("Pg", r(4)),
                ])),
            ),
        });
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (input, _) = rebuild_parent_tree(&input, "en").unwrap();
        let sources = crate::story_anchors::annotation_anchor_sources(&input).unwrap();
        assert_eq!(sources.len(), 3);
        let moves = sources
            .iter()
            .map(|s| crate::story_anchors::StoryAnnotationMove {
                annotation_id: s.annotation_id.clone(),
                source_page: 2,
                name_change: None,
                target_page: 1,
                old_rect: s.rect,
                new_rect: [
                    s.rect[0] + 30.0,
                    s.rect[1] + 40.0,
                    s.rect[2] + 30.0,
                    s.rect[3] + 40.0,
                ],
            })
            .collect::<Vec<_>>();
        let staged = crate::story_anchors::stage_identities(&input, &input, &moves).unwrap();
        let output = crate::story_anchors::apply_moves(&staged, &moves).unwrap();
        assert_eq!(
            validate_parent_tree(&output).unwrap().parent_tree_entries,
            5
        );
        let engine = ContentEngine::open_bytes(output).unwrap();
        let (store, _, index) = index_document(&engine, None).unwrap();
        let ids = structure_ids(&store, &index).unwrap();
        let page = engine.document().get_page(1).unwrap();
        for id in [b"annotation".as_slice(), b"reply".as_slice()] {
            let owner = store.dict(ids[id].as_reference().unwrap()).unwrap();
            assert_eq!(
                owner
                    .get("K")
                    .unwrap()
                    .as_dict()
                    .unwrap()
                    .get_reference("Pg"),
                Some((page.object_number, page.generation_number))
            );
        }
        let second = engine.document().get_page(2).unwrap();
        let second_page = store
            .dict((second.object_number, second.generation_number))
            .unwrap();
        assert!(second_page
            .get("Annots")
            .is_none_or(|annots| annots.as_array().is_some_and(|items| items.is_empty())));
    }

    #[test]
    fn shared_tagged_appearance_is_not_silently_moved_for_another_occurrence() {
        let mut objects = fixture_objects();
        objects[12].object.as_dict_mut().unwrap().insert("P", r(3));
        objects[13].object.as_dict_mut().unwrap().insert("Pg", r(3));
        objects[13].object.as_dict_mut().unwrap().insert(
            "K",
            PdfObject::Dictionary(dict(vec![
                ("Type", name("OBJR")),
                ("Obj", r(13)),
                ("Pg", r(3)),
            ])),
        );
        objects[12]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("AP", PdfObject::Dictionary(dict(vec![("N", r(11))])));
        // A second annotation shares the tagged Form; it is intentionally not
        // included in the requested move. Occurrence cloning is required.
        let mut annotation = objects[12].object.as_dict().unwrap().clone();
        annotation.remove("StructParent");
        objects.push(OutputObject {
            number: 16,
            object: PdfObject::Dictionary(annotation),
        });
        objects[3].object.as_dict_mut().unwrap().remove("Annots");
        objects[2]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("Annots", PdfObject::Array(vec![r(13), r(16)]));
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (input, _) = rebuild_parent_tree(&input, "en").unwrap();
        let engine = ContentEngine::open_bytes(input.clone()).unwrap();
        let (store, _, index) = index_document(&engine, None).unwrap();
        let (&annotation, _) = index.objects.iter().next().unwrap();
        assert!(store.dict(annotation).unwrap().contains_key("AP"));
        let page = engine.document().get_page(2).unwrap();
        assert!(annotation_page_migration(
            &input,
            &BTreeMap::from([(annotation, (page.object_number, page.generation_number))])
        )
        .is_err());
    }

    #[test]
    fn cancellation_discards_structure_work() {
        let input = fixture();
        let cancel = crate::cancel::CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            cancel.scope(|| rebuild_parent_tree(&input, "en")),
            Err(WellfriendError::Cancelled(_))
        ));
    }

    #[test]
    fn annotation_move_preserves_indirect_annots_array_and_existing_order() {
        let mut objects = fixture_objects();
        objects[12]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("NM", PdfObject::String(b"tagged-link".to_vec()));
        for (number, id) in [(16, "second-paint"), (17, "first-paint")] {
            objects.push(OutputObject {
                number,
                object: PdfObject::Dictionary(dict(vec![
                    ("Type", name("Annot")),
                    ("Subtype", name("Link")),
                    ("NM", PdfObject::String(id.as_bytes().to_vec())),
                    ("P", r(3)),
                    ("Rect", array(&[0, 0, 20, 20])),
                ])),
            });
        }
        objects.push(OutputObject {
            number: 18,
            object: PdfObject::Array(vec![r(17), r(16)]),
        });
        objects[2]
            .object
            .as_dict_mut()
            .unwrap()
            .insert("Annots", r(18));
        let input = PdfWriter::new(objects, 1).write().unwrap();
        let (input, _) = rebuild_parent_tree(&input, "en").unwrap();
        let (output, _) = crate::annotation_media_redaction::move_resize_annotation_pdf(
            &input,
            "tagged-link",
            1,
            [30.0, 40.0, 50.0, 60.0],
        )
        .unwrap();
        let engine = ContentEngine::open_bytes(output).unwrap();
        let reader = engine.document().reader();
        let page = engine.document().get_page(1).unwrap();
        let object = reader
            .get_object(page.object_number, page.generation_number)
            .unwrap();
        let annots = reader
            .resolve(object.as_dict().unwrap().get("Annots").unwrap().clone())
            .unwrap();
        let ids = annots
            .as_array()
            .unwrap()
            .iter()
            .map(|value| {
                let annotation = reader.resolve(value.clone()).unwrap();
                annotation
                    .as_dict()
                    .unwrap()
                    .get("NM")
                    .unwrap()
                    .as_string()
                    .unwrap()
                    .to_vec()
            })
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                b"first-paint".to_vec(),
                b"second-paint".to_vec(),
                b"tagged-link".to_vec()
            ]
        );
        let page = engine.document().get_page(2).unwrap();
        let object = reader
            .get_object(page.object_number, page.generation_number)
            .unwrap();
        assert!(!object.as_dict().unwrap().contains_key("Annots"));
    }
}
