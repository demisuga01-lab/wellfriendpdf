//! Copy-on-write for the selected /Contents slot, not just its stream object.
//! A page stream may be shared by other pages or repeated in the same array.
use super::*;

#[cfg(test)]
#[path = "advanced_vector_occurrence_tests.rs"]
mod tests;

type ObjectRef = (u32, u16);

fn invalid(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(format!("vector occurrence: {message}"))
}

/// Do not copy stream-owned accessibility identities without migrating the
/// corresponding owners. Page-owned MCIDs remain on the same page and do not
/// need rebinding when only its Contents reference changes.
pub(super) fn check_clone_ownership(
    reader: &crate::PdfReader,
    sources: &BTreeSet<ObjectRef>,
) -> Result<()> {
    for &(number, generation) in sources {
        let PdfObject::Stream { dict, .. } = reader.get_object(number, generation)? else {
            return Err(invalid("clone source is not a stream"));
        };
        if ["StructParent", "StructParents"].iter().any(|key| {
            dict.get(key)
                .is_some_and(|value| !matches!(value, PdfObject::Null))
        }) {
            return Err(WellfriendError::UnsupportedFeature(
                "vector occurrence clone requires migration of stream-owned structure keys".into(),
            ));
        }
    }
    let root = reader
        .root_reference()
        .ok_or_else(|| invalid("missing catalog"))?;
    let catalog = reader.get_object(root.0, root.1)?;
    let catalog = catalog
        .as_dict()
        .ok_or_else(|| invalid("invalid catalog"))?;
    let Some(structure) = catalog.get("StructTreeRoot") else {
        return Ok(());
    };
    let mut visits = 0usize;
    let mut active = BTreeSet::new();
    check_structure(reader, structure, sources, &mut active, &mut visits, 0)
}

// Follow only the reachable ownership graph (/K), never /P back-pointers,
// resource graphs or an unbounded scan of every historical PDF object.
fn check_structure(
    reader: &crate::PdfReader,
    value: &PdfObject,
    sources: &BTreeSet<ObjectRef>,
    active: &mut BTreeSet<ObjectRef>,
    visits: &mut usize,
    depth: usize,
) -> Result<()> {
    crate::cancel::check_current_cancel("vector clone structure ownership")?;
    *visits += 1;
    if depth > 128 || *visits > 100_000 {
        return Err(WellfriendError::ResourceLimit(
            "vector clone structure traversal budget exceeded".into(),
        ));
    }
    match value {
        PdfObject::Reference { number, generation } => {
            let id = (*number, *generation);
            if !active.insert(id) {
                return Err(invalid("cyclic structure ownership"));
            }
            check_structure(
                reader,
                &reader.get_object(id.0, id.1)?,
                sources,
                active,
                visits,
                depth + 1,
            )?;
            active.remove(&id);
        }
        PdfObject::Dictionary(dict) => {
            for key in ["Stm", "StmOwn", "Obj"] {
                if dict
                    .get(key)
                    .and_then(PdfObject::as_reference)
                    .is_some_and(|id| sources.contains(&id))
                {
                    return Err(WellfriendError::UnsupportedFeature(
                        "vector occurrence clone requires migration of MCR/OBJR stream ownership"
                            .into(),
                    ));
                }
            }
            if let Some(kids) = dict.get("K") {
                check_structure(reader, kids, sources, active, visits, depth + 1)?;
            }
        }
        PdfObject::Array(items) => {
            for item in items {
                check_structure(reader, item, sources, active, visits, depth + 1)?;
            }
        }
        PdfObject::Null | PdfObject::Integer(_) => {}
        _ => return Err(invalid("invalid reachable structure ownership")),
    }
    Ok(())
}

/// Validate the chain and retain the effective resource dictionary at each
/// invocation. Older Forms without /Resources use the page's resources,
/// independently of their inherited caller graphics state.
pub(super) fn invocation_resources(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
    before: &EditableVectorObject,
) -> Result<Vec<crate::PdfDictionary>> {
    let mut expected_owner = *page
        .contents
        .get(before.provenance.content_stream_index)
        .ok_or_else(|| invalid("selected Contents slot does not exist"))?;
    let mut resources = page.resources.clone();
    let mut result = Vec::new();
    for (index, invocation) in before.provenance.form_invocation_path.iter().enumerate() {
        if (
            invocation.owner_stream_object,
            invocation.owner_stream_generation,
        ) != expected_owner
            || invocation.depth != index + 1
        {
            return Err(invalid(
                "Form chain is not rooted in the selected Contents slot",
            ));
        }
        let xobjects = resolve_advanced_editing_dict(resources.get("XObject"), reader)
            .ok_or_else(|| invalid("Form caller has no XObject dictionary"))?;
        let form = (invocation.form_object, invocation.form_generation);
        if xobjects
            .get(&invocation.resource_name)
            .and_then(PdfObject::as_reference)
            != Some(form)
        {
            return Err(invalid(
                "Form resource does not match its source invocation",
            ));
        }
        result.push(resources.clone());
        let PdfObject::Stream { dict, .. } = reader.get_object(form.0, form.1)? else {
            return Err(invalid("Form chain contains a non-stream"));
        };
        resources = if let Some(value) = dict.get("Resources") {
            match reader.resolve(value.clone())? {
                PdfObject::Null => page.resources.clone(),
                PdfObject::Dictionary(next) => next,
                _ => return Err(invalid("Form has invalid Resources")),
            }
        } else {
            page.resources.clone()
        };
        expected_owner = form;
    }
    if expected_owner
        != (
            before.provenance.object_number,
            before.provenance.generation,
        )
    {
        return Err(invalid("Form chain does not end at the selected source"));
    }
    Ok(result)
}

