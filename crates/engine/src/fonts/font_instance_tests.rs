//! Complete-publication regression source. No tests/workloads executed here.
use super::super::variation_store::{u16_at, u32_at};
use super::*;
fn words(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|n| n.to_be_bytes()).collect()
}
pub(crate) fn tables(source: &[u8]) -> BTreeMap<[u8; 4], Vec<u8>> {
    Container::parse(source).unwrap().faces[0]
        .tables
        .iter()
        .map(|(tag, range)| (*tag, source[range.clone()].to_vec()))
        .collect()
}
pub(crate) fn source() -> Vec<u8> {
    let base = crate::fonts::pdf_embedding_fixtures::font(false, 0);
    let mut t = tables(&base);
    t.remove(b"CFF ");
    let mut head = t[b"head"].clone();
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    t.insert(*b"head", head);
    let mut glyph = words(&[1, 0, 0, 100, 100, 2, 0]);
    glyph.extend([1, 1, 1]);
    glyph.extend(words(&[0, 100, -100, 0, 0, 100]));
    glyph.push(0);
    assert_eq!(glyph.len(), 30);
    t.insert(*b"glyf", [glyph.clone(), glyph].concat());
    t.insert(
        *b"loca",
        [0u32, 0, 30, 60, 60]
            .into_iter()
            .flat_map(u32::to_be_bytes)
            .collect(),
    );
    let mut maxp = vec![0; 32];
    maxp[1] = 1;
    maxp[5] = 4;
    maxp[15] = 2;
    t.insert(*b"maxp", maxp);
    let mut fvar = words(&[1, 0, 16, 2, 1, 20, 0, 8]);
    fvar.extend(b"TEST");
    for value in [0i32, 0, 65536] {
        fvar.extend(value.to_be_bytes());
    }
    fvar.extend(words(&[0, 256]));
    t.insert(*b"fvar", fvar);
    // Explicit x deltas for points 0/1/2 and right phantom 4. No IUP ambiguity.
    let mut payload = vec![4, 0x83];
    payload.extend(words(&[0, 1, 1, 2]));
    payload.push(0x43);
    payload.extend(words(&[0, 100, 0, 40]));
    payload.push(0x83);
    let mut tuple = words(&[1, 10, payload.len() as i16, 0xa000u16 as i16, 16384]);
    tuple.extend(payload);
    let mut gvar = words(&[1, 0, 1, 0]);
    gvar.extend(0u32.to_be_bytes());
    gvar.extend(words(&[4, 1]));
    gvar.extend(40u32.to_be_bytes());
    for offset in [0, 0, tuple.len(), tuple.len(), tuple.len()] {
        gvar.extend((offset as u32).to_be_bytes());
    }
    gvar.extend(tuple);
    t.insert(*b"gvar", gvar);
    crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], t).unwrap()
}
pub(crate) fn request(source: &[u8]) -> FontInstanceRequest {
    FontInstanceRequest {
        selection: FontFaceSelection {
            source_sha256: digest(source).unwrap(),
            face_index: 0,
            allow_signature_removal: false,
        },
        coordinates: BTreeMap::from([("TEST".into(), 0.5)]),
        naming: FontInstanceNaming {
            family: "Fixture".into(),
            subfamily: "Half".into(),
            legacy_family: "Fixture Half".into(),
            postscript_name: "WFInstance-Half".into(),
            style_link: FontStyleLink::Regular,
        },
        accept_redundant_metric_differences: false,
        cff2_contours: None,
    }
}
fn repack(source: &[u8], change: impl FnOnce(&mut BTreeMap<[u8; 4], Vec<u8>>)) -> Vec<u8> {
    let mut t = tables(source);
    change(&mut t);
    crate::fonts::sfnt_subset::build_sfnt([0, 1, 0, 0], t).unwrap()
}
#[test]
fn public_asset_has_actual_selected_outlines_metrics_names_and_no_variable_tables() {
    let source = source();
    let choice = request(&source);
    let output = prepare_font_instance(&source, &choice).unwrap();
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    assert!(!face.is_variable());
    assert_eq!(face.glyph_hor_advance(ttf_parser::GlyphId(1)), Some(620));
    assert_eq!(
        face.glyph_bounding_box(ttf_parser::GlyphId(1))
            .unwrap()
            .x_max,
        150
    );
    assert_eq!(face.glyph_index('A'), Some(ttf_parser::GlyphId(1)));
    let t = tables(&output.bytes);
    assert!(t.keys().all(|tag| !retired(tag)));
    assert_eq!(&t[b"prep"], &[0xb0, 0x91, 0x89, 0x41, 1, 0x20, 0, 0x2d]);
    assert_eq!(
        face.names()
            .into_iter()
            .find(|n| n.name_id == 6 && n.platform_id == ttf_parser::PlatformId::Windows)
            .unwrap()
            .to_string()
            .unwrap(),
        "WFInstance-Half"
    );
    assert_eq!(output.report.normalized_coordinates, [8192]);
    assert!(output.report.structural_postconditions_checked);
    assert!(!output.report.independently_render_verified);
    assert_eq!(
        output.report.prepared_sha256,
        digest(&output.bytes).unwrap()
    );
}
#[test]
fn publication_is_deterministic_and_default_coordinate_is_explicit_in_receipt() {
    let source = source();
    let mut choice = request(&source);
    choice.coordinates.clear();
    let a = prepare_font_instance(&source, &choice).unwrap();
    let b = prepare_font_instance(&source, &choice).unwrap();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.report.coordinates["TEST"], 0.);
    assert_eq!(
        ttf_parser::Face::parse(&a.bytes, 0)
            .unwrap()
            .glyph_hor_advance(ttf_parser::GlyphId(1)),
        Some(600)
    );
}
#[test]
fn unknown_nonfinite_out_of_range_coordinates_and_stale_source_fail_before_publication() {
    let source = source();
    for value in [-0.1, 1.1, f32::NAN, f32::INFINITY] {
        let mut r = request(&source);
        r.coordinates.insert("TEST".into(), value);
        assert!(prepare_font_instance(&source, &r).is_err());
    }
    let mut r = request(&source);
    r.coordinates.insert("FAKE".into(), 0.);
    assert!(prepare_font_instance(&source, &r).is_err());
    r = request(&source);
    r.selection.source_sha256.clear();
    assert!(prepare_font_instance(&source, &r).is_err());
    r = request(&source);
    r.selection.face_index = 1;
    assert!(prepare_font_instance(&source, &r).is_err());
}
#[test]
fn unknown_and_unimplemented_variation_owners_are_not_silently_discarded() {
    for tag in [*b"COLR", *b"VARC", *b"MATH", *b"PRIV", *b"hdmx"] {
        let source = repack(&source(), |t| {
            t.insert(tag, vec![0; 8]);
        });
        let error = prepare_font_instance(&source, &request(&source))
            .unwrap_err()
            .to_string();
        assert!(error.contains(&super::tag(&tag)));
    }
}
#[test]
fn standalone_and_collection_signatures_require_explicit_removal_approval() {
    let signed = repack(&source(), |t| {
        t.insert(*b"DSIG", vec![0; 8]);
    });
    for bytes in [
        signed.clone(),
        crate::fonts::font_asset::tests::collection(&[&signed], 0x20000, true),
    ] {
        let mut r = request(&bytes);
        assert!(prepare_font_instance(&bytes, &r).is_err());
        r.selection.allow_signature_removal = true;
        let asset = prepare_font_instance(&bytes, &r).unwrap();
        assert!(asset.report.removed_signature);
        assert!(!asset.report.signature_verified);
        assert!(!tables(&asset.bytes).contains_key(b"DSIG"));
    }
}
#[test]
fn collection_selection_preserves_the_requested_face_and_not_face_zero() {
    let first = source();
    let second = repack(&first, |t| {
        t.get_mut(b"hmtx").unwrap()[4..6].copy_from_slice(&700u16.to_be_bytes());
    });
    let collection =
        crate::fonts::font_asset::tests::collection(&[&first, &second], 0x10000, false);
    let mut r = request(&collection);
    r.selection.face_index = 1;
    let output = prepare_font_instance(&collection, &r).unwrap();
    assert_eq!(output.report.face_index, 1);
    assert_eq!(output.report.source_face_count, 2);
    assert_eq!(
        ttf_parser::Face::parse(&output.bytes, 0)
            .unwrap()
            .glyph_hor_advance(ttf_parser::GlyphId(1)),
        Some(720)
    );
}
#[test]
fn permissions_and_no_subset_rights_survive_static_instancing() {
    for rights in [2, 4, 0x208] {
        let source = crate::fonts::pdf_embedding_fixtures::with_rights(source(), rights);
        assert!(prepare_font_instance(&source, &request(&source)).is_err());
    }
    let source = crate::fonts::pdf_embedding_fixtures::with_rights(source(), 0x108);
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    assert!(!output.report.subsetting_allowed);
    assert_eq!(u16_at(&tables(&output.bytes)[b"OS/2"], 8).unwrap(), 0x108);
}
#[test]
fn early_hint_queries_cannot_escape_as_a_successful_static_font() {
    let source = repack(&source(), |t| {
        t.insert(*b"prep", vec![0x91, 0x21]);
    });
    assert!(prepare_font_instance(&source, &request(&source))
        .unwrap_err()
        .to_string()
        .contains("hint semantics"));
}
#[test]
fn budget_and_cancel_fail_without_mutating_input_or_registering_provider_state() {
    let source = source();
    let original = source.clone();
    let r = request(&source);
    assert!(prepare_bounded(&source, &r, 128).is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(prepare_font_instance(&source, &r).is_err()));
    assert_eq!(source, original);
    let mut provider = crate::fonts::RegisteredFontProvider::default();
    let mut bad = r;
    bad.selection.source_sha256.clear();
    assert!(provider
        .register_font_instance_bytes("Instance", &source, &bad)
        .is_err());
    assert!(provider.is_empty());
}
#[test]
fn generated_font_checksums_and_nonidentity_tables_are_preserved() {
    let source = source();
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    let sum = output.bytes.chunks(4).fold(0u32, |sum, c| {
        let mut w = [0; 4];
        w[..c.len()].copy_from_slice(c);
        sum.wrapping_add(u32::from_be_bytes(w))
    });
    assert_eq!(sum, 0xb1b0afba);
    assert_eq!(tables(&source)[b"cmap"], tables(&output.bytes)[b"cmap"]);
    assert_eq!(u32_at(&output.bytes, 0).unwrap(), 0x10000);
}

