//! Regression source only. No compilation, font execution or PDF workloads were
//! performed while implementing this source-only increment.
use super::*;
use crate::fonts::{font_container::Container, pdf_embedding::EmbeddingInfo};
use std::collections::BTreeMap;

fn checksum(bytes: &[u8]) -> u32 {
    bytes.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}
fn put(out: &mut [u8], at: usize, n: u32) {
    out[at..at + 4].copy_from_slice(&n.to_be_bytes());
}
fn align(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// Generates a real file-relative TTC directory with exact identical table
/// sharing across faces; no downloaded or host-installed collection fixture.
pub(crate) fn collection(fonts: &[&[u8]], version: u32, signature: bool) -> Vec<u8> {
    let containers = fonts
        .iter()
        .map(|font| Container::parse(font).unwrap())
        .collect::<Vec<_>>();
    let header_len = 12 + fonts.len() * 4 + if version == 0x00020000 { 12 } else { 0 };
    let mut out = vec![0; header_len];
    out[..4].copy_from_slice(b"ttcf");
    put(&mut out, 4, version);
    put(&mut out, 8, fonts.len() as u32);
    let mut directories = Vec::new();
    for (index, container) in containers.iter().enumerate() {
        let at = out.len();
        directories.push(at);
        put(&mut out, 12 + index * 4, at as u32);
        out.resize(at + 12 + container.faces[0].tables.len() * 16, 0);
    }
    let mut shared = BTreeMap::<([u8; 4], Vec<u8>), u32>::new();
    for (index, container) in containers.iter().enumerate() {
        let face = &container.faces[0];
        let at = directories[index];
        out[at..at + 4].copy_from_slice(&face.version);
        out[at + 4..at + 6].copy_from_slice(&(face.tables.len() as u16).to_be_bytes());
        for (entry, (tag, range)) in face.tables.iter().enumerate() {
            let data = &fonts[index][range.clone()];
            let key = (*tag, data.to_vec());
            let offset = *shared.entry(key).or_insert_with(|| {
                align(&mut out);
                let offset = out.len() as u32;
                out.extend_from_slice(data);
                offset
            });
            let record = at + 12 + entry * 16;
            out[record..record + 4].copy_from_slice(tag);
            put(&mut out, record + 4, checksum(data));
            put(&mut out, record + 8, offset);
            put(&mut out, record + 12, data.len() as u32);
        }
    }
    if signature {
        assert_eq!(version, 0x00020000);
        align(&mut out);
        let offset = out.len() as u32;
        // A declared DSIG, not a cryptographically valid signature fixture.
        out.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
        let at = 12 + fonts.len() * 4;
        put(&mut out, at, u32::from_be_bytes(*b"DSIG"));
        put(&mut out, at + 4, 8);
        put(&mut out, at + 8, offset);
    }
    align(&mut out);
    out
}
fn source() -> &'static [u8] {
    crate::render::get_fallback_font("Helvetica").unwrap()
}
fn selection(bytes: &[u8], index: u32) -> FontFaceSelection {
    FontFaceSelection {
        source_sha256: hash(bytes).unwrap(),
        face_index: index,
        allow_signature_removal: false,
    }
}
fn tables(bytes: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    Container::parse(bytes).unwrap().faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, bytes[range.clone()].to_vec()))
        .collect()
}

#[test]
fn discover_and_extract_both_ttc_versions_with_shared_tables() {
    let second = crate::render::get_fallback_font("Times-Roman").unwrap();
    for version in [0x00010000, 0x00020000] {
        let bytes = collection(&[source(), second, source()], version, false);
        let catalog = inspect_font_asset(&bytes).unwrap();
        assert!(catalog.collection);
        assert_eq!(catalog.faces.len(), 3);
        assert_eq!(catalog.faces[1].face_index, 1);
        let input = Container::parse(&bytes).unwrap();
        assert_eq!(input.faces[0].tables, input.faces[2].tables);
        for (index, original) in [source(), second, source()].into_iter().enumerate() {
            let prepared = prepare_font_asset(&bytes, &selection(&bytes, index as u32)).unwrap();
            assert_eq!(checksum(&prepared.bytes), 0xB1B0AFBA);
            assert_eq!(prepared.report.face_index, index as u32);
            assert!(prepared.report.extracted_collection);
            let before = tables(original);
            let after = tables(&prepared.bytes);
            assert_eq!(before.len(), after.len());
            for (tag, mut data) in before {
                let mut actual = after[&tag].clone();
                if tag == *b"head" {
                    data[8..12].fill(0);
                    actual[8..12].fill(0);
                }
                assert_eq!(data, actual, "table {:?}", tag);
            }
        }
    }
}

