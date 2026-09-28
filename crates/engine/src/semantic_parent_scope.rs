//! Reciprocal content ownership for ParentTree recovery. A number-tree slot is
//! not sufficient evidence to attach an unrelated page/Form's MCID to a role.
use crate::error::{Result, WellfriendError};
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;
use crate::text::MarkedContentId;
use std::collections::HashSet;

type ObjectId = (u32, u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BindingEvidence {
    Matched,
    MissingKids,
    Conflicting,
}

pub(super) fn binding(
    reader: &PdfReader,
    element: &PdfObject,
    page: ObjectId,
    page_streams: &[ObjectId],
    target: MarkedContentId,
    unambiguous_owner: bool,
    appearance_owners: &mut crate::annotation_appearance::AppearanceOwnerIndex,
) -> Result<BindingEvidence> {
    let object = reader.resolve(element.clone())?;
    let Some(dict) = object.as_dict() else {
        return Ok(BindingEvidence::MissingKids);
    };
    let Some(kids) = dict.get("K").filter(|object| !object.is_null()) else {
        return Ok(BindingEvidence::MissingKids);
    };
    let inherited_page = optional_reference(dict, "Pg")?;
    let mut walk = Walk {
        reader,
        page,
        page_streams,
        target,
        unambiguous_owner,
        appearance_owners,
        active: HashSet::new(),
        visited: 0,
    };
    Ok(if walk.matches(kids, inherited_page, 0)? {
        BindingEvidence::Matched
    } else {
        BindingEvidence::Conflicting
    })
}

fn optional_reference(dict: &PdfDictionary, key: &str) -> Result<Option<ObjectId>> {
    match dict.get(key) {
        None | Some(PdfObject::Null) => Ok(None),
        Some(value) => value.as_reference().map(Some).ok_or_else(|| {
            WellfriendError::MalformedPdf(format!("structure {key} is not an indirect reference"))
        }),
    }
}

struct Walk<'a> {
    reader: &'a PdfReader,
    page: ObjectId,
    page_streams: &'a [ObjectId],
    target: MarkedContentId,
    unambiguous_owner: bool,
    appearance_owners: &'a mut crate::annotation_appearance::AppearanceOwnerIndex,
    active: HashSet<ObjectId>,
    visited: usize,
}

impl Walk<'_> {
    fn matches(
        &mut self,
        object: &PdfObject,
        inherited_page: Option<ObjectId>,
        depth: usize,
    ) -> Result<bool> {
        crate::cancel::check_current_cancel("ParentTree reciprocal content binding")?;
        self.visited += 1;
        if depth > 128 || self.visited > 250_000 {
            return Err(WellfriendError::ResourceLimit(
                "ParentTree reciprocal content traversal limit".into(),
            ));
        }
        match object {
            PdfObject::Reference { number, generation } => {
                let id = (*number, *generation);
                if !self.active.insert(id) {
                    return Err(WellfriendError::MalformedPdf(
                        "cyclic structure content binding".into(),
                    ));
                }
                let value = self.reader.get_object(id.0, id.1)?;
                let matched = self.matches(&value, inherited_page, depth + 1);
                self.active.remove(&id);
                matched
            }
            PdfObject::Integer(mcid) => Ok(inherited_page == Some(self.page)
                && self.target.stream.is_none()
                && *mcid == self.target.mcid),
            PdfObject::Array(items) => {
                for item in items {
                    if self.matches(item, inherited_page, depth + 1)? {
                        return Ok(true);
                    }
                }
                Ok(false)
            }
            PdfObject::Dictionary(dict) => {
                // A child structure element owns its own K. Do not descend and
                // mistake the grandchild's content for the proposed parent.
                if dict.get("S").is_some() || dict.get_name("Type") == Some("StructElem") {
                    return Ok(false);
                }
                if dict.get_integer("MCID") != Some(self.target.mcid) {
                    return Ok(false);
                }
                let page = optional_reference(dict, "Pg")?.or(inherited_page);
                if page != Some(self.page) {
                    return Ok(false);
                }
                let owner = optional_reference(dict, "StmOwn")?;
                let stream =
                    optional_reference(dict, "Stm")?.filter(|id| !self.page_streams.contains(id));
                if stream != self.target.stream {
                    return Ok(false);
                }
                if let Some(owner) = owner {
                    let Some(stream) = stream else {
                        return Ok(false);
                    };
                    self.appearance_owners
                        .validate(self.reader, self.page, owner, stream)?;
                    Ok(Some(owner) == self.target.stream_owner)
                } else {
                    Ok(self.target.stream_owner.is_none() || self.unambiguous_owner)
                }
            }
            _ => Ok(false),
        }
    }
}
