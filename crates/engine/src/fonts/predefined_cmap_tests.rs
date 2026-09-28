//! Source regressions only. No compilation, PDF generation or execution yet.
use super::*;
use crate::fonts::{FontDecodeSource, FontResolver};
use crate::{PdfDictionary as Dict, PdfObject as Obj};

fn stream(bytes: Vec<u8>) -> Obj {
    Obj::Stream {
        dict: Dict::empty(),
        raw: bytes,
    }
}
pub(crate) fn font(encoding: &str, ordering: &str) -> Dict {
    let mut system = Dict::empty();
    system.insert("Registry", Obj::String(b"Adobe".to_vec()));
    system.insert("Ordering", Obj::String(ordering.as_bytes().to_vec()));
    system.insert("Supplement", Obj::Integer(7));
    let mut desc = Dict::empty();
    desc.insert("Subtype", Obj::Name("CIDFontType2".into()));
    desc.insert("CIDSystemInfo", Obj::Dictionary(system));
    desc.insert("DW", Obj::Integer(1000));
    desc.insert(
        "W",
        Obj::Array(vec![
            Obj::Integer(3284),
            Obj::Array(vec![Obj::Integer(600)]),
        ]),
    );
    let mut font = Dict::empty();
    font.insert("Subtype", Obj::Name("Type0".into()));
    font.insert("BaseFont", Obj::Name("CMapSourceFixture".into()));
    font.insert("Encoding", Obj::Name(encoding.into()));
    font.insert("DescendantFonts", Obj::Array(vec![Obj::Dictionary(desc)]));
    font
}

#[test]
fn inventory_contains_pinned_encodings_and_unicode_collections() {
    assert_eq!(supported_names().len(), 202);
    assert_eq!(
        resources::ASSETS
            .iter()
            .filter(|a| a.kind == Kind::Unicode)
            .count(),
        5
    );
    let jis = lookup("UniJIS-UTF16-V").unwrap();
    assert_eq!(jis.collection, "Adobe-Japan1");
    assert!(jis.vertical && jis.unicode_preserving);
    assert_eq!(jis.code_size, 0);
    assert_eq!(
        unicode_for_code("UniJIS-UTF16-H", 0x65e5).as_deref(),
        Some("日")
    );
    assert!(lookup("90ms-RKSJ-H").is_some());
    assert!(lookup("90MS-RKSJ-H").is_none());
    assert!(unicode_for_code("Identity-H", 3284).is_none());
}

#[test]
fn all_vendored_resources_hash_parse_and_match_declared_metadata() {
    for asset in resources::ASSETS {
        let bytes = resources::decoded(asset).unwrap_or_else(|e| panic!("{}: {e}", asset.name));
        assert_eq!(bytes.len(), asset.decoded_len);
        let program =
            load_program(asset.name, asset.kind).unwrap_or_else(|e| panic!("{}: {e}", asset.name));
        assert_eq!(program.name.as_deref(), Some(asset.name));
        assert_eq!(program.space.fixed_length().unwrap_or(0), asset.code_size);
    }
}

#[test]
fn legacy_shift_jis_codes_are_not_treated_as_cids_or_unicode_scalars() {
    let resolver = FontResolver::new_from_dict_only(&font("90ms-RKSJ-H", "Japan1"));
    let code = resolver.codes(&[0x93, 0xfa]).next().unwrap().unwrap().code;
    assert_eq!(resolver.cid_for_character(code).unwrap(), 3284);
    assert_eq!(resolver.width_for_code(code), 600.0);
    assert_eq!(
        resolver.decode_code_with_source(code),
        ("日".into(), FontDecodeSource::PredefinedCMap)
    );
    assert_eq!(
        resolver.try_encode_existing("日").unwrap(),
        (vec![0x93, 0xfa], false)
    );
}

#[test]
fn utf8_utf16_and_utf32_preserve_supplementary_source_scalars() {
    let ch = char::from_u32(0x1b132).unwrap();
    let text = ch.to_string();
    for (name, bytes) in [
        ("UniJIS-UTF8-H", text.as_bytes().to_vec()),
        ("UniJIS-UTF16-H", vec![0xd8, 0x2c, 0xdd, 0x32]),
        ("UniJIS-UTF32-H", vec![0, 1, 0xb1, 0x32]),
    ] {
        let resolver = FontResolver::new_from_dict_only(&font(name, "Japan1"));
        let code = resolver.codes(&bytes).next().unwrap().unwrap().code;
        assert_eq!(resolver.cid_for_character(code).unwrap(), 12269, "{name}");
        assert_eq!(resolver.try_decode_string(&bytes).unwrap(), text, "{name}");
        assert_eq!(
            resolver.try_encode_existing(&text).unwrap(),
            (bytes, false),
            "{name}"
        );
    }
}

