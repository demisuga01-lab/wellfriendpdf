//! Strict identity normalization of the existing structure tree. No role,
//! reading order, MCID, lookup key or alternate-text inference is performed.
use super::*;

struct Node {
    slot: Slot,
    context: Slot,
    value: PdfObject,
    dict: PdfDictionary,
    reference: Option<Ref>,
    parent: Option<usize>,
}
struct Carrier {
    owner: usize,
    slot: Slot,
    value: PdfObject,
}
pub(super) struct Structure {
    nodes: Vec<Node>,
    slots: BTreeMap<Slot, usize>,
    refs: BTreeMap<Ref, usize>,
    hashes: BTreeMap<String, Vec<usize>>,
    aliases: BTreeMap<usize, Ref>,
    alias_nodes: BTreeMap<Ref, usize>,
    lookup_sources: BTreeMap<Ref, PdfObject>,
    lookup_bytes: std::cell::Cell<usize>,
    parents: BTreeMap<i64, PdfObject>,
    carriers: Vec<Carrier>,
    pub materialize: bool,
    bytes: usize,
    visits: usize,
}
pub(super) struct Prepared {
    pub dependencies: BTreeSet<String>,
    pub candidates: BTreeMap<String, Candidate>,
    pub needs_staging: bool,
}
pub(super) struct Applied {
    pub nodes: usize,
    pub parents: usize,
    pub lookup_values: usize,
}
fn member(reader: &PdfReader, dict: &PdfDictionary, key: &str) -> Result<Option<PdfObject>> {
    match dict
        .get(key)
        .map(|v| reader.resolve(v.clone()))
        .transpose()?
    {
        None | Some(PdfObject::Null) => Ok(None),
        value => Ok(value),
    }
}
fn indirect_names(reader: &PdfReader, dict: &PdfDictionary, keys: &[&str]) -> Result<bool> {
    for &key in keys {
        if dict.get(key).is_some_and(|v| v.as_reference().is_some())
            && resolved_name(reader, dict, key)?.is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn normalize_names(reader: &PdfReader, dict: &mut PdfDictionary, keys: &[&str]) -> Result<()> {
    for &key in keys {
        if let Some(name) = resolved_name(reader, dict, key)? {
            if dict.get_name(key) != Some(name.as_str()) {
                dict.insert(key, PdfObject::Name(name));
            }
        }
    }
    Ok(())
}
impl Structure {
    pub fn read(document: &PdfDocument) -> Result<Self> {
        let reader = document.reader();
        let catalog = document.get_catalog()?;
        let value = catalog
            .get("StructTreeRoot")
            .cloned()
            .ok_or_else(|| fail("tagged annotation has no StructTreeRoot"))?;
        let slot = Slot {
            owner: reader
                .root_reference()
                .ok_or_else(|| fail("missing catalog"))?,
            path: vec![Step::Key("StructTreeRoot".into())],
        };
        let (dict, reference, context) = dictionary_at(reader, &value, &slot)?;
        if resolved_name(reader, &dict, "Type")?.as_deref() != Some("StructTreeRoot") {
            return Err(fail("invalid structure root type"));
        }
        let materialize = reference.is_none() || indirect_names(reader, &dict, &["Type"])?;
        let mut result = Self {
            nodes: vec![Node {
                slot: slot.clone(),
                context,
                value,
                dict,
                reference,
                parent: None,
            }],
            slots: BTreeMap::from([(slot, 0)]),
            refs: BTreeMap::new(),
            hashes: BTreeMap::new(),
            aliases: BTreeMap::new(),
            alias_nodes: BTreeMap::new(),
            lookup_sources: BTreeMap::new(),
            lookup_bytes: std::cell::Cell::new(0),
            parents: crate::tagged_structure::annotation_parent_entries(document)?,
            carriers: Vec::new(),
            materialize,
            bytes: 0,
            visits: 0,
        };
        if let Some(r) = reference {
            if reader.get_object(r.0, r.1)?.as_dict().is_none() {
                return Err(fail(
                    "structure root reference chains require explicit normalization",
                ));
            }
            result.refs.insert(r, 0);
        }
        if let Some(k) = result.nodes[0].dict.get("K").cloned() {
            let context = result.nodes[0].context.key("K");
            result.kids(reader, &k, &context, 0, 0, &mut BTreeSet::new())?;
        }
        for index in 0..result.nodes.len() {
            for key in ["Type", "S"] {
                if let Some(value) = result.nodes[index].dict.get(key).cloned() {
                    result.snapshot_lookup(reader, &value)?;
                }
            }
        }
        for (index, node) in result.nodes.iter().enumerate() {
            let hash = fingerprint(&node.dict, &mut result.bytes)?;
            result.hashes.entry(hash).or_default().push(index);
        }
        result.lookup_bytes.set(result.bytes);
        // Collect all standard incoming ownership aliases before choosing any
        // new object numbers. Competing indirect shadows are not guessed away.
        let root = result.nodes[0].dict.clone();
        if let Some(tree) = root.get("ParentTree") {
            result.lookup_aliases(reader, tree, "Nums", 0, &mut BTreeSet::new())?;
        }
        if let Some(tree) = root.get("IDTree") {
            result.lookup_aliases(reader, tree, "Names", 0, &mut BTreeSet::new())?;
        }
        for index in 1..result.nodes.len() {
            let node = &result.nodes[index];
            let expected = node.parent.unwrap();
            let parent = node.dict.get("P").cloned();
            let refs = node.dict.get("Ref").cloned();
            match parent {
                Some(value) if !matches!(reader.resolve(value.clone())?, PdfObject::Null) => {
                    if result.locate(reader, &value)? != expected {
                        return Err(fail("structure P contradicts its reachable tree parent"));
                    }
                    result.record_alias(expected, &value)?;
                }
                _ if result.nodes[index].reference.is_none()
                    || result.nodes[expected].reference.is_none() =>
                {
                    result.materialize = true;
                }
                _ => return Err(fail(
                    "indirect structure element has a missing parent; explicit repair is required",
                )),
            }
            if let Some(value) = refs {
                let value = reader.resolve(value)?;
                for item in value
                    .as_array()
                    .ok_or_else(|| fail("structure Ref must be an array"))?
                {
                    if reader.resolve(item.clone())?.as_dict().is_none() {
                        return Err(fail("structure Ref requires non-null element owners"));
                    }
                    result.collect_value(reader, item, false, 0)?;
                }
            }
        }
        if result
            .lookup_sources
            .keys()
            .any(|r| result.refs.contains_key(r) || result.alias_nodes.contains_key(r))
        {
            return Err(fail(
                "a lookup node and a structure owner cannot share an identity",
            ));
        }
        Ok(result)
    }
    fn kids(
        &mut self,
        reader: &PdfReader,
        value: &PdfObject,
        slot: &Slot,
        parent: usize,
        depth: usize,
        active: &mut BTreeSet<Ref>,
    ) -> Result<()> {
        crate::cancel::check_current_cancel("structure occurrence inventory")?;
        self.visits += 1;
        if self.visits > MAX_VISITS || depth > 128 {
            return Err(fail("structure materialization budget exceeded"));
        }
        let id = value.as_reference();
        if id.is_some_and(|r| !active.insert(r)) {
            return Err(fail("cyclic structure content"));
        }
        let object = reader.resolve(value.clone())?;
        match object {
            PdfObject::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    self.kids(reader, item, &slot.index(i), parent, depth + 1, active)?;
                }
            }
            PdfObject::Dictionary(dict) => {
                if let Some(r) = id {
                    if reader.get_object(r.0, r.1)?.as_dict().is_none() {
                        return Err(fail(
                            "structure dictionary reference chains require explicit normalization",
                        ));
                    }
                }
                let kind = resolved_name(reader, &dict, "Type")?;
                self.materialize |= indirect_names(reader, &dict, &["Type"])?;
                if let Some(value) = dict.get("Type") {
                    self.snapshot_lookup(reader, value)?;
                }
                let context = id
                    .map(|owner| Slot {
                        owner,
                        path: Vec::new(),
                    })
                    .unwrap_or_else(|| slot.clone());
                if kind.as_deref() == Some("OBJR") {
                    let target = dict
                        .get("Obj")
                        .cloned()
                        .ok_or_else(|| fail("OBJR missing Obj"))?;
                    self.carriers.push(Carrier {
                        owner: parent,
                        slot: context.key("Obj"),
                        value: target,
                    });
                } else if kind.as_deref() != Some("MCR")
                    && !(kind.is_none() && !dict.contains_key("S") && dict.contains_key("MCID"))
                {
                    self.materialize |= indirect_names(reader, &dict, &["S"])?;
                    if kind.as_ref().is_some_and(|k| k != "StructElem")
                        || resolved_name(reader, &dict, "S")?.is_none()
                    {
                        return Err(fail("invalid structure element"));
                    }
                    let node = self.nodes.len();
                    if id.is_some_and(|r| self.refs.insert(r, node).is_some()) {
                        return Err(fail(
                            "shared structure element requires explicit ownership repair",
                        ));
                    }
                    self.materialize |= id.is_none();
                    self.slots.insert(slot.clone(), node);
                    self.nodes.push(Node {
                        slot: slot.clone(),
                        context: context.clone(),
                        value: value.clone(),
                        dict: dict.clone(),
                        reference: id,
                        parent: Some(parent),
                    });
                    if let Some(k) = dict.get("K") {
                        self.kids(reader, k, &context.key("K"), node, depth + 1, active)?;
                    }
                }
            }
            PdfObject::Integer(_) | PdfObject::Null => {}
            _ => return Err(fail("invalid structure K item")),
        }
        if let Some(r) = id {
            active.remove(&r);
        }
        Ok(())
    }
    fn locate(&self, reader: &PdfReader, value: &PdfObject) -> Result<usize> {
        crate::cancel::check_current_cancel("structure owner alias binding")?;
        if let Some(node) = value
            .as_reference()
            .and_then(|r| self.refs.get(&r).or_else(|| self.alias_nodes.get(&r)))
        {
            return Ok(*node);
        }
        let object = reader.resolve(value.clone())?;
        let dict = object
            .as_dict()
            .ok_or_else(|| fail("structure owner is not a dictionary"))?;
        let mut bytes = self.lookup_bytes.get();
        let hash = fingerprint(dict, &mut bytes);
        self.lookup_bytes.set(bytes);
        let hash = hash?;
        let nodes = self
            .hashes
            .get(&hash)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if nodes.len() != 1 || self.nodes[nodes[0]].dict != *dict {
            return Err(fail(
                "direct structure owner does not match one exact reachable element",
            ));
        }
        Ok(nodes[0])
    }
    fn record_alias(&mut self, node: usize, value: &PdfObject) -> Result<()> {
        if let Some(r) = value.as_reference() {
            if self.nodes[node].reference.is_some_and(|old| old != r)
                || self.aliases.insert(node, r).is_some_and(|old| old != r)
                || self.refs.get(&r).is_some_and(|old| *old != node)
                || self.alias_nodes.get(&r).is_some_and(|old| *old != node)
            {
                return Err(fail("structure owner has competing indirect shadows; explicit identity mapping is required"));
            }
            self.alias_nodes.insert(r, node);
        } else {
            self.materialize = true;
        }
        Ok(())
    }
    fn collect_value(
        &mut self,
        reader: &PdfReader,
        value: &PdfObject,
        allow_root: bool,
        depth: usize,
    ) -> Result<()> {
        crate::cancel::check_current_cancel("structure owner alias inventory")?;
        self.visits += 1;
        if self.visits > MAX_VISITS || depth > 128 {
            return Err(fail("structure owner alias budget exceeded"));
        }
        match reader.resolve(value.clone())? {
            PdfObject::Null => {}
            PdfObject::Array(items) => {
                for item in items {
                    self.collect_value(reader, &item, allow_root, depth + 1)?;
                }
            }
            PdfObject::Dictionary(_) => {
                let node = self.locate(reader, value)?;
                if node == 0 && !allow_root {
                    return Err(fail(
                        "structure content cannot be owned directly by StructTreeRoot",
                    ));
                }
                self.record_alias(node, value)?;
            }
            _ => return Err(fail("invalid structural ownership lookup value")),
        }
        Ok(())
    }
    fn lookup_aliases(
        &mut self,
        reader: &PdfReader,
        value: &PdfObject,
        key: &str,
        depth: usize,
        seen: &mut BTreeSet<Ref>,
    ) -> Result<()> {
        crate::cancel::check_current_cancel("structure lookup identity inventory")?;
        self.visits += 1;
        if self.visits > MAX_VISITS || depth > 64 {
            return Err(fail("structure lookup budget exceeded"));
        }
        if value.as_reference().is_some_and(|r| !seen.insert(r)) {
            return Err(fail("shared/cyclic structure lookup tree"));
        }
        let object = reader.resolve(value.clone())?;
        self.snapshot_lookup(reader, value)?;
        let dict = object
            .as_dict()
            .ok_or_else(|| fail("invalid structure lookup node"))?;
        if dict.contains_key(key) && dict.contains_key("Kids") {
            return Err(fail("mixed structural lookup node"));
        }
        if let Some(values) = member(reader, dict, key)? {
            self.snapshot_lookup(reader, dict.get(key).unwrap())?;
            let values = values
                .as_array()
                .ok_or_else(|| fail("invalid structural lookup entries"))?;
            if values.len() % 2 != 0 {
                return Err(fail("odd structural lookup entries"));
            }
            for pair in values.chunks_exact(2) {
                self.collect_value(reader, &pair[1], false, 0)?;
            }
        }
        if let Some(kids) = member(reader, dict, "Kids")? {
            self.snapshot_lookup(reader, dict.get("Kids").unwrap())?;
            for kid in kids
                .as_array()
                .ok_or_else(|| fail("invalid structure lookup Kids"))?
            {
                self.lookup_aliases(reader, kid, key, depth + 1, seen)?;
            }
        }
        Ok(())
    }
    fn snapshot_lookup(&mut self, reader: &PdfReader, value: &PdfObject) -> Result<()> {
        let mut value = value.clone();
        let mut seen = BTreeSet::new();
        while let Some(r) = value.as_reference() {
            crate::cancel::check_current_cancel("structure lookup source binding")?;
            if seen.len() > 128 || !seen.insert(r) {
                return Err(fail("cyclic structural lookup reference"));
            }
            let object = reader.get_object(r.0, r.1)?;
            if !self.lookup_sources.contains_key(&r) {
                object_hash(&object, &mut self.bytes)?;
                self.lookup_sources.insert(r, object.clone());
            }
            value = object;
        }
        Ok(())
    }
    pub fn prepare(
        &self,
        document: &PdfDocument,
        ids: &IdentityIndex,
        selected: &BTreeSet<String>,
    ) -> Result<Prepared> {
        let reader = document.reader();
        let mut sources = BTreeMap::<String, Vec<(String, Option<Ref>, PdfDictionary)>>::new();
        let mut bytes = 0usize;
        for page in document.get_pages()? {
            for (order, value) in annots(reader, (page.object_number, page.generation_number))?
                .1
                .into_iter()
                .enumerate()
            {
                crate::cancel::check_current_cancel("tagged annotation source incidence")?;
                let object = reader.resolve(value)?;
                let Some(dict) = object.as_dict() else {
                    continue;
                };
                let identity = &ids[&(page.page_number, order)];
                sources
                    .entry(fingerprint(dict, &mut bytes)?)
                    .or_default()
                    .push((identity.id.clone(), identity.reference, dict.clone()));
            }
        }
        // The final validator requires every OBJR target to have an indirect
        // identity. This dependency exists even if all StructElem owners are
        // already indirect; fixing only the explicitly selected carrier leaves
        // the transaction invalid when another direct carrier is present.
        let normalize_carriers = self.materialize
            || self
                .carriers
                .iter()
                .any(|carrier| carrier.value.as_reference().is_none());
        let mut result = Prepared {
            dependencies: BTreeSet::new(),
            candidates: BTreeMap::new(),
            needs_staging: normalize_carriers,
        };
        for carrier in &self.carriers {
            crate::cancel::check_current_cancel("tagged annotation materialization dependencies")?;
            let object = reader.resolve(carrier.value.clone())?;
            let Some(dict) = object.as_dict() else {
                if carrier.value.as_reference().is_none() && normalize_carriers {
                    return Err(fail(
                        "direct non-annotation OBJR target requires explicit materialization",
                    ));
                }
                continue;
            };
            let hash = fingerprint(dict, &mut bytes)?;
            let matches = sources.get(&hash).map(Vec::as_slice).unwrap_or_default();
            let matches = matches
                .iter()
                .filter(|(_, r, d)| {
                    *d == *dict
                        && (r.is_none()
                            || carrier.value.as_reference().is_none()
                            || *r == carrier.value.as_reference())
                })
                .collect::<Vec<_>>();
            let wanted = matches.iter().any(|(id, _, _)| selected.contains(id));
            if !wanted && !(normalize_carriers && carrier.value.as_reference().is_none()) {
                continue;
            }
            if matches.len() != 1 {
                return Err(fail(
                    "structure carrier needs one exact page annotation occurrence",
                ));
            }
            let (id, page_ref, dict) = matches[0];
            let key = member(reader, dict, "StructParent")?
                .and_then(|v| v.as_integer())
                .ok_or_else(|| fail("tagged annotation carrier has no StructParent key"))?;
            let owner = self
                .parents
                .get(&key)
                .ok_or_else(|| fail("tagged annotation has no ParentTree entry"))?;
            if self.locate(reader, owner)? != carrier.owner {
                return Err(fail("annotation OBJR and ParentTree disagree on ownership"));
            }
            if result
                .candidates
                .insert(
                    id.clone(),
                    Candidate {
                        slot: carrier.slot.clone(),
                        value: carrier.value.clone(),
                        reference: carrier.value.as_reference(),
                        dictionary: dict.clone(),
                        field_path: None,
                        direct_parent: false,
                    },
                )
                .is_some()
            {
                return Err(fail("annotation has multiple OBJR carriers"));
            }
            result.dependencies.insert(id.clone());
            result.needs_staging |= page_ref.is_some() && carrier.value.as_reference().is_none();
        }
        // A selected StructParent cannot silently disappear from the carrier set.
        for rows in sources.values() {
            for (id, _, dict) in rows {
                if selected.contains(id)
                    && member(reader, dict, "StructParent")?.is_some()
                    && !result.candidates.contains_key(id)
                {
                    return Err(fail("selected tagged annotation has no exact OBJR carrier"));
                }
            }
        }
        Ok(result)
    }
    pub fn apply(
        &self,
        current: &PdfDocument,
        objects: &mut BTreeMap<Ref, PdfObject>,
        next: &mut u32,
    ) -> Result<Applied> {
        let mut report = Applied {
            nodes: 0,
            parents: 0,
            lookup_values: 0,
        };
        if !self.materialize {
            return Ok(report);
        }
        let reader = current.reader();
        for (r, expected) in &self.lookup_sources {
            crate::cancel::check_current_cancel("structure lookup revision verification")?;
            if reader.get_object(r.0, r.1)? != *expected {
                return Err(fail("structure lookup changed before materialization"));
            }
        }
        // Annotation carrier updates may already have modified a direct root
        // inside the staged catalog. Validate source identity separately below,
        // then replace this effective value rather than its obsolete source copy.
        let staged_root_value = read_path(reader, objects, &self.nodes[0].slot)?;
        let mut allocations = Vec::with_capacity(self.nodes.len());
        let mut effective = Vec::with_capacity(self.nodes.len());
        let mut claimed = BTreeSet::new();
        for (index, node) in self.nodes.iter().enumerate() {
            crate::cancel::check_current_cancel("structure source revision verification")?;
            let original = read_path(reader, &BTreeMap::new(), &node.slot)?;
            if original != node.value || reader.resolve(original)?.as_dict() != Some(&node.dict) {
                return Err(fail("structure owner changed before materialization"));
            }
            let value = read_path(reader, objects, &node.slot)?;
            let value = resolve_overlay(reader, objects, value)?;
            effective.push(
                value
                    .as_dict()
                    .ok_or_else(|| fail("staged structure element disappeared"))?
                    .clone(),
            );
            let id = if let Some(r) = node.reference.or(self.aliases.get(&index).copied()) {
                if node.reference.is_none()
                    && reader.get_object(r.0, r.1)?.as_dict() != Some(&node.dict)
                {
                    return Err(fail("structure shadow changed before materialization"));
                }
                r
            } else {
                *next = next
                    .checked_add(1)
                    .ok_or_else(|| fail("structure object number exhausted"))?;
                (*next, 0)
            };
            if !claimed.insert(id) {
                return Err(fail("two structure owners share an output identity"));
            }
            if node.reference.is_none() {
                report.nodes += 1;
            }
            allocations.push(id);
        }
        for (index, node) in self.nodes.iter().enumerate().rev() {
            crate::cancel::check_current_cancel("structure identity materialization")?;
            let mut dict = effective[index].clone();
            normalize_names(
                reader,
                &mut dict,
                if index == 0 {
                    &["Type"]
                } else {
                    &["Type", "S"]
                },
            )?;
            if let Some(parent) = node.parent {
                let expected = reference(allocations[parent]);
                if dict.get("P") != Some(&expected) {
                    dict.insert("P", expected);
                    report.parents += 1;
                }
            }
            if let Some(k) = dict.get("K").cloned() {
                dict.insert(
                    "K",
                    self.rewrite_k(reader, objects, &k, &node.context.key("K"), &allocations, 0)?,
                );
            }
            if let Some(value) = dict.get("Ref").cloned() {
                dict.insert(
                    "Ref",
                    self.owner_value(reader, &value, &allocations, &mut report.lookup_values, 0)?,
                );
            }
            if index == 0 {
                for key in ["ParentTree", "IDTree"] {
                    if let Some(value) = dict.get(key).cloned() {
                        let rewritten = self.rewrite_lookup(
                            reader,
                            objects,
                            &value,
                            if key == "ParentTree" { "Nums" } else { "Names" },
                            &allocations,
                            &mut report.lookup_values,
                            0,
                            &mut BTreeSet::new(),
                        )?;
                        dict.insert(key, rewritten);
                    }
                }
            }
            objects.insert(allocations[index], PdfObject::Dictionary(dict));
        }
        let root = &self.nodes[0];
        let catalog = objects
            .get(&root.slot.owner)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| reader.get_object(root.slot.owner.0, root.slot.owner.1))?;
        let rewritten = replace_slot(
            reader,
            &catalog,
            &root.slot.path,
            &staged_root_value,
            &reference(allocations[0]),
            0,
        )?;
        if rewritten != catalog {
            objects.insert(root.slot.owner, rewritten);
        }
        Ok(report)
    }
    fn rewrite_k(
        &self,
        reader: &PdfReader,
        objects: &BTreeMap<Ref, PdfObject>,
        value: &PdfObject,
        slot: &Slot,
        allocated: &[Ref],
        depth: usize,
    ) -> Result<PdfObject> {
        crate::cancel::check_current_cancel("structure child identity rewrite")?;
        if depth > 128 {
            return Err(fail("structure K rewrite depth exceeded"));
        }
        if let Some(node) = self.slots.get(slot) {
            return Ok(reference(allocated[*node]));
        }
        let object = resolve_overlay(reader, objects, value.clone())?;
        if let PdfObject::Array(items) = &object {
            let mut rewritten = Vec::with_capacity(items.len());
            for (index, item) in items.iter().enumerate() {
                rewritten.push(self.rewrite_k(
                    reader,
                    objects,
                    item,
                    &slot.index(index),
                    allocated,
                    depth + 1,
                )?);
            }
            if rewritten.as_slice() != items.as_slice() {
                return Ok(PdfObject::Array(rewritten));
            }
        }
        if let PdfObject::Dictionary(mut dict) = object {
            // Content-item dictionaries retain their carrier slots; only an
            // equivalent indirect Type scalar needs canonicalizing here.
            if indirect_names(reader, &dict, &["Type"])? {
                normalize_names(reader, &mut dict, &["Type"])?;
                return Ok(PdfObject::Dictionary(dict));
            }
        }
        Ok(value.clone())
    }
    fn owner_value(
        &self,
        reader: &PdfReader,
        value: &PdfObject,
        allocated: &[Ref],
        count: &mut usize,
        depth: usize,
    ) -> Result<PdfObject> {
        crate::cancel::check_current_cancel("structure owner value rewrite")?;
        if depth > 128 {
            return Err(fail("structure owner value depth exceeded"));
        }
        match reader.resolve(value.clone())? {
            PdfObject::Null => Ok(value.clone()),
            PdfObject::Array(items) => {
                let rewritten = items
                    .iter()
                    .map(|item| self.owner_value(reader, item, allocated, count, depth + 1))
                    .collect::<Result<Vec<_>>>()?;
                if rewritten == items {
                    Ok(value.clone())
                } else {
                    Ok(PdfObject::Array(rewritten))
                }
            }
            PdfObject::Dictionary(_) => {
                let node = self.locate(reader, value)?;
                let result = reference(allocated[node]);
                if result != *value {
                    *count += 1;
                }
                Ok(result)
            }
            _ => Err(fail("invalid structure owner value")),
        }
    }
    fn rewrite_lookup(
        &self,
        reader: &PdfReader,
        objects: &mut BTreeMap<Ref, PdfObject>,
        value: &PdfObject,
        key: &str,
        allocated: &[Ref],
        count: &mut usize,
        depth: usize,
        seen: &mut BTreeSet<Ref>,
    ) -> Result<PdfObject> {
        crate::cancel::check_current_cancel("structure ownership lookup rewrite")?;
        if depth > 64 || seen.len() > MAX_VISITS {
            return Err(fail("structure lookup rewrite budget exceeded"));
        }
        if value.as_reference().is_some_and(|r| !seen.insert(r)) {
            return Err(fail("shared/cyclic structure lookup node"));
        }
        let object = resolve_overlay(reader, objects, value.clone())?;
        let mut dict = object
            .as_dict()
            .ok_or_else(|| fail("invalid structure lookup node"))?
            .clone();
        if let Some(entries) = member(reader, &dict, key)? {
            let old = entries
                .as_array()
                .ok_or_else(|| fail("invalid structure lookup entries"))?;
            if old.len() % 2 != 0 {
                return Err(fail("odd structure lookup entries"));
            }
            let mut entries = old.to_vec();
            for pair in entries.chunks_exact_mut(2) {
                pair[1] = self.owner_value(reader, &pair[1], allocated, count, 0)?;
            }
            if entries != old {
                dict.insert(key, PdfObject::Array(entries));
            }
        }
        if let Some(kids) = member(reader, &dict, "Kids")? {
            let old = kids
                .as_array()
                .ok_or_else(|| fail("invalid lookup children"))?;
            let mut new = Vec::with_capacity(old.len());
            for child in old {
                new.push(self.rewrite_lookup(
                    reader,
                    objects,
                    child,
                    key,
                    allocated,
                    count,
                    depth + 1,
                    seen,
                )?);
            }
            if new != old {
                dict.insert("Kids", PdfObject::Array(new));
            }
        }
        if PdfObject::Dictionary(dict.clone()) == object {
            return Ok(value.clone());
        }
        if let Some(r) = value.as_reference() {
            objects.insert(r, PdfObject::Dictionary(dict));
            Ok(value.clone())
        } else {
            Ok(PdfObject::Dictionary(dict))
        }
    }
}
fn resolve_overlay(
    reader: &PdfReader,
    objects: &BTreeMap<Ref, PdfObject>,
    mut value: PdfObject,
) -> Result<PdfObject> {
    let mut seen = BTreeSet::new();
    while let Some(r) = value.as_reference() {
        if seen.len() > 128 || !seen.insert(r) {
            return Err(fail("cyclic staged owner reference"));
        }
        value = objects
            .get(&r)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| reader.get_object(r.0, r.1))?;
    }
    Ok(value)
}
fn read_path(
    reader: &PdfReader,
    objects: &BTreeMap<Ref, PdfObject>,
    slot: &Slot,
) -> Result<PdfObject> {
    let mut value = objects
        .get(&slot.owner)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| reader.get_object(slot.owner.0, slot.owner.1))?;
    for step in &slot.path {
        crate::cancel::check_current_cancel("structure source path binding")?;
        value = match (step, resolve_overlay(reader, objects, value)?) {
            (Step::Key(key), PdfObject::Dictionary(dict)) => dict
                .get(key)
                .cloned()
                .ok_or_else(|| fail("structure source key disappeared"))?,
            (Step::Index(index), PdfObject::Array(items)) => items
                .get(*index)
                .cloned()
                .ok_or_else(|| fail("structure source index disappeared"))?,
            _ => return Err(fail("structure source path changed kind")),
        };
    }
    Ok(value)
}
