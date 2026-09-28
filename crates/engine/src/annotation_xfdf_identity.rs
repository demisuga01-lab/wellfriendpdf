//! Namespace checks for the existing canonical XFDF writer. No automatic NM
//! repair policy is inferred by an interchange import.
use super::*;

pub(super) fn validate_destination_names(
    identities: &crate::annotation_identity::IdentityIndex,
    updates: &BTreeMap<u32, AnnotationXfdfRecord>,
    creates: &[AnnotationXfdfRecord],
    deletes: &BTreeSet<String>,
) -> Result<()> {
    let mut names = BTreeMap::<(usize, String), (usize, bool)>::new();
    for source in identities.values() {
        crate::cancel::check_current_cancel("XFDF annotation name planning")?;
        if deletes.contains(&source.id) {
            continue;
        }
        let Some(name) = &source.name else { continue };
        let page = source
            .reference
            .and_then(|r| updates.get(&r.0))
            .map(|r| r.page)
            .unwrap_or(source.page);
        let state = names.entry((page, name.clone())).or_default();
        state.0 += 1;
        state.1 |= page != source.page;
    }
    for record in creates {
        let name = record.pdf_name.as_ref().unwrap_or(&record.id);
        let state = names.entry((record.page, name.clone())).or_default();
        state.0 += 1;
        state.1 = true;
    }
    if names
        .values()
        .any(|(count, arriving)| *count > 1 && *arriving)
    {
        return Err(WellfriendError::invalid_input("XFDF movement or creation would duplicate a destination-page NM; use explicit name-change approval or choose a distinct name/destination"));
    }
    Ok(())
}
