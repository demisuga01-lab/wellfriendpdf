//! Canonical page-owned annotation relationship graph. Geometry and XFDF use
//! the same interpretation of Popup/Parent, IRT and RT. Widget Parent is never
//! an annotation relationship. Popup Parent is optional.
use crate::annotation_identity::IdentityIndex;
use crate::{PdfDictionary, PdfDocument, PdfObject, PdfReader, Result, WellfriendError};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const MAX_NODES: usize = 100_000;
const MAX_EDGES: usize = 200_000;
const MAX_EDGE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Node {
    pub reference: Option<(u32, u16)>,
    pub page: usize,
    pub order: usize,
    pub subtype: String,
    pub reply_to: Option<String>,
    pub reply_type: Option<String>,
    pub popup: Option<String>,
    /// Effective parent, including ownership inferred from a unique Popup edge.
    pub parent: Option<String>,
    pub parent_explicit: bool,
}
impl Node {
    pub(crate) fn targets(&self) -> impl Iterator<Item = &String> {
        self.reply_to
            .iter()
            .chain(self.popup.iter())
            .chain(self.parent.iter())
    }
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Graph {
    pub nodes: BTreeMap<String, Node>,
}
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}

pub(crate) fn is_markup(subtype: &str) -> bool {
    matches!(
        subtype,
        "Text"
            | "FreeText"
            | "Line"
            | "Square"
            | "Circle"
            | "Polygon"
            | "PolyLine"
            | "Highlight"
            | "Underline"
            | "Squiggly"
            | "StrikeOut"
            | "Caret"
            | "Stamp"
            | "Ink"
            | "FileAttachment"
            | "Sound"
            | "Redact"
    )
}

fn edge(
    dict: &PdfDictionary,
    key: &str,
    refs: &BTreeMap<(u32, u16), String>,
) -> Result<Option<String>> {
    let Some(v) = dict.get(key).filter(|v| !matches!(v, PdfObject::Null)) else {
        return Ok(None);
    };
    let reference = v
        .as_reference()
        .ok_or_else(|| fail("annotation relationship must be an indirect reference"))?;
    refs.get(&reference)
        .cloned()
        .map(Some)
        .ok_or_else(|| fail("annotation relationship target is not uniquely page-owned"))
}

fn name(reader: &PdfReader, dict: &PdfDictionary, key: &str) -> Result<Option<String>> {
    match dict
        .get(key)
        .map(|v| reader.resolve(v.clone()))
        .transpose()?
    {
        None | Some(PdfObject::Null) => Ok(None),
        Some(PdfObject::Name(n)) => Ok(Some(n)),
        _ => Err(fail("annotation relationship type must be a name")),
    }
}

pub(crate) fn subtype(reader: &PdfReader, dict: &PdfDictionary) -> Result<String> {
    Ok(name(reader, dict, "Subtype")?.unwrap_or_else(|| "Unknown".into()))
}

impl Graph {
    pub(crate) fn read(document: &PdfDocument, identities: &IdentityIndex) -> Result<Self> {
        let reader = document.reader();
        let refs = identities
            .values()
            .filter_map(|i| i.reference.map(|r| (r, i.id.clone())))
            .collect::<BTreeMap<_, _>>();
        let mut graph = Self::default();
        for page in document.get_pages()? {
            crate::cancel::check_current_cancel("annotation relationship inventory")?;
            let object = reader.get_object(page.object_number, page.generation_number)?;
            let dict = object
                .as_dict()
                .ok_or_else(|| fail("invalid annotation page"))?;
            let Some(v) = dict.get("Annots") else {
                continue;
            };
            let object = reader.resolve(v.clone())?;
            let values = object
                .as_array()
                .ok_or_else(|| fail("invalid annotation array"))?;
            for (order, v) in values.iter().enumerate() {
                let Some(identity) = identities.get(&(page.page_number, order)) else {
                    continue;
                };
                let object = reader.resolve(v.clone())?;
                let dict = object
                    .as_dict()
                    .ok_or_else(|| fail("annotation identity has no dictionary"))?;
                let subtype = subtype(reader, dict)?;
                let parent = if subtype == "Popup" {
                    edge(dict, "Parent", &refs)?
                } else {
                    None
                };
                graph.nodes.insert(
                    identity.id.clone(),
                    Node {
                        reference: identity.reference,
                        page: identity.page,
                        order,
                        subtype,
                        reply_to: edge(dict, "IRT", &refs)?,
                        reply_type: name(reader, dict, "RT")?,
                        popup: edge(dict, "Popup", &refs)?,
                        parent_explicit: parent.is_some(),
                        parent,
                    },
                );
            }
        }
        // Infer omitted Popup Parent, but never overwrite conflicting ownership.
        let owners = graph
            .nodes
            .iter()
            .filter_map(|(id, n)| n.popup.as_ref().map(|popup| (id.clone(), popup.clone())))
            .collect::<Vec<_>>();
        let mut seen = BTreeSet::new();
        for (owner, popup) in owners {
            if !seen.insert(popup.clone()) {
                return Err(fail("popup has more than one owner"));
            }
            let target = graph
                .nodes
                .get_mut(&popup)
                .ok_or_else(|| fail("popup owner target missing"))?;
            if target.subtype != "Popup" || target.parent.as_ref().is_some_and(|p| p != &owner) {
                return Err(fail("popup Parent disagrees with markup Popup ownership"));
            }
            target.parent = Some(owner);
        }
        graph.validate()?;
        Ok(graph)
    }

