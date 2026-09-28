//! Semantic scope and traversal-budget regressions.
use super::*;

#[test]
fn semantic_structure_budget_scales_with_pages_but_remains_hard_bounded() {
    assert_eq!(semantic_structure_node_limit(0), MIN_STRUCT_NODES);
    assert_eq!(semantic_structure_node_limit(1), MIN_STRUCT_NODES);
    assert_eq!(semantic_structure_node_limit(25), MIN_STRUCT_NODES);
    assert_eq!(semantic_structure_node_limit(26), 260_000);
    assert_eq!(semantic_structure_node_limit(125), 1_250_000);
    assert_eq!(semantic_structure_node_limit(usize::MAX), MAX_STRUCT_NODES);
}
use crate::semantic_intelligence::{recover_parenttree_semantics, SemanticEvidenceKind};
use crate::text::{TextSearchOptions, TextSemanticOptions};

struct Fixture<'a> {
    page: &'a str,
    first: &'a str,
    second: &'a str,
    first_keys: &'a str,
    second_keys: &'a str,
    first_kid: &'a str,
    second_kid: &'a str,
    root_kids: &'a str,
    parent_nums: &'a str,
}

impl Default for Fixture<'_> {
    fn default() -> Self {
        Self {
            page: "/P << /MCID 0 >> BDC BT /F1 10 Tf 10 180 Td (PAGE) Tj ET EMC /A Do q 1 0 0 1 100 0 cm /B Do Q",
            first: "/P << /MCID 0 >> BDC BT /F1 10 Tf 10 100 Td (FORM-A) Tj ET EMC",
            second: "/P << /MCID 0 >> BDC BT /F1 10 Tf 10 100 Td (FORM-B) Tj ET EMC",
            first_keys: "/StructParents 1",
            second_keys: "/StructParents 2",
            first_kid: "<< /Type /MCR /Pg 3 0 R /Stm 5 0 R /MCID 0 >>",
            second_kid: "<< /Type /MCR /Pg 3 0 R /Stm 6 0 R /MCID 0 >>",
            root_kids: "[12 0 R 13 0 R 14 0 R]",
            parent_nums: "[0 [12 0 R] 1 [13 0 R] 2 [14 0 R]]",
        }
    }
}

fn stream(dict: &str, text: &str) -> Vec<u8> {
    format!(
        "<< {dict} /Length {} >>\nstream\n{text}\nendstream",
        text.len()
    )
    .into_bytes()
}

fn objects(config: &Fixture<'_>) -> Vec<Vec<u8>> {
    vec![
        b"<< /Type /Catalog /Pages 2 0 R /StructTreeRoot 11 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 4 0 R /StructParents 0 /Resources << /Font << /F1 8 0 R >> /XObject << /A 5 0 R /B 6 0 R >> >> >>".to_vec(),
        stream("", config.page),
        stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 300 300] {} /Resources << /Font << /F1 8 0 R >> >>", config.first_keys), config.first),
        stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 300 300] {} /Resources << /Font << /F1 8 0 R >> >>", config.second_keys), config.second),
        b"null".to_vec(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec(),
        b"null".to_vec(), b"null".to_vec(),
        format!("<< /Type /StructTreeRoot /K {} /ParentTree 15 0 R >>", config.root_kids).into_bytes(),
        b"<< /Type /StructElem /S /P /P 11 0 R /Pg 3 0 R /K 0 >>".to_vec(),
        format!("<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R /K {} >>", config.first_kid).into_bytes(),
        format!("<< /Type /StructElem /S /Code /P 11 0 R /Pg 3 0 R /K {} >>", config.second_kid).into_bytes(),
        format!("<< /Nums {} >>", config.parent_nums).into_bytes(),
        b"[13 0 R]".to_vec(),
    ]
}

