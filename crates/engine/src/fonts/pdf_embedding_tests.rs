//! Regression source only; not compiled or executed in this implementation.
use super::*;
use crate::fonts::cid_encoding::CidEncoding;
use crate::fonts::pdf_embedding_fixtures::{cff, font};

#[test]
fn cid_keyed_charset_is_not_assumed_to_be_glyph_order() {
    let identity = CffIdentity::parse(&cff(true)).unwrap();
    assert_eq!(identity.gid_to_cid, vec![0, 42, 7, 1000]);
    assert_eq!(identity.postscript_name, "WFCffFixture");
    assert_eq!(
        identity.system,
        CidSystem {
            registry: b"Wellfriend".to_vec(),
            ordering: b"FixtureCids".to_vec(),
            supplement: 0
        }
    );
    let info = EmbeddingInfo::parse(&font(true, 0)).unwrap();
    assert_eq!(info.cid(9, 1).unwrap(), 42);
    assert!(info.cid(9, 4).is_err());
}

#[test]
fn name_keyed_cff_uses_gid_cids_not_string_sids() {
    let identity = CffIdentity::parse(&cff(false)).unwrap();
    assert_eq!(identity.gid_to_cid, vec![0, 1, 2, 3]);
    assert_eq!(identity.system, CidSystem::default());
}

#[test]
fn generated_cmap_keeps_distinct_codes_for_the_same_native_cid() {
    let identity = CffIdentity::parse(&cff(true)).unwrap();
    for vertical in [false, true] {
        let (dict, bytes) = identity
            .encoding([(1, 1), (2, 2), (3, 1), (32, 3)], vertical)
            .unwrap();
        let map = CidEncoding::parse(&bytes, None).unwrap();
        assert_eq!(
            [map.cid(1), map.cid(2), map.cid(3), map.cid(32)],
            [42, 7, 42, 1000]
        );
        assert_eq!(map.wmode, Some(u8::from(vertical)));
        assert_eq!(
            dict.get_integer("WMode"),
            Some(i64::from(u8::from(vertical)))
        );
        assert_eq!(map.cid(4), 0);
    }
    assert!(identity.encoding([(1, 1), (1, 2)], false).is_err());
    assert!(identity.encoding([(1, 4)], false).is_err());
}

#[test]
fn cff_encoding_identity_is_deterministic_and_direction_specific() {
    let identity = CffIdentity::parse(&cff(true)).unwrap();
    let a = identity.encoding([(1, 1), (2, 2)], false).unwrap();
    let b = identity.encoding([(2, 2), (1, 1), (1, 1)], false).unwrap();
    assert_eq!(a, b);
    let v = identity.encoding([(1, 1), (2, 2)], true).unwrap();
    assert_ne!(a.0.get_name("CMapName"), v.0.get_name("CMapName"));
}

#[test]
fn full_font_permissions_and_no_subsetting_are_enforced() {
    for keyed in [false, true] {
        assert!(EmbeddingInfo::parse(&font(keyed, 0)).unwrap().may_subset);
        assert!(
            !EmbeddingInfo::parse(&font(keyed, 0x108))
                .unwrap()
                .may_subset
        );
        for rights in [2, 4, 0x208] {
            assert!(EmbeddingInfo::parse(&font(keyed, rights)).is_err());
        }
    }
    assert!(EmbeddingInfo::parse(b"ttcf").is_err());
}

#[test]
fn malformed_cff_metadata_and_duplicate_native_cids_are_rejected() {
    let mut bytes = cff(true);
    let charset = bytes
        .windows(7)
        .position(|p| p == [0, 0, 42, 0, 7, 3, 232])
        .unwrap();
    bytes[charset + 3..charset + 5].copy_from_slice(&42u16.to_be_bytes());
    assert!(CffIdentity::parse(&bytes).is_err());
    let mut bytes = cff(true);
    let sid = bytes
        .windows(5)
        .position(|p| p == [29, 0, 0, 1, 135])
        .unwrap();
    bytes[sid + 1..sid + 5].copy_from_slice(&9999u32.to_be_bytes());
    assert!(CffIdentity::parse(&bytes).is_err());
    for prefix in [0, 1, 3, 5, 8, 16] {
        assert!(CffIdentity::parse(&cff(true)[..prefix]).is_err());
    }
}

#[test]
fn standard_cff_strings_have_all_391_sids() {
    assert_eq!(standard_strings::STANDARD_NAMES.len(), 391);
    assert_eq!(standard_strings::STANDARD_NAMES[0], ".notdef");
    assert_eq!(standard_strings::STANDARD_NAMES[34], "A");
}