#[test]
fn standalone_preparation_is_byte_exact_and_does_not_drop_signature_table() {
    let mut data = tables(source());
    data.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]);
    let bytes = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], data).unwrap();
    let prepared = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    assert_eq!(prepared.bytes, bytes);
    assert!(!prepared.report.removed_signature);
    assert!(!prepared.report.signature_verified);
    assert!(!prepared.report.extracted_collection);
    assert_eq!(
        prepared.report.source_sha256,
        prepared.report.prepared_sha256
    );
}

#[test]
fn collection_dsig_removal_requires_explicit_hash_bound_approval() {
    let bytes = collection(&[source()], 0x00020000, true);
    assert!(inspect_font_asset(&bytes).unwrap().faces[0].signature_present);
    let mut choice = selection(&bytes, 0);
    assert!(prepare_font_asset(&bytes, &choice)
        .unwrap_err()
        .to_string()
        .contains("approval"));
    choice.allow_signature_removal = true;
    let prepared = prepare_font_asset(&bytes, &choice).unwrap();
    assert!(prepared.report.removed_signature);
    assert!(!prepared.report.signature_verified);
    assert!(!tables(&prepared.bytes).contains_key(b"DSIG"));
}

#[test]
fn per_face_dsig_is_not_copied_into_extracted_standalone() {
    let mut data = tables(source());
    data.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]);
    let signed = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], data).unwrap();
    let bytes = collection(&[&signed], 0x00010000, false);
    let mut choice = selection(&bytes, 0);
    assert!(prepare_font_asset(&bytes, &choice).is_err());
    choice.allow_signature_removal = true;
    assert!(
        prepare_font_asset(&bytes, &choice)
            .unwrap()
            .report
            .removed_signature
    );
}

#[test]
fn stale_hash_and_out_of_range_face_never_choose_first_face() {
    let bytes = collection(&[source()], 0x00010000, false);
    let mut choice = selection(&bytes, 0);
    choice.source_sha256 = "0".repeat(64);
    assert!(prepare_font_asset(&bytes, &choice).is_err());
    assert!(prepare_font_asset(&bytes, &selection(&bytes, 1)).is_err());
    assert!(prepare_font_asset(source(), &selection(source(), 1)).is_err());
}

#[test]
fn unsupported_and_truncated_collection_headers_are_errors() {
    let original = collection(&[source()], 0x00020000, false);
    for length in [0, 4, 8, 12, 16, 27, 35] {
        assert!(inspect_font_asset(&original[..length]).is_err());
    }
    for (at, value) in [
        (4, 0x00030000),
        (8, 0),
        (8, 257),
        (12, u32::MAX),
        (12, 0),
        (16, 1),
    ] {
        let mut bytes = original.clone();
        put(&mut bytes, at, value);
        assert!(inspect_font_asset(&bytes).is_err());
    }
}

#[test]
fn duplicate_tags_overlapping_headers_and_bad_table_extents_are_errors() {
    let original = collection(&[source()], 0x00010000, false);
    let at = u32::from_be_bytes(original[12..16].try_into().unwrap()) as usize;
    let mut duplicate = original.clone();
    let tag = duplicate[at + 12..at + 16].to_vec();
    duplicate[at + 28..at + 32].copy_from_slice(&tag);
    assert!(inspect_font_asset(&duplicate).is_err());
    for (field, value) in [(at + 20, 0), (at + 20, u32::MAX), (at + 24, u32::MAX)] {
        let mut bytes = original.clone();
        put(&mut bytes, field, value);
        assert!(inspect_font_asset(&bytes).is_err());
    }
}

#[test]
fn editable_permissions_and_no_subsetting_survive_extraction() {
    for (bits, allowed) in [
        (0, true),
        (2, false),
        (4, false),
        (8, true),
        (0x100, true),
        (0x208, false),
    ] {
        let font = crate::fonts::pdf_embedding_fixtures::with_rights(source().to_vec(), bits);
        let bytes = collection(&[&font], 0x00010000, false);
        let catalog = inspect_font_asset(&bytes).unwrap();
        assert_eq!(catalog.faces[0].permission_bits_allow_editing, allowed);
        let result = prepare_font_asset(&bytes, &selection(&bytes, 0));
        assert_eq!(result.is_ok(), allowed);
        if let Ok(prepared) = result {
            assert_eq!(prepared.report.subsetting_allowed, bits & 0x100 == 0);
            assert_eq!(tables(&prepared.bytes)[b"OS/2"], tables(&font)[b"OS/2"]);
        }
    }
}