#[test]
fn readback_receipts_cover_generated_and_preserved_tables_and_exact_owner_set() {
    let source = source();
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    let staged = tables(&output.bytes);
    let receipts = staged
        .iter()
        .map(|(tag, data)| (*tag, (data.len(), table_digest(tag, data).unwrap())))
        .collect::<TableReceipts>();
    verify_tables(&output.bytes, &receipts).unwrap();
    for tag in [*b"glyf", *b"hmtx", *b"name", *b"prep", *b"cmap"] {
        let changed = repack(&output.bytes, |t| {
            t.get_mut(&tag).unwrap()[0] ^= 1;
        });
        assert!(verify_tables(&changed, &receipts).is_err(), "{:?}", tag);
    }
    let lost = repack(&output.bytes, |t| {
        t.remove(b"name");
    });
    assert!(verify_tables(&lost, &receipts).is_err());
    let added = repack(&output.bytes, |t| {
        t.insert(*b"PRIV", vec![0]);
    });
    assert!(verify_tables(&added, &receipts).is_err());
    let mut head = staged[b"head"].clone();
    head[8..12].copy_from_slice(&123u32.to_be_bytes());
    assert_eq!(table_digest(b"head", &head).unwrap(), receipts[b"head"].1);
    head[44] ^= 1;
    assert_ne!(table_digest(b"head", &head).unwrap(), receipts[b"head"].1);
}

