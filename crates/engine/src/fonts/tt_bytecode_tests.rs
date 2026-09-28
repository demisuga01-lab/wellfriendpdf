//! Structural scan regressions; no hint programs are executed here.
use super::*;
fn inspect(data: &[u8]) -> Inventory {
    scan(data, Owner::Font, &mut Budget::default()).unwrap()
}
#[test]
fn fixed_and_variable_pushes_preserve_signed_words_and_unsigned_bytes() {
    for opcode in 0xb0..=0xbf {
        let words = opcode >= 0xb8;
        let count = usize::from(opcode & 7) + 1;
        let mut data = vec![opcode];
        for _ in 0..count {
            if words {
                data.extend([0xff, 0xfe]);
            } else {
                data.push(0xfe);
            }
        }
        let item = decode(&data, 0).unwrap();
        assert_eq!(item.range, 0..data.len());
        assert_eq!(item.push_count, count);
        assert_eq!(item.last_literal, Some(if words { -2 } else { 254 }));
    }
    let data = [0x41, 2, 0xc0, 0, 0x20, 0];
    let item = decode(&data, 0).unwrap();
    assert_eq!(item.push_count, 2);
    assert_eq!(item.last_literal, Some(8192));
    assert_eq!(decode(&[0x40, 0], 0).unwrap().last_literal, None);
}
#[test]
fn instruction_lookalikes_inside_push_data_are_never_executed_opcodes() {
    let data = [
        0x40, 8, 0x91, 0x88, 0x92, 0x89, 0x2c, 0x2d, 0x58, 0x59, 0x91,
    ];
    let p = inspect(&data);
    assert_eq!(p.instruction_count, 2);
    assert_eq!(p.queries.len(), 1);
    assert_eq!(p.queries[0].offset, 10);
    assert!(p.definitions.is_empty());
}
#[test]
fn truncated_immediate_payloads_and_missing_counts_are_rejected() {
    for data in [
        &[0x40][..],
        &[0x41][..],
        &[0x40, 2, 1][..],
        &[0x41, 1, 0][..],
        &[0xb7, 0][..],
        &[0xbf, 0][..],
    ] {
        assert!(scan(data, Owner::Font, &mut Budget::default()).is_err());
    }
}
#[test]
fn definition_body_ranges_do_not_include_headers_or_end_markers() {
    let data = [
        0xb0, 145, 0x89, 0x41, 1, 0, 0, 0x2d, 0xb0, 7, 0x2c, 0x91, 0x2d,
    ];
    let p = inspect(&data);
    assert_eq!(p.definitions.len(), 2);
    assert_eq!(p.definitions[0].identifier, Some(145));
    assert_eq!(p.definitions[0].body, 3..7);
    assert!(p.definitions[0].instruction);
    assert_eq!(p.definitions[1].identifier, Some(7));
    assert_eq!(p.definitions[1].body, 11..12);
    assert!(!p.definitions[1].instruction);
    assert!(p.queries[0].in_definition);
}
#[test]
fn conditionals_and_definitions_have_separate_balanced_owners() {
    let data = [
        0xb0, 1, 0x58, 0xb0, 0, 0x2c, 0xb0, 0, 0x58, 0x1b, 0x59, 0x2d, 0x59,
    ];
    assert_eq!(inspect(&data).definitions.len(), 1);
    for invalid in [
        &[0x2d][..],
        &[0x59][..],
        &[0x58, 0x1b, 0x1b, 0x59][..],
        &[0x2c, 0x89, 0x2d, 0x2d][..],
        &[0x58, 0x2c, 0x59, 0x2d][..],
        &[0x2c, 0x58, 0x2d][..],
        &[0x58][..],
        &[0x89][..],
    ] {
        assert!(scan(invalid, Owner::Font, &mut Budget::default()).is_err());
    }
}
#[test]
fn glyph_programs_cannot_install_functions_or_idefs() {
    for opcode in [0x2c, 0x89] {
        assert!(scan(
            &[0xb0, 0, opcode, 0x2d],
            Owner::Glyph(4),
            &mut Budget::default()
        )
        .is_err());
    }
}
#[test]
fn only_immediate_literal_selectors_and_identifiers_are_claimed_known() {
    let p = inspect(&[0xb0, 8, 0x88, 0x88, 0xb0, 145, 0x20, 0x89, 0x2d]);
    assert_eq!(p.queries[0].information_selector, Some(8));
    assert_eq!(p.queries[1].information_selector, None);
    assert_eq!(p.definitions[0].identifier, None);
    assert!(scan(
        &[0xb8, 0xff, 0xff, 0x89, 0x2d],
        Owner::Font,
        &mut Budget::default()
    )
    .is_err());
}
#[test]
fn literal_at_end_of_a_definition_does_not_leak_into_enclosing_stack_analysis() {
    let p = inspect(&[0xb0, 0, 0x2c, 0xb0, 145, 0x2d, 0x89, 0x2d]);
    assert_eq!(p.definitions[1].identifier, None);
}
#[test]
fn call_and_jump_inventory_keeps_top_level_and_definition_contexts_distinct() {
    let p = inspect(&[
        0xb0, 0, 0x2c, 0x2b, 0x1c, 0x91, 0x2d, 0x2a, 0x78, 0x79, 0x92, 0x8f,
    ]);
    assert_eq!(p.top_level_calls, 1);
    assert_eq!(p.top_level_jumps, 2);
    assert_eq!(p.relative_jumps, 3);
    assert_eq!(p.unknown_opcodes, 1);
    assert_eq!(p.queries[1].kind, QueryKind::LegacyData);
}
#[test]
fn aggregate_work_receipt_depth_and_cancellation_limits_are_enforced() {
    let mut budget = Budget {
        steps: 4_000_000,
        ..Budget::default()
    };
    assert!(scan(&[0x22], Owner::Font, &mut budget).is_err());
    let mut budget = Budget {
        events: 262144,
        ..Budget::default()
    };
    assert!(scan(&[0x91], Owner::Font, &mut budget).is_err());
    let mut budget = Budget {
        bytes: 64 * 1024 * 1024,
        ..Budget::default()
    };
    assert!(scan(&[0x22], Owner::Font, &mut budget).is_err());
    assert!(scan(&vec![0x58; 257], Owner::Font, &mut Budget::default()).is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    token.scope(|| assert!(scan(&[], Owner::Font, &mut Budget::default()).is_err()));
}
#[test]
fn program_and_definition_length_limits_are_independent() {
    assert!(scan(
        &vec![0x22; PROGRAM_LIMIT + 1],
        Owner::Font,
        &mut Budget::default()
    )
    .is_err());
    let mut data = vec![0xb0, 0, 0x2c];
    data.extend(vec![0x22; 65536]);
    data.push(0x2d);
    assert!(scan(&data, Owner::Font, &mut Budget::default()).is_err());
}
#[test]
fn empty_programs_and_zero_length_pushes_remain_structurally_valid() {
    assert_eq!(inspect(&[]).instruction_count, 0);
    let p = inspect(&[0x40, 0, 0x41, 0]);
    assert_eq!(p.instruction_count, 2);
    assert_eq!(p.maximum_literal_push, 0);
    assert!(p.queries.is_empty());
}
