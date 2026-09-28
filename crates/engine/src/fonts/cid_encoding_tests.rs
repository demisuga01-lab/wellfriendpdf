//! Unexecuted source regressions for declarative, fixed-width Encoding CMaps.
use super::*;

fn stream(bytes: &[u8], base: Option<PdfObject>, mode: Option<i64>) -> PdfObject {
    let mut dict = PdfDictionary::empty();
    if let Some(base) = base {
        dict.insert("UseCMap", base);
    }
    if let Some(mode) = mode {
        dict.insert("WMode", PdfObject::Integer(mode));
    }
    PdfObject::Stream {
        dict,
        raw: bytes.to_vec(),
    }
}

#[test]
fn cid_ranges_notdef_and_explicit_zero_are_distinct() {
    let map = CidEncoding::parse(
        b"1 begincodespacerange <00> <ff> endcodespacerange
        1 begincidrange <20> <22> 42 endcidrange
        1 begincidchar <23> 0 endcidchar
        1 beginnotdefrange <20> <30> 7 endnotdefrange",
        None,
    )
    .unwrap();
    assert_eq!(map.code_size, 1);
    assert_eq!(
        [
            map.cid(32),
            map.cid(33),
            map.cid(34),
            map.cid(35),
            map.cid(36),
            map.cid(49)
        ],
        [42, 43, 44, 0, 7, 0]
    );
}

#[test]
fn local_mapping_overrides_identity_base_without_erasing_other_cids() {
    let map = CidEncoding::parse(
        b"/Identity-H usecmap /WMode 1 def 1 begincidchar <0001> 42 endcidchar",
        None,
    )
    .unwrap();
    assert_eq!(map.cid(1), 42);
    assert_eq!(map.cid(2), 2);
    assert_eq!(map.wmode, Some(1));
    assert_eq!(map.code_size, 2);
}

#[test]
fn stream_dictionary_can_override_inherited_but_not_conflicting_local_wmode() {
    let bytes = b"1 begincidchar <0001> 42 endcidchar";
    let base = Some(PdfObject::Name("Identity-H".into()));
    let map = CidEncoding::read(&stream(bytes, base.clone(), Some(1)), None, 0).unwrap();
    assert_eq!(map.wmode, Some(1));
    assert_eq!(map.cid(1), 42);
    let bytes = b"/WMode 0 def 1 begincidchar <0001> 42 endcidchar";
    assert!(CidEncoding::read(&stream(bytes, base, Some(1)), None, 0).is_err());
}

#[test]
fn nested_stream_maps_keep_base_entries_and_override_selected_codes() {
    let base = stream(
        b"1 begincodespacerange <0000> <ffff> endcodespacerange
        2 begincidchar <0001> 42 <0002> 7 endcidchar",
        None,
        None,
    );
    let outer = stream(b"1 begincidchar <0002> 1000 endcidchar", Some(base), None);
    let map = CidEncoding::read(&outer, None, 0).unwrap();
    assert_eq!(map.cid(1), 42);
    assert_eq!(map.cid(2), 1000);
    assert_eq!(map.cid(3), 0);
}

#[test]
fn inheritance_depth_is_bounded() {
    let mut object = PdfObject::Name("Identity-H".into());
    for _ in 0..10 {
        object = stream(b"", Some(object), None);
    }
    assert!(CidEncoding::read(&object, None, 0).is_err());
}

#[test]
fn malformed_and_unsupported_maps_fail_instead_of_becoming_identity() {
    let space = "1 begincodespacerange <0000> <ffff> endcodespacerange ";
    for suffix in [
        "1 begincidchar <0001> 42 <0002> 7 endcidchar",
        "2 begincidchar <0001> 42 endcidchar",
        "1 begincidrange <0002> <0001> 7 endcidrange",
        "1 begincidrange <0001> <0003> 65535 endcidrange",
        "1 begincidchar <0001> -1 endcidchar",
        "1 begincidchar <01> 42 endcidchar",
        "1 beginbfchar <0001> <0041> endbfchar",
        "/WMode true def",
        "/WMode 2 def",
        "{ 1 } exec",
    ] {
        assert!(
            CidEncoding::parse(format!("{space}{suffix}").as_bytes(), None).is_err(),
            "{suffix}"
        );
    }
    assert_eq!(
        CidEncoding::parse(
            b"1 begincodespacerange <000000> <ffffff> endcodespacerange",
            None
        )
        .unwrap()
        .code_size,
        3
    );
    assert!(CidEncoding::parse(
        b"1 begincodespacerange <10> <20> endcodespacerange 1 begincidchar <21> 42 endcidchar",
        None
    )
    .is_err());
}

