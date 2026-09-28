//! Resolve direct annotation occurrences through their authoritative form/tag
//! owners. Matching requires exact dictionaries and unique page/owner incidence;
//! it is not a visual or name-based guess.
use super::*;

const MAX_VISITS: usize = 100_000;

pub(super) fn reachable_field_nodes(document: &PdfDocument) -> Result<Vec<ReachableFieldNode>> {
    Ok(fields::FieldIndex::read(document)?.snapshot())
}
#[path = "annotation_field_materialization.rs"]
mod fields;
#[path = "annotation_structure_materialization.rs"]
mod structures;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Step {
    Key(String),
    Index(usize),
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Slot {
    owner: Ref,
    path: Vec<Step>,
}
impl Slot {
    fn key(&self, key: &str) -> Self {
        let mut result = self.clone();
        result.path.push(Step::Key(key.into()));
        result
    }
    fn index(&self, index: usize) -> Self {
        let mut result = self.clone();
        result.path.push(Step::Index(index));
        result
    }
}
#[derive(Clone)]
struct Candidate {
    slot: Slot,
    value: PdfObject,
    reference: Option<Ref>,
    dictionary: PdfDictionary,
    field_path: Option<Vec<usize>>,
    direct_parent: bool,
}
pub(super) struct Binding {
    candidates: Vec<Candidate>,
    pub existing: Option<Ref>,
    pub parent_key: Option<i64>,
    field: bool,
    tagged: bool,
}
pub(super) struct Owners<'a> {
    source: &'a PdfDocument,
    current: &'a PdfDocument,
    identities: &'a IdentityIndex,
    fields: Option<fields::FieldIndex>,
    field_plan: fields::Plan,
    direct_counts: Option<BTreeMap<String, usize>>,
    structure: Option<structures::Structure>,
    structure_candidates: BTreeMap<String, Candidate>,
    structure_dependencies: BTreeSet<String>,
    structure_staging: bool,
    used_slots: BTreeSet<Slot>,
    used_refs: BTreeSet<Ref>,
    bound_fields: BTreeSet<Ref>,
    bound_tags: BTreeSet<Ref>,
    budget: usize,
}
fn resolved_name(reader: &PdfReader, dict: &PdfDictionary, key: &str) -> Result<Option<String>> {
    match dict
        .get(key)
        .map(|v| reader.resolve(v.clone()))
        .transpose()?
    {
        None | Some(PdfObject::Null) => Ok(None),
        Some(PdfObject::Name(n)) => Ok(Some(n)),
        _ => Err(fail("invalid annotation owner name")),
    }
}
fn fingerprint(dict: &PdfDictionary, budget: &mut usize) -> Result<String> {
    object_hash(&PdfObject::Dictionary(dict.clone()), budget)
}
impl<'a> Owners<'a> {
    pub fn new(
        source: &'a PdfDocument,
        current: &'a PdfDocument,
        identities: &'a IdentityIndex,
    ) -> Self {
        Self {
            source,
            current,
            identities,
            fields: None,
            field_plan: fields::Plan::default(),
            direct_counts: None,
            structure: None,
            structure_candidates: BTreeMap::new(),
            structure_dependencies: BTreeSet::new(),
            structure_staging: false,
            used_slots: BTreeSet::new(),
            used_refs: BTreeSet::new(),
            bound_fields: BTreeSet::new(),
            bound_tags: BTreeSet::new(),
            budget: 0,
        }
    }
    pub fn expand_fields(&mut self, selected: &BTreeSet<String>) -> Result<BTreeSet<String>> {
        // Do not scan unrelated forms for operations without widget operands.
        let graph = Graph::read(self.source, self.identities)?;
        if !selected
            .iter()
            .any(|id| graph.nodes.get(id).is_some_and(|n| n.subtype == "Widget"))
        {
            return Ok(BTreeSet::new());
        }
        let index = fields::FieldIndex::read(self.source)?;
        if !index.has_direct_fields() {
            self.fields = Some(index);
            return Ok(BTreeSet::new());
        }
        let plan = fields::Plan::prepare(&index, self.source, self.identities, selected)?;
        let dependencies = plan.dependencies.clone();
        self.fields = Some(index);
        self.field_plan = plan;
        Ok(dependencies)
    }
    pub fn needs_fields(&self) -> bool {
        !self.field_plan.is_empty()
    }
    pub fn expand_structures(&mut self, selected: &BTreeSet<String>) -> Result<BTreeSet<String>> {
        let reader = self.source.reader();
        let mut tagged = false;
        for page in self.source.get_pages()? {
            for (order, value) in annots(reader, (page.object_number, page.generation_number))?
                .1
                .into_iter()
                .enumerate()
            {
                crate::cancel::check_current_cancel("tagged annotation owner selection")?;
                if self
                    .identities
                    .get(&(page.page_number, order))
                    .is_none_or(|i| !selected.contains(&i.id))
                {
                    continue;
                }
                let object = reader.resolve(value)?;
                if let Some(dict) = object.as_dict() {
                    tagged |= dict
                        .get("StructParent")
                        .map(|v| reader.resolve(v.clone()))
                        .transpose()?
                        .is_some_and(|v| !matches!(v, PdfObject::Null));
                }
            }
        }
        if !tagged {
            return Ok(BTreeSet::new());
        }
        if self.structure.is_none() {
            self.structure = Some(structures::Structure::read(self.source)?);
        }
        let prepared =
            self.structure
                .as_ref()
                .unwrap()
                .prepare(self.source, self.identities, selected)?;
        self.structure_candidates = prepared.candidates;
        self.structure_dependencies = prepared.dependencies;
        self.structure_staging = prepared.needs_staging;
        Ok(self.structure_dependencies.clone())
    }
    pub fn needs_structures(&self) -> bool {
        self.structure_staging
    }
    pub fn structure_dependencies(&self) -> &BTreeSet<String> {
        &self.structure_dependencies
    }
    pub fn install_existing(
        &mut self,
        id: &str,
        output: Ref,
        objects: &mut BTreeMap<Ref, PdfObject>,
    ) -> Result<Option<i64>> {
        let Some(candidate) = self.structure_candidates.get(id).cloned() else {
            return Ok(None);
        };
        if candidate.reference.is_some_and(|r| r != output) {
            return Err(fail(
                "indirect annotation and OBJR refer to competing objects",
            ));
        }
        if !self.used_slots.insert(candidate.slot.clone()) {
            return Err(fail("two annotations claim the same structural carrier"));
        }
        let key = candidate
            .dictionary
            .get("StructParent")
            .map(|v| self.source.reader().resolve(v.clone()))
            .transpose()?
            .and_then(|v| v.as_integer());
        let binding = Binding {
            candidates: vec![candidate],
            existing: Some(output),
            parent_key: key,
            field: false,
            tagged: true,
        };
        self.install(&binding, output, objects)?;
        Ok(key)
    }
    pub fn finish_structures(
        &mut self,
        objects: &mut BTreeMap<Ref, PdfObject>,
        next: &mut u32,
    ) -> Result<(usize, usize, usize)> {
        let Some(structure) = &self.structure else {
            return Ok((0, 0, 0));
        };
        let result = structure.apply(self.current, objects, next)?;
        Ok((result.nodes, result.parents, result.lookup_values))
    }
    pub fn dependent_widgets(&self) -> &BTreeSet<String> {
        &self.field_plan.dependencies
    }
    pub fn finish_fields(
        &mut self,
        objects: &mut BTreeMap<Ref, PdfObject>,
        next: &mut u32,
        annotation_refs: &BTreeMap<String, Ref>,
    ) -> Result<(usize, usize)> {
        if self.field_plan.is_empty() {
            return Ok((0, 0));
        }
        let index = self
            .fields
            .as_ref()
            .ok_or_else(|| fail("field materialization inventory missing"))?;
        let result = self
            .field_plan
            .apply(index, self.current, objects, next, annotation_refs)?;
        self.bound_fields.extend(result.widgets);
        self.bound_tags.extend(result.tagged);
        Ok((result.ancestors, result.parents))
    }
    pub fn bind(&mut self, id: &str, dict: &PdfDictionary) -> Result<Binding> {
        let reader = self.source.reader();
        let field = resolved_name(reader, dict, "Subtype")?.as_deref() == Some("Widget");
        let parent_key = match dict
            .get("StructParent")
            .map(|v| reader.resolve(v.clone()))
            .transpose()?
        {
            None | Some(PdfObject::Null) => None,
            Some(PdfObject::Integer(key)) if key >= 0 => Some(key),
            _ => return Err(fail("invalid annotation StructParent key")),
        };
        let mut binding = Binding {
            candidates: Vec::new(),
            existing: None,
            parent_key,
            field,
            tagged: parent_key.is_some(),
        };
        if !field && parent_key.is_none() {
            return Ok(binding);
        }
        let hash = fingerprint(dict, &mut self.budget)?;
        if self.direct_counts.is_none() {
            let mut counts = BTreeMap::<String, usize>::new();
            for page in self.source.get_pages()? {
                for value in annots(reader, (page.object_number, page.generation_number))?.1 {
                    crate::cancel::check_current_cancel("annotation direct-owner incidence")?;
                    if let PdfObject::Dictionary(d) = value {
                        *counts
                            .entry(fingerprint(&d, &mut self.budget)?)
                            .or_default() += 1;
                    }
                }
            }
            self.direct_counts = Some(counts);
        }
        if self
            .direct_counts
            .as_ref()
            .and_then(|c| c.get(&hash))
            .copied()
            != Some(1)
        {
            return Err(fail("direct annotation has indistinguishable page occurrences; explicit owner mapping is required"));
        }
        if field {
            if self.fields.is_none() {
                self.fields = Some(fields::FieldIndex::read(self.source)?);
            }
            let matches = self
                .fields
                .as_ref()
                .and_then(|m| m.widgets.get(&hash))
                .map(Vec::as_slice)
                .unwrap_or_default();
            if matches.len() != 1 || matches[0].dictionary != *dict {
                return Err(fail("direct widget needs one exact reachable field owner; explicit owner mapping is required"));
            }
            binding.candidates.push(matches[0].clone());
        }
        if parent_key.is_some() {
            let candidate = self
                .structure_candidates
                .get(id)
                .ok_or_else(|| fail("tagged source has no prepared structure carrier"))?;
            if candidate.dictionary != *dict {
                return Err(fail("prepared structure carrier dictionary differs"));
            }
            binding.candidates.push(candidate.clone());
        }
        for candidate in &binding.candidates {
            if !self.used_slots.insert(candidate.slot.clone()) {
                return Err(fail(
                    "multiple annotation occurrences claim the same owner slot",
                ));
            }
            if let Some(r) = candidate.reference {
                if binding.existing.is_some_and(|previous| previous != r) {
                    return Err(fail("field and structure trees disagree on the annotation object; explicit owner mapping is required"));
                }
                binding.existing = Some(r);
            }
        }
        if let Some(r) = binding.existing {
            if self.identities.values().any(|i| i.reference == Some(r)) || !self.used_refs.insert(r)
            {
                return Err(fail(
                    "annotation owner object already belongs to a page occurrence",
                ));
            }
            // Compare the source-owned shadow object too, not only the copied
            // page dictionary. Unpublished image staging must not alter it.
            if self.current.reader().get_object(r.0, r.1)?.as_dict() != Some(dict) {
                return Err(fail(
                    "annotation authoritative owner changed before promotion",
                ));
            }
        }
        Ok(binding)
    }
    pub fn install(
        &mut self,
        binding: &Binding,
        output: Ref,
        objects: &mut BTreeMap<Ref, PdfObject>,
    ) -> Result<()> {
        for candidate in &binding.candidates {
            crate::cancel::check_current_cancel("annotation owner slot staging")?;
            if candidate
                .field_path
                .as_ref()
                .is_some_and(|p| self.field_plan.contains(p))
            {
                continue; // Rewritten once with its materialized parent/ancestors.
            }
            let root = objects
                .get(&candidate.slot.owner)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| {
                    self.current
                        .reader()
                        .get_object(candidate.slot.owner.0, candidate.slot.owner.1)
                })?;
            let rewritten = replace_slot(
                self.current.reader(),
                &root,
                &candidate.slot.path,
                &candidate.value,
                &reference(output),
                0,
            )?;
            if rewritten != root {
                objects.insert(candidate.slot.owner, rewritten);
            }
        }
        if binding.field {
            self.bound_fields.insert(output);
        }
        if binding.tagged {
            self.bound_tags.insert(output);
        }
        Ok(())
    }
    pub fn verify(&self, output: &[u8]) -> Result<(bool, bool)> {
        if !self.bound_fields.is_empty() {
            let document = PdfDocument::open_bytes(output.to_vec())?;
            let inventory = fields::FieldIndex::read(&document)?;
            let mut occurrences = BTreeMap::<Ref, usize>::new();
            for candidate in inventory.widgets.values().flatten() {
                if let Some(r) = candidate.reference {
                    if self.bound_fields.contains(&r) && candidate.direct_parent {
                        return Err(fail("promoted widget still has a direct field parent"));
                    }
                    *occurrences.entry(r).or_default() += 1;
                }
            }
            for r in &self.bound_fields {
                if occurrences.get(r) != Some(&1) {
                    return Err(fail(
                        "promoted widget lost unique reachable field ownership",
                    ));
                }
            }
        }
        if !self.bound_tags.is_empty() {
            crate::tagged_structure::validate_parent_tree(output)?;
        }
        Ok((!self.bound_fields.is_empty(), !self.bound_tags.is_empty()))
    }
}

