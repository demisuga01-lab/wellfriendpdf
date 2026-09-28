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
}
