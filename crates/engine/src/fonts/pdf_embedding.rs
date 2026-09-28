//! Shared editable sfnt classification and CFF character-code/CID/GID identity.
//! CFF charset lookup uses ttf-parser; the small DICT reader below only reads
//! ROS metadata not exposed by that crate. It never interprets CharStrings.
use crate::{PdfDictionary, PdfObject, Result, WellfriendError};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[path = "cff_standard_strings.rs"]
mod standard_strings;
#[cfg(test)]
#[path = "pdf_embedding_tests.rs"]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CidSystem {
    pub registry: Vec<u8>,
    pub ordering: Vec<u8>,
    pub supplement: u32,
}
impl Default for CidSystem {
    fn default() -> Self {
        Self {
            registry: b"Adobe".to_vec(),
            ordering: b"Identity".to_vec(),
            supplement: 0,
        }
    }
}
impl CidSystem {
    pub fn dictionary(&self) -> PdfDictionary {
        let mut dict = PdfDictionary::empty();
        dict.insert("Registry", PdfObject::String(self.registry.clone()));
        dict.insert("Ordering", PdfObject::String(self.ordering.clone()));
        dict.insert("Supplement", PdfObject::Integer(i64::from(self.supplement)));
        dict
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CffIdentity {
    pub system: CidSystem,
    pub gid_to_cid: Vec<u16>,
    pub postscript_name: String,
}

#[derive(Debug, Clone)]
pub(crate) struct EmbeddingInfo {
    pub cff: Option<CffIdentity>,
    pub may_subset: bool,
}

fn fail(message: &str) -> WellfriendError {
    WellfriendError::invalid_input(message)
}

/// Return the serialized OpenType OS/2 `fsType` value. Keeping this parsing in
/// one place prevents authoring, fallback selection, and static instancing
/// from applying different interpretations of the same licensing bits.
pub(crate) fn serialized_fs_type(face: &ttf_parser::Face<'_>) -> Option<u16> {
    face.raw_face()
        .table(ttf_parser::Tag::from_bytes(b"OS/2"))
        .and_then(|table| table.get(8..10))
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub(crate) fn editable_outline_embedding_allowed(face: &ttf_parser::Face<'_>) -> bool {
    let Some(fs_type) = serialized_fs_type(face) else {
        return true;
    };
    // Bits 0..3 are mutually exclusive embedding levels. Installable (0) and
    // editable (8) are accepted by an editor; restricted (2) and
    // preview/print (4) are not. Bit 9 permits bitmap strikes only and therefore
    // cannot authorize embedding outlines, even when combined with editable.
    matches!(fs_type & 0x000f, 0 | 0x0008) && fs_type & 0x0200 == 0
}

pub(crate) fn serialized_subsetting_allowed(face: &ttf_parser::Face<'_>) -> bool {
    serialized_fs_type(face).is_none_or(|fs_type| fs_type & 0x0100 == 0)
}

impl EmbeddingInfo {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        crate::cancel::check_current_cancel("editable font program classification")?;
        if bytes.len() > 256 * 1024 * 1024 {
            return Err(WellfriendError::ResourceLimit(
                "embedded font exceeds 256 MiB".into(),
            ));
        }
        if bytes.starts_with(b"ttcf") {
            return Err(WellfriendError::UnsupportedFeature(
                "select and prepare a collection face with fonts::font_asset before editable embedding".into(),
            ));
        }
        let face =
            ttf_parser::Face::parse(bytes, 0).map_err(|_| fail("invalid editable sfnt program"))?;
        if face
            .raw_face()
            .table(ttf_parser::Tag::from_bytes(b"CFF2"))
            .is_some()
        {
            return Err(WellfriendError::UnsupportedFeature(
                "CFF2 must be instantiated as an editable standalone CFF1 or glyf program".into(),
            ));
        }
        if !editable_outline_embedding_allowed(&face) {
            return Err(WellfriendError::UnsupportedFeature(
                "font permissions do not allow editable outline embedding".into(),
            ));
        }
        let cff = if face.tables().glyf.is_some() && face.tables().cff.is_none() {
            None
        } else if bytes.starts_with(b"OTTO")
            && face.tables().glyf.is_none()
            && face.tables().cff.is_some()
        {
            let table = face
                .raw_face()
                .table(ttf_parser::Tag::from_bytes(b"CFF "))
                .ok_or_else(|| fail("missing CFF table"))?;
            let identity = CffIdentity::parse(table)?;
            if identity.gid_to_cid.len() != usize::from(face.number_of_glyphs()) {
                return Err(fail("CFF and maxp glyph counts differ"));
            }
            Some(identity)
        } else {
            return Err(WellfriendError::UnsupportedFeature(
                "editable embedding requires standalone glyf or OpenType CFF1 outlines".into(),
            ));
        };
        // Read the serialized OS/2 fsType value directly for the PDF writer's
        // subsetting decision.  Some font-parser versions intentionally fold
        // licensing bits into a higher-level policy and have historically
        // reported `true` for editable fonts carrying bit 8 (`NoSubsetting`).
        // PDF embedding must preserve that independent bit exactly: editable
        // embedding does not imply permission to subset the font program.
        let may_subset = serialized_subsetting_allowed(&face);
        Ok(Self { cff, may_subset })
    }

    pub fn cid(&self, character_code: u16, gid: u16) -> Result<u16> {
        match &self.cff {
            Some(cff) => cff
                .gid_to_cid
                .get(usize::from(gid))
                .copied()
                .ok_or_else(|| fail("embedded CFF glyph is outside charset")),
            None => Ok(character_code),
        }
    }
    pub fn system(&self) -> CidSystem {
        self.cff
            .as_ref()
            .map_or_else(CidSystem::default, |cff| cff.system.clone())
    }
}

impl CffIdentity {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        crate::cancel::check_current_cancel("CFF embedding identity")?;
        let table =
            ttf_parser::cff::Table::parse(bytes).ok_or_else(|| fail("invalid CFF1 program"))?;
        let (postscript_name, ros) = cff_metadata(bytes)?;
        let keyed = table.glyph_cid(ttf_parser::GlyphId(0)).is_some();
        if keyed != ros.is_some() {
            return Err(fail("CFF charset and ROS disagree"));
        }
        let mut gid_to_cid = Vec::with_capacity(usize::from(table.number_of_glyphs()));
        let mut seen = BTreeSet::new();
        for gid in 0..table.number_of_glyphs() {
            if gid % 256 == 0 {
                crate::cancel::check_current_cancel("CFF charset identity")?;
            }
            let cid = if keyed {
                table
                    .glyph_cid(ttf_parser::GlyphId(gid))
                    .ok_or_else(|| fail("CFF charset has an unmapped GID"))?
            } else {
                gid
            };
            if !seen.insert(cid) {
                return Err(fail("CFF charset assigns one CID to multiple glyphs"));
            }
            gid_to_cid.push(cid);
        }
        Ok(Self {
            system: ros.unwrap_or_default(),
            gid_to_cid,
            postscript_name,
        })
    }

    /// Explicit character-code -> native CFF CID CMap. Unicode continues to be
    /// keyed by character code; reusable widths/W2 are keyed by native CID.
    pub fn encoding(
        &self,
        pairs: impl IntoIterator<Item = (u16, u16)>,
        vertical: bool,
    ) -> Result<(PdfDictionary, Vec<u8>)> {
        let mut mapping = BTreeMap::new();
        let mut count = 0usize;
        for (code, gid) in pairs {
            count += 1;
            if count.is_multiple_of(256) {
                crate::cancel::check_current_cancel("CFF encoding planning")?;
            }
            if count > 1_000_000 {
                return Err(WellfriendError::ResourceLimit(
                    "CFF encoding occurrence budget".into(),
                ));
            }
            let cid = *self
                .gid_to_cid
                .get(usize::from(gid))
                .ok_or_else(|| fail("CFF glyph outside charset"))?;
            if mapping.insert(code, cid).is_some_and(|old| old != cid) {
                return Err(fail("one character code selects conflicting CFF glyphs"));
            }
        }
        let mut hash = Sha256::new();
        hash.update(&self.system.registry);
        hash.update([0]);
        hash.update(&self.system.ordering);
        hash.update(self.system.supplement.to_be_bytes());
        hash.update([u8::from(vertical)]);
        for (code, cid) in &mapping {
            hash.update(code.to_be_bytes());
            hash.update(cid.to_be_bytes());
        }
        let name = format!("WellfriendCff{:x}", hash.finalize());
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
        let mut text = format!("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry <{}> /Ordering <{}> /Supplement {} >> def\n/CMapName /{name} def\n/CMapType 1 def\n/WMode {} def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
            hex(&self.system.registry), hex(&self.system.ordering), self.system.supplement, u8::from(vertical));
        let entries = mapping.into_iter().collect::<Vec<_>>();
        for chunk in entries.chunks(100) {
            text.push_str(&format!("{} begincidchar\n", chunk.len()));
            for (code, cid) in chunk {
                text.push_str(&format!("<{code:04X}> {cid}\n"));
            }
            text.push_str("endcidchar\n");
        }
        text.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
        let mut dict = PdfDictionary::empty();
        dict.insert("Type", PdfObject::Name("CMap".into()));
        dict.insert("CMapName", PdfObject::Name(name));
        dict.insert(
            "CIDSystemInfo",
            PdfObject::Dictionary(self.system.dictionary()),
        );
        dict.insert("WMode", PdfObject::Integer(i64::from(u8::from(vertical))));
        Ok((dict, text.into_bytes()))
    }
}

fn index<'a>(bytes: &'a [u8], position: &mut usize) -> Result<Vec<&'a [u8]>> {
    let pair = bytes
        .get(*position..(*position).saturating_add(2))
        .ok_or_else(|| fail("truncated CFF INDEX"))?;
    let count = usize::from(u16::from_be_bytes([pair[0], pair[1]]));
    *position += 2;
    if count == 0 {
        return Ok(Vec::new());
    }
    let size = usize::from(
        *bytes
            .get(*position)
            .ok_or_else(|| fail("truncated CFF INDEX offSize"))?,
    );
    *position += 1;
    if !(1..=4).contains(&size) {
        return Err(fail("invalid CFF INDEX offSize"));
    }
    let offsets = bytes
        .get(*position..(*position).saturating_add((count + 1) * size))
        .ok_or_else(|| fail("truncated CFF INDEX offsets"))?;
    *position += offsets.len();
    let mut values = Vec::with_capacity(count + 1);
    for chunk in offsets.chunks_exact(size) {
        values.push(chunk.iter().fold(0usize, |n, b| (n << 8) | usize::from(*b)));
    }
    if values.first() != Some(&1) || values.windows(2).any(|v| v[1] < v[0]) {
        return Err(fail("invalid CFF INDEX offset order"));
    }
    let base = *position;
    *position = base
        .checked_add(values[count] - 1)
        .ok_or_else(|| fail("CFF INDEX overflow"))?;
    if *position > bytes.len() {
        return Err(fail("CFF INDEX exceeds data"));
    }
    Ok(values
        .windows(2)
        .map(|v| &bytes[base + v[0] - 1..base + v[1] - 1])
        .collect())
}

