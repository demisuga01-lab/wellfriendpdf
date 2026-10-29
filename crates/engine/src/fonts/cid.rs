use crate::filters::decode_stream_lossless;
use crate::object::{PdfDictionary, PdfObject};
use crate::reader::PdfReader;

/// Cache native CID -> GID for sfnt-wrapped CFF1. Bare CFF uses the existing
/// rasterizer's charset path; mapping it here too would map the value twice.
pub(crate) fn sfnt_cff_gid_map(
    desc: Option<&PdfDictionary>,
    reader: Option<&PdfReader>,
) -> std::result::Result<Option<Vec<u16>>, String> {
    let Some(desc) = desc.filter(|d| d.get_name("Subtype") == Some("CIDFontType0")) else {
        return Ok(None);
    };
    let resolve = |obj: &PdfObject| {
        reader.map_or_else(
            || Ok(obj.clone()),
            |r| r.resolve(obj.clone()).map_err(|e| e.to_string()),
        )
    };
    let Some(descriptor) = desc.get("FontDescriptor") else {
        return Ok(None);
    };
    let descriptor = resolve(descriptor)?;
    let Some(file) = descriptor.as_dict().and_then(|d| d.get("FontFile3")) else {
        return Ok(None);
    };
    let stream = resolve(file)?;
    let PdfObject::Stream { dict, raw } = &stream else {
        return Err("invalid CFF font file".into());
    };
    if dict.get_name("Subtype") != Some("OpenType") {
        return Ok(None);
    }
    let bytes = match reader {
        Some(reader) => {
            decode_stream_lossless(&stream, reader)
                .map_err(|e| e.to_string())?
                .data
        }
        None => crate::filters::decode_stream_from_dict(dict, raw).map_err(|e| e.to_string())?,
    };
    if bytes.len() > 256 * 1024 * 1024 {
        return Err("CFF program size limit".into());
    }
    let face = ttf_parser::Face::parse(&bytes, 0)
        .map_err(|_| "invalid OpenType CFF container".to_string())?;
    let cff = face
        .raw_face()
        .table(ttf_parser::Tag::from_bytes(b"CFF "))
        .ok_or_else(|| "OpenType CIDFontType0 has no CFF1 table".to_string())?;
    let identity = super::pdf_embedding::CffIdentity::parse(cff).map_err(|e| e.to_string())?;
    if identity.gid_to_cid.len() != usize::from(face.number_of_glyphs()) {
        return Err("CFF and maxp glyph counts differ".into());
    }
    let max = identity.gid_to_cid.iter().copied().max().unwrap_or(0);
    let mut map = vec![0; usize::from(max) + 1];
    for (gid, cid) in identity.gid_to_cid.into_iter().enumerate() {
        map[usize::from(cid)] = gid as u16;
    }
    Ok(Some(map))
}

/// Map a CID to a TrueType glyph id for CIDFontType2 descendants.
///
/// `/CIDToGIDMap /Identity` means CID == GID. When the map is a stream, it is a
/// big-endian u16 array indexed by CID. Missing entries fall back to identity so
/// malformed or truncated maps degrade the same way the old renderer did.
pub(crate) fn cid_to_gid(cid: u16, desc_dict: Option<&PdfDictionary>, reader: &PdfReader) -> u16 {
    let Some(map_obj) = desc_dict.and_then(|dict| dict.get("CIDToGIDMap")) else {
        return cid;
    };
    match map_obj {
        PdfObject::Name(name) if name == "Identity" => cid,
        PdfObject::Stream { .. } => {
            gid_from_map_object(cid, map_obj.clone(), reader).unwrap_or(cid)
        }
        PdfObject::Reference { .. } => {
            gid_from_map_object(cid, map_obj.clone(), reader).unwrap_or(cid)
        }
        _ => cid,
    }
}

pub(crate) fn cid_font_has_embedded_program(
    desc_dict: Option<&PdfDictionary>,
    reader: &PdfReader,
) -> bool {
    let Some(desc_dict) = desc_dict else {
        return false;
    };
    let Some(descriptor_obj) = desc_dict.get("FontDescriptor") else {
        return false;
    };
    let Ok(PdfObject::Dictionary(descriptor)) = reader.resolve(descriptor_obj.clone()) else {
        return false;
    };
    descriptor.contains_key("FontFile")
        || descriptor.contains_key("FontFile2")
        || descriptor.contains_key("FontFile3")
}

fn gid_from_map_object(cid: u16, object: PdfObject, reader: &PdfReader) -> Option<u16> {
    let resolved = reader.resolve(object).ok()?;
    let PdfObject::Stream { dict, raw } = resolved else {
        return None;
    };
    let raw_fallback = raw.clone();
    let stream = PdfObject::Stream { dict, raw };
    let bytes = decode_stream_lossless(&stream, reader)
        .map(|decoded| decoded.data)
        .unwrap_or(raw_fallback);
    gid_from_map_bytes(cid, &bytes)
}