#[test]
fn vertical_resource_keeps_horizontal_entries_and_changes_punctuation() {
    let h = load_program("UniJIS-UTF16-H", Kind::Cid).unwrap();
    let v = load_program("UniJIS-UTF16-V", Kind::Cid).unwrap();
    let day = CharacterCode::new(0x65e5, 2).unwrap();
    let comma = CharacterCode::new(0x3001, 2).unwrap();
    assert_eq!(h.cid(day), 3284);
    assert_eq!(v.cid(day), 3284);
    assert_eq!(h.cid(comma), 634);
    assert_eq!(v.cid(comma), 7887);
    assert_eq!(v.wmode, Some(1));
}

#[test]
fn identity_font_uses_its_declared_character_collection_for_extraction() {
    let resolver = FontResolver::new_from_dict_only(&font("Identity-H", "Japan1"));
    assert_eq!(resolver.try_decode_string(&[0x0c, 0xd4]).unwrap(), "日");
    assert_eq!(
        resolver.try_encode_existing("日").unwrap().0,
        vec![0x0c, 0xd4]
    );
}

#[test]
fn explicit_tounicode_overrides_collection_without_changing_native_cid() {
    let mut dict = font("90ms-RKSJ-H", "Japan1");
    dict.insert(
        "ToUnicode",
        stream(b"1 beginbfchar <93fa> <0058> endbfchar".to_vec()),
    );
    let resolver = FontResolver::new_from_dict_only(&dict);
    let code = CharacterCode::new(0x93fa, 2).unwrap();
    assert_eq!(
        resolver.decode_code_with_source(code),
        ("X".into(), FontDecodeSource::ToUnicode)
    );
    assert_eq!(resolver.cid_for_character(code).unwrap(), 3284);
    assert_eq!(
        resolver.try_encode_existing("X").unwrap(),
        (vec![0x93, 0xfa], false)
    );
}

#[test]
fn mismatched_character_collections_fail_encoding_validation() {
    let resolver = FontResolver::new_from_dict_only(&font("90ms-RKSJ-H", "GB1"));
    assert!(resolver
        .validate_encoding()
        .unwrap_err()
        .contains("collections disagree"));
    assert!(resolver.try_decode_string(&[0x93, 0xfa]).is_err());
}

#[test]
fn unknown_named_cmaps_do_not_fall_back_to_identity() {
    assert!(looks_like_predefined_name("Unregistered-Legacy-H"));
    assert!(!is_supported_name("Unregistered-Legacy-H"));
    let resolver = FontResolver::new_from_dict_only(&font("Unregistered-Legacy-H", "Japan1"));
    assert!(resolver.validate_encoding().is_err());
    assert!(resolver.cid_for_code(65).is_err());
}

#[test]
fn embedded_encoding_can_inherit_and_override_a_named_resource() {
    let program = Program::parse(
        b"/UniJIS-UTF16-H usecmap 1 begincidchar <65e5> 42 endcidchar",
        Kind::Cid,
        None,
        false,
    )
    .unwrap();
    assert_eq!(program.cid(CharacterCode::new(0x65e5, 2).unwrap()), 42);
    assert_eq!(program.cid(CharacterCode::new(0x3001, 2).unwrap()), 634);
    assert_eq!(program.space.fixed_length(), None);
}

#[test]
fn stream_dictionary_collection_is_checked_against_body_and_font() {
    let mut properties = Dict::empty();
    let mut system = Dict::empty();
    system.insert("Registry", Obj::String(b"Adobe".to_vec()));
    system.insert("Ordering", Obj::String(b"GB1".to_vec()));
    system.insert("Supplement", Obj::Integer(6));
    properties.insert("CIDSystemInfo", Obj::Dictionary(system));
    let conflicting = Obj::Stream {
        dict: properties.clone(),
        raw: b"/UniJIS-UTF16-H usecmap".to_vec(),
    };
    assert!(crate::fonts::cmap_stream::read(&conflicting, None, Kind::Cid, 0).is_err());
    let mut dict = font("Identity-H", "Japan1");
    dict.insert("Encoding",Obj::Stream {dict:properties,raw:b"1 begincodespacerange <00> <ff> endcodespacerange 1 begincidchar <41> 1 endcidchar".to_vec()});
    assert!(FontResolver::new_from_dict_only(&dict)
        .validate_encoding()
        .is_err());
}

#[test]
fn inherited_cmap_cannot_silently_change_its_character_collection() {
    assert!(Program::parse(
        b"/UniJIS-UTF16-H usecmap
        /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 6 >> def",
        Kind::Cid,
        None,
        false
    )
    .is_err());
}

#[test]
fn named_tounicode_base_supports_local_overrides() {
    let mut properties = Dict::empty();
    properties.insert("UseCMap", Obj::Name("Adobe-Japan1-UCS2".into()));
    let unicode = Obj::Stream {
        dict: properties,
        raw: b"/Adobe-Japan1-UCS2 usecmap 1 beginbfchar <0cd4> <0058> endbfchar".to_vec(),
    };
    let mut dict = font("Identity-H", "Japan1");
    dict.insert("ToUnicode", unicode);
    let resolver = FontResolver::new_from_dict_only(&dict);
    assert_eq!(resolver.try_decode_string(&[0x0c, 0xd4]).unwrap(), "X");
    assert_eq!(
        resolver.try_decode_string(&[0x02, 0x79]).unwrap(),
        "\u{3000}"
    );
}

