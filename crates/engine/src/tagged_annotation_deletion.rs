//! Compose explicit annotation deletion with page migration. Remove only exact
//! OBJR/MCR carriers. Empty semantic containers and unrelated metadata remain.
use super::*;

pub(super) fn stage(
    input: &[u8],
    moves: &BTreeMap<ObjectRef, ObjectRef>,
    deleted: &BTreeSet<ObjectRef>,
) -> Result<Vec<IncrementalObject>> {
    let migrated = annotation_page_migration(input, moves)?;
    if deleted.is_empty() {
        return Ok(migrated);
    }
    let engine = ContentEngine::open_bytes(input.to_vec())?;
    if !engine
        .document()
        .get_catalog()?
        .contains_key("StructTreeRoot")
    {
        return Ok(migrated);
    }
    validate_parent_tree(input)?;
    let (store, _, index) = index_document(&engine, None)?;
    let mut content = ContentScopes::new(&engine);
    content.collect(&index)?;
    let mut staged = migrated
        .into_iter()
        .map(|o| ((o.number, o.generation), o.object))
        .collect::<BTreeMap<_, _>>();
    let pages = engine.document().get_pages()?;
    let mut page_scopes = BTreeSet::new();
    let mut owners = BTreeMap::<ObjectRef, BTreeSet<ObjectRef>>::new();
    let mut visited = 0usize;
    for page in &pages {
        crate::cancel::check_current_cancel("annotation deletion ownership analysis")?;
        let page_id = (page.object_number, page.generation_number);
        page_scopes.extend(content.descendants(page_id)?);
        let dict = store.dict(page_id)?;
        let Some(annots) = dict.get("Annots") else {
            continue;
        };
        let object = store.resolve(annots)?;
        for annotation in object
            .as_array()
            .ok_or_else(|| fail("invalid annotation array"))?
        {
            let Some(id) = annotation.as_reference() else {
                continue;
            };
            let dict = store.dict(id)?;
            if let Some(ap) = dict.get("AP") {
                let mut roots = BTreeSet::new();
                appearance_streams(
                    &store,
                    ap,
                    &mut roots,
                    &mut BTreeSet::new(),
                    &mut visited,
                    0,
                )?;
                for root in roots {
                    for scope in content.descendants(root)? {
                        owners.entry(scope).or_default().insert(id);
                    }
                }
            }
        }
    }
    let mut removed_scopes = BTreeSet::new();
    for (scope, owners) in owners {
        if !owners.iter().any(|o| deleted.contains(o))
            || !index.marked.contains_key(&scope) && !index.objects.contains_key(&scope)
        {
            continue;
        }
        if owners.iter().any(|o| !deleted.contains(o)) || page_scopes.contains(&scope) {
            return Err(fail("deleting shared tagged annotation appearance requires occurrence-specific semantic cloning"));
        }
        removed_scopes.insert(scope);
    }
    let mut removed_objects = deleted.clone();
    removed_objects.extend(removed_scopes.iter().copied());
    let mut visits = 0usize;
    for id in &index.nodes {
        crate::cancel::check_current_cancel("annotation structural carrier deletion")?;
        // Start with migrated Pg values so deletion cannot discard earlier
        // changes when both operations touch one StructElem's K array.
        let mut dict = match staged.get(id) {
            Some(o) => o
                .as_dict()
                .cloned()
                .ok_or_else(|| fail("invalid staged structure element"))?,
            None => store.dict(*id)?,
        };
        let Some(kids) = dict.get("K") else { continue };
        let (replacement, changed) = prune(
            &store,
            kids,
            &removed_objects,
            &removed_scopes,
            0,
            &mut visits,
        )?;
        if changed {
            match replacement {
                Some(value) => {
                    dict.insert("K", value);
                }
                None => {
                    dict.remove("K");
                }
            }
            staged.insert(*id, PdfObject::Dictionary(dict));
        }
    }
    Ok(staged
        .into_iter()
        .map(|((number, generation), object)| IncrementalObject {
            number,
            generation,
            object,
        })
        .collect())
}

fn prune(
    store: &Store<'_>,
    value: &PdfObject,
    objects: &BTreeSet<ObjectRef>,
    streams: &BTreeSet<ObjectRef>,
    depth: usize,
    visits: &mut usize,
) -> Result<(Option<PdfObject>, bool)> {
    *visits = visits.saturating_add(1);
    if depth > 128 || *visits > MAX_NODES {
        return Err(fail("annotation structural deletion budget exceeded"));
    }
    let object = store.resolve(value)?;
    match object {
        PdfObject::Array(items) => {
            let mut result = Vec::new();
            let mut changed = false;
            for item in items {
                let (replacement, did_change) =
                    prune(store, &item, objects, streams, depth + 1, visits)?;
                changed |= did_change;
                if let Some(item) = replacement {
                    result.push(item);
                }
            }
            if changed {
                Ok((
                    (!result.is_empty()).then_some(PdfObject::Array(result)),
                    true,
                ))
            } else {
                Ok((Some(value.clone()), false))
            }
        }
        PdfObject::Dictionary(dict)
            if dict.get_name("Type") == Some("OBJR")
                && dict
                    .get_reference("Obj")
                    .is_some_and(|r| objects.contains(&r)) =>
        {
            Ok((None, true))
        }
        PdfObject::Dictionary(dict)
            if dict.get_name("Type") == Some("MCR")
                && dict
                    .get_reference("Stm")
                    .is_some_and(|r| streams.contains(&r)) =>
        {
            Ok((None, true))
        }
        _ => Ok((Some(value.clone()), false)),
    }
}