fn open(objects: Vec<Vec<u8>>) -> ContentEngine {
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

fn engine(config: Fixture<'_>) -> ContentEngine {
    open(objects(&config))
}

#[test]
fn identical_page_and_form_mcids_resolve_to_different_structure_elements() {
    let engine = engine(Fixture::default());
    let document = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(document.elements.len(), 3);
    for (element, expected, stream) in document
        .elements
        .iter()
        .zip(["PAGE", "FORM-A", "FORM-B"])
        .zip([None, Some((5, 0)), Some((6, 0))])
        .map(|((e, t), s)| (e, t, s))
    {
        assert_eq!(element.text, expected);
        assert_eq!(element.mcids[0].stream, stream);
    }
}

#[test]
fn semantic_chars_spans_and_search_retain_scoped_membership_and_roles() {
    let engine = engine(Fixture::default());
    let text = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();
    assert_eq!(text.pages[0].counters.mcids_mapped, 3);
    assert_eq!(text.pages[0].counters.mcids_unmapped, 0);
    for (needle, stream, role) in [
        ("PAGE", None, "P"),
        ("FORM-A", Some((5, 0)), "H1"),
        ("FORM-B", Some((6, 0)), "Code"),
    ] {
        let hits = text.search(needle, &TextSearchOptions::default());
        assert_eq!(hits.len(), 1, "{needle}");
        assert_eq!(
            hits[0].marked_content,
            vec![MarkedContentId {
                mcid: 0,
                stream,
                stream_owner: None
            }]
        );
        let spans = text.pages[0]
            .blocks
            .iter()
            .flat_map(|b| &b.lines)
            .flat_map(|line| &line.spans)
            .collect::<Vec<_>>();
        let span = spans.iter().find(|s| s.text == needle).unwrap();
        assert_eq!(span.struct_role.as_deref(), Some(role));
        assert_eq!(span.marked_content, hits[0].marked_content);
    }
}

#[test]
fn parenttree_recovery_uses_the_forms_own_keys_and_reciprocal_mcrs() {
    let engine = engine(Fixture::default());
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    assert_eq!(report.nodes.len(), 3);
    assert_eq!(report.orphan_mcid_count, 0);
    assert_eq!(report.conflict_count, 0);
    let form = report
        .nodes
        .iter()
        .find(|n| n.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(form.text, "FORM-A");
    assert_eq!(form.role, "H1");
    assert_eq!(form.evidence, SemanticEvidenceKind::SpecDerivedStructure);
}

#[test]
fn empty_forward_tree_recovers_forms_without_page_mcid_aliasing() {
    let engine = engine(Fixture {
        root_kids: "[]",
        ..Default::default()
    });
    let document = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(document.elements.len(), 3);
    assert!(document
        .elements
        .iter()
        .any(|el| el.text == "FORM-A" && el.mcids[0].stream == Some((5, 0))));
}

#[test]
fn indirect_parenttree_arrays_are_resolved() {
    let engine = engine(Fixture {
        parent_nums: "[0 [12 0 R] 1 16 0 R 2 [14 0 R]]",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    assert_eq!(
        report
            .nodes
            .iter()
            .find(|n| n.stream == Some((5, 0)))
            .unwrap()
            .role,
        "H1"
    );
}

#[test]
fn repeated_form_occurrences_retain_transformed_geometry() {
    let engine = engine(Fixture {
        page: "/A Do q 1 0 0 1 100 0 cm /A Do Q",
        root_kids: "[13 0 R]",
        ..Default::default()
    });
    let doc = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(doc.elements[0].text.matches("FORM-A").count(), 2);
    let bbox = doc.elements[0].bbox.unwrap();
    assert!(bbox[2] > 110.0 && bbox[0] == 10.0);
}

#[test]
fn explicit_page_content_stream_is_normalized_to_page_namespace() {
    let engine = engine(Fixture {
        root_kids: "[13 0 R]",
        first_kid: "<< /Type /MCR /Pg 3 0 R /Stm 4 0 R /MCID 0 >>",
        ..Default::default()
    });
    let doc = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(doc.elements[0].text, "PAGE");
    assert_eq!(doc.elements[0].mcids[0].stream, None);
}

#[test]
fn missing_form_parenttree_key_does_not_borrow_the_page_entry() {
    let engine = engine(Fixture {
        first_keys: "",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    let node = report
        .nodes
        .iter()
        .find(|n| n.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(node.evidence, SemanticEvidenceKind::OrphanContent);
    assert_eq!(node.text, "FORM-A");
    assert_ne!(node.role, "P");
}

#[test]
fn competing_page_and_form_structparents_keys_are_conflicts() {
    let engine = engine(Fixture {
        first_keys: "/StructParents 0",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    assert!(report.conflict_count > 0);
    assert!(report
        .nodes
        .iter()
        .filter(|node| node.stream.is_none() || node.stream == Some((5, 0)))
        .all(|node| node.evidence == SemanticEvidenceKind::ConflictingContent));
}

#[test]
fn duplicate_numbertree_keys_are_not_last_wins_role_assignments() {
    let engine = engine(Fixture {
        parent_nums: "[0 [12 0 R] 1 [13 0 R] 1 [14 0 R] 2 [14 0 R]]",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    let node = report
        .nodes
        .iter()
        .find(|node| node.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(node.evidence, SemanticEvidenceKind::ConflictingContent);
    assert_eq!(node.role, "Span");
}

#[test]
fn reciprocal_mcr_mismatch_does_not_assign_the_other_forms_role() {
    let engine = engine(Fixture {
        parent_nums: "[0 [12 0 R] 1 [14 0 R] 2 [14 0 R]]",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    let node = report
        .nodes
        .iter()
        .find(|node| node.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(node.evidence, SemanticEvidenceKind::ConflictingContent);
    assert_ne!(node.role, "Code");
}

#[test]
fn missing_reciprocal_k_is_disclosed_as_repaired_not_spec_derived() {
    let mut data = objects(&Fixture::default());
    data[12] = b"<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R >>".to_vec();
    let report = recover_parenttree_semantics(&open(data), &[1]).unwrap();
    let node = report
        .nodes
        .iter()
        .find(|node| node.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(node.evidence, SemanticEvidenceKind::RepairedStructure);
    assert!(node
        .diagnostics
        .iter()
        .any(|d| d == "missing_reciprocal_content_reference"));
}

#[test]
fn null_parenttree_slot_is_an_orphan_not_a_page_tag() {
    let engine = engine(Fixture {
        parent_nums: "[0 [12 0 R] 1 [null] 2 [14 0 R]]",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    assert_eq!(
        report
            .nodes
            .iter()
            .find(|node| node.stream == Some((5, 0)))
            .unwrap()
            .evidence,
        SemanticEvidenceKind::OrphanContent
    );
}

#[test]
fn identical_geometry_and_text_in_different_namespaces_are_not_deduplicated() {
    let engine = engine(Fixture {
        page: "/A Do /B Do",
        second: Fixture::default().first,
        ..Default::default()
    });
    let text = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();
    let memberships = text.pages[0]
        .blocks
        .iter()
        .flat_map(|b| &b.marked_content)
        .copied()
        .collect::<BTreeSet<_>>();
    assert!(memberships.contains(&MarkedContentId {
        mcid: 0,
        stream: Some((5, 0)),
        stream_owner: None
    }));
    assert!(memberships.contains(&MarkedContentId {
        mcid: 0,
        stream: Some((6, 0)),
        stream_owner: None
    }));
}

#[test]
fn conflicting_forward_structure_bindings_are_not_first_wins() {
    let engine = engine(Fixture {
        second_kid: Fixture::default().first_kid,
        ..Default::default()
    });
    let text = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();
    let form = text.pages[0]
        .blocks
        .iter()
        .flat_map(|b| &b.lines)
        .flat_map(|l| &l.spans)
        .find(|s| s.text == "FORM-A")
        .unwrap();
    assert_eq!(form.struct_role, None);
    assert!(text
        .diagnostics
        .iter()
        .any(|d| d.code == "text.structure.duplicate_mcid"));
}

#[test]
fn malformed_stream_refs_and_invalid_owner_contexts_are_explicit_errors() {
    for first_kid in [
        "<< /Type /MCR /MCID 0 /Stm 8 0 R >>",
        "<< /Type /MCR /MCID 0 /Stm 5 >>",
        "<< /Type /MCR /MCID 0 /StmOwn 3 0 R >>",
        "<< /Type /MCR /MCID 0 /Stm 5 0 R /StmOwn 3 0 R >>",
    ] {
        let engine = engine(Fixture {
            first_kid,
            ..Default::default()
        });
        assert!(
            extract_semantic_document(&engine, &[1]).is_err(),
            "{first_kid}"
        );
    }
}

#[test]
fn reused_form_across_pages_has_one_parenttree_container_not_a_key_conflict() {
    let mut data = objects(&Fixture { root_kids: "[13 0 R]", first_kid: "[<< /Type /MCR /Pg 3 0 R /Stm 5 0 R /MCID 0 >> << /Type /MCR /Pg 17 0 R /Stm 5 0 R /MCID 0 >>]", ..Default::default() });
    data[1] = b"<< /Type /Pages /Count 2 /Kids [3 0 R 17 0 R] >>".to_vec();
    data.push(b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 300] /Contents 18 0 R /Resources << /XObject << /A 5 0 R >> >> >>".to_vec());
    data.push(stream("", "/A Do"));
    let engine = open(data);
    let report = recover_parenttree_semantics(&engine, &[1, 2]).unwrap();
    assert_eq!(report.conflict_count, 0);
    assert_eq!(
        report
            .nodes
            .iter()
            .filter(|n| n.stream == Some((5, 0)))
            .count(),
        2
    );
    let second = extract_semantic_document(&engine, &[2]).unwrap();
    assert_eq!(second.elements[0].text, "FORM-A");
    assert_eq!(second.elements[0].page, Some(2));
    assert_eq!(second.elements[0].mcids.len(), 1);
    let both = extract_semantic_document(&engine, &[1, 2]).unwrap();
    assert_eq!(both.elements[0].page, None);
    assert_eq!(both.elements[0].bbox, None);
}

#[test]
fn cyclic_structure_is_not_returned_as_successful_partial_semantics() {
    let mut data = objects(&Fixture::default());
    data[12] = b"<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R /K [13 0 R] >>".to_vec();
    assert!(extract_semantic_document(&open(data), &[1]).is_err());
}

#[test]
fn cancellation_reaches_structure_and_recovery() {
    let engine = engine(Fixture::default());
    let cancel = crate::cancel::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| extract_semantic_document(&engine, &[1]))
        .is_err());
    assert!(cancel
        .scope(|| recover_parenttree_semantics(&engine, &[1]))
        .is_err());
}

#[test]
fn serialized_membership_disambiguates_same_integer_without_losing_legacy_field() {
    let engine = engine(Fixture::default());
    let text = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();
    let hit = text
        .search("FORM-A", &TextSearchOptions::default())
        .remove(0);
    let json = serde_json::to_value(hit).unwrap();
    assert_eq!(json["mcids"], serde_json::json!([0]));
    assert_eq!(
        json["marked_content"],
        serde_json::json!([{ "mcid": 0, "stream": [5, 0] }])
    );
}

#[test]
fn repaired_parenttree_evidence_is_not_upgraded_to_authored_tag_proof() {
    let mut data = objects(&Fixture {
        root_kids: "[]",
        ..Default::default()
    });
    data[12] = b"<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R >>".to_vec();
    let engine = open(data);
    let context = extract_text_structure_context(&engine, &[1], 100, 100).unwrap();
    let entry = context
        .entries
        .iter()
        .find(|e| e.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(entry.role_source, TextRoleSource::Heuristic);
    assert!(entry.confidence <= 0.6);
}

#[test]
fn malformed_explicit_page_or_mcid_does_not_use_inherited_page_as_a_guess() {
    for first_kid in [
        "<< /Type /MCR /Pg 8 0 R /Stm 5 0 R /MCID 0 >>",
        "<< /Type /MCR /Pg /Bad /Stm 5 0 R /MCID 0 >>",
        "<< /Type /MCR /Stm 5 0 R /MCID (zero) >>",
        "<< /Type /MCR /Stm 5 0 R >>",
    ] {
        let engine = engine(Fixture {
            first_kid,
            ..Default::default()
        });
        assert!(extract_semantic_document(&engine, &[1]).is_err());
    }
}

#[test]
fn parenttree_cannot_claim_a_child_structure_elements_content_as_its_own() {
    let engine = engine(Fixture {
        first_kid: "14 0 R",
        ..Default::default()
    });
    let report = recover_parenttree_semantics(&engine, &[1]).unwrap();
    let first = report
        .nodes
        .iter()
        .find(|node| node.stream == Some((5, 0)))
        .unwrap();
    assert_eq!(first.evidence, SemanticEvidenceKind::ConflictingContent);
}

fn appearance_objects() -> Vec<Vec<u8>> {
    let mut out = objects(&Fixture {
        page: "",
        root_kids: "[13 0 R 14 0 R]",
        first_kid: "<< /Type /MCR /MCID 0 /Stm 5 0 R /StmOwn 7 0 R >>",
        second_kid: "<< /Type /MCR /MCID 0 /Stm 6 0 R /StmOwn 9 0 R >>",
        ..Default::default()
    });
    out[2] = b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 600] /Contents 4 0 R /Annots [7 0 R 9 0 R] /Resources << /Font << /F1 8 0 R >> >> >>".to_vec();
    out[6] = b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 300 300] /AP << /N 5 0 R >> >>".to_vec();
    out[8] =
        b"<< /Type /Annot /Subtype /Stamp /Rect [300 300 600 600] /AP << /N 6 0 R >> >>".to_vec();
    out
}

fn appearance_chunks(engine: &ContentEngine) -> Vec<crate::text::ScopedTextChunk> {
    engine
        .collect_page_scoped_text_chunks_including_appearances(
            1,
            &crate::text::TextTraversalLimits::default(),
            &crate::cancel::CancelToken::none(),
        )
        .unwrap()
}

#[test]
fn explicit_appearance_owners_bind_tags_and_parenttree_in_page_coordinates() {
    let engine = open(appearance_objects());
    let chunks = appearance_chunks(&engine);
    assert_eq!(chunks.len(), 2);
    for (chunk, stream, owner, x, y) in [
        (&chunks[0], (5, 0), (7, 0), 10.0, 100.0),
        (&chunks[1], (6, 0), (9, 0), 310.0, 400.0),
    ] {
        assert!(chunk.form_path.is_empty());
        assert_eq!(chunk.appearance.as_ref().unwrap().annotation, owner);
        assert_eq!(
            (chunk.mcid_owner, chunk.mcid_stream_owner),
            (Some(stream), Some(owner))
        );
        assert!((chunk.chunk.x - x).abs() < 1e-6);
        assert!((chunk.chunk.y - y).abs() < 1e-6);
    }
    let semantic = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(semantic.elements[0].text, "FORM-A");
    assert_eq!(semantic.elements[0].mcids[0].stream_owner, Some((7, 0)));
    assert_eq!(semantic.elements[1].text, "FORM-B");
    let recovered = recover_parenttree_semantics(&engine, &[1]).unwrap();
    assert_eq!(recovered.nodes.len(), 2);
    assert!(recovered
        .nodes
        .iter()
        .all(|node| node.evidence == SemanticEvidenceKind::SpecDerivedStructure));
    assert_eq!(recovered.nodes[0].stream_owner, Some((7, 0)));
}

#[test]
fn legacy_page_only_extraction_does_not_silently_add_appearance_targets() {
    let engine = open(appearance_objects());
    assert!(engine
        .collect_page_scoped_text_chunks(1)
        .unwrap()
        .is_empty());
    assert!(engine
        .collect_page_marked_text_chunks(1)
        .unwrap()
        .is_empty());
    assert_eq!(appearance_chunks(&engine).len(), 2);
}

#[test]
fn shared_appearance_has_distinct_annotation_occurrence_and_mcid_owners() {
    let mut objects = appearance_objects();
    objects[8] =
        b"<< /Type /Annot /Subtype /Stamp /Rect [300 300 600 600] /AP << /N 5 0 R >> >>".to_vec();
    let engine = open(objects);
    let chunks = appearance_chunks(&engine);
    assert_eq!(chunks[0].chunk.text, chunks[1].chunk.text);
    assert_eq!(chunks[0].mcid_owner, chunks[1].mcid_owner);
    assert_ne!(chunks[0].mcid_stream_owner, chunks[1].mcid_stream_owner);
    assert_ne!(chunks[0].chunk.x, chunks[1].chunk.x);
    assert_ne!(
        chunks[0].clone().into_marked().marked_content_id(),
        chunks[1].clone().into_marked().marked_content_id()
    );
}

#[test]
fn omitted_owner_is_inferred_only_for_one_extracted_namespace() {
    let mut objects = appearance_objects();
    objects[12] = b"<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R /K << /Type /MCR /MCID 0 /Stm 5 0 R >> >>".to_vec();
    let engine = open(objects.clone());
    let semantic = extract_semantic_document(&engine, &[1]).unwrap();
    assert_eq!(semantic.elements[0].mcids[0].stream_owner, Some((7, 0)));
    assert_eq!(
        recover_parenttree_semantics(&engine, &[1]).unwrap().nodes[0].evidence,
        SemanticEvidenceKind::SpecDerivedStructure
    );
    objects[8] =
        b"<< /Type /Annot /Subtype /Stamp /Rect [300 300 600 600] /AP << /N 5 0 R >> >>".to_vec();
    let engine = open(objects);
    assert!(extract_semantic_document(&engine, &[1]).is_err());
    assert!(recover_parenttree_semantics(&engine, &[1])
        .unwrap()
        .nodes
        .iter()
        .all(|node| node.evidence == SemanticEvidenceKind::ConflictingContent));
}

#[test]
fn appearance_state_selection_does_not_extract_inactive_text_or_synthesize_missing_state() {
    let mut objects = appearance_objects();
    objects[6] = b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 300 300] /AS /Yes /AP << /N << /Yes 5 0 R /Off 6 0 R >> >> >>".to_vec();
    objects[8] = b"<< /Type /Annot /Subtype /Stamp /Rect [300 300 600 600] /AS /Missing /AP << /N << /Yes 6 0 R >> >> >>".to_vec();
    let chunks = appearance_chunks(&open(objects));
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].chunk.text, "FORM-A");
}

#[test]
fn inactive_appearance_mcr_is_valid_but_contributes_no_active_text() {
    let mut objects = appearance_objects();
    objects[6] = b"<< /Type /Annot /Subtype /Stamp /Rect [0 0 300 300] /AS /Off /AP << /N << /Yes 5 0 R /Off 6 0 R >> >> >>".to_vec();
    let semantic = extract_semantic_document(&open(objects), &[1]).unwrap();
    assert!(semantic.elements[0].text.is_empty());
    assert_eq!(semantic.elements[0].mcids[0].stream_owner, Some((7, 0)));
}

#[test]
fn annotation_owner_must_reference_stream_and_belong_to_mcr_page() {
    for owner in ["3 0 R", "8 0 R", "9 0 R"] {
        let mut objects = appearance_objects();
        objects[12] = format!("<< /Type /StructElem /S /H1 /P 11 0 R /Pg 3 0 R /K << /Type /MCR /MCID 0 /Stm 5 0 R /StmOwn {owner} >> >>").into_bytes();
        assert!(extract_semantic_document(&open(objects), &[1]).is_err());
    }
}

#[test]
fn appearance_matrix_translation_is_normalized_before_rect_mapping() {
    let mut objects = appearance_objects();
    objects[4] = stream("/Type /XObject /Subtype /Form /BBox [0 0 100 20] /Matrix [1 0 0 1 90 -40] /Resources << /Font << /F1 8 0 R >> >>", "BT /F1 10 Tf 0 0 Td (A) Tj ET");
    objects[6] =
        b"<< /Type /Annot /Subtype /Stamp /Rect [30 60 230 100] /AP << /N 5 0 R >> >>".to_vec();
    let chunks = appearance_chunks(&open(objects));
    assert!((chunks[0].chunk.x - 30.0).abs() < 1e-6);
    assert!((chunks[0].chunk.y - 60.0).abs() < 1e-6);
    assert!((chunks[0].chunk.font_size - 20.0).abs() < 1e-6);
}

#[test]
fn nested_forms_preserve_appearance_occurrence_but_use_their_own_mcid_namespace() {
    let mut objects = appearance_objects();
    objects[4] = stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /XObject << /Nested 6 0 R >> >>", "/Nested Do");
    let chunks = appearance_chunks(&open(objects));
    assert_eq!(chunks[0].appearance.as_ref().unwrap().annotation, (7, 0));
    assert_eq!(chunks[0].form_path.len(), 1);
    assert_eq!(chunks[0].mcid_owner, Some((6, 0)));
    assert_eq!(chunks[0].mcid_stream_owner, None);
    assert_eq!(chunks[1].mcid_owner, Some((6, 0)));
    assert_eq!(chunks[1].mcid_stream_owner, Some((9, 0)));
}

#[test]
fn outer_appearance_actualtext_and_mcid_own_nested_unmarked_glyphs() {
    let mut objects = appearance_objects();
    objects[4] = stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /XObject << /Nested 6 0 R >> >>", "/Span << /MCID 3 /ActualText (LOGICAL) >> BDC /Nested Do EMC");
    objects[5] = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /Font << /F1 8 0 R >> >>",
        "BT /F1 10 Tf (A) Tj ET",
    );
    let chunks = appearance_chunks(&open(objects));
    assert_eq!(chunks[0].chunk.text, "LOGICAL");
    assert_eq!(
        (
            chunks[0].mcid,
            chunks[0].mcid_owner,
            chunks[0].mcid_stream_owner
        ),
        (Some(3), Some((5, 0)), Some((7, 0)))
    );
}

#[test]
fn appearance_resource_fallback_uses_page_not_preceding_appearance() {
    let mut objects = appearance_objects();
    objects[5] = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 300]",
        "BT /F1 10 Tf (B) Tj ET",
    );
    assert_eq!(appearance_chunks(&open(objects.clone()))[1].chunk.text, "B");
    objects[5] = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << >>",
        "BT /F1 10 Tf (B) Tj ET",
    );
    assert!(open(objects)
        .collect_page_scoped_text_chunks_including_appearances(
            1,
            &crate::text::TextTraversalLimits::default(),
            &crate::cancel::CancelToken::none()
        )
        .is_err());
}

#[test]
fn page_and_appearances_share_one_operation_and_invocation_budget() {
    let engine = open(appearance_objects());
    for limits in [
        crate::text::TextTraversalLimits {
            max_form_invocations: 1,
            ..Default::default()
        },
        crate::text::TextTraversalLimits {
            max_operations: 1,
            ..Default::default()
        },
        crate::text::TextTraversalLimits {
            max_chunks: 1,
            ..Default::default()
        },
    ] {
        assert!(engine
            .collect_page_scoped_text_chunks_including_appearances(
                1,
                &limits,
                &crate::cancel::CancelToken::none()
            )
            .is_err());
    }
}

#[test]
fn appearance_owner_membership_json_roundtrips_without_breaking_legacy_ids() {
    let legacy: MarkedContentId = serde_json::from_str(r#"{"mcid":0,"stream":[5,0]}"#).unwrap();
    assert_eq!(legacy.stream_owner, None);
    let id = MarkedContentId {
        mcid: 0,
        stream: Some((5, 0)),
        stream_owner: Some((7, 0)),
    };
    assert_eq!(
        serde_json::from_str::<MarkedContentId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
    assert_ne!(legacy, id);
}

#[test]
fn cancelled_appearance_extraction_and_recursive_appearance_forms_fail_explicitly() {
    let mut objects = appearance_objects();
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(matches!(
        open(objects.clone()).collect_page_scoped_text_chunks_including_appearances(
            1,
            &crate::text::TextTraversalLimits::default(),
            &token,
        ),
        Err(WellfriendError::Cancelled(_))
    ));
    objects[4] = stream("/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /XObject << /Self 5 0 R >> >>", "/Self Do");
    assert!(open(objects)
        .collect_page_scoped_text_chunks_including_appearances(
            1,
            &crate::text::TextTraversalLimits::default(),
            &crate::cancel::CancelToken::none(),
        )
        .is_err());
}

#[test]
fn repeated_appearance_streams_share_decoding_without_losing_occurrences() {
    let mut objects = appearance_objects();
    let text = "BT /F1 10 Tf (A) Tj ET";
    objects[4] = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 300 300] /Resources << /Font << /F1 8 0 R >> >>",
        text,
    );
    objects[8] =
        b"<< /Type /Annot /Subtype /Stamp /Rect [300 300 600 600] /AP << /N 5 0 R >> >>".to_vec();
    let engine = open(objects);
    let limits = crate::text::TextTraversalLimits {
        max_form_decoded_bytes: text.len() as u64,
        ..Default::default()
    };
    let chunks = engine
        .collect_page_scoped_text_chunks_including_appearances(
            1,
            &limits,
            &crate::cancel::CancelToken::none(),
        )
        .unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].chunk.text, chunks[1].chunk.text);
    assert_ne!(chunks[0].appearance, chunks[1].appearance);
}