fn cff_metadata(bytes: &[u8]) -> Result<(String, Option<CidSystem>)> {
    if bytes.first() != Some(&1) {
        return Err(fail("expected CFF1 header"));
    }
    let mut pos = usize::from(*bytes.get(2).ok_or_else(|| fail("truncated CFF header"))?);
    if pos < 4 {
        return Err(fail("invalid CFF header size"));
    }
    let names = index(bytes, &mut pos)?;
    let top = index(bytes, &mut pos)?;
    let strings = index(bytes, &mut pos)?;
    if names.len() != 1 || top.len() != 1 || top[0].len() > 1024 * 1024 {
        return Err(fail("embedding needs a single bounded CFF font"));
    }
    let name = names[0];
    if name.is_empty()
        || name.len() > 127
        || name
            .iter()
            .any(|b| !(33..=126).contains(b) || b"[](){}<>/%".contains(b))
    {
        return Err(fail("invalid CFF PostScript font name"));
    }
    let postscript_name =
        String::from_utf8(name.to_vec()).map_err(|_| fail("invalid CFF font name"))?;
    let dict = top[0];
    let mut pos = 0;
    let mut stack: Vec<Option<i32>> = Vec::new();
    let mut ros = None;
    while pos < dict.len() {
        crate::cancel::check_current_cancel("CFF ROS dictionary")?;
        let b = dict[pos];
        pos += 1;
        let value = match b {
            28 => {
                let n = dict
                    .get(pos..pos + 2)
                    .ok_or_else(|| fail("CFF DICT integer"))?;
                pos += 2;
                Some(i32::from(i16::from_be_bytes([n[0], n[1]])))
            }
            29 => {
                let n = dict
                    .get(pos..pos + 4)
                    .ok_or_else(|| fail("CFF DICT integer"))?;
                pos += 4;
                Some(i32::from_be_bytes([n[0], n[1], n[2], n[3]]))
            }
            30 => {
                loop {
                    let n = *dict.get(pos).ok_or_else(|| fail("unterminated CFF real"))?;
                    pos += 1;
                    if n >> 4 == 13 {
                        return Err(fail("reserved CFF real nibble"));
                    }
                    // The lower nibble is padding after an upper terminator.
                    if n >> 4 == 15 {
                        break;
                    }
                    if n & 15 == 13 {
                        return Err(fail("reserved CFF real nibble"));
                    }
                    if n & 15 == 15 {
                        break;
                    }
                }
                None
            }
            32..=246 => Some(i32::from(b) - 139),
            247..=254 => {
                let n = i32::from(*dict.get(pos).ok_or_else(|| fail("CFF DICT integer"))?);
                pos += 1;
                Some(if b <= 250 {
                    (i32::from(b) - 247) * 256 + n + 108
                } else {
                    -(i32::from(b) - 251) * 256 - n - 108
                })
            }
            0..=21 => {
                let op = if b == 12 {
                    let v = *dict
                        .get(pos)
                        .ok_or_else(|| fail("CFF DICT escaped operator"))?;
                    pos += 1;
                    1200 + u16::from(v)
                } else {
                    u16::from(b)
                };
                if op == 1230 {
                    if ros.is_some() || stack.len() != 3 {
                        return Err(fail("invalid or duplicate CFF ROS"));
                    }
                    let sid = |n: Option<i32>| -> Result<Vec<u8>> {
                        let id = n
                            .and_then(|n| usize::try_from(n).ok())
                            .ok_or_else(|| fail("invalid CFF ROS SID"))?;
                        let bytes = if id < 391 {
                            standard_strings::STANDARD_NAMES
                                .get(id)
                                .map(|s| s.as_bytes())
                        } else {
                            strings.get(id - 391).copied()
                        }
                        .ok_or_else(|| fail("CFF ROS SID outside String INDEX"))?;
                        if bytes.is_empty() || bytes.len() > 4096 {
                            return Err(fail("CFF ROS string size"));
                        }
                        Ok(bytes.to_vec())
                    };
                    ros = Some(CidSystem {
                        registry: sid(stack[0])?,
                        ordering: sid(stack[1])?,
                        supplement: stack[2]
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or_else(|| fail("invalid CFF supplement"))?,
                    });
                }
                stack.clear();
                continue;
            }
            _ => return Err(fail("reserved CFF DICT token")),
        };
        if stack.len() >= 48 {
            return Err(fail("CFF DICT operand limit"));
        }
        stack.push(value);
    }
    if !stack.is_empty() {
        return Err(fail("unconsumed CFF DICT operands"));
    }
    Ok((postscript_name, ros))
}
