use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use wellfriendpdf_engine::writer::{OutputObject, PdfWriter};
use wellfriendpdf_engine::{
    analyze_document_subsystems, apply_document_security, apply_document_subsystems,
    canonicalize_pdf, edit_paragraph_reflow_pdf, edit_text_operator, AccessibilityMutationKind,
    CanonicalizeOptions, ContentEngine, DocumentSecurityAction, DocumentSecurityRequest,
    DocumentSecuritySubsystem, DocumentSubsystemsAction, DocumentSubsystemsRequest,
    DocumentSubsystemsSubsystem, GeometricReflowRequest, OperatorTextEditRequest,
    ParagraphEditOperation, ParagraphReflowOptions, PdfDictionary, PdfObject, Result,
};

fn main() -> std::result::Result<(), Box<dyn Error>> {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("benchmarks/results/latest"));
    fs::create_dir_all(&out_dir)?;

    let base = source_edit_fixture();
    let paragraph = one_page_pdf("BT /F1 12 Tf 72 720 Td (Hello source paragraph) Tj ET\n");
    let table = ruled_table_pdf();
    let scan = include_bytes!("../tests/fixtures/image_only.pdf").to_vec();

    let mut rows = Vec::new();
    rows.push(measure("open_parse", "core", &base, 3, 25, || {
        let engine = ContentEngine::open_bytes(base.clone())?;
        Ok(json!({"pages": engine.page_count()?}))
    }));
    rows.push(measure("page_count_model", "core", &base, 3, 25, || {
        let engine = ContentEngine::open_bytes(base.clone())?;
        Ok(json!({"pages": engine.page_count()?}))
    }));
    rows.push(measure("text_extraction", "core", &base, 3, 25, || {
        let engine = ContentEngine::open_bytes(base.clone())?;
        Ok(json!({"text_len": engine.get_page_text(1)?.len()}))
    }));
    rows.push(measure(
        "render_page_png_72dpi",
        "core",
        &base,
        3,
        12,
        || {
            let engine = ContentEngine::open_bytes(base.clone())?;
            Ok(json!({"png_bytes": engine.render_page_png_fast(1, 72)?.len()}))
        },
    ));
    rows.push(measure(
        "canonical_noop_rewrite",
        "core",
        &base,
        3,
        16,
        || {
            let engine = ContentEngine::open_bytes(base.clone())?;
            let (bytes, report) = canonicalize_pdf(&engine, &CanonicalizeOptions::default())?;
            ContentEngine::open_bytes(bytes.clone())?;
            Ok(json!({"output_bytes": bytes.len(), "objects": report.object_count}))
        },
    ));
    rows.push(measure("linearized_save", "core", &base, 3, 16, || {
        let engine = ContentEngine::open_bytes(base.clone())?;
        let bytes = wellfriendpdf_engine::linearize_pdf(&engine)?;
        ContentEngine::open_bytes(bytes.clone())?;
        Ok(json!({"output_bytes": bytes.len()}))
    }));
    rows.push(measure(
        "source_text_replacement_save_reopen",
        "editing",
        &base,
        3,
        16,
        || {
            let (bytes, report) = edit_text_operator(
                &base,
                &OperatorTextEditRequest {
                    page: 1,
                    source_text: "ABC".to_string(),
                    replacement_text: "DEF".to_string(),
                    signature_policy_override: false,
                },
            )?;
            let text = ContentEngine::open_bytes(bytes.clone())?.get_page_text(1)?;
            Ok(json!({
                "output_bytes": bytes.len(),
                "changed_pages": report.changed_pages,
                "replacement_present": text.contains("DEF"),
                "old_absent": !text.contains("ABC")
            }))
        },
    ));
    rows.push(measure("paragraph_reflow_save_reopen", "editing", &paragraph, 3, 12, || {
        let (bytes, report) = edit_paragraph_reflow_pdf(
            paragraph.clone(),
            "Hello source paragraph",
            ParagraphEditOperation::Replace {
                replacement: "Hello edited paragraph".to_string(),
            },
            ParagraphReflowOptions::default(),
        )?;
        ContentEngine::open_bytes(bytes.clone())?;
        Ok(json!({"output_bytes": bytes.len(), "edits": report.edits, "lines": report.lines_written}))
    }));
    rows.push(measure("table_cell_edit_save_reopen", "editing", &table, 3, 12, || {
        let analysis = analyze_document_subsystems(&table)?;
        let table_id = analysis.table_evidence["tables"][0]["table_id"]
            .as_str()
            .ok_or_else(|| wellfriendpdf_engine::WellfriendError::MalformedPdf("benchmark table not resolved".into()))?
            .to_string();
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::Table,
            action: Some(DocumentSubsystemsAction::TableEditCell {
                table_id,
                row: 0,
                col: 0,
                replacement_text: "Renamed".to_string(),
            }),
            reflow: Some(reflow("Alpha", "Renamed", [50.0, 600.0, 175.0, 680.0])),
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (bytes, report) = apply_document_subsystems(&table, &request)?;
        let text = ContentEngine::open_bytes(bytes.clone())?.get_page_text(1)?;
        Ok(json!({"output_bytes": bytes.len(), "operation": report.operation, "replacement_present": text.contains("Renamed")}))
    }));
    rows.push(measure(
        "annotation_create_appearance_save_reopen",
        "editing",
        &base,
        3,
        12,
        || {
            let request = DocumentSubsystemsRequest {
                subsystem: DocumentSubsystemsSubsystem::AnnotationAppearance,
                action: Some(DocumentSubsystemsAction::AnnotationCreate {
                    page: 1,
                    subtype: "free_text".to_string(),
                    rect: [40.0, 40.0, 150.0, 80.0],
                    contents: "review note".to_string(),
                    uri: None,
                }),
                reflow: None,
                approved: true,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            let (bytes, report) = apply_document_subsystems(&base, &request)?;
            ContentEngine::open_bytes(bytes.clone())?;
            Ok(json!({"output_bytes": bytes.len(), "operation": report.operation}))
        },
    ));
    rows.push(measure(
        "form_text_create_appearance_save_reopen",
        "editing",
        &base,
        3,
        12,
        || {
            let request = DocumentSubsystemsRequest {
                subsystem: DocumentSubsystemsSubsystem::FormData,
                action: Some(DocumentSubsystemsAction::FormCreateText {
                    field_name: "customer".to_string(),
                    page: 1,
                    rect: [40.0, 100.0, 180.0, 124.0],
                    value: "Ada".to_string(),
                }),
                reflow: None,
                approved: true,
                form_data: None,
                form_data_format: None,
                use_semantic_document_flow: false,
            };
            let (bytes, report) = apply_document_subsystems(&base, &request)?;
            ContentEngine::open_bytes(bytes.clone())?;
            Ok(json!({"output_bytes": bytes.len(), "operation": report.operation}))
        },
    ));
    rows.push(measure("ocr_searchable_layer_save_reopen", "ocr", &scan, 3, 12, || {
        let request = DocumentSubsystemsRequest {
            subsystem: DocumentSubsystemsSubsystem::OcrSearchableLayer,
            action: Some(DocumentSubsystemsAction::OcrAddSearchableText {
                page: 1,
                text: "Scanned".to_string(),
                rect: [72.0, 72.0, 144.0, 96.0],
                font_size: 12.0,
                provider_id: "fixture_provider".to_string(),
                provider_version: Some("1".to_string()),
                confidence: 0.95,
            }),
            reflow: None,
            approved: true,
            form_data: None,
            form_data_format: None,
            use_semantic_document_flow: false,
        };
        let (bytes, report) = apply_document_subsystems(&scan, &request)?;
        let text = ContentEngine::open_bytes(bytes.clone())?.get_page_text(1)?;
        Ok(json!({"output_bytes": bytes.len(), "operation": report.operation, "searchable_text_present": text.contains("Scanned")}))
    }));
    rows.push(measure("redaction_residual_verification", "security", &paragraph, 3, 12, || {
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::Redaction,
            action: Some(DocumentSecurityAction::RedactText {
                terms: vec!["Hello".to_string()],
                pages: vec![1],
                strict: true,
            }),
            approved: true,
            language: None,
            full_rewrite_acknowledged: true,
        };
        let (bytes, report) = apply_document_security(&paragraph, &request)?;
        ContentEngine::open_bytes(bytes.clone())?;
        Ok(json!({"output_bytes": bytes.len(), "status": report.status, "typed_result": report.typed_result}))
    }));
    rows.push(measure("accessibility_structure_repair", "security", &base, 3, 12, || {
        let request = DocumentSecurityRequest {
            subsystem: DocumentSecuritySubsystem::AccessibilityRepair,
            action: Some(DocumentSecurityAction::RepairAfterMutation {
                mutation: AccessibilityMutationKind::Reflow,
                lang: Some("en-US".to_string()),
            }),
            approved: true,
            language: Some("en-US".to_string()),
            full_rewrite_acknowledged: false,
        };
        let (bytes, report) = apply_document_security(&base, &request)?;
        ContentEngine::open_bytes(bytes.clone())?;
        Ok(json!({"output_bytes": bytes.len(), "changed_structure_nodes": report.changed_structure_nodes}))
    }));

    let raw = json!({
        "schema_version": "repository_professionalization.benchmark.raw.v1",
        "commit": option_env!("GIT_COMMIT").unwrap_or("working_tree"),
        "run_policy": {"warmups": 3, "single_process": true, "worker_count": 1},
        "corpus": corpus_manifest(&base, &paragraph, &table, &scan),
        "results": rows,
    });
    write_json(out_dir.join("raw-results.json"), &raw)?;
    write_json(out_dir.join("summary.json"), &summarize(&raw))?;
    write_json(out_dir.join("correctness.json"), &correctness(&raw))?;
    write_json(out_dir.join("environment.json"), &environment())?;
    write_json(out_dir.join("tool-versions.json"), &tool_versions())?;
    write_csv(
        out_dir.join("results.csv"),
        raw["results"].as_array().unwrap(),
    )?;
    write_summary_md(out_dir.join("summary.md"), &raw)?;
    Ok(())
}

