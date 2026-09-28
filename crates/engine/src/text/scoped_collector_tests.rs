//! Source regressions only. Added without executing PDFs, builds or tests.
use super::*;
use crate::engine::ContentEngine;

fn stream(dict: &str, content: &str) -> Vec<u8> {
    format!(
        "<< {dict} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
    .into_bytes()
}

const LOCAL: &str = "/Resources << /Font << /F1 9 0 R >> /XObject << /Next 6 0 R /Own 10 0 R >> /ExtGState << /NoFont << /ca 1 >> /G << /Font [9 0 R 10] >> >> >>";
const TEXT: &str = "BT /F1 10 Tf (A) Tj ET";

fn fixture(
    page: &str,
    outer: &str,
    outer_dict: &str,
    next: &str,
    next_dict: &str,
) -> ContentEngine {
    let objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 500 500] /Contents 4 0 R /Resources << /Font << /F1 8 0 R >> /XObject << /Outer 5 0 R /Next 6 0 R /Own 7 0 R >> /Properties << /P << /ActualText (logical) /MCID 4 >> >> >> >>".to_vec(),
        stream("", page),
        stream(&format!("/Subtype /Form /BBox [0 0 500 500] {outer_dict}"), outer),
        stream(&format!("/Subtype /Form /BBox [0 0 500 500] {next_dict}"), next),
        stream("/Subtype /Form /BBox [0 0 500 500]", TEXT),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /FirstChar 65 /LastChar 65 /Widths [500] /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /A] >> >>".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /FirstChar 65 /LastChar 65 /Widths [700] /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [65 /B] >> >>".to_vec(),
        stream("/Subtype /Form /BBox [0 0 500 500] /Resources << /Font << /F1 9 0 R >> >>", TEXT),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    ContentEngine::open_bytes(bytes).unwrap()
}

fn logical(chunks: &[ScopedTextChunk]) -> String {
    chunks.iter().map(|item| item.chunk.text.as_str()).collect()
}

#[test]
fn ordinary_extraction_includes_nested_forms_in_source_order() {
    let engine = fixture("/Outer Do", "/Next Do", LOCAL, "/Own Do", "");
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "A");
    assert_eq!(
        chunks[0]
            .form_path
            .iter()
            .map(|p| p.object)
            .collect::<Vec<_>>(),
        vec![(5, 0), (6, 0), (7, 0)]
    );
    assert!(engine.get_page_text(1).unwrap().contains('A'));
}

#[test]
fn explicit_resources_replace_and_null_uses_page_not_caller() {
    for resources in ["", "/Resources null"] {
        let engine = fixture("/Outer Do", "/Next Do", LOCAL, TEXT, resources);
        assert_eq!(
            logical(&engine.collect_page_scoped_text_chunks(1).unwrap()),
            "A"
        );
    }
    for resources in ["/Resources <<>>", "/Resources 42"] {
        let engine = fixture("/Outer Do", "/Next Do", LOCAL, TEXT, resources);
        assert!(engine.collect_page_scoped_text_chunks(1).is_err());
    }
}

#[test]
fn inherited_font_is_object_bound_and_q_restores_it_after_local_tf() {
    let engine = fixture(
        "BT /F1 10 Tf ET /Outer Do",
        "BT (A) Tj q /F1 10 Tf (A) Tj Q (A) Tj ET",
        LOCAL,
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "ABA");
    assert_eq!(
        chunks.iter().map(|c| c.chunk.width).collect::<Vec<_>>(),
        vec![5.0, 7.0, 5.0]
    );
}

#[test]
fn extgstate_without_font_preserves_inherited_object_and_font_entry_rebinds() {
    let engine = fixture(
        "BT /F1 10 Tf ET /Outer Do",
        "/NoFont gs BT (A) Tj ET /G gs BT (A) Tj ET",
        LOCAL,
        "",
        "",
    );
    assert_eq!(
        logical(&engine.collect_page_scoped_text_chunks(1).unwrap()),
        "AB"
    );
}

#[test]
fn missing_inherited_font_cannot_bind_child_resource_by_name() {
    let engine = fixture("/Outer Do", "BT (A) Tj ET", LOCAL, "", "");
    assert!(engine.collect_page_scoped_text_chunks(1).is_err());
}

#[test]
fn repeated_forms_are_replayed_with_independent_geometry_and_provenance() {
    let engine = fixture(
        "q 1 0 0 1 10 20 cm /Outer Do Q q 1 0 0 1 30 40 cm /Outer Do Q",
        "BT /F1 10 Tf 1 2 Td (A) Tj ET",
        "/Matrix [2 0 0 3 5 7]",
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "AA");
    assert_eq!(
        (
            chunks[0].chunk.x,
            chunks[0].chunk.y,
            chunks[0].chunk.width,
            chunks[0].chunk.font_size
        ),
        (17.0, 33.0, 10.0, 30.0)
    );
    assert_eq!((chunks[1].chunk.x, chunks[1].chunk.y), (37.0, 53.0));
    assert_ne!(chunks[0].form_path, chunks[1].form_path);
}