    /// Recompute reciprocal Popup edges after explicit popup reparent/delete.
    /// Parentless popups remain parentless; multiple children cannot silently
    /// replace one another in the owner's single Popup slot.
    pub(crate) fn rebuild_popups(&mut self) -> Result<()> {
        for node in self.nodes.values_mut() {
            node.popup = None;
        }
        let parents = self
            .nodes
            .iter()
            .filter(|(_, n)| n.subtype == "Popup")
            .filter_map(|(id, n)| n.parent.as_ref().map(|p| (id.clone(), p.clone())))
            .collect::<Vec<_>>();
        for (popup, parent) in parents {
            let owner=self.nodes.get_mut(&parent).ok_or_else(||fail("surviving popup still names a deleted or missing parent; reparent or delete it explicitly"))?;
            if owner.popup.replace(popup).is_some() {
                return Err(fail("more than one popup requested for one owner"));
            }
        }
        Ok(())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.nodes.len() > MAX_NODES {
            return Err(fail("annotation relationship node budget exceeded"));
        }
        let mut edge_count = 0usize;
        let mut bytes = 0usize;
        for (id, node) in &self.nodes {
            crate::cancel::check_current_cancel("annotation relationship validation")?;
            bytes = bytes.saturating_add(id.len());
            for target in node.targets() {
                edge_count += 1;
                bytes = bytes.saturating_add(target.len());
                let other=self.nodes.get(target).ok_or_else(||fail("dangling annotation relationship; explicitly delete or reparent dependents"))?;
                if id == target || other.page != node.page {
                    return Err(fail(
                        "annotation relationship is self-referential or crosses page ownership",
                    ));
                }
            }
            if edge_count > MAX_EDGES || bytes > MAX_EDGE_BYTES {
                return Err(fail("annotation relationship edge budget exceeded"));
            }
            if let Some(reply) = &node.reply_to {
                let target = &self.nodes[reply];
                if !is_markup(&node.subtype) || !is_markup(&target.subtype) {
                    return Err(fail("reply relationships require markup annotations"));
                }
                let rt = node.reply_type.as_deref().unwrap_or("R");
                if !matches!(rt, "R" | "Group") || rt == "Group" && target.reply_to.is_some() {
                    return Err(fail("invalid reply type or non-primary Group target"));
                }
            } else if node.reply_type.is_some() {
                return Err(fail("RT without IRT is invalid"));
            }
            if let Some(popup) = &node.popup {
                let target = &self.nodes[popup];
                if !is_markup(&node.subtype)
                    || target.subtype != "Popup"
                    || target.parent.as_ref() != Some(id)
                {
                    return Err(fail("invalid reciprocal Popup ownership"));
                }
            }
            if node.subtype == "Popup" {
                if let Some(parent) = &node.parent {
                    if self.nodes[parent].popup.as_ref() != Some(id)
                        || !is_markup(&self.nodes[parent].subtype)
                    {
                        return Err(fail("popup Parent has no reciprocal markup owner"));
                    }
                } else if node.parent_explicit {
                    return Err(fail("popup has explicit but missing parent"));
                }
            } else if node.parent.is_some() || node.parent_explicit {
                return Err(fail("non-popup annotation has popup parent semantics"));
            }
        }
        // A functional IRT graph has out-degree <= 1. Global completed nodes
        // avoid walking every reply's full ancestry (quadratic on long threads).
        let mut done = BTreeSet::new();
        for start in self.nodes.keys() {
            if done.contains(start) {
                continue;
            }
            let mut path = BTreeSet::new();
            let mut cursor = start;
            loop {
                crate::cancel::check_current_cancel("annotation reply cycle validation")?;
                if done.contains(cursor) {
                    break;
                }
                if !path.insert(cursor.clone()) {
                    return Err(fail("cyclic annotation reply graph"));
                }
                match &self.nodes[cursor].reply_to {
                    Some(next) => cursor = next,
                    None => break,
                }
            }
            done.extend(path);
        }
        Ok(())
    }
}