/// Move one pending stream mutation to a fresh object and rewire exactly one
/// slot. Preserve any page/resource updates already staged by the caller.
/// Validation completes before the supplied update set is changed.
pub(super) fn stage(
    reader: &crate::PdfReader,
    page: &crate::document::PdfPage,
    stream_index: usize,
    updates: &mut Vec<IncrementalObject>,
) -> Result<ObjectRef> {
    let source = *page
        .contents
        .get(stream_index)
        .ok_or_else(|| invalid("invalid Contents slot"))?;
    check_clone_ownership(reader, &BTreeSet::from([source]))?;
    let mut ids = BTreeSet::new();
    for update in updates.iter() {
        if !ids.insert(update.number) {
            return Err(invalid("duplicate pending object number"));
        }
    }
    let source_slot = updates
        .iter()
        .position(|update| (update.number, update.generation) == source)
        .ok_or_else(|| invalid("selected Contents stream has no pending mutation"))?;
    if !matches!(updates[source_slot].object, PdfObject::Stream { .. }) {
        return Err(invalid("pending Contents mutation is not a stream"));
    }
    let page_slot = updates.iter().position(|update| {
        (update.number, update.generation) == (page.object_number, page.generation_number)
    });
    let page_object = match page_slot {
        Some(index) => updates[index].object.clone(),
        None => reader.get_object(page.object_number, page.generation_number)?,
    };
    let mut dictionary = page_object
        .as_dict()
        .cloned()
        .ok_or_else(|| invalid("page is not a dictionary"))?;
    let mut contents = match dictionary.get("Contents") {
        Some(value @ PdfObject::Reference { .. }) => vec![value.clone()],
        Some(PdfObject::Array(items)) => items.clone(),
        _ => return Err(invalid("page Contents topology is unsupported")),
    };
    if contents.len() != page.contents.len()
        || contents
            .iter()
            .zip(&page.contents)
            .any(|(object, id)| object.as_reference() != Some(*id))
    {
        return Err(invalid(
            "pending page Contents no longer match source slot identities",
        ));
    }
    let number = reader
        .object_ids()
        .into_iter()
        .map(|(number, _)| number)
        .chain(updates.iter().map(|update| update.number))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| {
            WellfriendError::ResourceLimit("vector occurrence object space exhausted".into())
        })?;
    contents[stream_index] = PdfObject::Reference {
        number,
        generation: 0,
    };
    dictionary.insert(
        "Contents",
        if contents.len() == 1 {
            contents.remove(0)
        } else {
            PdfObject::Array(contents)
        },
    );
    updates[source_slot].number = number;
    updates[source_slot].generation = 0;
    if let Some(index) = page_slot {
        updates[index].object = PdfObject::Dictionary(dictionary);
    } else {
        updates.push(IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dictionary),
        });
    }
    Ok((number, 0))
}

pub(super) fn receipt(
    before: &EditableVectorObject,
    source: ObjectRef,
    clone: ObjectRef,
) -> String {
    format!(
        "page:{} Contents[{}]: {} {} R -> {} {} R; source retained",
        before.provenance.page,
        before.provenance.content_stream_index,
        source.0,
        source.1,
        clone.0,
        clone.1
    )
}

/// Reports must contain a target from the saved revision. Copying the old
/// provenance leaves stale offsets, resource names and parent Form identities.
pub(super) fn rebound(
    output: &[u8],
    before: &EditableVectorObject,
    leaf: ObjectRef,
    start: usize,
    replacement_bytes: usize,
) -> Result<EditableVectorObject> {
    let end = start
        .checked_add(replacement_bytes)
        .ok_or_else(|| invalid("replacement range overflow"))?;
    let inventory = list_vector_objects(output, before.provenance.page)?;
    let observed = inventory
        .objects
        .iter()
        .map(|object| {
            let p = &object.provenance;
            format!(
                "Contents[{}] {} {} R [{}..{}]",
                p.content_stream_index,
                p.object_number,
                p.generation,
                p.operation_byte_start,
                p.operation_byte_end
            )
        })
        .take(32)
        .collect::<Vec<_>>();
    let mut candidates = inventory.objects.into_iter().filter(|object| {
        let p = &object.provenance;
        p.content_stream_index == before.provenance.content_stream_index
            && (p.object_number, p.generation) == leaf
            && p.operation_byte_start >= start
            && p.operation_byte_end <= end
            && (leaf
                != (
                    before.provenance.object_number,
                    before.provenance.generation,
                )
                || (p.form_invocation_path.len() == before.provenance.form_invocation_path.len()
                    && p.form_invocation_path
                        .iter()
                        .zip(&before.provenance.form_invocation_path)
                        .all(|(new, old)| {
                            new.owner_stream_object == old.owner_stream_object
                                && new.owner_stream_generation == old.owner_stream_generation
                                && new.owner_operation_byte_start == old.owner_operation_byte_start
                                && new.form_object == old.form_object
                                && new.form_generation == old.form_generation
                        })))
    });
    let result = candidates.next().ok_or_else(|| {
        invalid(&format!(
            "saved replacement vector was not found at Contents[{}] {} {} R [{}..{}]; observed {}",
            before.provenance.content_stream_index,
            leaf.0,
            leaf.1,
            start,
            end,
            observed.join(", ")
        ))
    })?;
    if candidates.next().is_some() {
        return Err(invalid("saved replacement vector has ambiguous ownership"));
    }
    Ok(result)
}