#[test]
fn stat_referenced_source_style_survives_complete_static_publication() {
    let source = repack(&source(), |t| {
        let values = [(2, "Original style"), (256, "Axis"), (257, "Half")];
        let mut name = words(&[0, 3, 42]);
        let mut payload = Vec::new();
        for (id, text) in values {
            let bytes = text
                .encode_utf16()
                .flat_map(u16::to_be_bytes)
                .collect::<Vec<_>>();
            name.extend(words(&[
                3,
                1,
                0x409,
                id,
                bytes.len() as i16,
                payload.len() as i16,
            ]));
            payload.extend(bytes);
        }
        name.extend(payload);
        t.insert(*b"name", name);
        let mut stat = vec![0, 1, 0, 2];
        stat.extend(words(&[8, 1]));
        stat.extend(20u32.to_be_bytes());
        stat.extend(words(&[1]));
        stat.extend(28u32.to_be_bytes());
        stat.extend(words(&[2]));
        stat.extend(b"TEST");
        stat.extend(words(&[256, 0, 2, 1, 0, 0, 257]));
        stat.extend(32768i32.to_be_bytes());
        t.insert(*b"STAT", stat);
    });
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    assert_eq!(
        (
            output.report.retained_stat_values,
            output.report.removed_stat_values
        ),
        (1, 0)
    );
    let id = output.report.relocated_stat_name_ids[&2];
    let t = tables(&output.bytes);
    assert_eq!(u16_at(&t[b"STAT"], 18).unwrap(), id);
    let face = ttf_parser::Face::parse(&output.bytes, 0).unwrap();
    let label = |id| {
        face.names()
            .into_iter()
            .find(|n| n.name_id == id && n.platform_id == ttf_parser::PlatformId::Windows)
            .unwrap()
            .to_string()
            .unwrap()
    };
    assert_eq!(label(id), "Original style");
    assert_eq!(label(2), "Regular");
    assert_eq!(label(6), "WFInstance-Half");
}

