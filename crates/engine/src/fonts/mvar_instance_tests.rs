//! Synthetic metric-table regressions. Added in source; not executed here.
use super::*;

fn put16(data: &mut [u8], at: usize, n: u16) {
    data[at..at + 2].copy_from_slice(&n.to_be_bytes());
}
fn fixture(records: &[(Tag, i16)], stride: usize) -> Vec<u8> {
    let start = 12 + records.len() * stride;
    let mut out = vec![0; start];
    put16(&mut out, 0, 1);
    put16(&mut out, 6, stride as u16);
    put16(&mut out, 8, records.len() as u16);
    put16(
        &mut out,
        10,
        if records.is_empty() { 0 } else { start as u16 },
    );
    for (i, (tag, _)) in records.iter().enumerate() {
        out[12 + i * stride..16 + i * stride].copy_from_slice(tag);
        put16(&mut out, 18 + i * stride, i as u16);
    }
    if records.is_empty() {
        return out;
    }
    // One axis/region/set; the item rows correspond to the sorted value tags.
    for n in [1u16, 0, 12, 1, 0, 22, 1, 1, 0, 16384, 16384] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    for n in [records.len() as u16, 1, 1, 0] {
        out.extend_from_slice(&n.to_be_bytes());
    }
    for (_, delta) in records {
        out.extend_from_slice(&delta.to_be_bytes());
    }
    out
}
fn tables(records: &[(Tag, i16)]) -> Tables {
    let mut os2 = vec![0; 96];
    put16(&mut os2, 0, 4);
    for (at, n) in [(68, 800), (70, -200), (72, 100), (86, 450), (88, 700)] {
        put16(&mut os2, at, n as i16 as u16);
    }
    let mut hhea = vec![0; 36];
    put16(&mut hhea, 0, 1);
    hhea[4..10].copy_from_slice(&os2[68..74]);
    let mut vhea = hhea.clone();
    put16(&mut vhea, 2, 0x1000);
    let mut post = vec![0; 32];
    put16(&mut post, 0, 3);
    let mut gasp = vec![0, 1, 0, 11];
    for max in (1u16..=10).map(|i| i * 10).chain([0xffff]) {
        gasp.extend_from_slice(&max.to_be_bytes());
        gasp.extend_from_slice(&3u16.to_be_bytes());
    }
    [
        (*b"OS/2", os2),
        (*b"hhea", hhea),
        (*b"vhea", vhea),
        (*b"post", post),
        (*b"gasp", gasp),
        (*b"MVAR", fixture(records, 8)),
    ]
    .into_iter()
    .map(|(tag, data)| (tag, Arc::<[u8]>::from(data)))
    .collect()
}
fn coordinates(n: i16) -> [ttf_parser::NormalizedCoordinate; 1] {
    [ttf_parser::NormalizedCoordinate::from(n)]
}
fn signed(data: &[u8], at: usize) -> i16 {
    i16::from_be_bytes(data[at..at + 2].try_into().unwrap())
}

#[test]
fn all_registered_tags_modify_their_own_fields() {
    let mut tags = [
        *b"hasc", *b"hdsc", *b"hlgp", *b"hcla", *b"hcld", *b"hcrs", *b"hcrn", *b"hcof", *b"vasc",
        *b"vdsc", *b"vlgp", *b"vcrs", *b"vcrn", *b"vcof", *b"xhgt", *b"cpht", *b"sbxs", *b"sbys",
        *b"sbxo", *b"sbyo", *b"spxs", *b"spys", *b"spxo", *b"spyo", *b"strs", *b"stro", *b"unds",
        *b"undo",
    ]
    .to_vec();
    tags.extend((b'0'..=b'9').map(|digit| [b'g', b's', b'p', digit]));
    tags.sort_unstable();
    assert_eq!(tags.len(), 38);
    let source = tables(&tags.iter().map(|tag| (*tag, 2)).collect::<Vec<_>>());
    let before = source.clone();
    let stage = freeze(&source, &coordinates(8192)).unwrap();
    assert!(stage.ignored_tags.is_empty());
    for tag in tags {
        let f = field(tag).unwrap();
        assert_eq!(
            read(&stage.tables[&f.table], f).unwrap(),
            read(&source[&f.table], f).unwrap() + 1
        );
    }
    assert_eq!(stage.changes.len(), 41); // 38 targets plus synchronized hhea trio.
    assert!(stage.synchronized_hhea);
    assert_eq!(source, before);
    assert!(!stage.tables.contains_key(b"MVAR")); // Removal belongs to the full-font commit.
}

#[test]
fn negative_fractional_delta_rounds_once_towards_positive_infinity() {
    let source = tables(&[(*b"hasc", -1), (*b"hdsc", -3)]);
    let stage = freeze(&source, &coordinates(8192)).unwrap();
    assert_eq!(signed(&stage.tables[b"OS/2"], 68), 800);
    assert_eq!(signed(&stage.tables[b"OS/2"], 70), -201);
}

#[test]
fn equal_hhea_metrics_follow_os2_but_intentional_differences_do_not() {
    let mut source = tables(&[(*b"hasc", 40), (*b"hdsc", -20), (*b"hlgp", 10)]);
    let stage = freeze(&source, &coordinates(16384)).unwrap();
    assert!(stage.synchronized_hhea);
    assert_eq!(
        &stage.tables[b"OS/2"][68..74],
        &stage.tables[b"hhea"][4..10]
    );
    let mut distinct = source[b"hhea"].to_vec();
    put16(&mut distinct, 4, 1000);
    source.insert(*b"hhea", distinct.into());
    let stage = freeze(&source, &coordinates(16384)).unwrap();
    assert!(!stage.synchronized_hhea);
    assert!(!stage.tables.contains_key(b"hhea"));
}

