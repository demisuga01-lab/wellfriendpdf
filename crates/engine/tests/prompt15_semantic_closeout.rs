use wellfriendpdf_engine::{
    mock_tableformer_proposal_set, sdk, segment_cjk_dictionary_text_with_provider,
    AdvancedChunkMode, CjkDictionaryProvider, ContentEngine, ParentTreeRecoveryStatus,
    SemanticBindingOptions, TableProposalMergeOutcomeKind,
};

struct PdfBuilder {
    objects: Vec<Vec<u8>>,
}

impl PdfBuilder {
    fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    fn add(&mut self, body: &str) {
        self.objects.push(body.as_bytes().to_vec());
    }

    fn add_stream(&mut self, stream: &[u8]) {
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = b"%PDF-1.7\n".to_vec();
        let mut offsets = Vec::new();
        for (index, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets {
            pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
                self.objects.len() + 1
            )
            .as_bytes(),
        );
        pdf
    }
}

fn broken_parenttree_pdf() -> Vec<u8> {
    let content =
        b"/P <</MCID 0>> BDC\nBT /F1 12 Tf 1 0 0 1 72 720 Tm (Recovered ParentTree) Tj ET\nEMC\n";
    let mut builder = PdfBuilder::new();
    builder.add(
        "<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> \
         /StructTreeRoot 6 0 R >>",
    );
    builder.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    builder.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /StructParents 0 >>",
    );
    builder.add_stream(content);
    builder.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    builder.add("<< /Type /StructTreeRoot /ParentTree 7 0 R /K [] >>");
    builder.add("<< /Nums [0 [8 0 R null]] /Limits [1 0] >>");
    builder.add("<< /Type /StructElem /S /ArticleRole /P 6 0 R /Pg 3 0 R /K 0 >>");
    builder.build()
}

#[test]
fn semantic_bundle_preserves_parenttree_text_provenance_and_proposal_status() {
    let bytes = broken_parenttree_pdf();
    let engine = ContentEngine::open_bytes(bytes.clone()).unwrap();
    let report = engine
        .semantic_binding_report(&SemanticBindingOptions {
            pages: vec![1],
            search_query: Some("Recovered".to_string()),
            table_proposals: Some(mock_tableformer_proposal_set(1)),
            chunk_options: wellfriendpdf_engine::AdvancedChunkOptions {
                mode: AdvancedChunkMode::CjkTokenAware,
                target_tokens: 32,
                overlap_tokens: 0,
                cjk_token_aware: true,
                ..Default::default()
            },
            ..Default::default()
        })
        .unwrap();

    assert_eq!(report.schema_version, "prompt15.semantic_binding.v1");
    assert!(matches!(
        report.parenttree_recovery.status,
        ParentTreeRecoveryStatus::RecoveredFromParentTree
            | ParentTreeRecoveryStatus::RecoveredWithConflicts
    ));
    assert_eq!(report.summary.page_count, 1);
    assert!(report.summary.recovered_parenttree_node_count >= 1);
    assert!(!report.search_results.is_empty());
    assert!(report
        .rag_chunks
        .chunks
        .iter()
        .any(|chunk| chunk.text.contains("Recovered ParentTree")));
    assert!(report.rag_chunks.chunks.iter().all(|chunk| {
        chunk.stable_hash.starts_with("sha256:")
            && !chunk.source_spans.is_empty()
            && !chunk.citations.is_empty()
    }));
    let merge = report.table_proposal_merge.as_ref().unwrap();
    assert!(merge.deterministic_primary);
    assert_eq!(merge.accepted_count, 1);
    assert_eq!(
        merge.outcomes[0].outcome,
        TableProposalMergeOutcomeKind::CandidateRegion
    );
    assert!(!merge.outcomes[0].author_original);
    assert!(!report.privacy.cloud_upload_default);

    let first = serde_json::to_vec(&report).unwrap();
    let second = serde_json::to_vec(
        &ContentEngine::open_bytes(bytes)
            .unwrap()
            .semantic_binding_report(&SemanticBindingOptions {
                pages: vec![1],
                search_query: Some("Recovered".to_string()),
                table_proposals: Some(mock_tableformer_proposal_set(1)),
                chunk_options: wellfriendpdf_engine::AdvancedChunkOptions {
                    mode: AdvancedChunkMode::CjkTokenAware,
                    target_tokens: 32,
                    overlap_tokens: 0,
                    cjk_token_aware: true,
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap(),
    )
    .unwrap();
    assert_eq!(first, second);
}

#[test]
fn sdk_prompt15_envelopes_share_schema_and_search_rejects_empty_queries() {
    let bytes = broken_parenttree_pdf();
    let semantic: serde_json::Value =
        serde_json::from_str(&sdk::semantic_binding_report_json(&bytes, &[1], None).unwrap())
            .unwrap();
    assert_eq!(semantic["kind"], "semantic_binding_report");
    assert_eq!(
        semantic["report"]["schema_version"],
        "prompt15.semantic_binding.v1"
    );

    let chunks: serde_json::Value =
        serde_json::from_str(&sdk::advanced_chunk_report_json(&bytes, &[1], None).unwrap())
            .unwrap();
    assert_eq!(chunks["kind"], "advanced_rag_chunk_set");
    assert_eq!(chunks["report"]["raw_text_rewritten"], false);

    let search: serde_json::Value = serde_json::from_str(
        &sdk::semantic_search_report_json(&bytes, &[1], "ParentTree", None).unwrap(),
    )
    .unwrap();
    assert_eq!(search["kind"], "semantic_search_report");
    assert_eq!(search["report"]["provenance_preserved"], true);

    assert!(sdk::semantic_search_report_json(&bytes, &[1], "   ", None).is_err());
}

#[test]
fn cjk_dictionary_segmentation_advances_across_fullwidth_punctuation() {
    let provider = CjkDictionaryProvider::builtin_fixture();
    let tokens = segment_cjk_dictionary_text_with_provider(
        "alpha\u{ff0c}\u{673a}\u{5668}\u{5b66}\u{4e60} beta",
        &provider,
    );
    assert_eq!(
        tokens
            .iter()
            .map(|token| token.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "alpha",
            "\u{ff0c}",
            "\u{673a}\u{5668}\u{5b66}\u{4e60}",
            "beta"
        ]
    );
    assert_eq!(tokens[1].char_range, [5, 6]);
    assert_eq!(tokens[1].language, "punctuation");
}

#[test]
fn semantic_search_handles_large_mixed_script_fixture_without_panicking() {
    let bytes = include_bytes!("fixtures/tracemonkey.pdf");
    let report = ContentEngine::open_bytes(bytes.to_vec())
        .unwrap()
        .semantic_search_report(&[], "the", None)
        .unwrap();
    assert_eq!(report.query, "the");
    assert!(report.provenance_preserved);
}
