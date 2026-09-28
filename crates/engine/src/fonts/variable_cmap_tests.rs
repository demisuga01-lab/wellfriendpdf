//! Length-aware CMap regressions. Source only; not compiled or executed.
use super::character_code::{CharacterCode as Code, CodeRange, CodeSpace};
use super::cmap_program::{Kind, Program};
use super::{cmap::ToUnicodeCMap, FontResolver};
use crate::writer::{write_incremental_update, IncrementalObject};
use crate::{PdfDictionary as Dict, PdfObject as Obj};

pub(crate) const ENCODED: &[u8] = &[0x41, 0, 0x41, 0x81, 0, 1, 0x90, 0, 0, 0x41, 0x20];
pub(crate) const LOGICAL: &str = "ABfi\u{1f600} ";
const SPACES:&str="5 begincodespacerange <20> <20> <41> <41> <0041> <0041> <810000> <81ffff> <90000000> <90ffffff> endcodespacerange";
fn stream(bytes: Vec<u8>) -> Obj {
    Obj::Stream {
        dict: Dict::empty(),
        raw: bytes,
    }
}
pub(crate) fn font_dictionary(vertical: bool) -> Dict {
    let encoding=stream(format!("/WMode {} def {SPACES} 5 begincidchar <41> 42 <0041> 7 <810001> 42 <90000041> 7 <20> 1000 endcidchar",u8::from(vertical)).into_bytes());
    let unicode=stream(format!("{SPACES} 5 beginbfchar <41> <0041> <0041> <0042> <810001> <00660069> <90000041> <d83dde00> <20> <0020> endbfchar").into_bytes());
    let mut file = Dict::empty();
    file.insert("Subtype", Obj::Name("OpenType".into()));
    let mut descriptor = Dict::empty();
    descriptor.insert("Type", Obj::Name("FontDescriptor".into()));
    descriptor.insert("FontName", Obj::Name("WFCffFixture".into()));
    descriptor.insert("Flags", Obj::Integer(4));
    descriptor.insert(
        "FontFile3",
        Obj::Stream {
            dict: file,
            raw: super::pdf_embedding_fixtures::font(true, 0),
        },
    );
    let mut desc = Dict::empty();
    desc.insert("Subtype", Obj::Name("CIDFontType0".into()));
    desc.insert("BaseFont", Obj::Name("WFCffFixture".into()));
    desc.insert("DW", Obj::Integer(999));
    desc.insert(
        "CIDSystemInfo",
        Obj::Dictionary(
            super::pdf_embedding::EmbeddingInfo::parse(&super::pdf_embedding_fixtures::font(
                true, 0,
            ))
            .unwrap()
            .system()
            .dictionary(),
        ),
    );
    desc.insert(
        "W",
        Obj::Array(vec![
            Obj::Integer(7),
            Obj::Array(vec![Obj::Integer(600)]),
            Obj::Integer(42),
            Obj::Array(vec![Obj::Integer(600)]),
            Obj::Integer(1000),
            Obj::Array(vec![Obj::Integer(300)]),
        ]),
    );
    desc.insert(
        "W2",
        Obj::Array(vec![
            Obj::Integer(7),
            Obj::Array(vec![Obj::Integer(-900), Obj::Integer(0), Obj::Integer(0)]),
            Obj::Integer(42),
            Obj::Array(vec![Obj::Integer(-900), Obj::Integer(0), Obj::Integer(0)]),
        ]),
    );
    desc.insert("FontDescriptor", Obj::Dictionary(descriptor));
    let mut font = Dict::empty();
    font.insert("Type", Obj::Name("Font".into()));
    font.insert("Subtype", Obj::Name("Type0".into()));
    font.insert("BaseFont", Obj::Name("WFCffFixture".into()));
    font.insert("Encoding", encoding);
    font.insert("ToUnicode", unicode);
    font.insert("DescendantFonts", Obj::Array(vec![Obj::Dictionary(desc)]));
    font
}

