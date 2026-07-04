use std::path::PathBuf;

use oxide_engine::{
    text::build_text_semantic_page, CjkSegmentationMode, ContentEngine, FontDecodeSource,
    TextMappingSource, TextRole, TextRoleSource, TextSearchOptions, TextSemanticOptions,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("corpus")
        .join("pdfs")
        .join("generated")
        .join(name)
}

#[test]
fn semantic_text_model_serializes_words_and_search_quads() {
    let engine = ContentEngine::open_path(fixture("generated_basic_text.pdf")).unwrap();
    let model = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();

    assert_eq!(model.pages.len(), 1);
    assert!(
        model.counters.words >= 10,
        "expected words in semantic model"
    );
    assert!(
        model
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .flat_map(|block| &block.lines)
            .flat_map(|line| &line.words)
            .any(|word| word.text == "quick"),
        "expected word-level reconstruction"
    );

    let json = serde_json::to_string(&model).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["pages"][0]["page"], 1);

    let matches = engine
        .search_text(
            &[1],
            "quick brown",
            TextSearchOptions {
                case_sensitive: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert!(
        !matches[0].quads.is_empty(),
        "search match has source quads"
    );
}

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

fn tagged_rolemap_pdf() -> Vec<u8> {
    let content =
        b"/MyHeading <</MCID 0>> BDC\nBT /F1 12 Tf 1 0 0 1 72 720 Tm (Tagged Title) Tj ET\nEMC\n";
    let mut b = PdfBuilder::new();
    b.add(
        "<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> \
         /StructTreeRoot 6 0 R >>",
    );
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    );
    b.add_stream(content);
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    b.add("<< /Type /StructTreeRoot /RoleMap << /MyHeading /H1 >> /K [7 0 R] >>");
    b.add("<< /Type /StructElem /S /MyHeading /P 6 0 R /Pg 3 0 R /K 0 >>");
    b.build()
}

fn duplicate_mcid_pdf() -> Vec<u8> {
    let content = b"/P <</MCID 0>> BDC\nBT /F1 12 Tf 1 0 0 1 72 720 Tm (Repeated) Tj ET\nEMC\n";
    let mut b = PdfBuilder::new();
    b.add(
        "<< /Type /Catalog /Pages 2 0 R /MarkInfo << /Marked true >> \
         /StructTreeRoot 6 0 R >>",
    );
    b.add("<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    b.add(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
    );
    b.add_stream(content);
    b.add("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>");
    b.add("<< /Type /StructTreeRoot /K [7 0 R 8 0 R] >>");
    b.add("<< /Type /StructElem /S /P /P 6 0 R /Pg 3 0 R /K 0 >>");
    b.add("<< /Type /StructElem /S /Span /P 6 0 R /Pg 3 0 R /K 0 >>");
    b.build()
}

#[test]
fn semantic_text_model_attaches_structtree_rolemap_and_mcid_to_chars() {
    let engine = ContentEngine::open_bytes(tagged_rolemap_pdf()).unwrap();
    let model = engine
        .extract_text_semantic_model(
            &[1],
            TextSemanticOptions {
                include_structure: true,
                ..TextSemanticOptions::default()
            },
        )
        .unwrap();

    assert_eq!(model.pages[0].structure.mapped_mcids, 1);
    let chars: Vec<_> = model.pages[0]
        .blocks
        .iter()
        .flat_map(|block| &block.lines)
        .flat_map(|line| &line.chars)
        .collect();
    assert!(
        chars.iter().any(|ch| {
            ch.mcid == Some(0)
                && ch.struct_role.as_deref() == Some("H1")
                && ch.original_role.as_deref() == Some("MyHeading")
                && ch.role_source == TextRoleSource::RoleMap
        }),
        "expected MCID and RoleMap metadata on semantic chars"
    );

    let matches = engine
        .search_text(
            &[1],
            "Tagged Title",
            TextSearchOptions {
                case_sensitive: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].mcids, vec![0]);
    assert_eq!(matches[0].role, TextRole::Heading);
    assert_eq!(matches[0].role_source, TextRoleSource::RoleMap);
}

