//! One source identity resolver for native story editing, XFDF and annotation
//! appearance tools. PDF NM is page-local; an editing ID is document-wide.
use crate::reader::PdfReader;
use crate::{PdfDictionary, PdfDocument, PdfObject, Result, WellfriendError};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const STABLE_ID: &str = "WFStoryAnnotationID";
#[derive(Debug, Clone)]
pub(crate) struct AnnotationIdentity {
    pub id: String,
    pub name: Option<String>,
    pub reference: Option<(u32, u16)>,
    pub page: usize,
    pub provenance: &'static str,
}
pub(crate) type IdentityIndex = BTreeMap<(usize, usize), AnnotationIdentity>;
fn fail(s: &str) -> WellfriendError {
    WellfriendError::invalid_input(s)
}

pub(crate) fn text_id(
    reader: &PdfReader,
    dict: &PdfDictionary,
    key: &str,
) -> Result<Option<String>> {
    let Some(v) = dict.get(key) else {
        return Ok(None);
    };
    let v = reader.resolve(v.clone())?;
    if matches!(&v,PdfObject::String(b) if b.len()>16_384)
        || matches!(&v,PdfObject::Name(n) if n.len()>4096)
    {
        return Err(fail("annotation encoded identity budget exceeded"));
    }
    let text = match v {
        PdfObject::Null => return Ok(None),
        PdfObject::String(b) => crate::info::decode_pdf_text_string(&b),
        PdfObject::Name(n) => n,
        _ => return Err(fail("annotation identity must be a text string or name")),
    };
    if text.len() > 4096 {
        return Err(fail("annotation identity budget exceeded"));
    }
    Ok((!text.is_empty()).then_some(text))
}
pub(crate) fn text_string(text: &str) -> PdfObject {
    let mut bytes = vec![0xfe, 0xff];
    bytes.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    PdfObject::String(bytes)
}
pub(crate) fn stamp(
    reader: &PdfReader,
    dict: &PdfDictionary,
    id: &str,
) -> Result<Option<PdfDictionary>> {
    if let Some(existing) = text_id(reader, dict, STABLE_ID)? {
        if existing != id {
            return Err(fail("annotation identity changed before staging"));
        }
        return Ok(None);
    }
    let mut dict = dict.clone();
    dict.insert(STABLE_ID, text_string(id));
    Ok(Some(dict))
}