fn measure<F>(
    name: &str,
    category: &str,
    corpus_bytes: &[u8],
    warmups: usize,
    iterations: usize,
    mut f: F,
) -> Value
where
    F: FnMut() -> Result<Value>,
{
    for _ in 0..warmups {
        let _ = f();
    }
    let mut durations = Vec::new();
    let mut failures = Vec::new();
    let mut last = Value::Null;
    for _ in 0..iterations {
        let start = Instant::now();
        match f() {
            Ok(value) => {
                durations.push(start.elapsed().as_secs_f64() * 1000.0);
                last = value;
            }
            Err(err) => failures.push(err.to_string()),
        }
    }
    durations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let median = percentile(&durations, 0.50);
    let p95 = percentile(&durations, 0.95);
    json!({
        "task": name,
        "category": category,
        "evidence_class": "measured_directly",
        "iterations": iterations,
        "successes": durations.len(),
        "failures": failures.len(),
        "median_ms": median,
        "p95_ms": p95,
        "throughput_ops_per_sec_median": if median > 0.0 { 1000.0 / median } else { 0.0 },
        "input_bytes": corpus_bytes.len(),
        "input_sha256": hex_digest(corpus_bytes),
        "peak_rss_kib": peak_rss_kib(),
        "last_result": last,
        "failure_samples": failures.into_iter().take(3).collect::<Vec<_>>()
    })
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let index = (((values.len() - 1) as f64) * p).round() as usize;
    values[index.min(values.len() - 1)]
}