fn indirect_streams(object: &mut Obj, next: &mut u32, changes: &mut Vec<IncrementalObject>) {
    match object {
        Obj::Dictionary(dict) => {
            for (_, value) in dict.entries_mut() {
                indirect_streams(value, next, changes);
            }
        }
        Obj::Array(items) => {
            for item in items {
                indirect_streams(item, next, changes);
            }
        }
        Obj::Stream { .. } => {
            let number = *next;
            *next += 1;
            let object = std::mem::replace(
                object,
                Obj::Reference {
                    number,
                    generation: 0,
                },
            );
            changes.push(IncrementalObject {
                number,
                generation: 0,
                object,
            });
        }
        _ => {}
    }
}
pub(crate) fn pdf(vertical: bool, body: &[u8]) -> Vec<u8> {
    pdf_with_font(font_dictionary(vertical), body)
}
pub(crate) fn pdf_with_font(font_dict: Dict, body: &[u8]) -> Vec<u8> {
    let mut builder = crate::authoring::PdfBuilder::new();
    builder.add_page(crate::authoring::PageSize::LETTER);
    let engine = crate::ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    let reader = engine.document().reader();
    let page = engine.document().get_page(1).unwrap();
    let mut updates = Vec::new();
    let mut next = 100;
    let mut font = Obj::Dictionary(font_dict);
    indirect_streams(&mut font, &mut next, &mut updates);
    let mut fonts = Dict::empty();
    fonts.insert("F", font);
    let mut resources = Dict::empty();
    resources.insert("Font", Obj::Dictionary(fonts));
    let mut page_dict = reader
        .get_object(page.object_number, page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    page_dict.insert("Resources", Obj::Dictionary(resources));
    page_dict.insert(
        "Contents",
        Obj::Reference {
            number: next,
            generation: 0,
        },
    );
    updates.push(IncrementalObject {
        number: next,
        generation: 0,
        object: stream(body.to_vec()),
    });
    updates.push(IncrementalObject {
        number: page.object_number,
        generation: page.generation_number,
        object: Obj::Dictionary(page_dict),
    });
    write_incremental_update(reader, updates).unwrap()
}

#[test]
fn one_through_four_byte_codes_keep_value_length_and_exact_byte_ranges() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    resolver.validate_source_encoding().unwrap();
    assert_eq!(resolver.code_size(), 0);
    let codes = resolver
        .codes(ENCODED)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        codes
            .iter()
            .map(|c| (c.byte_start, c.byte_end))
            .collect::<Vec<_>>(),
        [(0, 1), (1, 3), (3, 6), (6, 10), (10, 11)]
    );
    assert_eq!(
        codes.iter().map(|c| c.code.len()).collect::<Vec<_>>(),
        [1, 2, 3, 4, 1]
    );
    assert_ne!(codes[0].code, codes[1].code);
    assert_eq!(codes[0].code.value(), codes[1].code.value());
    assert_eq!(resolver.try_decode_string(ENCODED).unwrap(), LOGICAL);
    assert_eq!(
        codes
            .iter()
            .map(|c| resolver.cid_for_character(c.code).unwrap())
            .collect::<Vec<_>>(),
        [42, 7, 42, 7, 1000]
    );
}

#[test]
fn decoded_lengths_control_word_spacing_and_vertical_metrics() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(true));
    let codes = resolver
        .codes(ENCODED)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(resolver.is_vertical());
    assert_eq!(
        resolver.vertical_metrics_for_code(codes[2].code),
        (-900.0, 0.0, 0.0)
    );
    assert_eq!(resolver.width_for_code(codes[4].code), 300.0);
    assert!(codes[4].code.is_word_space());
    assert!(!Code::new(32, 2).unwrap().is_word_space());
}

#[test]
fn reverse_encoding_preserves_ligature_and_four_byte_source_codes() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    assert_eq!(
        resolver.try_encode_existing(LOGICAL).unwrap(),
        (ENCODED.to_vec(), false)
    );
    assert!(resolver.try_encode_existing("unmapped").is_err());
}

#[test]
fn renderer_and_extractor_use_the_same_original_code_boundaries() {
    for vertical in [false, true] {
        let bytes = pdf(
            vertical,
            b"BT /F 10 Tf 2 Tw 1 Tc 1 0 0 1 30 700 Tm <4100418100019000004120> Tj ET",
        );
        let engine = crate::ContentEngine::open_bytes(bytes).unwrap();
        let resources = engine.get_page_resources(1).unwrap();
        let reader = engine.document().reader();
        let glyphs =
            crate::render::text_decode::try_decode_text_bytes(ENCODED, "F", &resources, reader)
                .unwrap();
        assert_eq!(
            glyphs.iter().map(|g| g.code).collect::<Vec<_>>(),
            [1, 2, 1, 2, 3]
        );
        assert_eq!(
            glyphs.iter().map(|g| g.is_space).collect::<Vec<_>>(),
            [false, false, false, false, true]
        );
        assert!(glyphs.iter().all(|g| g.is_vertical == vertical));
        let chunks = engine.collect_page_text_chunks(1).unwrap();
        assert_eq!(
            chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
            LOGICAL
        );
        if !vertical {
            assert!((chunks[0].width - 34.0).abs() < 1e-8);
        }
    }
}

