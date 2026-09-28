//! Metadata and STAT regression source, deliberately unexecuted.
use super::*;
fn names() -> Names {
    Names {
        records: BTreeMap::from([
            ([3, 1, 0x409, 0], utf16("Copyright retained")),
            ([3, 1, 0x409, 2], utf16("Old Regular")),
            ([3, 1, 0x409, 13], utf16("Original licence")),
            ([3, 1, 0x409, 256], utf16("Weight")),
            ([3, 1, 0x409, 257], utf16("Style")),
        ]),
        languages: Vec::new(),
        version: 0,
        relocated: BTreeMap::new(),
    }
}
fn labels() -> FontInstanceNaming {
    FontInstanceNaming {
        family: "New Family".into(),
        subfamily: "Selected".into(),
        legacy_family: "New Family Selected".into(),
        postscript_name: "NewFamily-Selected".into(),
        style_link: FontStyleLink::BoldItalic,
    }
}
fn value(format: u16, value: i32) -> Vec<u8> {
    let mut out = vec![
        0;
        if format == 2 {
            20
        } else if format == 3 {
            16
        } else {
            12
        }
    ];
    word(&mut out, 0, format);
    word(&mut out, 6, 257);
    out[8..12].copy_from_slice(&value.to_be_bytes());
    if format == 2 {
        out[12..16].copy_from_slice(&0i32.to_be_bytes());
        out[16..20].copy_from_slice(&65536i32.to_be_bytes());
    }
    if format == 3 {
        out[12..16].copy_from_slice(&65536i32.to_be_bytes());
    }
    out
}
fn table(values: &[Vec<u8>]) -> Vec<u8> {
    let mut out = vec![0; 28 + values.len() * 2];
    dword(&mut out, 0, 0x10002).unwrap();
    word(&mut out, 4, 8);
    word(&mut out, 6, 1);
    dword(&mut out, 8, 20).unwrap();
    word(&mut out, 12, values.len() as u16);
    dword(&mut out, 14, 28).unwrap();
    word(&mut out, 18, 2);
    out[20..24].copy_from_slice(b"TEST");
    word(&mut out, 24, 256);
    for (i, value) in values.iter().enumerate() {
        let at = out.len();
        word(&mut out, 28 + i * 2, (at - 28) as u16);
        out.extend(value);
    }
    out
}
#[test]
fn localized_notice_and_custom_name_bytes_survive_relocation_and_renaming() {
    let mut original = names();
    original.version = 1;
    original.languages.push(utf16("fr-CA"));
    original
        .records
        .insert([0, 4, 0x8000, 13], utf16("Licence française"));
    original.records.insert([1, 0, 0, 7], vec![0x8e, 0xff, 0]);
    let encoded = original.encode().unwrap();
    let mut decoded = Names::parse(&encoded).unwrap();
    let count = decoded.rename(&labels(), "unique").unwrap();
    assert_eq!(count, 1);
    let output = Names::parse(&decoded.encode().unwrap()).unwrap();
    for (key, value) in original.records {
        if !identity(key[3]) {
            assert_eq!(output.records[&key], value);
        }
    }
    assert_eq!(output.languages, [utf16("fr-CA")]);
    assert_eq!(output.records[&[3, 1, 0x409, 2]], utf16("Bold Italic"));
    assert_eq!(output.records[&[0, 4, 0, 17]], utf16("Selected"));
}
#[test]
fn name_serialization_shares_strings_and_has_sorted_record_keys() {
    let mut n = names();
    n.records.insert([0, 4, 0, 13], utf16("Original licence"));
    let bytes = n.encode().unwrap();
    let roundtrip = Names::parse(&bytes).unwrap();
    assert_eq!(n.records, roundtrip.records);
    let offsets = bytes[6..6 + n.records.len() * 12]
        .chunks_exact(12)
        .filter(|record| u16_at(record, 6).unwrap() == 13)
        .map(|record| u16_at(record, 10).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(offsets[0], offsets[1]);
}
#[test]
fn malformed_name_owners_duplicates_and_unknown_languages_are_rejected() {
    let original = names().encode().unwrap();
    for case in 0..4 {
        let mut bad = original.clone();
        match case {
            0 => word(&mut bad, 4, 6),
            1 => word(&mut bad, 10, 0x8000),
            2 => {
                let key = bad[6..14].to_vec();
                bad[18..26].copy_from_slice(&key);
            }
            _ => word(&mut bad, 2, 4097),
        }
        assert!(Names::parse(&bad).is_err());
    }
}
#[test]
fn aliased_name_storage_cannot_expand_without_an_aggregate_bound() {
    // Small input points many records at the same long string. The copying
    // budget applies before each allocation, not after the whole map exists.
    let count = 256usize;
    let header = 6 + count * 12;
    let mut table = vec![0; header + 65534];
    word(&mut table, 2, count as u16);
    word(&mut table, 4, header as u16);
    for i in 0..count {
        word(&mut table, 6 + i * 12, 3);
        word(&mut table, 8 + i * 12, 1);
        word(&mut table, 10 + i * 12, 0x409);
        word(&mut table, 12 + i * 12, 256 + i as u16);
        word(&mut table, 14 + i * 12, 65534);
    }
    assert!(matches!(
        Names::parse(&table),
        Err(WellfriendError::ResourceLimit(_))
    ));
}
#[test]
fn stat_filters_formats_one_two_three_and_keeps_other_family_sibling_values() {
    let mut sibling = value(1, 65536);
    word(&mut sibling, 4, 1);
    let input = table(&[
        value(1, 0),
        value(2, 0),
        value(3, 0),
        value(1, 65536),
        sibling,
    ]);
    let (output, kept, removed) =
        stat(&input, &BTreeMap::from([(*b"TEST", 0.)]), &mut names()).unwrap();
    assert_eq!((kept, removed), (4, 1));
    assert_eq!(u16_at(&output, 12).unwrap(), 4);
    let (_, kept, removed) =
        stat(&input, &BTreeMap::from([(*b"TEST", 0.5)]), &mut names()).unwrap();
    assert_eq!((kept, removed), (2, 3));
}
#[test]
fn stat_referenced_identity_strings_get_private_ids_before_identity_names_change() {
    let mut input = table(&[value(1, 0)]);
    let at = 28 + usize::from(u16_at(&input, 28).unwrap());
    word(&mut input, at + 6, 2);
    let mut n = names();
    let (output, _, _) = stat(&input, &BTreeMap::from([(*b"TEST", 0.)]), &mut n).unwrap();
    let relocated = n.relocated[&2];
    assert!(relocated >= 256);
    assert_ne!(relocated, 256);
    assert_ne!(relocated, 257);
    assert_eq!(u16_at(&output, 18).unwrap(), relocated);
    n.rename(&labels(), "unique").unwrap();
    assert_eq!(n.records[&[3, 1, 0x409, relocated]], utf16("Old Regular"));
    assert_eq!(n.records[&[3, 1, 0x409, 2]], utf16("Bold Italic"));
}
#[test]
fn stat_format_four_uses_every_axis_and_rejects_duplicate_indices() {
    let mut combo = vec![0; 20];
    word(&mut combo, 0, 4);
    word(&mut combo, 2, 2);
    word(&mut combo, 6, 257);
    word(&mut combo, 14, 1);
    combo[16..20].copy_from_slice(&65536i32.to_be_bytes());
    let mut input = table(&[]);
    input.truncate(28);
    input.extend(b"WDTH");
    input.extend([1, 0, 0, 1]);
    word(&mut input, 6, 2);
    dword(&mut input, 14, 36).unwrap();
    word(&mut input, 12, 1);
    input.extend([0, 2]);
    input.extend(combo);
    let mut coords = BTreeMap::from([(*b"TEST", 0.), (*b"WDTH", 1.)]);
    assert_eq!(stat(&input, &coords, &mut names()).unwrap().1, 1);
    coords.insert(*b"WDTH", 0.);
    assert_eq!(stat(&input, &coords, &mut names()).unwrap().1, 0);
    word(&mut input, 38 + 14, 0);
    assert!(stat(&input, &coords, &mut names()).is_err());
}
#[test]
fn stat_invalid_axes_name_owners_and_value_ranges_do_not_publish_partial_metadata() {
    let input = table(&[value(2, 0)]);
    for case in 0..4 {
        let mut bad = input.clone();
        let at = 28 + usize::from(u16_at(&bad, 28).unwrap());
        match case {
            0 => word(&mut bad, at + 2, 1),
            1 => word(&mut bad, at + 6, 999),
            2 => bad[at + 12..at + 16].copy_from_slice(&65536i32.to_be_bytes()),
            _ => word(&mut bad, 28, 0),
        }
        assert!(stat(&bad, &BTreeMap::from([(*b"TEST", 0.)]), &mut names()).is_err());
    }
    assert!(stat(&input, &BTreeMap::from([(*b"MISS", 0.)]), &mut names()).is_err());
}
#[test]
fn registered_style_fields_use_selected_axes_and_preserve_permissions_and_mvar_fields() {
    let mut tables =
        crate::fonts::font_instance::tests::tables(&crate::fonts::font_instance::tests::source());
    tables.get_mut(b"OS/2").unwrap()[8..10].copy_from_slice(&0x108u16.to_be_bytes());
    tables.get_mut(b"OS/2").unwrap()[26..28].copy_from_slice(&77i16.to_be_bytes());
    let original = tables.clone();
    let coords = BTreeMap::from([(*b"wght", 650.), (*b"wdth", 75.), (*b"slnt", -12.5)]);
    let result = freeze(&tables, &coords, &labels(), "unique", &[500, 600, 0]).unwrap();
    let os2 = &result.tables[b"OS/2"];
    assert_eq!(u16_at(os2, 2).unwrap(), 550);
    assert_eq!(u16_at(os2, 4).unwrap(), 650);
    assert_eq!(u16_at(os2, 6).unwrap(), 3);
    assert_eq!(u16_at(os2, 8).unwrap(), 0x108);
    assert_eq!(u16_at(os2, 26).unwrap(), 77);
    assert_eq!(u16_at(os2, 62).unwrap() & 97, 33);
    assert_eq!(u16_at(&result.tables[b"head"], 44).unwrap() & 0x63, 0x23);
    assert_eq!(
        i32::from_be_bytes(result.tables[b"post"][4..8].try_into().unwrap()),
        -819200
    );
    assert_eq!(tables, original);
}
#[test]
fn explicit_names_reject_invalid_postscript_delimiters_controls_and_overlong_values() {
    for bad in ["", "Bad Name", "Bad/Name", "Bad%Name", "Bad[Name]", "é"] {
        let mut n = labels();
        n.postscript_name = bad.into();
        assert!(n.validate().is_err());
    }
    let mut n = labels();
    n.family = "x".repeat(257);
    assert!(n.validate().is_err());
    n.family = "Name\n".into();
    assert!(n.validate().is_err());
}
#[test]
fn nonfinite_coordinates_and_cancelled_metadata_return_no_stage() {
    let tables =
        crate::fonts::font_instance::tests::tables(&crate::fonts::font_instance::tests::source());
    assert!(freeze(
        &tables,
        &BTreeMap::from([(*b"TEST", f32::NAN)]),
        &labels(),
        "u",
        &[]
    )
    .is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&tables, &BTreeMap::new(), &labels(), "u", &[]).is_err()));
}