fn source_edit_fixture() -> Vec<u8> {
    let content = b"BT /F1 12 Tf 10 150 Td (ABC) Tj ET\n2 w 1 0 0 RG 20 20 40 30 re S\n".to_vec();
    let mut catalog = PdfDictionary::empty();
    catalog.insert("Type", PdfObject::Name("Catalog".to_string()));
    catalog.insert(
        "Pages",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );
    let mut pages = PdfDictionary::empty();
    pages.insert("Type", PdfObject::Name("Pages".to_string()));
    pages.insert("Count", PdfObject::Integer(1));
    pages.insert(
        "Kids",
        PdfObject::Array(vec![PdfObject::Reference {
            number: 3,
            generation: 0,
        }]),
    );
    let mut font = PdfDictionary::empty();
    font.insert("Type", PdfObject::Name("Font".to_string()));
    font.insert("Subtype", PdfObject::Name("Type1".to_string()));
    font.insert("BaseFont", PdfObject::Name("Helvetica".to_string()));
    font.insert("Encoding", PdfObject::Name("WinAnsiEncoding".to_string()));
    let mut fonts = PdfDictionary::empty();
    fonts.insert(
        "F1",
        PdfObject::Reference {
            number: 5,
            generation: 0,
        },
    );
    let mut resources = PdfDictionary::empty();
    resources.insert("Font", PdfObject::Dictionary(fonts));
    let mut page = PdfDictionary::empty();
    page.insert("Type", PdfObject::Name("Page".to_string()));
    page.insert(
        "Parent",
        PdfObject::Reference {
            number: 2,
            generation: 0,
        },
    );
    page.insert(
        "MediaBox",
        PdfObject::Array(vec![
            PdfObject::Integer(0),
            PdfObject::Integer(0),
            PdfObject::Integer(200),
            PdfObject::Integer(200),
        ]),
    );
    page.insert("Resources", PdfObject::Dictionary(resources));
    page.insert(
        "Contents",
        PdfObject::Reference {
            number: 4,
            generation: 0,
        },
    );
    let mut content_dict = PdfDictionary::empty();
    content_dict.insert("Length", PdfObject::Integer(content.len() as i64));
    PdfWriter::new(
        vec![
            OutputObject {
                number: 1,
                object: PdfObject::Dictionary(catalog),
            },
            OutputObject {
                number: 2,
                object: PdfObject::Dictionary(pages),
            },
            OutputObject {
                number: 3,
                object: PdfObject::Dictionary(page),
            },
            OutputObject {
                number: 4,
                object: PdfObject::Stream {
                    dict: content_dict,
                    raw: content,
                },
            },
            OutputObject {
                number: 5,
                object: PdfObject::Dictionary(font),
            },
        ],
        1,
    )
    .write()
    .expect("benchmark fixture")
}

