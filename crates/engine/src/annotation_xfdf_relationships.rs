//! Plan final relationship state before object allocation. Reciprocal-only
//! updates preserve the owner's original dictionary instead of importing it.
use super::*;
use crate::annotation_relationships::{Graph, Node};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnnotationRelationshipChange {
    pub annotation_id: String,
    pub key: String,
    pub previous: Option<String>,
    pub replacement: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnnotationRelationshipReport {
    #[serde(default)]
    pub promoted_source_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_normalization: Option<AnnotationPromotionReport>,
    pub changes: Vec<AnnotationRelationshipChange>,
    pub source_identities_persisted: Vec<String>,
    pub reciprocal_only_updates: Vec<String>,
    pub output_graph_verified: bool,
    pub tagged_ownership_rebuilt: bool,
}

pub(super) struct Plan {
    before: Graph,
    after: Graph,
    affected: BTreeSet<String>,
    pub report: AnnotationRelationshipReport,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}
fn keys(node: Option<&Node>) -> [Option<String>; 4] {
    node.map(|n| {
        [
            n.reply_to.clone(),
            n.reply_type.clone(),
            n.popup.clone(),
            n.parent.clone(),
        ]
    })
    .unwrap_or_default()
}
impl Plan {
    pub(super) fn new(
        document: &PdfDocument,
        ids: &crate::annotation_identity::IdentityIndex,
        updates: &BTreeMap<u32, AnnotationXfdfRecord>,
        creates: &[AnnotationXfdfRecord],
        deletes: &BTreeSet<String>,
        policy: &AnnotationConflictPolicy,
    ) -> Result<Self> {
        let before = Graph::read(document, ids)?;
        let mut after = before.clone();
        let mut affected = BTreeSet::new();
        for id in deletes {
            if let Some(node) = after.nodes.remove(id) {
                if node.subtype == "Widget" {
                    return Err(fail("XFDF annotation deletion cannot orphan an AcroForm widget; use canonical field deletion"));
                }
                affected.insert(id.clone());
            }
        }
        let mut explicit = BTreeSet::new();
        for record in updates.values().chain(creates) {
            crate::cancel::check_current_cancel("XFDF relationship planning")?;
            affected.insert(record.id.clone());
            explicit.insert(record.id.clone());
            let node = after
                .nodes
                .entry(record.id.clone())
                .or_insert_with(|| Node {
                    reference: None,
                    page: record.page,
                    order: usize::MAX,
                    subtype: record.subtype.clone(),
                    reply_to: None,
                    reply_type: None,
                    popup: None,
                    parent: None,
                    parent_explicit: false,
                });
            if node.subtype != record.subtype {
                return Err(fail(
                    "XFDF update cannot change an existing annotation subtype",
                ));
            }
            node.page = record.page;
            if record.clear_reply {
                if record.reply_to.is_some() || record.reply_type.is_some() {
                    return Err(fail("clear-reply conflicts with explicit reply fields"));
                }
                node.reply_to = None;
                node.reply_type = None;
            } else if *policy == AnnotationConflictPolicy::Replace {
                node.reply_to = record.reply_to.clone();
                node.reply_type = record.reply_type.clone();
            } else {
                if record.reply_to.is_some() {
                    node.reply_to = record.reply_to.clone();
                }
                if record.reply_type.is_some() {
                    node.reply_type = record.reply_type.clone();
                }
            }
            if record.detach_popup {
                if node.subtype != "Popup" || record.popup_for.is_some() {
                    return Err(fail(
                        "detach-popup requires a Popup without a simultaneous new parent",
                    ));
                }
                node.parent = None;
                node.parent_explicit = false;
            }
            if let Some(parent) = &record.popup_for {
                if node.subtype != "Popup" {
                    return Err(fail("popup-for is valid only on a Popup annotation"));
                }
                node.parent = Some(parent.clone());
                node.parent_explicit = true;
            }
        }
        if after.nodes.len() > MAX_ANNOTATIONS {
            return Err(fail(
                "XFDF resulting annotation count exceeds inventory budget",
            ));
        }
        after.rebuild_popups()?;
        after.validate()?;
        // Closure includes old and new neighbors. This persists the identities
        // used in the relationship receipt, including anonymous counterparties.
        let mut adjacent = BTreeMap::<String, BTreeSet<String>>::new();
        for graph in [&before, &after] {
            for (id, node) in &graph.nodes {
                for target in node.targets() {
                    adjacent
                        .entry(id.clone())
                        .or_default()
                        .insert(target.clone());
                    adjacent
                        .entry(target.clone())
                        .or_default()
                        .insert(id.clone());
                }
            }
        }
        let mut pending = affected.iter().cloned().collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            crate::cancel::check_current_cancel("XFDF relationship dependency closure")?;
            if let Some(neighbors) = adjacent.get(&id) {
                for neighbor in neighbors {
                    if affected.insert(neighbor.clone()) {
                        pending.push(neighbor.clone());
                    }
                }
            }
        }
        let mut report = AnnotationRelationshipReport::default();
        for id in &affected {
            let old = before.nodes.get(id);
            let new = after.nodes.get(id);
            if old.is_some_and(|n| n.reference.is_none()) {
                return Err(fail("changing a relationship component with a direct annotation requires explicit promotion"));
            }
            for ((key, previous), replacement) in ["IRT", "RT", "Popup", "Parent"]
                .into_iter()
                .zip(keys(old))
                .zip(keys(new))
            {
                if previous != replacement {
                    report.changes.push(AnnotationRelationshipChange {
                        annotation_id: id.clone(),
                        key: key.into(),
                        previous,
                        replacement,
                    });
                }
            }
            if old.is_some() && new.is_some() {
                report.source_identities_persisted.push(id.clone());
                if !explicit.contains(id) {
                    report.reciprocal_only_updates.push(id.clone());
                }
            }
        }
        Ok(Self {
            before,
            after,
            affected,
            report,
        })
    }

    pub(super) fn patches(
        &self,
        outputs: &BTreeMap<String, u32>,
    ) -> Result<BTreeMap<u32, (PdfDictionary, bool)>> {
        // These small dictionaries only contain canonical relationship keys
        // and identity. Their absence is handled by install(), not merge guesswork.
        let mut result = BTreeMap::new();
        for id in &self.affected {
            let Some(node) = self.after.nodes.get(id) else {
                continue;
            };
            let Some(reference) = self.before.nodes.get(id).and_then(|n| n.reference) else {
                continue;
            };
            let mut patch = PdfDictionary::empty();
            self.install(id, &mut patch, outputs)?;
            result.insert(reference.0, (patch, node.subtype == "Popup"));
        }
        Ok(result)
    }

    pub(super) fn install(
        &self,
        id: &str,
        dict: &mut PdfDictionary,
        outputs: &BTreeMap<String, u32>,
    ) -> Result<()> {
        let node = self
            .after
            .nodes
            .get(id)
            .ok_or_else(|| fail("missing final annotation relationship node"))?;
        for (key, target) in [
            ("IRT", node.reply_to.as_ref()),
            ("Popup", node.popup.as_ref()),
        ] {
            set_ref(dict, key, target, outputs)?;
        }
        match &node.reply_type {
            Some(rt) => {
                dict.insert("RT", PdfObject::Name(rt.clone()));
            }
            None => {
                dict.remove("RT");
            }
        }
        if node.subtype == "Popup" {
            set_ref(
                dict,
                "Parent",
                if node.parent_explicit {
                    node.parent.as_ref()
                } else {
                    None
                },
                outputs,
            )?;
        }
        dict.insert(
            crate::annotation_identity::STABLE_ID,
            crate::annotation_identity::text_string(id),
        );
        Ok(())
    }

    pub(super) fn incoming(
        &self,
        updates: &BTreeMap<u32, AnnotationXfdfRecord>,
        outputs: &BTreeMap<String, u32>,
    ) -> BTreeMap<usize, Vec<u32>> {
        let mut rows = Vec::new();
        for (id, node) in &self.before.nodes {
            let Some(reference) = node.reference else {
                continue;
            };
            if let Some(record) = updates.get(&reference.0).filter(|r| r.page != node.page) {
                rows.push((record.page, node.page, node.order, outputs[id]));
            }
        }
        rows.sort_unstable();
        let mut result = BTreeMap::<usize, Vec<u32>>::new();
        for (destination, _, _, number) in rows {
            result.entry(destination).or_default().push(number);
        }
        result
    }

    pub(super) fn verify(
        &mut self,
        document: &PdfDocument,
        identities: &crate::annotation_identity::IdentityIndex,
        outputs: &BTreeMap<String, u32>,
    ) -> Result<()> {
        let observed = Graph::read(document, identities)?;
        if observed.nodes.len() != self.after.nodes.len() {
            return Err(fail("annotation relationship output cardinality differs"));
        }
        let by_ref = observed
            .nodes
            .values()
            .filter_map(|n| n.reference.map(|r| (r, n)))
            .collect::<BTreeMap<_, _>>();
        let actual_ref = |id: &String| -> Result<(u32, u16)> {
            observed
                .nodes
                .get(id)
                .and_then(|n| n.reference)
                .ok_or_else(|| fail("saved relationship refers to a non-indirect annotation"))
        };
        let mut paint_order = BTreeMap::<usize, Vec<((u8, usize, usize, String), usize)>>::new();
        for (id, expected) in &self.after.nodes {
            crate::cancel::check_current_cancel("XFDF saved relationship verification")?;
            let Some(&number) = outputs.get(id) else {
                continue;
            }; // Untouched direct source.
            let actual = by_ref
                .get(&(number, 0))
                .ok_or_else(|| fail("saved annotation relationship source missing"))?;
            let rank = match self.before.nodes.get(id) {
                Some(before) => (
                    (before.page != expected.page) as u8,
                    before.page,
                    before.order,
                    id.clone(),
                ),
                None => (2, expected.page, usize::MAX, id.clone()),
            };
            paint_order
                .entry(expected.page)
                .or_default()
                .push((rank, actual.order));
            if actual.page != expected.page
                || actual.subtype != expected.subtype
                || actual.reply_type != expected.reply_type
                || actual.parent_explicit != expected.parent_explicit
            {
                return Err(fail("saved annotation relationship metadata differs"));
            }
            for (expected, actual) in [
                (expected.reply_to.as_ref(), actual.reply_to.as_ref()),
                (expected.popup.as_ref(), actual.popup.as_ref()),
                (expected.parent.as_ref(), actual.parent.as_ref()),
            ] {
                let wanted = expected
                    .map(|id| {
                        outputs
                            .get(id)
                            .copied()
                            .map(|n| (n, 0))
                            .ok_or_else(|| fail("relationship target allocation missing"))
                    })
                    .transpose()?;
                let found = actual.map(&actual_ref).transpose()?;
                if wanted != found {
                    return Err(fail("saved annotation relationship target differs"));
                }
            }
        }
        for rows in paint_order.values_mut() {
            rows.sort_by(|a, b| a.0.cmp(&b.0));
            if rows.windows(2).any(|pair| pair[0].1 >= pair[1].1) {
                return Err(fail(
                    "saved annotation paint order differs from source-preserving transaction order",
                ));
            }
        }
        let saved_ids = identities
            .values()
            .map(|i| (i.id.as_str(), i.reference))
            .collect::<BTreeMap<_, _>>();
        for id in &self.affected {
            if self.after.nodes.contains_key(id)
                && saved_ids.get(id.as_str()).copied().flatten() != outputs.get(id).map(|n| (*n, 0))
            {
                return Err(fail("relationship participant identity did not persist"));
            }
        }
        self.report.output_graph_verified = true;
        Ok(())
    }
}

fn set_ref(
    dict: &mut PdfDictionary,
    key: &str,
    target: Option<&String>,
    outputs: &BTreeMap<String, u32>,
) -> Result<()> {
    if let Some(target) = target {
        let number = *outputs
            .get(target)
            .ok_or_else(|| fail("annotation relationship target was deleted or not allocated"))?;
        dict.insert(
            key,
            PdfObject::Reference {
                number,
                generation: 0,
            },
        );
    } else {
        dict.remove(key);
    }
    Ok(())
}

pub(super) fn apply_patch(dict: &mut PdfDictionary, patch: &PdfDictionary, is_popup: bool) {
    for key in ["IRT", "RT", "Popup", crate::annotation_identity::STABLE_ID] {
        match patch.get(key) {
            Some(value) => {
                dict.insert(key, value.clone());
            }
            None => {
                dict.remove(key);
            }
        }
    }
    if is_popup {
        match patch.get("Parent") {
            Some(value) => {
                dict.insert("Parent", value.clone());
            }
            None => {
                dict.remove("Parent");
            }
        }
    }
}