#[test]
fn otc_extraction_preserves_cff1_native_cids_and_ros() {
    let named = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let keyed = crate::fonts::pdf_embedding_fixtures::font(true, 0);
    let bytes = collection(&[&named, &keyed], 0x00020000, false);
    for (index, expected) in [(0, vec![0, 1, 2, 3]), (1, vec![0, 42, 7, 1000])] {
        let prepared = prepare_font_asset(&bytes, &selection(&bytes, index)).unwrap();
        assert!(prepared.bytes.starts_with(b"OTTO"));
        let identity = EmbeddingInfo::parse(&prepared.bytes).unwrap().cff.unwrap();
        assert_eq!(identity.gid_to_cid, expected);
        assert_eq!(
            identity.system.registry,
            if index == 1 {
                b"Wellfriend".as_slice()
            } else {
                b"Adobe".as_slice()
            }
        );
    }
}

#[test]
fn cff2_is_discovered_but_never_relabelled_cff1() {
    let mut data = tables(&crate::fonts::pdf_embedding_fixtures::font(false, 0));
    // Deliberately not a valid CFF2 program: format detection must not ignore its
    // presence and proceed as if a glyf/CFF1 program had been selected.
    data.remove(b"CFF ");
    data.insert(*b"CFF2", vec![2, 0, 5, 0, 0]);
    let font = crate::fonts::sfnt_subset::build_sfnt(*b"OTTO", data).unwrap();
    let bytes = collection(&[&font], 0x00010000, false);
    assert_eq!(
        inspect_font_asset(&bytes).unwrap().faces[0].outline_format,
        OutlineFormat::Cff2
    );
    assert!(prepare_font_asset(&bytes, &selection(&bytes, 0)).is_err());
}

#[test]
fn extracted_layout_tables_produce_identical_shaping_to_original_standalone() {
    let bytes = collection(&[source()], 0x00010000, false);
    let prepared = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    for text in ["AVATAR office", "a\u{301}", "שלום 123 ABC"] {
        let expected = crate::fonts::TextShaper::shape(source(), text, Default::default()).unwrap();
        let actual =
            crate::fonts::TextShaper::shape(&prepared.bytes, text, Default::default()).unwrap();
        assert_eq!(actual, expected);
    }
}

#[test]
fn rebuilt_head_directory_checksum_uses_zero_adjustment() {
    let bytes = collection(&[source()], 0x00010000, false);
    let prepared = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    let data = &prepared.bytes;
    let count = u16::from_be_bytes(data[4..6].try_into().unwrap());
    for i in 0..usize::from(count) {
        let at = 12 + i * 16;
        let offset = u32::from_be_bytes(data[at + 8..at + 12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(data[at + 12..at + 16].try_into().unwrap()) as usize;
        let mut table = data[offset..offset + length].to_vec();
        if &data[at..at + 4] == b"head" {
            table[8..12].fill(0);
        }
        assert_eq!(
            checksum(&table),
            u32::from_be_bytes(data[at + 4..at + 8].try_into().unwrap())
        );
    }
}

#[test]
fn authoring_selected_otc_face_embeds_and_reopens() {
    use crate::authoring::{PageSize, PdfBuilder, TextStyle};
    let font = crate::fonts::pdf_embedding_fixtures::font(true, 0);
    let bytes = collection(&[source(), &font], 0x00010000, false);
    let mut builder = PdfBuilder::new();
    let (face, report) = builder
        .register_font_face_bytes("Chosen", &bytes, &selection(&bytes, 1))
        .unwrap();
    assert_eq!(report.face_index, 1);
    builder
        .add_page(PageSize::LETTER)
        .draw_text("AB A", 30.0, 700.0, &TextStyle::new(face, 12.0))
        .unwrap();
    let output = builder.to_bytes().unwrap();
    let engine = crate::ContentEngine::open_bytes(output).unwrap();
    assert!(engine.get_page_text(1).unwrap().contains("AB A"));
}

#[test]
fn provider_registration_is_atomic_on_stale_selection() {
    use crate::fonts::{FontMatchRequest, FontProvider, RegisteredFontProvider};
    let mut provider = RegisteredFontProvider::default();
    let bytes = collection(&[source()], 0x00010000, false);
    let report = provider
        .register_font_face_bytes("Chosen", &bytes, &selection(&bytes, 0))
        .unwrap();
    let fingerprint = provider.cache_fingerprint().to_owned();
    let matched = provider
        .match_font(&FontMatchRequest::new("Chosen"))
        .unwrap();
    assert_eq!(hash(&matched.bytes).unwrap(), report.prepared_sha256);
    let mut stale = selection(&bytes, 0);
    stale.source_sha256.clear();
    assert!(provider
        .register_font_face_bytes("Chosen", &bytes, &stale)
        .is_err());
    assert_eq!(provider.cache_fingerprint(), fingerprint);
}

#[test]
fn cancelled_font_preparation_does_not_register_a_face() {
    let bytes = collection(&[source()], 0x00010000, false);
    let choice = selection(&bytes, 0);
    let cancel = crate::CancelToken::new();
    cancel.cancel();
    assert!(cancel.scope(|| inspect_font_asset(&bytes)).is_err());
    assert!(cancel
        .scope(|| prepare_font_asset(&bytes, &choice))
        .is_err());
    let mut provider = crate::fonts::RegisteredFontProvider::default();
    assert!(cancel
        .scope(|| provider.register_font_face_bytes("Chosen", &bytes, &choice))
        .is_err());
    assert!(provider.is_empty());
}

#[test]
fn output_expansion_budget_is_checked_before_materialization() {
    let bytes = collection(&[source()], 0x00010000, false);
    assert!(matches!(
        prepare_font_asset_bounded(&bytes, &selection(&bytes, 0), 1024),
        Err(WellfriendError::ResourceLimit(_))
    ));
    assert!(matches!(
        prepare_font_asset_bounded(source(), &selection(source(), 0), 1024),
        Err(WellfriendError::ResourceLimit(_))
    ));
}

#[test]
fn malformed_permission_bits_do_not_turn_into_installable_rights() {
    let mut data = tables(source());
    data.insert(*b"OS/2", vec![0]);
    let bytes = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], data).unwrap();
    assert!(inspect_font_asset(&bytes)
        .unwrap_err()
        .to_string()
        .contains("OS/2"));
    assert!(prepare_font_asset(&bytes, &selection(&bytes, 0)).is_err());
}