fn one_page_pdf(content: &str) -> Vec<u8> {
    let stream = format!(
        "<< /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    );
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>".to_string(),
        stream,
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
    ];
    let mut output = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
    }
    let xref = output.len();
    output.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes(),
    );
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF",
            objects.len() + 1
        )
        .as_bytes(),
    );
    output
}

fn ruled_table_pdf() -> Vec<u8> {
    let mut content = String::from("1 w 0 0 0 RG\n");
    for y in [600.0, 640.0, 680.0] {
        content.push_str(&format!("50 {y} m 300 {y} l S\n"));
    }
    for x in [50.0, 175.0, 300.0] {
        content.push_str(&format!("{x} 600 m {x} 680 l S\n"));
    }
    for (x, y, text) in [
        (60.0, 650.0, "Alpha"),
        (190.0, 650.0, "Beta"),
        (60.0, 610.0, "Gamma"),
        (190.0, 610.0, "Delta"),
    ] {
        content.push_str(&format!("BT /F1 12 Tf 1 0 0 1 {x} {y} Tm ({text}) Tj ET\n"));
    }
    one_page_pdf(&content)
}

fn reflow(source: &str, replacement: &str, region: [f64; 4]) -> GeometricReflowRequest {
    serde_json::from_value(json!({
        "requested_mode": "geometric_block",
        "page": 1,
        "source_text": source,
        "replacement_text": replacement,
        "region": region,
        "language": "en"
    }))
    .expect("benchmark reflow request")
}

fn corpus_manifest(base: &[u8], paragraph: &[u8], table: &[u8], scan: &[u8]) -> Value {
    json!({
        "name": "repository-generated compact benchmark corpus",
        "file_count": 4,
        "total_bytes": base.len() + paragraph.len() + table.len() + scan.len(),
        "categories": [
            {"name": "born_digital_text", "sha256": hex_digest(base), "bytes": base.len(), "pages": 1},
            {"name": "paragraph_reflow_text", "sha256": hex_digest(paragraph), "bytes": paragraph.len(), "pages": 1},
            {"name": "ruled_table", "sha256": hex_digest(table), "bytes": table.len(), "pages": 1},
            {"name": "compact_scan_fixture", "sha256": hex_digest(scan), "bytes": scan.len(), "pages": 1}
        ],
        "provenance": "repository-generated fixtures plus checked-in compact scan fixture"
    })
}