#[test]
fn direct_collector_remains_direct_only_for_form_edit_postconditions() {
    let engine = fixture("/Outer Do", TEXT, "", "", "");
    let ops = engine.get_page_content(1).unwrap();
    let mut collector = TextCollector::new(
        engine.get_page_resources(1).unwrap(),
        engine.document().reader(),
    );
    assert!(collector.collect(&ops).is_empty());
    assert_eq!(
        logical(
            &collector
                .collect_scoped(&ops, &TextTraversalLimits::default(), &CancelToken::none())
                .unwrap()
        ),
        "A"
    );
}

#[test]
fn named_actual_text_spans_form_occurrences_and_is_emitted_once() {
    let engine = fixture(
        "/Span /P BDC /Outer Do /Outer Do EMC",
        "BT /F1 10 Tf (A) Tj ET",
        "",
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "logical");
    assert_eq!(chunks[0].mcid, Some(4));
    assert_eq!(chunks[0].mcid_owner, None);
    assert!(chunks[0].chunk.is_actual_text);
}

#[test]
fn outer_actual_text_is_not_lost_when_inner_actual_text_is_empty() {
    let engine = fixture(
        "/Span << /ActualText (outer) >> BDC /Outer Do EMC",
        "/Span << /ActualText () >> BDC BT /F1 10 Tf (A) Tj ET EMC",
        "",
        "",
        "",
    );
    assert_eq!(
        logical(&engine.collect_page_scoped_text_chunks(1).unwrap()),
        "outer"
    );
}

#[test]
fn form_mcids_do_not_attach_to_equal_page_mcid() {
    let engine = fixture(
        "/Span << /MCID 4 >> BDC BT /F1 10 Tf (A) Tj ET EMC /Outer Do",
        "/Span << /MCID 4 >> BDC BT /F1 10 Tf (A) Tj ET EMC",
        "",
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(chunks[0].mcid_owner, None);
    assert_eq!(chunks[1].mcid_owner, Some((5, 0)));
    let marked = engine.collect_page_marked_text_chunks(1).unwrap();
    assert_eq!(marked[0].mcid, Some(4));
    assert_eq!(marked[1].mcid, Some(4));
    assert_eq!(marked[1].mcid_owner, Some((5, 0)));
    assert_eq!(chunks[1].clone().into_page_marked().mcid, None);
}

#[test]
fn property_names_resolve_in_current_form_resources() {
    let engine = fixture("/Outer Do", "/Span /P BDC BT /F1 10 Tf (A) Tj ET EMC", "/Resources << /Font << /F1 9 0 R >> /Properties << /P << /ActualText (child) /MCID 9 >> >> >>", "", "");
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "child");
    assert_eq!(
        (chunks[0].mcid, chunks[0].mcid_owner),
        (Some(9), Some((5, 0)))
    );
}

#[test]
fn unrelated_nested_dictionary_keys_are_not_actual_text_or_mcid() {
    let engine = fixture(
        "/Span << /Review << /ActualText (wrong) /MCID 5 >> /Accepted true >> BDC /Outer Do EMC",
        TEXT,
        "",
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "A");
    assert_eq!(chunks[0].mcid, None);
}