#[test]
fn full_four_byte_code_space_is_compiled_without_enumerating_codes() {
    let program=Program::parse(b"1 begincodespacerange <00000000> <ffffffff> endcodespacerange 1 begincidchar <ffffffff> 42 endcidchar",Kind::Cid,None,false).unwrap();
    let mut offset = 0;
    let code = program.space.next(&[255; 4], &mut offset).unwrap();
    assert_eq!(code.value(), u32::MAX);
    assert_eq!(program.cid(code), 42);
    assert_eq!(program.cids.len(), 1);
}

#[test]
fn rectangular_bounds_prefix_conflicts_and_truncation_are_explicit() {
    let space = CodeSpace::new(vec![CodeRange::new(
        Code::new(0x814000, 3).unwrap(),
        Code::new(0x81fc7f, 3).unwrap(),
    )
    .unwrap()])
    .unwrap();
    assert!(space.contains(Code::new(0x818050, 3).unwrap()));
    assert!(!space.contains(Code::new(0x818080, 3).unwrap()));
    assert!(!space.contains(Code::new(0x813f50, 3).unwrap()));
    let conflict = vec![
        CodeRange::new(Code::new(0x81, 1).unwrap(), Code::new(0x81, 1).unwrap()).unwrap(),
        CodeRange::new(Code::new(0x8100, 2).unwrap(), Code::new(0x81ff, 2).unwrap()).unwrap(),
    ];
    assert!(CodeSpace::new(conflict).is_err());
    let mut offset = 0;
    assert!(space.next(&[0x81, 0x80], &mut offset).is_err());
    assert_eq!(offset, 0);
    let mut beyond_input = usize::MAX;
    assert!(space.next(&[], &mut beyond_input).is_err());
    assert_eq!(beyond_input, usize::MAX);
}

#[test]
fn malformed_sequences_are_not_zero_padded_or_partially_encodable() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    for input in [vec![0], vec![0x81, 0], vec![0x90, 0, 0], vec![0x42]] {
        assert!(resolver.try_decode_string(&input).is_err());
        let mut iter = resolver.codes(&input);
        assert!(iter.next().unwrap().is_err());
        assert!(iter.next().is_none());
    }
}

#[test]
fn cancellation_stops_parsing_decoding_and_reverse_encoding() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| {
        assert!(ToUnicodeCMap::try_parse(b"1 beginbfchar <41> <0041> endbfchar").is_err());
        let map = ToUnicodeCMap::parse(b"1 beginbfchar <41> <0041> endbfchar");
        assert!(map.is_empty() && map.validate().is_err());
        assert!(resolver.try_decode_string(ENCODED).is_err());
        assert!(resolver.try_encode_existing(LOGICAL).is_err());
    });
    assert_eq!(resolver.try_decode_string(ENCODED).unwrap(), LOGICAL);
}

#[test]
fn tounicode_grammar_ignores_operator_words_inside_metadata() {
    let map = ToUnicodeCMap::try_parse(
        b"/Notice (1 beginbfchar <41> <0058> endbfchar) def
        /Meta << /Text (beginbfrange) >> def
        1 begincodespacerange <00> <ff> endcodespacerange
        1 beginbfchar <41> <0041> endbfchar",
    )
    .unwrap();
    assert_eq!(map.lookup(65), Some("A"));
    let broken = ToUnicodeCMap::parse(b"2 beginbfchar <41> <0041> endbfchar");
    assert!(broken.validate().is_err());
    assert!(broken.is_empty());
}

#[test]
fn tounicode_four_byte_ranges_arrays_and_utf16_scalars_are_retained() {
    let map = ToUnicodeCMap::try_parse(
        b"1 begincodespacerange <90000000> <90ffffff> endcodespacerange
        1 beginbfrange <90000041> <90000043> [<00660069> <d83dde00> <0041>] endbfrange",
    )
    .unwrap();
    assert_eq!(
        map.lookup_code(Code::new(0x90000041, 4).unwrap()),
        Some("fi")
    );
    assert_eq!(
        map.lookup_code(Code::new(0x90000042, 4).unwrap()),
        Some("\u{1f600}")
    );
    assert_eq!(
        map.lookup_code(Code::new(0x90000043, 4).unwrap()),
        Some("A")
    );
    assert_eq!(map.code_size(), 4);
}

#[test]
fn invalid_utf16_and_oversized_ranges_fail_before_partial_output() {
    for cmap in [
        "1 beginbfchar <41> <d800> endbfchar",
        "1 beginbfchar <41> <004142> endbfchar",
        "1 beginbfchar <41> [<0041>] endbfchar",
        "1 beginbfrange <41> <42> [<0041>] endbfrange",
        "1 beginbfrange <41> <42> <ffff> endbfrange",
        "1 beginbfrange <00000000> <ffffffff> <0041> endbfrange",
    ] {
        assert!(ToUnicodeCMap::try_parse(cmap.as_bytes()).is_err(), "{cmap}");
    }
    let carry = ToUnicodeCMap::try_parse(b"1 beginbfrange <41> <42> <00ff> endbfrange").unwrap();
    assert_eq!(carry.lookup(0x41), Some("\u{00ff}"));
    assert_eq!(carry.lookup(0x42), Some("\u{0100}"));
}