fn gid_from_map_bytes(cid: u16, bytes: &[u8]) -> Option<u16> {
    let offset = usize::from(cid).checked_mul(2)?;
    let pair = bytes.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([pair[0], pair[1]]))
}

/// Recover a CID -> Unicode map from the descendant font's embedded sfnt program
/// for a composite (Type0) font that has no `/ToUnicode` and no predefined-CMap
/// Unicode source. The embedded TrueType/OpenType `cmap` maps Unicode -> GID;
/// inverting it and composing with CID -> GID (the `/CIDToGIDMap`, or the CFF
/// charset for CIDFontType0) yields CID -> Unicode. This mirrors the recovery
/// MuPDF performs for Identity-encoded subsets: without it, Identity-H text with
/// no ToUnicode extracts as a run of U+FFFD. Built only on demand (when there is
/// no ToUnicode), so it adds nothing to the common decode path.
pub(crate) fn embedded_cid_unicode_map(
    desc: Option<&PdfDictionary>,
    reader: Option<&PdfReader>,
) -> Option<std::collections::BTreeMap<u16, String>> {
    use std::collections::BTreeMap;
    let desc = desc?;
    let resolve = |obj: &PdfObject| -> Option<PdfObject> {
        match reader {
            Some(r) => r.resolve(obj.clone()).ok(),
            None => Some(obj.clone()),
        }
    };
    let decode_any = |stream: &PdfObject| -> Option<Vec<u8>> {
        let PdfObject::Stream { dict, raw } = stream else {
            return None;
        };
        match reader {
            Some(r) => decode_stream_lossless(stream, r).ok().map(|d| d.data),
            None => crate::filters::decode_stream_from_dict(dict, raw).ok(),
        }
    };
    let descriptor = resolve(desc.get("FontDescriptor")?)?;
    let descriptor = descriptor.as_dict()?.clone();
    let file = descriptor
        .get("FontFile2")
        .or_else(|| descriptor.get("FontFile3"))?;
    let stream = resolve(file)?;
    let bytes = decode_any(&stream)?;
    if bytes.is_empty() || bytes.len() > 256 * 1024 * 1024 {
        return None;
    }
    let face = ttf_parser::Face::parse(&bytes, 0).ok()?;
    let cmap = face.tables().cmap?;
    // Prefer a full-repertoire Unicode subtable, then BMP Unicode, then any.
    let mut best: Option<(u8, ttf_parser::cmap::Subtable)> = None;
    for sub in cmap.subtables {
        if !sub.is_unicode() {
            continue;
        }
        let rank = match (sub.platform_id, sub.encoding_id) {
            (ttf_parser::PlatformId::Unicode, 6) => 3,
            (ttf_parser::PlatformId::Windows, 10) => 3,
            (ttf_parser::PlatformId::Windows, 1) => 2,
            _ => 1,
        };
        if best.as_ref().map_or(true, |(r, _)| rank > *r) {
            best = Some((rank, sub));
        }
    }
    let sub = best?.1;
    let mut gid_to_unicode: BTreeMap<u16, String> = BTreeMap::new();
    sub.codepoints(|cp| {
        if let Some(gid) = sub.glyph_index(cp) {
            if let Some(ch) = char::from_u32(cp) {
                if !ch.is_control() {
                    gid_to_unicode.entry(gid.0).or_insert_with(|| ch.to_string());
                }
            }
        }
    });
    if gid_to_unicode.is_empty() {
        return None;
    }
    let mut out: BTreeMap<u16, String> = BTreeMap::new();
    if desc.get_name("Subtype") == Some("CIDFontType0") {
        // sfnt-wrapped CFF: the charset maps CID -> GID (vector index == CID).
        match sfnt_cff_gid_map(Some(desc), reader) {
            Ok(Some(cid_to_gid)) => {
                for (cid, gid) in cid_to_gid.into_iter().enumerate() {
                    if let (Ok(cid), Some(text)) = (u16::try_from(cid), gid_to_unicode.get(&gid)) {
                        out.insert(cid, text.clone());
                    }
                }
            }
            _ => out = gid_to_unicode,
        }
    } else {
        // CIDFontType2 (TrueType): /CIDToGIDMap, defaulting to Identity.
        let cid_to_gid_stream = desc
            .get("CIDToGIDMap")
            .and_then(|o| resolve(o))
            .filter(|o| matches!(o, PdfObject::Stream { .. }))
            .and_then(|o| decode_any(&o));
        match cid_to_gid_stream {
            Some(data) => {
                for (cid, pair) in data.chunks_exact(2).enumerate() {
                    let gid = u16::from_be_bytes([pair[0], pair[1]]);
                    if let (Ok(cid), Some(text)) = (u16::try_from(cid), gid_to_unicode.get(&gid)) {
                        out.insert(cid, text.clone());
                    }
                }
            }
            None => out = gid_to_unicode, // Identity CIDToGIDMap: CID == GID.
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::ContentEngine;
    use crate::fonts::resolver::get_descendant_font;

    fn cff_descriptor(bytes: Vec<u8>, subtype: &str) -> PdfDictionary {
        let mut file = PdfDictionary::empty();
        file.insert("Subtype", PdfObject::Name(subtype.into()));
        let mut descriptor = PdfDictionary::empty();
        descriptor.insert(
            "FontFile3",
            PdfObject::Stream {
                dict: file,
                raw: bytes,
            },
        );
        let mut descendant = PdfDictionary::empty();
        descendant.insert("Subtype", PdfObject::Name("CIDFontType0".into()));
        descendant.insert("FontDescriptor", PdfObject::Dictionary(descriptor));
        descendant
    }

    #[test]
    fn sfnt_cff_native_cids_map_once_and_reading_does_not_require_edit_permissions() {
        let bytes = crate::fonts::pdf_embedding_fixtures::font(true, 4);
        let desc = cff_descriptor(bytes, "OpenType");
        let map = sfnt_cff_gid_map(Some(&desc), None).unwrap().unwrap();
        assert_eq!(map[42], 1);
        assert_eq!(map[7], 2);
        assert_eq!(map[1000], 3);
        assert_eq!(map[8], 0);
    }

    #[test]
    fn bare_cff_keeps_its_rasterizer_owned_cid_mapping() {
        let desc = cff_descriptor(
            crate::fonts::pdf_embedding_fixtures::cff(true),
            "CIDFontType0C",
        );
        assert!(sfnt_cff_gid_map(Some(&desc), None).unwrap().is_none());
    }

    #[test]
    fn cid_to_gid_map_bytes_are_big_endian_pairs() {
        let bytes = [0x00, 0x00, 0x00, 0x05, 0x01, 0x2C];

        assert_eq!(gid_from_map_bytes(0, &bytes), Some(0));
        assert_eq!(gid_from_map_bytes(1, &bytes), Some(5));
        assert_eq!(gid_from_map_bytes(2, &bytes), Some(300));
        assert_eq!(gid_from_map_bytes(3, &bytes), None);
    }

    #[test]
    fn mixedfonts_embedded_cid_truetype_uses_cid_to_gid_stream() {
        let fixture = format!(
            "{}/../../tests/corpus/pdfs/pdfjs/mixedfonts.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let engine = ContentEngine::open_path(fixture).expect("open mixedfonts fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font = resources
            .fonts
            .values()
            .find(|dict| dict.get_name("BaseFont") == Some("DejaVuSans"))
            .expect("embedded DejaVuSans Type0 font");
        let descendant = get_descendant_font(font, reader).expect("descendant CID font");

        assert!(cid_font_has_embedded_program(Some(&descendant), reader));
        assert_ne!(
            cid_to_gid(65, Some(&descendant), reader),
            65,
            "fixture carries a non-identity CIDToGIDMap stream"
        );
    }

    #[test]
    fn mixedfonts_nonembedded_cid_font_is_detected_as_fallback() {
        let fixture = format!(
            "{}/../../tests/corpus/pdfs/pdfjs/mixedfonts.pdf",
            env!("CARGO_MANIFEST_DIR")
        );
        let engine = ContentEngine::open_path(fixture).expect("open mixedfonts fixture");
        let resources = engine.get_page_resources(1).expect("page resources");
        let reader = engine.document().reader();
        let font = resources
            .fonts
            .values()
            .find(|dict| {
                dict.get_name("BaseFont")
                    .is_some_and(|name| name.starts_with("ArialUnicodeMS"))
            })
            .expect("nonembedded ArialUnicodeMS Type0 font");
        let descendant = get_descendant_font(font, reader).expect("descendant CID font");

        assert!(!cid_font_has_embedded_program(Some(&descendant), reader));
    }
    #[test]
    fn embedded_cid_unicode_recovers_from_sfnt_cmap() {
        // The fixture's `cmap` maps U+0020->GID3, U+0041->GID1, U+0042->GID2,
        // and its CFF charset maps CID42->GID1, CID7->GID2, CID1000->GID3.
        // Composed, the recovered CID -> Unicode must read the glyphs back as
        // real characters even though the font carries no /ToUnicode.
        let bytes = crate::fonts::pdf_embedding_fixtures::font(true, 4);
        let desc = cff_descriptor(bytes, "OpenType");
        let map = embedded_cid_unicode_map(Some(&desc), None).expect("recovered map");
        assert_eq!(map.get(&42).map(String::as_str), Some("A"));
        assert_eq!(map.get(&7).map(String::as_str), Some("B"));
        assert_eq!(map.get(&1000).map(String::as_str), Some(" "));
    }

}