#[test]
fn form_scope_cannot_consume_parent_marked_or_saved_state() {
    for invalid in ["Q", "EMC", "BT", "q", "(A) Tj"] {
        let engine = fixture(
            "q /Span << /ActualText (parent) >> BDC /Outer Do EMC Q",
            invalid,
            "",
            "",
            "",
        );
        assert!(
            engine.collect_page_scoped_text_chunks(1).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn cycle_and_decode_errors_propagate_instead_of_successful_partial_text() {
    for (outer, entries) in [("/Outer Do", ""), (TEXT, "/Filter /DCTDecode")] {
        let engine = fixture("BT /F1 10 Tf (A) Tj ET /Outer Do", outer, entries, "", "");
        assert!(engine.collect_page_scoped_text_chunks(1).is_err());
        assert!(engine.get_page_text(1).is_err());
        assert!(crate::text::TextExtractor::extract_default(&engine).is_err());
    }
}

#[test]
fn form_limits_and_cancel_propagate() {
    let engine = fixture("/Outer Do /Outer Do", TEXT, "", "", "");
    let limits = TextTraversalLimits {
        max_form_invocations: 1,
        ..Default::default()
    };
    assert!(matches!(
        engine.collect_page_scoped_text_chunks_with_limits(1, &limits, &CancelToken::none()),
        Err(WellfriendError::ResourceLimit(_))
    ));
    let token = CancelToken::new();
    token.cancel();
    assert!(matches!(
        engine.collect_page_scoped_text_chunks_with_limits(
            1,
            &TextTraversalLimits::default(),
            &token
        ),
        Err(WellfriendError::Cancelled(_))
    ));
}

#[test]
fn unique_form_decode_cache_does_not_suppress_occurrences() {
    let engine = fixture("/Outer Do /Outer Do", TEXT, "", "", "");
    let limits = TextTraversalLimits {
        max_form_decoded_bytes: TEXT.len() as u64,
        ..Default::default()
    };
    assert_eq!(
        logical(
            &engine
                .collect_page_scoped_text_chunks_with_limits(1, &limits, &CancelToken::none())
                .unwrap()
        ),
        "AA"
    );
}

#[test]
fn inline_image_data_is_not_scanned_as_text() {
    let engine = fixture(
        "/Outer Do",
        "BI /W 1 /H 1 /CS /RGB /BPC 8 ID abc EI BT /F1 10 Tf (A) Tj ET",
        "",
        "",
        "",
    );
    assert_eq!(
        logical(&engine.collect_page_scoped_text_chunks(1).unwrap()),
        "A"
    );
}

#[test]
fn invisible_form_text_remains_flagged_for_search_and_ocr() {
    let engine = fixture("/Outer Do", "BT /F1 10 Tf 3 Tr (A) Tj ET", "", "", "");
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert!(chunks[0].chunk.is_invisible);
    assert_eq!(logical(&chunks), "A");
}

#[test]
fn form_scope_restores_parent_font_and_transform_on_return() {
    let engine = fixture(
        "BT /F1 10 Tf ET /Outer Do BT 4 8 Td (A) Tj ET",
        "2 0 0 2 20 30 cm BT /F1 10 Tf (A) Tj ET",
        LOCAL,
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!(logical(&chunks), "BA");
    assert_eq!(
        (chunks[1].chunk.x, chunks[1].chunk.y, chunks[1].chunk.width),
        (4.0, 8.0, 5.0)
    );
}

#[test]
fn rise_uses_text_and_page_axes_not_an_untransformed_y_offset() {
    let engine = fixture(
        "2 0 0 3 10 20 cm BT /F1 10 Tf 0 1 -1 0 100 200 Tm 5 Ts (A) Tj ET",
        "",
        "",
        "",
        "",
    );
    let chunks = engine.collect_page_scoped_text_chunks(1).unwrap();
    assert_eq!((chunks[0].chunk.x, chunks[0].chunk.y), (200.0, 620.0));
    assert_eq!(chunks[0].chunk.width, 15.0);
}

#[test]
fn glyph_and_tj_emission_enforce_limits_before_growing_the_whole_output() {
    let engine = fixture("/Outer Do", "BT /F1 10 Tf [(AA) 0 (AA)] TJ ET", "", "", "");
    for limits in [
        TextTraversalLimits {
            max_text_bytes: 3,
            ..Default::default()
        },
        TextTraversalLimits {
            max_chunks: 1,
            ..Default::default()
        },
    ] {
        assert!(matches!(
            engine.collect_page_scoped_text_chunks_with_limits(1, &limits, &CancelToken::none()),
            Err(WellfriendError::ResourceLimit(_))
        ));
    }
}

#[test]
fn cancelled_glyph_loop_does_not_return_a_partial_chunk() {
    let engine = fixture("", "", "", "", "");
    let mut collector = TextCollector::new(
        engine.get_page_resources(1).unwrap(),
        engine.document().reader(),
    );
    let mut chunks = Vec::new();
    collector.process_op(
        &ContentOperation::new("Tf", vec![Operand::Name("F1".into()), Operand::Integer(10)]),
        &mut chunks,
    );
    let token = CancelToken::new();
    token.cancel();
    let mut budget = super::super::TextEmissionBudget {
        remaining_bytes: 1024,
        remaining_chunks: 16,
        cancel: &token,
    };
    assert!(matches!(
        collector.process_op_checked(
            &ContentOperation::new("Tj", vec![Operand::String(b"AAAA".to_vec())]),
            &mut chunks,
            Some(&mut budget)
        ),
        Err(WellfriendError::Cancelled(_))
    ));
    assert!(chunks.is_empty());
}

#[test]
fn malformed_operands_and_nonfinite_composed_geometry_are_errors() {
    for content in [
        "BT /F1 10 Tf 20 Tm (A) Tj ET",
        "BT /F1 10 Tf [true] TJ ET",
        "BT /F1 10 Tf (A) Tj ET /Outer 7 Do",
    ] {
        let engine = fixture(content, "", "", "", "");
        assert!(engine.collect_page_scoped_text_chunks(1).is_err());
    }
    let engine = fixture("", "", "", "", "");
    let mut collector = TextCollector::new(
        engine.get_page_resources(1).unwrap(),
        engine.document().reader(),
    );
    let mut ops = ContentParser::parse(b"BT /F1 10 Tf (A) Tj ET").unwrap();
    for _ in 0..2 {
        ops.insert(
            0,
            ContentOperation::new(
                "cm",
                vec![
                    Operand::Real(1e200),
                    Operand::Integer(0),
                    Operand::Integer(0),
                    Operand::Real(1e200),
                    Operand::Integer(0),
                    Operand::Integer(0),
                ],
            ),
        );
    }
    assert!(collector
        .collect_scoped(&ops, &TextTraversalLimits::default(), &CancelToken::none())
        .is_err());
}

#[test]
fn malformed_root_tokens_do_not_turn_into_partial_success() {
    let engine = fixture("BT /F1 10 Tf (A) Tj ET ]", "", "", "", "");
    assert!(engine.collect_page_scoped_text_chunks(1).is_err());
    assert!(engine.get_page_text(1).is_err());
}