#[test]
fn zero_instance_does_not_copy_unchanged_tables() {
    let source = tables(&[(*b"hasc", 50)]);
    let stage = freeze(&source, &coordinates(0)).unwrap();
    assert!(stage.tables.is_empty());
    assert!(stage.changes.is_empty());
    assert!(!stage.synchronized_hhea);
}

#[test]
fn signed_and_unsigned_overflow_leave_all_source_tables_unchanged() {
    for (tag, table, at, base, delta) in [
        (*b"cpht", *b"OS/2", 88, 32760u16, 100i16),
        (*b"hcla", *b"OS/2", 74, 65530, 100),
        (*b"hcld", *b"OS/2", 76, 0, -1),
    ] {
        let mut source = tables(&[(tag, delta)]);
        let mut bytes = source[&table].to_vec();
        put16(&mut bytes, at, base);
        source.insert(table, bytes.into());
        let before = source.clone();
        assert!(freeze(&source, &coordinates(16384)).is_err());
        assert_eq!(source, before);
    }
}

#[test]
fn gasp_ranges_cannot_cross_or_change_the_terminal_sentinel() {
    let source = tables(&[(*b"gsp0", 10)]); // First 10 becomes 20, equal to next.
    assert!(freeze(&source, &coordinates(16384)).is_err());
    let mut source = tables(&[(*b"gsp1", 1)]);
    source.insert(
        *b"gasp",
        vec![0, 1, 0, 2, 0, 10, 0, 3, 255, 255, 0, 3].into(),
    );
    assert!(freeze(&source, &coordinates(16384)).is_err());
}

#[test]
fn unrecognized_private_tags_are_reported_without_inventing_targets() {
    let source = tables(&[(*b"PRIV", 40), (*b"hasc", 10)]);
    let stage = freeze(&source, &coordinates(16384)).unwrap();
    assert_eq!(stage.ignored_tags, vec![*b"PRIV"]);
    assert_eq!(signed(&stage.tables[b"OS/2"], 68), 810);
}

#[test]
fn record_extensions_are_skipped_using_the_declared_stride() {
    let records = [(*b"hasc", 40), (*b"hdsc", -30)];
    let mut source = tables(&records);
    let mut extended = fixture(&records, 12);
    put16(&mut extended, 2, 1); // Compatible future minor revision.
    extended[20..24].fill(0xee);
    extended[32..36].fill(0xdd);
    source.insert(*b"MVAR", extended.into());
    let stage = freeze(&source, &coordinates(16384)).unwrap();
    assert_eq!(signed(&stage.tables[b"OS/2"], 68), 840);
    assert_eq!(signed(&stage.tables[b"OS/2"], 70), -230);
}

#[test]
fn unsorted_duplicate_truncated_and_overlapping_records_are_rejected() {
    for records in [vec![(*b"hdsc", 1), (*b"hasc", 1)], vec![(*b"hasc", 1); 2]] {
        assert!(freeze(&tables(&records), &coordinates(16384)).is_err());
    }
    for broken in [vec![0, 1], {
        let mut mvar = fixture(&[(*b"hasc", 1)], 8);
        put16(&mut mvar, 10, 12);
        mvar
    }] {
        let mut source = tables(&[]);
        source.insert(*b"MVAR", broken.into());
        assert!(freeze(&source, &coordinates(16384)).is_err());
    }
}

#[test]
fn target_table_and_os2_height_version_must_exist() {
    let mut source = tables(&[(*b"cpht", 1)]);
    let mut os2 = source[b"OS/2"].to_vec();
    put16(&mut os2, 0, 1);
    source.insert(*b"OS/2", os2.into());
    assert!(freeze(&source, &coordinates(16384)).is_err());
    source.remove(b"OS/2");
    assert!(freeze(&source, &coordinates(16384)).is_err());
}

#[test]
fn no_table_empty_records_and_no_variation_index_are_noops() {
    assert!(freeze(&Tables::new(), &[]).unwrap().tables.is_empty());
    assert!(freeze(&tables(&[]), &[]).unwrap().tables.is_empty());
    let mut source = tables(&[(*b"hasc", 100)]);
    let mut mvar = source[b"MVAR"].to_vec();
    mvar[16..20].fill(255); // NO_VARIATION_INDEX
    source.insert(*b"MVAR", mvar.into());
    assert!(freeze(&source, &coordinates(16384))
        .unwrap()
        .tables
        .is_empty());
}

#[test]
fn mvar_long_words_and_wrong_coordinate_count_are_rejected() {
    let mut source = tables(&[(*b"hasc", 100)]);
    assert!(freeze(&source, &[]).is_err());
    let mut mvar = source[b"MVAR"].to_vec();
    // Store at 20, item data at +22, wordDeltaCount at +2.
    put16(&mut mvar, 44, 0x8001);
    mvar.extend_from_slice(&[0, 0]); // Structurally valid LONG_WORDS payload.
    source.insert(*b"MVAR", mvar.into());
    assert!(freeze(&source, &coordinates(16384)).is_err());
}

#[test]
fn cancellation_prevents_publication_and_preserves_source() {
    let source = tables(&[(*b"hasc", 1)]);
    let before = source.clone();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(freeze(&source, &coordinates(16384)).is_err()));
    assert_eq!(source, before);
}