#[test]
fn semantic_text_model_reports_duplicate_structtree_mcid() {
    let engine = ContentEngine::open_bytes(duplicate_mcid_pdf()).unwrap();
    let model = engine
        .extract_text_semantic_model(
            &[1],
            TextSemanticOptions {
                include_structure: true,
                ..TextSemanticOptions::default()
            },
        )
        .unwrap();

    assert!(
        model
            .diagnostics
            .iter()
            .any(|diag| diag.code == "text.structure.duplicate_mcid"),
        "duplicate MCID should be visible in semantic diagnostics"
    );
}

#[test]
fn semantic_model_records_char_mapping_source_provenance() {
    let chunk = oxide_engine::TextChunk {
        text: "AB".to_string(),
        x: 10.0,
        y: 100.0,
        font_size: 10.0,
        font_name: "F1".to_string(),
        width: 20.0,
        is_rtl: false,
        is_vertical: false,
        is_invisible: false,
        is_actual_text: false,
        mapping_sources: vec![FontDecodeSource::ToUnicode, FontDecodeSource::GlyphName],
    };
    let model = build_text_semantic_page(
        1,
        [0.0, 0.0, 200.0, 200.0],
        vec![chunk],
        &TextSemanticOptions {
            include_structure: false,
            ..TextSemanticOptions::default()
        },
    );
    let chars = &model.blocks[0].lines[0].chars;
    assert_eq!(chars[0].mapping_source, TextMappingSource::ToUnicode);
    assert_eq!(chars[1].mapping_source, TextMappingSource::GlyphName);
    assert_eq!(model.blocks[0].provenance_summary.tounicode, 1);
    assert_eq!(model.blocks[0].provenance_summary.glyph_name, 1);
}

#[test]
fn cjk_simple_segmentation_groups_bounded_runs_and_preserves_search_quads() {
    let chunk = oxide_engine::TextChunk {
        text: "東京大学ABC、京都".to_string(),
        x: 10.0,
        y: 100.0,
        font_size: 10.0,
        font_name: "F1".to_string(),
        width: 100.0,
        is_rtl: false,
        is_vertical: false,
        is_invisible: false,
        is_actual_text: false,
        mapping_sources: vec![
            FontDecodeSource::ToUnicode;
            "東京大学ABC、京都".chars().count()
        ],
    };
    let page = build_text_semantic_page(
        1,
        [0.0, 0.0, 200.0, 200.0],
        vec![chunk],
        &TextSemanticOptions {
            include_structure: false,
            cjk_segmentation: CjkSegmentationMode::Simple,
            ..TextSemanticOptions::default()
        },
    );
    let words: Vec<_> = page.blocks[0].lines[0]
        .words
        .iter()
        .map(|word| word.text.as_str())
        .collect();
    assert!(words.contains(&"東京大学"));
    assert!(words.contains(&"ABC"));
    assert!(words.contains(&"京都"));
    assert!(page.counters.cjk_simple_tokens >= 2);
}

#[test]
fn cjk_simple_segmentation_respects_max_run_cap() {
    let text = "東京大学京都";
    let chunk = oxide_engine::TextChunk {
        text: text.to_string(),
        x: 10.0,
        y: 100.0,
        font_size: 10.0,
        font_name: "F1".to_string(),
        width: 100.0,
        is_rtl: false,
        is_vertical: false,
        is_invisible: false,
        is_actual_text: false,
        mapping_sources: vec![FontDecodeSource::ToUnicode; text.chars().count()],
    };
    let page = build_text_semantic_page(
        1,
        [0.0, 0.0, 200.0, 200.0],
        vec![chunk],
        &TextSemanticOptions {
            include_structure: false,
            cjk_segmentation: CjkSegmentationMode::Simple,
            max_cjk_run_chars: 2,
            ..TextSemanticOptions::default()
        },
    );
    let words: Vec<_> = page.blocks[0].lines[0]
        .words
        .iter()
        .map(|word| word.text.as_str())
        .collect();
    assert_eq!(words, vec!["東京", "大学", "京都"]);
}