#[test]
fn later_mapping_definitions_replace_earlier_ranges() {
    let program = Program::parse(
        b"1 begincodespacerange <00> <ff> endcodespacerange
        1 begincidrange <41> <43> 10 endcidrange
        1 begincidchar <42> 99 endcidchar
        1 begincidrange <42> <43> 20 endcidrange",
        Kind::Cid,
        None,
        false,
    )
    .unwrap();
    assert_eq!(program.cid(Code::new(65, 1).unwrap()), 10);
    assert_eq!(program.cid(Code::new(66, 1).unwrap()), 20);
    assert_eq!(program.cid(Code::new(67, 1).unwrap()), 21);
    let unicode = ToUnicodeCMap::try_parse(
        b"1 beginbfrange <41> <43> <0041> endbfrange
        1 beginbfchar <42> <0058> endbfchar",
    )
    .unwrap();
    assert_eq!(unicode.lookup(65), Some("A"));
    assert_eq!(unicode.lookup(66), Some("X"));
    assert_eq!(unicode.lookup(67), Some("C"));
    assert!(unicode.codes_for_text("B").is_empty());
}

#[test]
fn tounicode_destination_limit_and_final_byte_increment_are_preserved() {
    let at_limit = format!("1 beginbfchar <41> <{}> endbfchar", "0041".repeat(256));
    let map = ToUnicodeCMap::try_parse(at_limit.as_bytes()).unwrap();
    assert_eq!(map.lookup(65), Some("A".repeat(256).as_str()));
    let too_long = format!("1 beginbfchar <41> <{}> endbfchar", "0041".repeat(257));
    assert!(ToUnicodeCMap::try_parse(too_long.as_bytes()).is_err());
    let map = ToUnicodeCMap::try_parse(b"1 beginbfrange <41> <42> <00660068> endbfrange").unwrap();
    assert_eq!(map.lookup(65), Some("fh"));
    assert_eq!(map.lookup(66), Some("fi"));
}

#[test]
fn tounicode_inheritance_overrides_selected_codes_atomically() {
    let base = stream(
        b"/CMapName /BaseUnicode def 1 begincodespacerange <00> <ff> endcodespacerange
        2 beginbfchar <41> <0041> <42> <0042> endbfchar"
            .to_vec(),
    );
    let mut dict = Dict::empty();
    dict.insert("UseCMap", base);
    let child = Obj::Stream {
        dict,
        raw: b"/BaseUnicode usecmap 1 beginbfchar <42> <00660069> endbfchar".to_vec(),
    };
    let map = ToUnicodeCMap::load(&child, None);
    map.validate().unwrap();
    assert_eq!(map.lookup(65), Some("A"));
    assert_eq!(map.lookup(66), Some("fi"));
}

#[test]
fn contradictory_tounicode_lengths_are_rejected_for_source_mutation() {
    let mut font = font_dictionary(false);
    font.insert(
        "ToUnicode",
        stream(b"1 beginbfchar <0042> <0042> endbfchar".to_vec()),
    );
    let resolver = FontResolver::new_from_dict_only(&font);
    assert!(resolver.validate_encoding().is_ok());
    assert!(resolver.validate_source_encoding().is_err());
    assert!(resolver.try_encode_existing("B").is_err());
}

#[test]
fn reverse_mapping_reports_alternative_complete_paths() {
    let mut font = Dict::empty();
    font.insert("Subtype", Obj::Name("Type1".into()));
    font.insert(
        "ToUnicode",
        stream(
            b"1 begincodespacerange <00> <ff> endcodespacerange
        3 beginbfchar <01> <0066> <02> <0069> <03> <00660069> endbfchar"
                .to_vec(),
        ),
    );
    let resolver = FontResolver::new_from_dict_only(&font);
    assert_eq!(resolver.try_encode_existing("fi").unwrap(), (vec![3], true));
}

#[test]
fn legacy_numeric_projection_does_not_collapse_distinct_length_mappings() {
    let bytes = b"2 begincodespacerange <41> <41> <0041> <0041> endcodespacerange
        2 beginbfchar <41> <0041> <0041> <0042> endbfchar";
    let map = ToUnicodeCMap::try_parse(bytes).unwrap();
    assert_eq!(map.lookup_code(Code::new(65, 1).unwrap()), Some("A"));
    assert_eq!(map.lookup_code(Code::new(65, 2).unwrap()), Some("B"));
    assert!(super::cmap::parse_to_unicode_cmap(bytes).get(&65).is_none());
}
