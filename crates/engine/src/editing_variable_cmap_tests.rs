//! Source-only redaction byte-boundary regressions, not sanitization evidence.
use super::*;
use crate::fonts::variable_cmap_tests::{font_dictionary, ENCODED};

#[test]
fn redaction_keeps_whole_three_and_four_byte_codes_and_preserves_advance() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    let state = RedactionState {
        font_size: 10.0,
        ..Default::default()
    };
    // The second source code (two bytes, B) occupies x=6..12.
    let redactions = [RedactionEdit {
        rect: ImageRect::new(7.0, -1.0, 4.0, 4.0),
        polygon: Vec::new(),
        options: RedactionOptions::default(),
    }];
    let mut report = RedactionReport::default();
    let output =
        redact_string_bytes(ENCODED, &state, Some(&resolver), &redactions, &mut report).unwrap();
    let surviving = output
        .iter()
        .filter_map(|op| {
            if let Operand::String(bytes) = op {
                Some(bytes.as_slice())
            } else {
                None
            }
        })
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(surviving, [0x41, 0x81, 0, 1, 0x90, 0, 0, 0x41, 0x20]);
    assert_eq!(
        resolver.try_decode_string(&surviving).unwrap(),
        "Afi\u{1f600} "
    );
    assert_eq!(state.string_advance(ENCODED, Some(&resolver)), 27.0);
    assert!(output
        .iter()
        .any(|op| op.as_number().is_some_and(|n| (n + 600.0).abs() < 1e-9)));
}

#[test]
fn redaction_does_not_split_a_multi_scalar_source_glyph() {
    let resolver = FontResolver::new_from_dict_only(&font_dictionary(false));
    let state = RedactionState {
        font_size: 10.0,
        ..Default::default()
    };
    let redactions = [RedactionEdit {
        rect: ImageRect::new(13.0, -1.0, 4.0, 4.0),
        polygon: Vec::new(),
        options: RedactionOptions::default(),
    }];
    let mut report = RedactionReport::default();
    let output =
        redact_string_bytes(ENCODED, &state, Some(&resolver), &redactions, &mut report).unwrap();
    let surviving = output
        .iter()
        .filter_map(|op| {
            if let Operand::String(bytes) = op {
                Some(bytes.as_slice())
            } else {
                None
            }
        })
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        resolver.try_decode_string(&surviving).unwrap(),
        "AB\u{1f600} "
    );
}