fn summarize(raw: &Value) -> Value {
    let rows = raw["results"].as_array().cloned().unwrap_or_default();
    json!({
        "schema_version": "repository_professionalization.benchmark.summary.v1",
        "verdict": if rows.iter().all(|row| row["failures"].as_u64().unwrap_or(1) == 0) { "passed" } else { "completed_with_failures" },
        "task_count": rows.len(),
        "successful_task_count": rows.iter().filter(|row| row["failures"].as_u64().unwrap_or(1) == 0).count(),
        "results": rows.iter().map(|row| json!({
            "task": row["task"],
            "category": row["category"],
            "median_ms": row["median_ms"],
            "p95_ms": row["p95_ms"],
            "throughput_ops_per_sec_median": row["throughput_ops_per_sec_median"],
            "input_bytes": row["input_bytes"],
            "peak_rss_kib": row["peak_rss_kib"],
            "failures": row["failures"],
            "evidence_class": row["evidence_class"],
        })).collect::<Vec<_>>()
    })
}

fn correctness(raw: &Value) -> Value {
    let rows = raw["results"].as_array().cloned().unwrap_or_default();
    json!({
        "schema_version": "repository_professionalization.benchmark.correctness.v1",
        "all_outputs_reopened_or_verified": rows.iter().all(|row| row["failures"].as_u64().unwrap_or(1) == 0),
        "task_count": rows.len(),
        "failed_tasks": rows.iter().filter(|row| row["failures"].as_u64().unwrap_or(0) > 0).map(|row| row["task"].clone()).collect::<Vec<_>>()
    })
}

fn environment() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "logical_cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "rust_profile": "release",
        "worker_count": 1,
        "peak_rss_kib_at_end": peak_rss_kib(),
    })
}

fn tool_versions() -> Value {
    json!({
        "wellfriendpdf_engine": wellfriendpdf_engine::ENGINE_VERSION,
        "benchmark_harness": "repository_benchmarks.rs",
        "comparators": "see competitor-comparison for external tools"
    })
}

fn write_json(path: impl AsRef<Path>, value: &Value) -> std::result::Result<(), Box<dyn Error>> {
    fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}

fn write_csv(path: impl AsRef<Path>, rows: &[Value]) -> std::result::Result<(), Box<dyn Error>> {
    let mut csv = String::from("task,category,median_ms,p95_ms,throughput_ops_per_sec_median,input_bytes,failures,peak_rss_kib\n");
    for row in rows {
        csv.push_str(&format!(
            "{},{},{:.6},{:.6},{:.6},{},{},{}\n",
            row["task"].as_str().unwrap_or(""),
            row["category"].as_str().unwrap_or(""),
            row["median_ms"].as_f64().unwrap_or(0.0),
            row["p95_ms"].as_f64().unwrap_or(0.0),
            row["throughput_ops_per_sec_median"].as_f64().unwrap_or(0.0),
            row["input_bytes"].as_u64().unwrap_or(0),
            row["failures"].as_u64().unwrap_or(0),
            row["peak_rss_kib"].as_u64().unwrap_or(0),
        ));
    }
    fs::write(path, csv)?;
    Ok(())
}

fn write_summary_md(
    path: impl AsRef<Path>,
    raw: &Value,
) -> std::result::Result<(), Box<dyn Error>> {
    let mut md = String::from("# Current benchmark summary\n\n");
    md.push_str("Compact repository-generated fixtures measured in one release-profile process with one worker.\n\n");
    md.push_str("| Task | Category | Median ms | P95 ms | Failures |\n");
    md.push_str("|---|---:|---:|---:|---:|\n");
    for row in raw["results"].as_array().unwrap() {
        md.push_str(&format!(
            "| {} | {} | {:.3} | {:.3} | {} |\n",
            row["task"].as_str().unwrap_or(""),
            row["category"].as_str().unwrap_or(""),
            row["median_ms"].as_f64().unwrap_or(0.0),
            row["p95_ms"].as_f64().unwrap_or(0.0),
            row["failures"].as_u64().unwrap_or(0),
        ));
    }
    fs::write(path, md)?;
    Ok(())
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn peak_rss_kib() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(status) = fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:") {
                    return rest
                        .split_whitespace()
                        .next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .unwrap_or(0);
                }
            }
        }
    }
    0
}
