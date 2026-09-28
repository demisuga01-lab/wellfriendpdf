//! Reference-preserving removal of explicitly approved page leaves. Content
//! eligibility/ownership belongs to the story planner; this writer independently
//! refuses live incoming references and preserves labels of surviving pages.
use super::*;
type Ref = (u32, u16);
fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}
fn reference(id: Ref) -> PdfObject {
    PdfObject::Reference {
        number: id.0,
        generation: id.1,
    }
}

#[derive(Default)]
pub(crate) struct Departures {
    pub tags: BTreeSet<Ref>,
    pub annotations: BTreeSet<Ref>,
}

/// A bounded graph walk, not a scan of historical/unreachable xref objects.
/// Only real page-tree Kids are exempt; a /Pages-looking private object is not.
pub(crate) fn protected_pages(
    doc: &PdfDocument,
    candidates: &BTreeMap<Ref, usize>,
    departures: &Departures,
) -> Result<BTreeMap<usize, String>> {
    if candidates.is_empty() {
        return Ok(BTreeMap::new());
    }
    let reader = doc.reader();
    let root = doc
        .get_catalog()?
        .get_reference("Pages")
        .ok_or_else(|| fail("missing page tree"))?;
    let mut tree = BTreeSet::new();
    fn tree_nodes(
        reader: &PdfReader,
        id: Ref,
        seen: &mut BTreeSet<Ref>,
        depth: usize,
    ) -> Result<()> {
        crate::cancel::check_current_cancel("pruning page-tree inventory")?;
        if depth > 256 || seen.len() >= 200_000 || !seen.insert(id) {
            return Err(fail("cyclic/oversized pruning page tree"));
        }
        let object = reader.get_object(id.0, id.1)?;
        let dict = object
            .as_dict()
            .ok_or_else(|| fail("invalid page-tree node"))?;
        match dict.get_name("Type") {
            Some("Page") => {}
            Some("Pages") => {
                let kids = reader.resolve(
                    dict.get("Kids")
                        .cloned()
                        .ok_or_else(|| fail("missing page-tree Kids"))?,
                )?;
                for kid in kids
                    .as_array()
                    .ok_or_else(|| fail("invalid page-tree Kids"))?
                {
                    tree_nodes(
                        reader,
                        kid.as_reference()
                            .ok_or_else(|| fail("direct page-tree child"))?,
                        seen,
                        depth + 1,
                    )?;
                }
            }
            _ => return Err(fail("invalid page-tree type")),
        }
        Ok(())
    }
    tree_nodes(reader, root, &mut tree, 0)?;
    struct Walk<'a> {
        reader: &'a PdfReader,
        tree: &'a BTreeSet<Ref>,
        candidates: &'a BTreeMap<Ref, usize>,
        departures: &'a Departures,
        seen: BTreeSet<(Ref, bool, bool)>,
        budget: usize,
        protected: BTreeMap<usize, String>,
        numeric: bool,
    }
    impl Walk<'_> {
        fn visit(
            &mut self,
            value: &PdfObject,
            owner: Option<Ref>,
            tag_path: bool,
            allowed: bool,
            depth: usize,
        ) -> Result<()> {
            if self.budget == 0 || depth > 256 {
                return Err(fail("pruning reference graph budget exceeded"));
            }
            self.budget -= 1;
            crate::cancel::check_current_cancel("pruning incoming reference walk")?;
            match value {
                PdfObject::Reference { number, generation } => {
                    let id = (*number, *generation);
                    if let Some(page) = self.candidates.get(&id) {
                        if !allowed {
                            self.protected
                                .entry(*page)
                                .or_insert_with(|| "live_incoming_page_reference".into());
                        }
                    }
                    // A /Kids array may itself be indirect. Preserve the edge
                    // exemption through arrays, never arbitrary dictionaries.
                    // Revisit aliases reached with different exemption context.
                    if self.seen.insert((id, tag_path, allowed)) {
                        let object = self.reader.get_object(id.0, id.1)?;
                        self.visit(&object, Some(id), tag_path, allowed, depth + 1)?;
                    }
                }
                PdfObject::Array(items) => {
                    for item in items {
                        self.visit(item, owner, tag_path, allowed, depth + 1)?;
                    }
                }
                PdfObject::Dictionary(dict) | PdfObject::Stream { dict, .. } => {
                    let tag_owner = owner.is_some_and(|r| self.departures.tags.contains(&r))
                        && dict.get_name("Type") == Some("StructElem");
                    let tagged = tag_owner || tag_path && dict.get_name("Type") == Some("MCR");
                    let page_tree = owner.is_some_and(|r| self.tree.contains(&r))
                        && dict.get_name("Type") == Some("Pages");
                    let annotation = owner
                        .is_some_and(|r| self.departures.annotations.contains(&r))
                        && dict.get_name("Subtype").is_some();
                    if dict.get_name("S") == Some("JavaScript")
                        || dict.contains_key("JavaScript")
                        || dict.contains_key("PrintPageRange")
                    {
                        self.numeric = true;
                    }
                    for (key, child) in dict.iter() {
                        // Local integer destinations/ranges can encode ordinal
                        // dependencies outside the indirect-page reference graph.
                        if matches!(key.as_str(), "Dest" | "D" | "OpenAction")
                            && self
                                .reader
                                .resolve(child.clone())?
                                .as_array()
                                .and_then(|a| a.first())
                                .is_some_and(|v| matches!(v, PdfObject::Integer(_)))
                        {
                            self.numeric = true;
                        }
                        let permit = (page_tree && key == "Kids")
                            || (tagged && key == "Pg")
                            || (annotation && key == "P");
                        self.visit(child, owner, tagged && key == "K", permit, depth + 1)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }
    let mut walk = Walk {
        reader,
        tree: &tree,
        candidates,
        departures,
        seen: BTreeSet::new(),
        budget: 2_000_000,
        protected: BTreeMap::new(),
        numeric: false,
    };
    walk.visit(
        &PdfObject::Dictionary(reader.trailer().clone()),
        None,
        false,
        false,
        0,
    )?;
    if walk.numeric {
        for &page in candidates.values() {
            walk.protected
                .entry(page)
                .or_insert_with(|| "numeric_page_dependency_or_script_requires_review".into());
        }
    }
    Ok(walk.protected)
}

/// Preserve the original label of every surviving page, including a label run
/// which begins on a removed page. Gaps introduce new /St entries as needed.
fn labels(doc: &PdfDocument, removed: &BTreeSet<usize>) -> Result<Option<PdfObject>> {
    let catalog = doc.get_catalog()?;
    let Some(value) = catalog.get("PageLabels") else {
        return Ok(None);
    };
    fn read(
        reader: &PdfReader,
        value: &PdfObject,
        seen: &mut BTreeSet<Ref>,
        entries: &mut BTreeMap<usize, PdfDictionary>,
        depth: usize,
    ) -> Result<()> {
        if depth > 64 || seen.len() > 200_000 || entries.len() > 200_000 {
            return Err(fail("page-label tree budget exceeded"));
        }
        if let Some(id) = value.as_reference() {
            if !seen.insert(id) {
                return Err(fail("shared/cyclic page-label tree"));
            }
            return read(
                reader,
                &reader.get_object(id.0, id.1)?,
                seen,
                entries,
                depth + 1,
            );
        }
        let dict = value
            .as_dict()
            .ok_or_else(|| fail("invalid page-label tree"))?;
        if dict
            .iter()
            .any(|(k, _)| !matches!(k.as_str(), "Nums" | "Kids" | "Limits"))
        {
            return Err(fail(
                "opaque page-label tree properties require preservation review",
            ));
        }
        if dict.contains_key("Nums") && dict.contains_key("Kids") {
            return Err(fail("mixed page-label tree node"));
        }
        if let Some(nums) = dict.get("Nums") {
            let resolved = reader.resolve(nums.clone())?;
            let nums = resolved
                .as_array()
                .ok_or_else(|| fail("invalid page-label Nums"))?;
            if nums.len() % 2 != 0 {
                return Err(fail("odd page-label Nums"));
            }
            for pair in nums.chunks_exact(2) {
                if entries.len() >= 200_000 {
                    return Err(fail("page-label entry budget exceeded"));
                }
                let key = pair[0]
                    .as_integer()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| fail("invalid page-label index"))?;
                let label = reader
                    .resolve(pair[1].clone())?
                    .as_dict()
                    .cloned()
                    .ok_or_else(|| fail("invalid page label"))?;
                if entries.insert(key, label).is_some() {
                    return Err(fail("duplicate page-label index"));
                }
            }
        }
        if let Some(kids) = dict.get("Kids") {
            let resolved = reader.resolve(kids.clone())?;
            for child in resolved
                .as_array()
                .ok_or_else(|| fail("invalid page-label Kids"))?
            {
                read(reader, child, seen, entries, depth + 1)?;
            }
        }
        Ok(())
    }
    let mut entries = BTreeMap::new();
    read(doc.reader(), value, &mut BTreeSet::new(), &mut entries, 0)?;
    let count = doc.get_pages()?.len();
    if !entries.contains_key(&0) || entries.keys().any(|&k| k >= count) {
        return Err(fail("page labels need an in-range index-zero entry"));
    }
    let mut nums = Vec::new();
    let mut previous = None;
    let mut output_index = 0i64;
    for index in 0..count {
        crate::cancel::check_current_cancel("pruning page labels")?;
        if removed.contains(&(index + 1)) {
            continue;
        }
        let (&start, label) = entries
            .range(..=index)
            .next_back()
            .ok_or_else(|| fail("page label not covered"))?;
        if previous.is_none_or(|(old, run)| old + 1 != index || run != start) {
            let mut label = label.clone();
            if let Some(style) = label.get("S") {
                if !matches!(style.as_name(), Some("D" | "R" | "r" | "A" | "a")) {
                    return Err(fail("unknown page-label numbering style"));
                }
                let first = match label.get("St") {
                    None => 1,
                    Some(value) => doc
                        .reader()
                        .resolve(value.clone())?
                        .as_integer()
                        .ok_or_else(|| fail("invalid page-label start type"))?,
                };
                if first < 1 {
                    return Err(fail("invalid page-label start"));
                }
                let number = first
                    .checked_add((index - start) as i64)
                    .ok_or_else(|| fail("page label overflow"))?;
                label.insert("St", PdfObject::Integer(number));
            }
            nums.push(PdfObject::Integer(output_index));
            nums.push(PdfObject::Dictionary(label));
        }
        previous = Some((index, start));
        output_index += 1;
    }
    let mut root = PdfDictionary::empty();
    root.insert("Nums", PdfObject::Array(nums));
    Ok(Some(PdfObject::Dictionary(root)))
}

pub(crate) fn validate_labels(doc: &PdfDocument, removed: &BTreeSet<usize>) -> Result<()> {
    if !removed.is_empty() {
        labels(doc, removed)?;
    }
    Ok(())
}

pub(crate) fn remove(doc: &PdfDocument, removed: &BTreeSet<usize>) -> Result<Vec<u8>> {
    let pages = doc.get_pages()?;
    if removed.is_empty()
        || removed.len() >= pages.len()
        || removed.iter().any(|&p| p == 0 || p > pages.len())
    {
        return Err(fail("invalid page-pruning set"));
    }
    let targets = removed
        .iter()
        .map(|&p| {
            (
                (pages[p - 1].object_number, pages[p - 1].generation_number),
                p,
            )
        })
        .collect::<BTreeMap<_, _>>();
    if !protected_pages(doc, &targets, &Departures::default())?.is_empty() {
        return Err(fail("pruning would leave a live page dependency"));
    }
    let reader = doc.reader();
    let root_id = reader
        .root_reference()
        .ok_or_else(|| fail("missing pruning catalog"))?;
    let mut catalog = doc.get_catalog()?;
    let tree = catalog
        .get_reference("Pages")
        .ok_or_else(|| fail("missing pruning Pages"))?;
    fn rewrite(
        reader: &PdfReader,
        id: Ref,
        targets: &BTreeMap<Ref, usize>,
        updates: &mut Vec<IncrementalObject>,
        seen: &mut BTreeSet<Ref>,
        depth: usize,
    ) -> Result<usize> {
        if depth > 256 || seen.len() >= 200_000 || !seen.insert(id) {
            return Err(fail("pruning page-tree cycle/budget"));
        }
        crate::cancel::check_current_cancel("pruning page-tree rewrite")?;
        let mut dict = reader
            .get_object(id.0, id.1)?
            .as_dict()
            .cloned()
            .ok_or_else(|| fail("invalid pruning node"))?;
        if dict.get_name("Type") == Some("Page") {
            return Ok(usize::from(!targets.contains_key(&id)));
        }
        if dict.get_name("Type") != Some("Pages") {
            return Err(fail("invalid pruning page-tree type"));
        }
        let mut kept = Vec::new();
        let mut count = 0usize;
        let kids = reader.resolve(
            dict.get("Kids")
                .cloned()
                .ok_or_else(|| fail("missing pruning Kids"))?,
        )?;
        for kid in kids
            .as_array()
            .ok_or_else(|| fail("invalid pruning Kids"))?
        {
            let child = kid
                .as_reference()
                .ok_or_else(|| fail("direct pruning child"))?;
            let n = rewrite(reader, child, targets, updates, seen, depth + 1)?;
            count = count
                .checked_add(n)
                .ok_or_else(|| fail("page count overflow"))?;
            if n > 0 {
                kept.push(reference(child));
            }
        }
        dict.insert("Kids", PdfObject::Array(kept));
        dict.insert("Count", PdfObject::Integer(count as i64));
        updates.push(IncrementalObject {
            number: id.0,
            generation: id.1,
            object: PdfObject::Dictionary(dict),
        });
        Ok(count)
    }
    let mut updates = Vec::new();
    if rewrite(
        reader,
        tree,
        &targets,
        &mut updates,
        &mut BTreeSet::new(),
        0,
    )? != pages.len() - removed.len()
    {
        return Err(fail("pruning page count mismatch"));
    }
    if let Some(labels) = labels(doc, removed)? {
        catalog.insert("PageLabels", labels);
    }
    updates.push(IncrementalObject {
        number: root_id.0,
        generation: root_id.1,
        object: PdfObject::Dictionary(catalog),
    });
    write_incremental_update(reader, updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        authoring::{PageSize, PdfBuilder},
        ContentEngine,
    };
    #[test]
    fn indirect_page_tree_arrays_are_not_authority_for_external_aliases() {
        let mut builder = PdfBuilder::new();
        for _ in 0..3 {
            builder.add_page(PageSize::custom(100.0, 100.0));
        }
        let engine = ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
        let reader = engine.document().reader();
        let root = reader.root_reference().unwrap();
        let mut catalog = engine.document().get_catalog().unwrap();
        let pages_id = catalog.get_reference("Pages").unwrap();
        let mut tree = reader
            .get_object(pages_id.0, pages_id.1)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        let kids = tree.get("Kids").unwrap().clone();
        let id = (
            reader.object_ids().iter().map(|r| r.0).max().unwrap() + 1,
            0,
        );
        tree.insert("Kids", reference(id));
        let bytes = write_incremental_update(
            reader,
            vec![
                IncrementalObject {
                    number: id.0,
                    generation: 0,
                    object: kids,
                },
                IncrementalObject {
                    number: pages_id.0,
                    generation: pages_id.1,
                    object: PdfObject::Dictionary(tree),
                },
            ],
        )
        .unwrap();
        let engine = ContentEngine::open_bytes(bytes).unwrap();
        let p = engine.document().get_page(2).unwrap();
        let targets = BTreeMap::from([((p.object_number, p.generation_number), 2)]);
        assert!(
            protected_pages(engine.document(), &targets, &Departures::default())
                .unwrap()
                .is_empty()
        );
        let output = remove(engine.document(), &BTreeSet::from([2])).unwrap();
        assert_eq!(
            ContentEngine::open_bytes(output)
                .unwrap()
                .page_count()
                .unwrap(),
            2
        );
        catalog.insert("PrivatePageList", reference(id));
        let bytes = write_incremental_update(
            engine.document().reader(),
            vec![IncrementalObject {
                number: root.0,
                generation: root.1,
                object: PdfObject::Dictionary(catalog),
            }],
        )
        .unwrap();
        let alias = ContentEngine::open_bytes(bytes).unwrap();
        assert!(
            protected_pages(alias.document(), &targets, &Departures::default())
                .unwrap()
                .contains_key(&2)
        );
        assert!(remove(alias.document(), &BTreeSet::from([2])).is_err());
    }
}
