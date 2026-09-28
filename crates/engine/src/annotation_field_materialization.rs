//! Plan direct field ancestors before annotation allocation. /Fields and /Kids
//! define ownership; missing Parent below a direct owner can be reconstructed,
//! but a contradictory explicit Parent is never silently overridden.
use super::*;
type Path = Vec<usize>;
#[derive(Clone)]
pub(super) struct FieldNode {
    slot: Slot,
    value: PdfObject,
    reference: Option<Ref>,
    dict: PdfDictionary,
    children: usize,
    widget: bool,
}
pub(super) struct FieldIndex {
    pub widgets: BTreeMap<String, Vec<Candidate>>,
    nodes: BTreeMap<Path, FieldNode>,
}
impl FieldIndex {
    pub fn snapshot(&self) -> Vec<ReachableFieldNode> {
        self.nodes
            .iter()
            .map(|(path, node)| ReachableFieldNode {
                path: path.clone(),
                reference: node.reference,
                dictionary: node.dict.clone(),
                widget: node.widget,
            })
            .collect()
    }
    pub fn has_direct_fields(&self) -> bool {
        self.nodes.values().any(|n| n.reference.is_none())
    }
    pub fn read(document: &PdfDocument) -> Result<Self> {
        let reader = document.reader();
        let catalog = document.get_catalog()?;
        let mut result = Self {
            widgets: BTreeMap::new(),
            nodes: BTreeMap::new(),
        };
        let Some(form) = catalog.get("AcroForm") else {
            return Ok(result);
        };
        if matches!(reader.resolve(form.clone())?, PdfObject::Null) {
            return Ok(result);
        }
        let root = Slot {
            owner: reader
                .root_reference()
                .ok_or_else(|| fail("missing catalog"))?,
            path: vec![Step::Key("AcroForm".into())],
        };
        let (form, _, context) = dictionary_at(reader, form, &root)?;
        if let Some(fields) = form.get("Fields") {
            let mut walk = Walk {
                reader,
                seen: BTreeSet::new(),
                visits: 0,
                bytes: 0,
                index: &mut result,
            };
            walk.array(fields, &context.key("Fields"), &[], Parent::Root, 0)?;
        }
        Ok(result)
    }
}
#[derive(Clone, Copy)]
enum Parent {
    Root,
    Indirect(Ref),
    Direct,
}
struct Walk<'a, 'b> {
    reader: &'a PdfReader,
    seen: BTreeSet<Ref>,
    visits: usize,
    bytes: usize,
    index: &'b mut FieldIndex,
}
impl Walk<'_, '_> {
    fn array(
        &mut self,
        value: &PdfObject,
        slot: &Slot,
        prefix: &[usize],
        parent: Parent,
        depth: usize,
    ) -> Result<()> {
        if depth > 128 {
            return Err(fail("annotation field-owner depth exceeded"));
        }
        let object = self.reader.resolve(value.clone())?;
        let entries = object
            .as_array()
            .ok_or_else(|| fail("invalid field-owner array"))?;
        for (index, value) in entries.iter().enumerate() {
            crate::cancel::check_current_cancel("field ancestor inventory")?;
            self.visits += 1;
            if self.visits > MAX_VISITS {
                return Err(fail("annotation field-owner budget exceeded"));
            }
            let incoming = slot.index(index);
            let (dict, id, context) = dictionary_at(self.reader, value, &incoming)?;
            if id.is_some_and(|r| !self.seen.insert(r)) {
                return Err(fail("shared/cyclic form field owner"));
            }
            let actual = dict
                .get("Parent")
                .map(|v| self.reader.resolve(v.clone()))
                .transpose()?;
            let explicit = actual
                .as_ref()
                .is_some_and(|v| !matches!(v, PdfObject::Null));
            let parent_ref = dict.get_reference("Parent");
            let valid = match parent {
                Parent::Root | Parent::Direct => !explicit,
                Parent::Indirect(r) => explicit && parent_ref == Some(r),
            };
            if !valid {
                return Err(fail(
                    "field owner Parent disagrees with its reachable tree position",
                ));
            }
            let mut path = prefix.to_vec();
            path.push(index);
            let widget = resolved_name(self.reader, &dict, "Subtype")?.as_deref() == Some("Widget");
            let kids = dict
                .get("Kids")
                .map(|v| self.reader.resolve(v.clone()))
                .transpose()?;
            let children = match kids {
                None | Some(PdfObject::Null) => 0,
                Some(PdfObject::Array(ref values)) => values.len(),
                _ => return Err(fail("invalid field Kids")),
            };
            if widget && children != 0 {
                return Err(fail("merged/widget owner cannot contain a Kids array"));
            }
            let hash = fingerprint(&dict, &mut self.bytes)?;
            if widget {
                self.index.widgets.entry(hash).or_default().push(Candidate {
                    slot: incoming.clone(),
                    value: value.clone(),
                    reference: id,
                    dictionary: dict.clone(),
                    field_path: Some(path.clone()),
                    direct_parent: matches!(parent, Parent::Direct),
                });
            }
            self.index.nodes.insert(
                path.clone(),
                FieldNode {
                    slot: incoming,
                    value: value.clone(),
                    reference: id,
                    dict: dict.clone(),
                    children,
                    widget,
                },
            );
            if children != 0 {
                self.array(
                    dict.get("Kids").unwrap(),
                    &context.key("Kids"),
                    &path,
                    id.map(Parent::Indirect).unwrap_or(Parent::Direct),
                    depth + 1,
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Plan {
    affected: BTreeSet<Path>,
    aliases: BTreeMap<Path, String>,
    pub dependencies: BTreeSet<String>,
}
pub(super) struct Applied {
    pub ancestors: usize,
    pub parents: usize,
    pub widgets: BTreeSet<Ref>,
    pub tagged: BTreeSet<Ref>,
}
struct PageWidget {
    id: String,
    reference: Option<Ref>,
    dict: PdfDictionary,
}
impl Plan {
    pub fn prepare(
        index: &FieldIndex,
        document: &PdfDocument,
        ids: &IdentityIndex,
        selected: &BTreeSet<String>,
    ) -> Result<Self> {
        let reader = document.reader();
        let mut page_widgets = BTreeMap::<String, Vec<PageWidget>>::new();
        let mut budget = 0usize;
        for page in document.get_pages()? {
            for (order, value) in annots(reader, (page.object_number, page.generation_number))?
                .1
                .into_iter()
                .enumerate()
            {
                crate::cancel::check_current_cancel("field widget source incidence")?;
                let object = reader.resolve(value)?;
                let Some(dict) = object.as_dict() else {
                    continue;
                };
                if resolved_name(reader, dict, "Subtype")?.as_deref() != Some("Widget") {
                    continue;
                }
                let identity = &ids[&(page.page_number, order)];
                let hash = fingerprint(dict, &mut budget)?;
                page_widgets.entry(hash).or_default().push(PageWidget {
                    id: identity.id.clone(),
                    reference: identity.reference,
                    dict: dict.clone(),
                });
            }
        }
        let mut result = Self::default();
        for (hash, sources) in &page_widgets {
            for source in sources.iter().filter(|s| selected.contains(&s.id)) {
                let candidates = index
                    .widgets
                    .get(hash)
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                if candidates.len() != 1 || candidates[0].dictionary != source.dict {
                    return Err(fail("selected widget needs one exact reachable field owner; explicit owner mapping is required"));
                }
                let candidate = &candidates[0];
                if source.reference.is_some()
                    && candidate.reference.is_some()
                    && source.reference != candidate.reference
                {
                    return Err(fail(
                        "page and field tree refer to different widget objects",
                    ));
                }
                let path = candidate.field_path.as_ref().unwrap();
                // A direct field copy of an already indirect page widget must
                // be connected to that object even without a direct ancestor.
                if candidate.reference.is_none() && source.reference.is_some() {
                    result.affected.insert(path.clone());
                }
                for depth in 1..path.len() {
                    let ancestor = path[..depth].to_vec();
                    if index.nodes[&ancestor].reference.is_none() {
                        result.affected.insert(ancestor);
                    }
                }
            }
        }
        // Materializing a direct field changes each immediate child's Parent.
        // Recurse only through direct children; an existing indirect descendant
        // remains the stable owner of its otherwise unaffected subtree.
        let mut pending = result.affected.iter().cloned().collect::<Vec<_>>();
        while let Some(path) = pending.pop() {
            crate::cancel::check_current_cancel("direct field dependency closure")?;
            let node = &index.nodes[&path];
            if node.reference.is_some() {
                continue;
            }
            for child in 0..node.children {
                let mut child_path = path.clone();
                child_path.push(child);
                if result.affected.insert(child_path.clone()) {
                    pending.push(child_path);
                }
            }
        }
        for path in &result.affected {
            let node = &index.nodes[path];
            if !node.widget {
                continue;
            }
            let hash = fingerprint(&node.dict, &mut budget)?;
            let sources = page_widgets
                .get(&hash)
                .map(Vec::as_slice)
                .unwrap_or_default();
            let matches = sources
                .iter()
                .filter(|s| {
                    node.reference.is_none()
                        || s.reference == node.reference
                        || s.reference.is_none()
                })
                .collect::<Vec<_>>();
            if matches.len() > 1 {
                return Err(fail("field normalization has ambiguous page widget copies; explicit owner mapping is required"));
            }
            if let Some(source) = matches.first() {
                if source.dict != node.dict {
                    return Err(fail("field normalization dictionary fingerprint collision"));
                }
                result.aliases.insert(path.clone(), source.id.clone());
                result.dependencies.insert(source.id.clone());
            } else if node.reference.is_none()
                && node
                    .dict
                    .get("StructParent")
                    .map(|v| reader.resolve(v.clone()))
                    .transpose()?
                    .is_some_and(|v| !matches!(v, PdfObject::Null))
            {
                return Err(fail(
                    "hidden direct tagged widget requires explicit structure-owner materialization",
                ));
            }
        }
        Ok(result)
    }
    pub fn is_empty(&self) -> bool {
        self.affected.is_empty()
    }
    pub fn contains(&self, path: &Path) -> bool {
        self.affected.contains(path)
    }
    pub fn apply(
        &self,
        index: &FieldIndex,
        current: &PdfDocument,
        objects: &mut BTreeMap<Ref, PdfObject>,
        next: &mut u32,
        annotation_refs: &BTreeMap<String, Ref>,
    ) -> Result<Applied> {
        let reader = current.reader();
        let mut allocations = BTreeMap::<Path, Ref>::new();
        let mut applied = Applied {
            ancestors: 0,
            parents: 0,
            widgets: BTreeSet::new(),
            tagged: BTreeSet::new(),
        };
        for path in &self.affected {
            crate::cancel::check_current_cancel("direct field source validation")?;
            let node = &index.nodes[path];
            let actual = read_slot(
                reader,
                &reader.get_object(node.slot.owner.0, node.slot.owner.1)?,
                &node.slot.path,
            )?;
            if actual != node.value || reader.resolve(actual)?.as_dict() != Some(&node.dict) {
                return Err(fail("field ancestor changed before materialization"));
            }
            let id = if let Some(alias) = self.aliases.get(path) {
                let id = *annotation_refs
                    .get(alias)
                    .ok_or_else(|| fail("dependent widget was not staged"))?;
                if node.reference.is_some_and(|r| r != id) {
                    return Err(fail("dependent field/widget object allocation disagrees"));
                }
                id
            } else if let Some(id) = node.reference {
                id
            } else {
                *next = next
                    .checked_add(1)
                    .ok_or_else(|| fail("field object number exhausted"))?;
                (*next, 0)
            };
            if node.reference.is_none() && !node.widget {
                applied.ancestors += 1;
            }
            allocations.insert(path.clone(), id);
        }
        let mut order = self.affected.iter().collect::<Vec<_>>();
        order.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        for path in order {
            crate::cancel::check_current_cancel("field ancestor object staging")?;
            let node = &index.nodes[path];
            let id = allocations[path];
            let mut dict = objects
                .get(&id)
                .and_then(PdfObject::as_dict)
                .cloned()
                .unwrap_or_else(|| node.dict.clone());
            if !node.widget
                && node.reference.is_none()
                && dict
                    .get("StructParent")
                    .map(|v| reader.resolve(v.clone()))
                    .transpose()?
                    .is_some_and(|v| !matches!(v, PdfObject::Null))
            {
                return Err(fail("direct non-widget field has structural ownership requiring explicit materialization"));
            }
            let parent = if path.len() == 1 {
                None
            } else {
                let parent_path = path[..path.len() - 1].to_vec();
                Some(
                    allocations
                        .get(&parent_path)
                        .copied()
                        .or(index.nodes[&parent_path].reference)
                        .ok_or_else(|| {
                            fail("direct field ancestor missing from dependency closure")
                        })?,
                )
            };
            let old = dict.get("Parent").cloned();
            match parent {
                Some(r) => {
                    dict.insert("Parent", reference(r));
                }
                None => {
                    dict.remove("Parent");
                }
            }
            if dict.get("Parent") != old.as_ref() {
                applied.parents += 1;
            }
            if node.children != 0 {
                let object = reader.resolve(
                    dict.get("Kids")
                        .cloned()
                        .ok_or_else(|| fail("field children disappeared"))?,
                )?;
                let mut kids = object
                    .as_array()
                    .ok_or_else(|| fail("invalid staged field children"))?
                    .to_vec();
                if kids.len() != node.children {
                    return Err(fail("field child ordering changed before materialization"));
                }
                for (index, child) in kids.iter_mut().enumerate() {
                    let mut child_path = path.clone();
                    child_path.push(index);
                    if let Some(r) = allocations.get(&child_path) {
                        *child = reference(*r);
                    }
                }
                if object.as_array() != Some(kids.as_slice()) {
                    dict.insert("Kids", PdfObject::Array(kids));
                }
            }
            if node.widget {
                applied.widgets.insert(id);
                if dict
                    .get("StructParent")
                    .map(|v| reader.resolve(v.clone()))
                    .transpose()?
                    .is_some_and(|v| !matches!(v, PdfObject::Null))
                {
                    applied.tagged.insert(id);
                }
            }
            objects.insert(id, PdfObject::Dictionary(dict));
            let parent_path = path[..path.len() - 1].to_vec();
            if !self.affected.contains(&parent_path) {
                let root = objects
                    .get(&node.slot.owner)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| reader.get_object(node.slot.owner.0, node.slot.owner.1))?;
                let rewritten = replace_slot(
                    reader,
                    &root,
                    &node.slot.path,
                    &node.value,
                    &reference(id),
                    0,
                )?;
                if rewritten != root {
                    objects.insert(node.slot.owner, rewritten);
                }
            }
        }
        Ok(applied)
    }
}
fn read_slot(reader: &PdfReader, root: &PdfObject, path: &[Step]) -> Result<PdfObject> {
    let mut value = root.clone();
    for step in path {
        crate::cancel::check_current_cancel("field source path verification")?;
        let object = reader.resolve(value)?;
        value = match (step, object) {
            (Step::Key(key), PdfObject::Dictionary(dict)) => dict
                .get(key)
                .cloned()
                .ok_or_else(|| fail("field source key disappeared"))?,
            (Step::Index(i), PdfObject::Array(items)) => items
                .get(*i)
                .cloned()
                .ok_or_else(|| fail("field source index disappeared"))?,
            _ => return Err(fail("field source path changed kind")),
        };
    }
    Ok(value)
}