#[test]
fn canonical_tokens_do_not_read_commands_inside_strings_or_comments() {
    let map = CidEncoding::parse(
        b"% 1 begincidchar <01> 99 endcidchar
        /Notice (2 begincidchar <01> 99 endcidchar) def
        /WMode 0 def 1 begincodespacerange <00> <ff> endcodespacerange
        1 begincidchar <01> 42 endcidchar",
        None,
    )
    .unwrap();
    assert_eq!(map.cid(1), 42);
    assert_eq!(map.cid(2), 0);
}

#[test]
fn code_space_checks_each_byte_and_survives_inheritance() {
    let base = stream(
        b"1 begincodespacerange <8140> <9ffc> endcodespacerange",
        None,
        None,
    );
    for code in ["813f", "81fd", "9fff", "a040"] {
        let child = stream(
            format!("1 begincidchar <{code}> 42 endcidchar").as_bytes(),
            Some(base.clone()),
            None,
        );
        assert!(CidEncoding::read(&child, None, 0).is_err(), "{code}");
    }
    let child = stream(
        b"1 begincidchar <8240> 42 endcidchar",
        Some(base.clone()),
        None,
    );
    assert_eq!(CidEncoding::read(&child, None, 0).unwrap().cid(0x8240), 42);
    let child = stream(
        b"1 begincodespacerange <0000> <ffff> endcodespacerange",
        Some(base),
        None,
    );
    assert!(CidEncoding::read(&child, None, 0).is_err());
}

#[test]
fn nested_metadata_does_not_override_mapping_wmode() {
    let map = CidEncoding::parse(
        b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap
        /WMode 0 def /Extras << /WMode 1 /Nested [<< /WMode 1 >>] >> def
        /CIDSystemInfo 4 dict dup begin /Registry (Fixture) def /Ordering (Identity) def /Supplement 0 def /WMode 1 def end def
        1 begincodespacerange <00> <ff> endcodespacerange
        1 begincidchar <01> 42 endcidchar
        endcmap CMapName currentdict /CMap defineresource pop end end",
        None,
    )
    .unwrap();
    assert_eq!(map.wmode, Some(0));
    assert_eq!(map.cid(1), 42);
    for bytes in [
        b"/Meta << /WMode 1".as_slice(),
        b"/Meta [ >>",
        b"/Meta [ begincidchar ]",
    ] {
        assert!(CidEncoding::parse(bytes, None).is_err());
    }
}

#[test]
fn resolver_uses_original_code_for_unicode_and_native_cid_for_metrics() {
    let encoding = stream(
        b"/WMode 1 def 1 begincodespacerange <00> <ff> endcodespacerange
        2 begincidchar <20> 42 <21> 7 endcidchar",
        None,
        None,
    );
    let unicode = stream(
        b"1 begincodespacerange <00> <ff> endcodespacerange
        2 beginbfchar <20> <0041> <21> <0020> endbfchar",
        None,
        None,
    );
    let mut desc = PdfDictionary::empty();
    desc.insert("Subtype", PdfObject::Name("CIDFontType2".into()));
    desc.insert("DW", PdfObject::Integer(999));
    desc.insert(
        "W",
        PdfObject::Array(vec![
            PdfObject::Integer(7),
            PdfObject::Array(vec![PdfObject::Integer(0)]),
            PdfObject::Integer(42),
            PdfObject::Array(vec![PdfObject::Integer(600)]),
        ]),
    );
    desc.insert(
        "W2",
        PdfObject::Array(vec![
            PdfObject::Integer(42),
            PdfObject::Array(vec![
                PdfObject::Integer(-900),
                PdfObject::Integer(300),
                PdfObject::Integer(850),
            ]),
        ]),
    );
    let mut font = PdfDictionary::empty();
    font.insert("Subtype", PdfObject::Name("Type0".into()));
    font.insert("Encoding", encoding);
    font.insert("ToUnicode", unicode);
    font.insert(
        "DescendantFonts",
        PdfObject::Array(vec![PdfObject::Dictionary(desc)]),
    );
    let resolver = crate::fonts::FontResolver::new_from_dict_only(&font);
    resolver.validate_encoding().unwrap();
    assert_eq!(resolver.decode_char(32), "A");
    assert_eq!(resolver.decode_char(33), " ");
    assert_eq!(resolver.glyph_width(32), 600.0);
    assert_eq!(resolver.glyph_width(33), 0.0);
    assert_eq!(resolver.vertical_metrics(32), (-900.0, 300.0, 850.0));
    assert!(resolver.is_space_code(32));
    assert!(!resolver.is_space_code(33));
    assert!(resolver.is_vertical());
    assert_eq!(resolver.code_size(), 1);
}
