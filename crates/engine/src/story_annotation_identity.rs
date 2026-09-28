//! Editing identities are not PDF page-local annotation names. Allocate exact
//! source-object identities, persist selected identities before rewriting, and
//! plan explicitly approved destination-name repairs independently of geometry.
use super::*;

pub(super) use crate::annotation_identity::text_string;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryAnnotationNameChange {
    pub previous: String,
    pub replacement: String,
}

/// Simulate final page namespaces before mutation. Reserve unchanged resident
/// names first; arrivals are processed in original annotation painting order.
/// Generated page insertion changes ordinal positions, not page-local identity.
pub(super) fn plan_names(
    entries: &BTreeMap<String, Entry>,
    moves: &mut [StoryAnnotationMove],
    insertion: Option<(usize, usize)>,
    unrepresented_names: &[(usize, String)],
) -> Result<()> {
    let shift = |page: usize| -> Result<usize> {
        let (after, count) = insertion.unwrap_or((0, 0));
        if page > after {
            page.checked_add(count)
                .ok_or_else(|| fail("annotation page index overflow"))
        } else {
            Ok(page)
        }
    };
    let selected = moves
        .iter()
        .map(|m| (m.annotation_id.clone(), m.target_page))
        .collect::<BTreeMap<_, _>>();
    if selected.len() != moves.len() {
        return Err(fail("duplicate annotation movement"));
    }
    let mut occupied = BTreeMap::<usize, BTreeSet<String>>::new();
    // Entries without editable Rect still reserve the page-local namespace.
    for (page, name) in unrepresented_names {
        occupied
            .entry(shift(*page)?)
            .or_default()
            .insert(name.clone());
    }
    for (id, entry) in entries {
        crate::cancel::check_current_cancel("annotation destination namespace")?;
        let page = shift(entry.source.page)?;
        if selected.get(id).is_none_or(|&target| target == page) {
            if let Some(name) = &entry.source.name {
                occupied.entry(page).or_default().insert(name.clone());
            }
        }
    }
    let mut order = (0..moves.len()).collect::<Vec<_>>();
    for m in moves.iter_mut() {
        m.name_change = None;
        if !entries.contains_key(&m.annotation_id) {
            return Err(fail("name planning source missing"));
        }
    }
    // A generated repair must not consume a later arrival's existing name.
    // Reserve the final namespace before assigning any generated names.
    let mut reserved = occupied.clone();
    for m in moves.iter() {
        if let Some(name) = &entries[&m.annotation_id].source.name {
            reserved
                .entry(m.target_page)
                .or_default()
                .insert(name.clone());
        }
    }
    order.sort_by_key(|&i| {
        let e = &entries[&moves[i].annotation_id];
        (e.source.page, e.annotation_order)
    });
    for i in order {
        crate::cancel::check_current_cancel("annotation destination naming")?;
        let movement = &mut moves[i];
        let entry = &entries[&movement.annotation_id];
        if movement.target_page == shift(entry.source.page)? {
            continue;
        }
        let Some(name) = &entry.source.name else {
            continue;
        };
        let used = occupied.entry(movement.target_page).or_default();
        if used.insert(name.clone()) {
            continue;
        }
        let base = format!(
            "WFStory-{:x}",
            Sha256::digest(movement.annotation_id.as_bytes())
        );
        let mut replacement = base.clone();
        let mut suffix = 0usize;
        let target_reserved = reserved.entry(movement.target_page).or_default();
        while !target_reserved.insert(replacement.clone()) {
            suffix += 1;
            replacement = format!("{base}-{suffix}");
        }
        used.insert(replacement.clone());
        movement.name_change = Some(StoryAnnotationNameChange {
            previous: name.clone(),
            replacement,
        });
    }
    Ok(())
}

pub(super) fn validate_names(
    entries: &BTreeMap<String, Entry>,
    moves: &[StoryAnnotationMove],
    unrepresented_names: &[(usize, String)],
) -> Result<()> {
    let mut planned = moves.to_vec();
    plan_names(entries, &mut planned, None, unrepresented_names)?;
    if planned
        .iter()
        .zip(moves)
        .any(|(a, b)| a.name_change != b.name_change)
    {
        return Err(fail(
            "annotation destination names differ from the approved preview",
        ));
    }
    Ok(())
}