#[test]
fn cached_programs_are_shared_and_cancellation_does_not_poison_them() {
    let first = load_program("UniJIS-UTF16-H", Kind::Cid).unwrap();
    let second = load_program("UniJIS-UTF16-H", Kind::Cid).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(load_program("UniJIS-UTF16-H", Kind::Cid).is_err()));
    assert!(load_program("UniJIS-UTF16-H", Kind::Cid).is_ok());
}

#[test]
fn nested_metadata_does_not_replace_the_declared_collection() {
    let map = Program::parse(
        b"/CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 7
        /Private [ /Ordering (Wrong) ] >> def
        1 begincodespacerange <00> <ff> endcodespacerange",
        Kind::Cid,
        None,
        false,
    )
    .unwrap();
    assert_eq!(map.system.unwrap().ordering, b"Japan1");
}

#[test]
fn unavailable_collection_does_not_invent_unicode_from_numeric_cids() {
    let resolver = FontResolver::new_from_dict_only(&font("Identity-H", "PrivateCollection"));
    assert!(resolver.validate_encoding().is_ok());
    assert!(resolver.validate_source_encoding().is_err());
    assert_eq!(
        resolver.decode_char_with_source(65),
        ("\u{fffd}".into(), FontDecodeSource::Unknown)
    );
}

#[test]
fn cmap_name_and_writing_mode_ignore_string_lookalikes() {
    let mut dict = font("Identity-H", "Japan1");
    dict.insert(
        "Encoding",
        stream(
            b"/Notice (/CMapName /UniJIS-UTF16-V /WMode 1 def) def
        /CMapName /Local-H def /WMode 0 def
        1 begincodespacerange <0000> <ffff> endcodespacerange
        1 begincidchar <0001> 3284 endcidchar"
                .to_vec(),
        ),
    );
    let resolver = FontResolver::new_from_dict_only(&dict);
    assert!(!resolver.is_vertical());
    assert_eq!(
        crate::fonts::resolver::predefined_cmap_name(&dict, None).as_deref(),
        Some("Local-H")
    );
    assert_eq!(resolver.try_decode_string(&[0, 1]).unwrap(), "日");
}

#[test]
fn truncated_vertical_metric_triple_cannot_index_past_its_array() {
    let mut descendant = Dict::empty();
    descendant.insert(
        "W2",
        Obj::Array(vec![
            Obj::Integer(1),
            Obj::Integer(1),
            Obj::Integer(-900),
            Obj::Integer(300),
        ]),
    );
    assert_eq!(
        crate::fonts::resolver::lookup_cid_vertical(1, 600.0, &descendant),
        (-1000.0, 300.0, 880.0)
    );
}

#[test]
fn reopened_pdf_uses_named_encoding_before_cidtogid_and_width_lookup() {
    let mut dict = font("90ms-RKSJ-H", "Japan1");
    let mut desc = dict.get_array("DescendantFonts").unwrap()[0]
        .as_dict()
        .unwrap()
        .clone();
    let mut descriptor = Dict::empty();
    descriptor.insert("Flags", Obj::Integer(4));
    descriptor.insert(
        "FontFile2",
        stream(include_bytes!("../../fonts/LiberationSans-Regular.ttf").to_vec()),
    );
    desc.insert("FontDescriptor", Obj::Dictionary(descriptor));
    let mut gids = vec![0u8; 3285 * 2];
    gids[3284 * 2 + 1] = 1;
    desc.insert("CIDToGIDMap", stream(gids));
    dict.insert("DescendantFonts", Obj::Array(vec![Obj::Dictionary(desc)]));
    let bytes = crate::fonts::variable_cmap_tests::pdf_with_font(
        dict,
        b"BT /F 10 Tf 1 0 0 1 30 700 Tm <93FA> Tj ET",
    );
    let engine = crate::ContentEngine::open_bytes(bytes).unwrap();
    let resources = engine.get_page_resources(1).unwrap();
    let glyphs = crate::render::text_decode::try_decode_text_bytes(
        &[0x93, 0xfa],
        "F",
        &resources,
        engine.document().reader(),
    )
    .unwrap();
    assert_eq!(glyphs.len(), 1);
    assert_eq!(glyphs[0].code, 1);
    assert!(glyphs[0].is_gid);
    assert_eq!(glyphs[0].unicode, '日');
    assert_eq!(glyphs[0].width, Some(600.0));
    assert_eq!(
        engine
            .collect_page_text_chunks(1)
            .unwrap()
            .iter()
            .map(|c| c.text.as_str())
            .collect::<String>(),
        "日"
    );
}