#[test]
fn variable_face_extraction_preserves_axes_and_does_not_claim_static_instantiation() {
    let mut data = tables(source());
    let mut fvar = vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0, 0, 0, 8];
    fvar.extend_from_slice(b"wght");
    for value in [100i32, 400, 900] {
        fvar.extend_from_slice(&(value * 65536).to_be_bytes());
    }
    fvar.extend_from_slice(&[0, 0, 1, 0]);
    data.insert(*b"fvar", fvar.clone());
    let variable = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], data).unwrap();
    let bytes = collection(&[&variable], 0x00010000, false);
    let prepared = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    assert_eq!(tables(&prepared.bytes)[b"fvar"], fvar);
    assert_eq!(prepared.report.retained_variation_axes.len(), 1);
    assert_eq!(prepared.report.retained_variation_axes[0].tag, "wght");
    assert_eq!(prepared.report.retained_variation_axes[0].default, 400.0);
}

#[test]
fn exact_face_directory_aliases_remain_explicitly_selectable() {
    let mut bytes = collection(&[source(), source()], 0x00010000, false);
    let first = u32::from_be_bytes(bytes[12..16].try_into().unwrap());
    put(&mut bytes, 16, first);
    assert_eq!(inspect_font_asset(&bytes).unwrap().faces.len(), 2);
    let first = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    let second = prepare_font_asset(&bytes, &selection(&bytes, 1)).unwrap();
    assert_eq!(first.bytes, second.bytes);
    assert_ne!(first.report.face_index, second.report.face_index);
}

#[test]
fn interleaved_opaque_table_ranges_are_copied_without_disjointness_assumption() {
    let mut data = tables(source());
    data.insert(*b"TST1", vec![1, 2, 3, 4, 5, 6, 7, 8]);
    data.insert(*b"TST2", vec![5, 6, 7, 8]);
    let source = crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], data).unwrap();
    let mut bytes = collection(&[&source], 0x00010000, false);
    let parsed = Container::parse(&bytes).unwrap();
    let offset = parsed.faces[0].tables[b"TST1"].start as u32;
    let directory = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
    let position = parsed.faces[0]
        .tables
        .keys()
        .position(|tag| tag == b"TST2")
        .unwrap();
    put(&mut bytes, directory + 12 + position * 16 + 8, offset + 4);
    let prepared = prepare_font_asset(&bytes, &selection(&bytes, 0)).unwrap();
    let result = tables(&prepared.bytes);
    assert_eq!(result[b"TST1"], vec![1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(result[b"TST2"], vec![5, 6, 7, 8]);
}