/// A container reference is copied into its owning dictionary, not rewritten
/// globally: shared /Kids arrays must not mutate an unrelated field/owner.
fn replace_slot(
    reader: &PdfReader,
    root: &PdfObject,
    path: &[Step],
    expected: &PdfObject,
    replacement: &PdfObject,
    depth: usize,
) -> Result<PdfObject> {
    if depth > 128 {
        return Err(fail("annotation owner patch depth exceeded"));
    }
    if path.is_empty() {
        if root != expected {
            return Err(fail("annotation owner slot changed before promotion"));
        }
        return Ok(replacement.clone());
    }
    let resolved = reader.resolve(root.clone())?;
    match (&path[0], resolved) {
        (Step::Key(key), PdfObject::Dictionary(mut dict)) => {
            let child = dict
                .get(key)
                .ok_or_else(|| fail("annotation owner key disappeared"))?;
            let value = replace_slot(reader, child, &path[1..], expected, replacement, depth + 1)?;
            if &value == child {
                return Ok(root.clone());
            }
            dict.insert(key.clone(), value);
            Ok(PdfObject::Dictionary(dict))
        }
        (Step::Index(index), PdfObject::Array(mut values)) => {
            let child = values
                .get(*index)
                .ok_or_else(|| fail("annotation owner index disappeared"))?;
            let value = replace_slot(reader, child, &path[1..], expected, replacement, depth + 1)?;
            if &value == child {
                return Ok(root.clone());
            }
            values[*index] = value;
            Ok(PdfObject::Array(values))
        }
        _ => Err(fail("annotation owner path changed kind")),
    }
}

fn dictionary_at(
    reader: &PdfReader,
    value: &PdfObject,
    slot: &Slot,
) -> Result<(PdfDictionary, Option<Ref>, Slot)> {
    // Indirect dictionary owners retain identity. Arrays are copied on write.
    let object = reader.resolve(value.clone())?;
    let dict = object
        .as_dict()
        .ok_or_else(|| fail("invalid annotation owner dictionary"))?
        .clone();
    let id = value.as_reference();
    let context = id
        .map(|owner| Slot {
            owner,
            path: Vec::new(),
        })
        .unwrap_or_else(|| slot.clone());
    Ok((dict, id, context))
}
