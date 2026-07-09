use oxide_engine::{
    merge_layout_proposals_deterministic, CloudLayoutBackendConfig, ContentEngine,
    LayoutBackendInput, LayoutCloudPayloadPolicy, LayoutLocalBackendConfig, LayoutMergePolicy,
    LayoutProposalSet, MockCloudLayoutBackend, MockLocalLayoutBackend, ParentTreeRecoveryStatus,
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

    fn add(&mut self, body: &str) -> usize {
        self.objects.push(body.as_bytes().to_vec());
        self.objects.len()
    }

    fn add_stream(&mut self, stream: &[u8]) -> usize {
        let mut body = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        body.extend_from_slice(stream);
        body.extend_from_slice(b"\nendstream");
        self.objects.push(body);
        self.objects.len()
    }

    fn build(&self) -> Vec<u8> {
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.7\n");
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(body);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref_start = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
        pdf.extend_from_slice(b"0000000000 65535 f \n");
        for off in offsets {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF",
                self.objects.len() + 1,
                xref_start
            )
            .as_bytes(),
        );
        pdf
    }
}

fn broken_parenttree_pdf() -> Vec<u8> {
    let content =
        b"/P <</MCID 0>> BDC\nBT /F1 12 Tf 1 0 0 1 72 720 Tm (Recovered ParentTree) Tj ET\nEMC\n";
    let mut b = PdfBuilder::new();
    b.add(
        "<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> \
         /StructTreeRoot 6 0 R >>",
    );
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R /StructParents 0 >>",
    );
    b.add_stream(content);
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    b.add("<< /Type /StructTreeRoot /ParentTree 7 0 R /K [] >>");
    b.add("<< /Nums [0 [8 0 R null]] /Limits [1 0] >>");
    b.add("<< /Type /StructElem /S /ArticleRole /P 6 0 R /Pg 3 0 R /K 0 >>");
    b.build()
}

#[test]
fn parenttree_recovery_builds_provenance_graph_from_broken_tags() {
    let engine = ContentEngine::open_bytes(broken_parenttree_pdf()).unwrap();
    let report = engine.recover_parenttree_semantics(&[1]).unwrap();

    assert!(matches!(
        report.status,
        ParentTreeRecoveryStatus::RecoveredFromParentTree
            | ParentTreeRecoveryStatus::RecoveredWithConflicts
    ));
    assert_eq!(report.recovered_node_count, 1);
    assert_eq!(report.repaired_role_map_count, 1);
    assert_eq!(report.pages[0].struct_parents, Some(0));
    assert_eq!(report.nodes[0].page, 1);
    assert_eq!(report.nodes[0].mcid, 0);
    assert_eq!(report.nodes[0].role, "Span");
    assert_eq!(report.nodes[0].text, "Recovered ParentTree");
    assert!(report
        .diagnostics
        .iter()
        .any(|diag| diag.code == "parenttree.malformed_limits"));

    let document = engine.extract_semantic_document(&[1]).unwrap();
    assert!(document.tagged);
    assert_eq!(document.elements[0].text, "Recovered ParentTree");
}

#[test]
fn layout_mock_backends_merge_deterministically_and_cloud_fails_closed() {
    let input = LayoutBackendInput::metadata_only(vec![1]);
    let local = MockLocalLayoutBackend::new(LayoutLocalBackendConfig {
        enabled: true,
        ..Default::default()
    });
    let proposal = local.propose(&input);
    assert_eq!(proposal.proposed_regions.len(), 1);

    let merge = merge_layout_proposals_deterministic(&proposal, &LayoutMergePolicy::default());
    assert_eq!(merge.accepted_count, 1);
    assert_eq!(merge.rejected_count, 0);

    let cloud = MockCloudLayoutBackend::new(CloudLayoutBackendConfig::default());
    let blocked = cloud.propose(&input);
    assert!(blocked.proposed_regions.is_empty());
    assert!(blocked
        .diagnostics
        .iter()
        .any(|diag| diag.code == "cloud_mock_disabled_by_default"));

    let allowed_input = LayoutBackendInput {
        allow_cloud_upload: true,
        payload: oxide_engine::LayoutInputPayloadKind::TextSpans,
        privacy_mode: oxide_engine::LayoutPrivacyMode::CloudExplicitOptIn,
        text_available: true,
        ..LayoutBackendInput::metadata_only(vec![1])
    };
    let cloud = MockCloudLayoutBackend::new(CloudLayoutBackendConfig {
        enabled: true,
        endpoint: Some("https://layout.invalid/mock".to_string()),
        api_key_env: Some("OXIDE_LAYOUT_API_KEY".to_string()),
        payload_policy: LayoutCloudPayloadPolicy::TextOnly,
        user_acknowledged_privacy: true,
        ..Default::default()
    });
    let cloud_result = cloud.propose(&allowed_input);
    assert_eq!(cloud_result.proposed_regions.len(), 1);
    assert!(cloud_result
        .privacy_flags
        .iter()
        .any(|flag| flag == "explicit_upload_allowed"));
}

#[test]
fn malformed_layout_proposal_schema_is_rejected() {
    let mut set = MockLocalLayoutBackend::new(LayoutLocalBackendConfig {
        enabled: true,
        ..Default::default()
    })
    .propose(&LayoutBackendInput::metadata_only(vec![1]));
    set.proposed_regions[0].confidence = 1.5;
    let report = oxide_engine::validate_layout_proposal_set(&LayoutProposalSet {
        deterministic_merge_outcome: "test_invalid_schema".to_string(),
        ..set
    });
    assert_eq!(report.rejected_count, 1);
    assert_eq!(report.conflict_count, 1);
}