#[test]
fn ordinary_static_kerning_survives_but_variation_kern_is_not_treated_as_opaque() {
    let kern = words(&[0, 1, 0, 20, 1, 1, 6, 0, 0, 1, 2, -30]);
    let source = repack(&source(), |t| {
        t.insert(*b"kern", kern.clone());
    });
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    assert_eq!(tables(&output.bytes)[b"kern"], kern);
    assert!(output.report.preserved_tables.iter().any(|t| t == "kern"));
    let bad = repack(&source, |t| {
        t.get_mut(b"kern").unwrap()[0..4].copy_from_slice(&0x10000u32.to_be_bytes());
    });
    assert!(prepare_font_instance(&bad, &request(&bad)).is_err());
}
#[test]
fn class_kerning_validates_actual_row_offsets_and_glyph_domains() {
    // 14-byte subtable header, two six-byte class records, two 4-byte rows.
    let kern = words(&[
        0, 1, 0, 34, 0x201, 4, 14, 20, 26, 1, 1, 30, 2, 1, 2, 0, 0, 0, -25,
    ]);
    static_kern(&kern, 4).unwrap();
    let source = repack(&source(), |t| {
        t.insert(*b"kern", kern.clone());
    });
    let output = prepare_font_instance(&source, &request(&source)).unwrap();
    assert_eq!(tables(&output.bytes)[b"kern"], kern);
    for (at, value) in [
        (4 + 18, 25),
        (4 + 18, 31),
        (4 + 18, 34),
        (4 + 24, 4),
        (4 + 24, 1),
        (4 + 20, 4),
        (4 + 26, 1),
    ] {
        let mut bad = kern.clone();
        bad[at..at + 2].copy_from_slice(&(value as u16).to_be_bytes());
        assert!(static_kern(&bad, 4).is_err(), "field {at} value {value}");
    }
}
#[test]
fn approved_asset_and_authoring_save_reopen_use_the_same_static_program() {
    use crate::authoring::{PageSize, PdfBuilder, TextStyle};
    let source = source();
    let r = request(&source);
    let (approved, report) =
        crate::editing_transactions::ApprovedFontAsset::from_font_instance("Selected", &source, &r)
            .unwrap();
    let mut doc = PdfBuilder::new();
    let (face, registered) = doc
        .register_font_instance_bytes("Selected", &source, &r)
        .unwrap();
    assert_eq!(report.prepared_sha256, registered.prepared_sha256);
    assert_eq!(digest(&approved.bytes).unwrap(), report.prepared_sha256);
    doc.add_page(PageSize::custom(200., 200.))
        .draw_text("AB", 10., 50., &TextStyle::new(face, 12.))
        .unwrap();
    let pdf = doc.to_bytes().unwrap();
    assert!(crate::ContentEngine::open_bytes(pdf)
        .unwrap()
        .get_page_text(1)
        .unwrap()
        .contains("AB"));
}