pub(crate) fn index(document: &PdfDocument, limit: usize) -> Result<IdentityIndex> {
    let reader = document.reader();
    let revision = format!("{:x}", Sha256::digest(reader.file_bytes()));
    let mut pending = Vec::new();
    let mut references = BTreeSet::new();
    let mut names = BTreeMap::<String, usize>::new();
    let mut stored = BTreeSet::new();
    let mut reserved = BTreeSet::new();
    let mut bytes = 0usize;
    let mut visits = 0usize;
    for page in document.get_pages()? {
        crate::cancel::check_current_cancel("annotation source identity inventory")?;
        let p = reader.get_object(page.object_number, page.generation_number)?;
        let dict = p.as_dict().ok_or_else(|| fail("invalid annotation page"))?;
        let Some(annots) = dict.get("Annots") else {
            continue;
        };
        let annots = reader.resolve(annots.clone())?;
        let annots = annots
            .as_array()
            .ok_or_else(|| fail("invalid Annots array"))?;
        visits = visits.saturating_add(annots.len());
        if visits > limit {
            return Err(fail("annotation identity inventory budget exceeded"));
        }
        for (ordinal, value) in annots.iter().enumerate() {
            crate::cancel::check_current_cancel("annotation identity allocation")?;
            let reference = value.as_reference();
            if let Some(r) = reference {
                if !references.insert(r) {
                    return Err(fail("annotation object has duplicate page ownership"));
                }
            }
            let object = reader.resolve(value.clone())?;
            let Some(dict) = object.as_dict() else {
                continue;
            };
            let name = text_id(reader, dict, "NM")?;
            let persisted = text_id(reader, dict, STABLE_ID)?;
            if let Some(n) = &name {
                *names.entry(n.clone()).or_default() += 1;
                reserved.insert(n.clone());
                bytes = bytes.saturating_add(n.len());
            }
            if let Some(id) = &persisted {
                if !stored.insert(id.clone()) {
                    return Err(fail("persisted annotation identity has multiple owners; explicit import remapping is required"));
                }
                reserved.insert(id.clone());
                bytes = bytes.saturating_add(id.len());
            }
            if bytes > 16 * 1024 * 1024 {
                return Err(fail("annotation identity byte budget exceeded"));
            }
            pending.push((
                page.page_number,
                (page.object_number, page.generation_number),
                ordinal,
                reference,
                name,
                persisted,
            ));
        }
    }
    let mut result = BTreeMap::new();
    for (page, page_ref, ordinal, reference, name, persisted) in pending {
        crate::cancel::check_current_cancel("annotation identity allocation")?;
        let legacy = name
            .as_ref()
            .filter(|n| names[*n] == 1 && !stored.contains(*n));
        let (id, provenance) = if let Some(id) = persisted {
            (id, "persisted_editing_id")
        } else if let Some(name) = legacy {
            (name.clone(), "pdf_nm_preserved")
        } else {
            let (prefix, seed) = if let Some((number, generation)) = reference {
                (
                    if name.is_some() {
                        "wf-object"
                    } else {
                        "wf-anonymous"
                    },
                    format!("annotation-v1:{revision}:{number}:{generation}"),
                )
            } else {
                (
                    "wf-direct",
                    format!(
                        "annotation-direct-v1:{revision}:{}:{}:{ordinal}",
                        page_ref.0, page_ref.1
                    ),
                )
            };
            let base = format!("{prefix}:{:x}", Sha256::digest(seed.as_bytes()));
            let mut id = base.clone();
            let mut suffix = 0usize;
            while reserved.contains(&id) {
                suffix += 1;
                id = format!("{base}-{suffix}");
            }
            reserved.insert(id.clone());
            bytes = bytes.saturating_add(id.len());
            (id, "generated_stable_id")
        };
        if bytes > 16 * 1024 * 1024 {
            return Err(fail("annotation identity byte budget exceeded"));
        }
        result.insert(
            (page, ordinal),
            AnnotationIdentity {
                id,
                name,
                reference,
                page,
                provenance,
            },
        );
    }
    Ok(result)
}

/// Resolve external NM aliases once without quadratic scans. Revision-bound
/// SDK records select exact editing IDs; unbound external records may use a
/// unique raw name, but never silently choose between multiple occurrences.
pub(crate) struct NameLookup {
    ids: BTreeSet<String>,
    names: BTreeMap<String, BTreeSet<String>>,
}
impl NameLookup {
    pub(crate) fn new(index: &IdentityIndex) -> Self {
        let mut result = Self {
            ids: BTreeSet::new(),
            names: BTreeMap::new(),
        };
        for i in index.values() {
            result.ids.insert(i.id.clone());
            if let Some(name) = &i.name {
                result
                    .names
                    .entry(name.clone())
                    .or_default()
                    .insert(i.id.clone());
            }
        }
        result
    }
    pub(crate) fn resolve(&self, id: &str, bound: bool) -> Result<String> {
        let exact = self.ids.contains(id);
        if !exact
            && [
                "wf-object:",
                "wf-anonymous:",
                "wf-direct:",
                "wellfriendpdf-p17-",
            ]
            .iter()
            .any(|p| id.starts_with(p))
        {
            return Err(fail("unknown revision-scoped annotation ID; re-export instead of recreating a stale source occurrence"));
        }
        let aliases = self.names.get(id);
        if bound {
            if !exact && aliases.is_some() {
                return Err(fail(
                    "bound XFDF requires the exact source editing ID, not a page-local NM alias",
                ));
            }
            return Ok(id.into());
        }
        if let Some(aliases) = aliases {
            if aliases.len() != 1 || exact && !aliases.contains(id) {
                return Err(fail("ambiguous page-local annotation name; use a current revision-bound export and its editing identity"));
            }
            return Ok(aliases.iter().next().unwrap().clone());
        }
        Ok(id.into())
    }
}
