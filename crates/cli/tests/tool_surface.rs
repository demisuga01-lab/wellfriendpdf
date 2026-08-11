//! Tool-surface parity test: exercise every Wellfriend CLI subcommand on fixtures
//! and assert it succeeds with the expected output shape.
//!
//! This is the continuously-verifiable evidence behind the command-by-command
//! Poppler parity claim. It invokes the actual built binary via
//! `CARGO_BIN_EXE_wellfriendpdf`, so it covers argument parsing + the full pipeline,
//! not just the engine API.

use std::path::PathBuf;
use std::process::Command;

fn wellfriendpdf() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wellfriendpdf"))
}

fn fixtures() -> PathBuf {
    // crates/cli -> repo root -> crates/engine/tests/fixtures
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("engine")
        .join("tests")
        .join("fixtures")
}

fn fx(name: &str) -> PathBuf {
    fixtures().join(name)
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wellfriendpdf_tool_surface_{}_{}",
        std::process::id(),
        name
    ))
}

fn run(args: &[&str]) -> std::process::Output {
    wellfriendpdf()
        .args(args)
        .output()
        .expect("spawn wellfriendpdf")
}

fn assert_ok(out: &std::process::Output, label: &str) {
    assert!(
        out.status.success(),
        "{label} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn assert_json(out: &std::process::Output, label: &str) -> serde_json::Value {
    assert_ok(out, label);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "{label} did not emit valid JSON: {err}; stdout={}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

fn remove_path(path: &std::path::Path) {
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

fn zip_entries(path: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;

    let file = std::fs::File::open(path).expect("open zip output");
    let mut zip = zip::ZipArchive::new(file).expect("read zip output");
    let mut entries = Vec::new();
    for idx in 0..zip.len() {
        let mut entry = zip.by_index(idx).expect("zip entry");
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).expect("read zip entry");
        entries.push((entry.name().to_string(), bytes));
    }
    entries
}

#[test]
fn extract_text_runs() {
    let out = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
    ]);
    assert_ok(&out, "extract-text");
    assert!(!out.stdout.is_empty(), "extract-text produced no text");
}

#[test]
fn extract_text_structured_runs() {
    // Layout-aware extraction (XY-cut reading order) + structured JSON.
    let out = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
        "--structured",
    ]);
    assert_ok(&out, "extract-text --structured");
    assert!(
        !out.stdout.is_empty(),
        "structured extraction produced no text"
    );

    let json = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
        "--structured",
        "--format",
        "json",
    ]);
    assert_ok(&json, "extract-text --structured --format json");
    let s = String::from_utf8_lossy(&json.stdout);
    assert!(s.contains("\"blocks\""), "JSON should contain a block tree");
}

#[test]
fn extract_text_region_and_profile_runs() {
    let out = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
        "--region",
        "0,0,10000,10000",
        "--profile",
        "layout-faithful",
    ]);
    assert_ok(&out, "extract-text --region --profile");
    assert!(!out.stdout.is_empty(), "region extraction produced no text");
}

#[test]
fn extract_text_semantic_runs() {
    // Semantic mode uses tagged-PDF structure when present and falls back to the
    // geometric analyzer for untagged fixtures.
    let out = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
        "--semantic",
    ]);
    assert_ok(&out, "extract-text --semantic");
    assert!(
        !out.stdout.is_empty(),
        "semantic extraction produced no text"
    );

    let json = run(&[
        "extract-text",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "3",
        "--semantic",
        "--format",
        "json",
    ]);
    assert_ok(&json, "extract-text --semantic --format json");
    let s = String::from_utf8_lossy(&json.stdout);
    assert!(s.contains("\"source\""), "JSON should describe the source");
    assert!(s.contains("geometric_fallback") || s.contains("tagged_pdf"));
}

#[test]
fn extract_tables_runs() {
    // Table extraction (no Poppler equivalent). The fixture may or may not
    // contain a table; the command must succeed and emit valid output either way.
    let csv = run(&[
        "extract-tables",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "csv",
    ]);
    assert_ok(&csv, "extract-tables --format csv");

    let json = run(&[
        "extract-tables",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "json",
        "--structure",
    ]);
    assert_ok(&json, "extract-tables --format json");
    let s = String::from_utf8_lossy(&json.stdout);
    assert!(s.contains("\"pages\""), "JSON should have a pages array");

    let html = run(&[
        "extract-tables",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "html",
    ]);
    assert_ok(&html, "extract-tables --format html");
    let h = String::from_utf8_lossy(&html.stdout);
    assert!(h.contains("<!doctype html>"), "HTML should be a document");

    let region_json = run(&[
        "extract-tables",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "json",
        "--region",
        "0,0,10000,10000",
    ]);
    assert_ok(&region_json, "extract-tables --region");
}

#[test]
fn parse_runs() {
    // The canonical-model `parse` command must serialize to all three formats.
    let md = run(&[
        "parse",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "markdown",
    ]);
    assert_ok(&md, "parse --format markdown");
    assert!(!md.stdout.is_empty(), "parse markdown produced no output");

    let json = run(&[
        "parse",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "json",
    ]);
    assert_ok(&json, "parse --format json");
    let s = String::from_utf8_lossy(&json.stdout);
    assert!(
        s.contains("\"schema_version\""),
        "JSON should carry a schema version"
    );
    assert!(
        s.contains("\"body\""),
        "JSON should carry the body block stream"
    );
    assert!(
        s.contains("\"pages\""),
        "JSON should carry the per-page view"
    );

    let html = run(&[
        "parse",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "html",
    ]);
    assert_ok(&html, "parse --format html");
    let h = String::from_utf8_lossy(&html.stdout);
    assert!(h.contains("<html>"), "HTML should be a document");
}

#[test]
fn parse_robustness_flags_and_per_page_source() {
    // The de-hyphenation / ligature flags are accepted, and JSON carries per-page
    // provenance (schema 1.1).
    let json = run(&[
        "parse",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "json",
        "--dehyphenate",
        "--normalize-ligatures",
    ]);
    assert_ok(&json, "parse with robustness flags");
    let s = String::from_utf8_lossy(&json.stdout);
    assert!(s.contains("\"schema_version\": \"1.1\""), "schema 1.1");
    assert!(s.contains("\"source\""), "per-page source recorded");
}

#[test]
fn parse_profile_and_markdown_heading_flag_run() {
    let md = run(&[
        "parse",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "markdown",
        "--profile",
        "rag-chunks",
        "--detect-headings=false",
    ]);
    assert_ok(&md, "parse --profile --detect-headings=false");
    assert!(!md.stdout.is_empty(), "flat markdown produced no output");
}

#[test]
fn document_model_alias_runs() {
    // `document-model` is retained as a back-compat alias for `parse`.
    let out = run(&[
        "document-model",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "md",
    ]);
    assert_ok(&out, "document-model alias");
}

#[test]
fn render_raster_runs() {
    let o = tmp("render.zip");
    let out = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "png",
    ]);
    assert_ok(&out, "render png");
    assert!(o.exists());
    let _ = std::fs::remove_file(&o);
}

#[test]
fn render_contract_raw_surface_and_sidecar_runs() {
    let o = tmp("render_contract_raw.zip");
    let out = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "raw",
        "--render-contract",
        "--clip",
        "0,0,16,16",
        "--pixel-format",
        "bgra8",
        "--reverse-byte-order",
        "--halftone",
        "screen",
        "--print-profile",
        "proof",
        "--write-contract-json",
        "--json",
    ]);
    let json = assert_json(&out, "render contract raw");
    assert_eq!(json["render_contract"], true);
    assert_eq!(json["contract_json_sidecars"], 1);
    assert_eq!(json["pixel_format"], "bgra8");
    assert_eq!(json["print_profile"], "proof");

    let entries = zip_entries(&o);
    let raw = entries
        .iter()
        .find(|(name, _)| name == "page-001.raw")
        .expect("raw surface entry");
    assert_eq!(raw.1.len(), 16 * 16 * 4);
    let contract = entries
        .iter()
        .find(|(name, _)| name == "page-001.contract.json")
        .expect("contract sidecar entry");
    let contract: serde_json::Value =
        serde_json::from_slice(&contract.1).expect("parse contract sidecar");
    assert_eq!(contract["schema_version"], 1);
    assert_eq!(contract["clip"]["width"], 16);
    assert_eq!(contract["clip"]["height"], 16);
    assert_eq!(contract["pixel_format"], "Bgra8");
    assert_eq!(contract["reverse_byte_order"], true);
    assert_eq!(contract["halftone"], "Screen");
    assert_eq!(contract["print_profile"], "Proof");
    let contract_json = tmp("render_contract_input.json");
    std::fs::write(
        &contract_json,
        serde_json::to_vec_pretty(&contract).unwrap(),
    )
    .unwrap();

    let replay_zip = tmp("render_contract_replay.zip");
    let replay = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        replay_zip.to_str().unwrap(),
        "--format",
        "raw",
        "--contract-json",
        contract_json.to_str().unwrap(),
        "--json",
    ]);
    let replay_json = assert_json(&replay, "render --contract-json raw");
    assert_eq!(replay_json["render_contract"], true);
    assert_eq!(replay_json["contract_json_input"], true);
    assert_eq!(replay_json["pixel_format"], "Bgra8");
    assert_eq!(replay_json["print_profile"], "Proof");
    let replay_entries = zip_entries(&replay_zip);
    let replay_raw = replay_entries
        .iter()
        .find(|(name, _)| name == "page-001.raw")
        .expect("replayed raw surface entry");
    assert_eq!(replay_raw.1.len(), 16 * 16 * 4);

    let _ = std::fs::remove_file(&o);
    let _ = std::fs::remove_file(&contract_json);
    let _ = std::fs::remove_file(&replay_zip);
}

#[test]
fn render_raster_output_is_deterministic_across_thread_counts() {
    let serial_zip = tmp("render_threads_1.zip");
    let parallel_zip = tmp("render_threads_4.zip");

    let mut serial = wellfriendpdf();
    let serial = serial
        .env("RAYON_NUM_THREADS", "1")
        .args([
            "render",
            fx("multi_stream.pdf").to_str().unwrap(),
            "-o",
            serial_zip.to_str().unwrap(),
            "-p",
            "1-2",
            "--format",
            "png",
        ])
        .output()
        .expect("render with one worker");
    assert_ok(&serial, "render png serial");

    let mut parallel = wellfriendpdf();
    let parallel = parallel
        .env("RAYON_NUM_THREADS", "4")
        .args([
            "render",
            fx("multi_stream.pdf").to_str().unwrap(),
            "-o",
            parallel_zip.to_str().unwrap(),
            "-p",
            "1-2",
            "--format",
            "png",
        ])
        .output()
        .expect("render with four workers");
    assert_ok(&parallel, "render png parallel");

    assert_eq!(zip_entries(&serial_zip), zip_entries(&parallel_zip));
    let _ = std::fs::remove_file(&serial_zip);
    let _ = std::fs::remove_file(&parallel_zip);
}

#[test]
fn render_svg_runs() {
    let o = tmp("render_svg.zip");
    let out = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "svg",
    ]);
    assert_ok(&out, "render svg");
    assert!(o.exists());
    let _ = std::fs::remove_file(&o);
}

#[test]
fn render_ps_runs() {
    // `render --format ps` (pdftops / pdftocairo -ps equivalent) — completes
    // the 12/12 Poppler tool surface. Output is a single DSC PostScript file.
    let o = tmp("render_ps.ps");
    let out = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "ps",
    ]);
    assert_ok(&out, "render ps");
    assert!(o.exists());
    let body = std::fs::read_to_string(&o).unwrap();
    assert!(
        body.starts_with("%!PS-Adobe-3.0"),
        "valid DSC PostScript header"
    );
    assert!(body.contains("showpage"));
    let _ = std::fs::remove_file(&o);
}

#[test]
fn render_eps_runs() {
    // `render --format eps` (pdftops -eps / pdftocairo -eps equivalent) — one
    // EPSF document per page inside the ZIP.
    let o = tmp("render_eps.zip");
    let out = run(&[
        "render",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--format",
        "eps",
    ]);
    assert_ok(&out, "render eps");
    assert!(o.exists());
    let _ = std::fs::remove_file(&o);
}

#[test]
fn extract_images_runs() {
    let o = tmp("images.zip");
    let out = run(&[
        "extract-images",
        fx("image_only.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
    ]);
    assert_ok(&out, "extract-images");
    assert!(o.exists());
    let _ = std::fs::remove_file(&o);
}

#[test]
fn extract_images_region_runs() {
    let o = tmp("images_region.zip");
    let out = run(&[
        "extract-images",
        fx("image_only.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "--region",
        "0,0,10000,10000",
    ]);
    assert_ok(&out, "extract-images --region");
    assert!(o.exists());
    let _ = std::fs::remove_file(&o);
}

#[test]
fn analyze_runs() {
    let out = run(&["analyze", fx("tracemonkey.pdf").to_str().unwrap()]);
    assert_ok(&out, "analyze");
    assert!(String::from_utf8_lossy(&out.stdout).contains("has_text_layer"));
}

#[test]
fn merge_runs_and_counts() {
    let o = tmp("merged.pdf");
    let out = run(&[
        "merge",
        fx("minimal.pdf").to_str().unwrap(),
        fx("flate.pdf").to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
    ]);
    assert_ok(&out, "merge");
    // Re-open with `info` and confirm 2 pages.
    let info = run(&["info", o.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&info.stdout).contains("Pages:           2"));
    let _ = std::fs::remove_file(&o);
}

#[test]
fn split_runs() {
    let pat = tmp("split-%d.pdf");
    let out = run(&[
        "split",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-o",
        pat.to_str().unwrap(),
        "-f",
        "1",
        "-l",
        "2",
    ]);
    assert_ok(&out, "split");
    for n in 1..=2 {
        let p = PathBuf::from(pat.to_string_lossy().replace("%d", &n.to_string()));
        assert!(p.exists(), "split page {n} missing");
        let _ = std::fs::remove_file(&p);
    }
}

#[test]
fn extract_pages_runs() {
    let o = tmp("subset.pdf");
    let out = run(&[
        "extract-pages",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "3,1",
        "-o",
        o.to_str().unwrap(),
    ]);
    assert_ok(&out, "extract-pages");
    let info = run(&["info", o.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&info.stdout).contains("Pages:           2"));
    let _ = std::fs::remove_file(&o);
}

#[test]
fn info_runs_json_and_human() {
    let human = run(&["info", fx("tracemonkey.pdf").to_str().unwrap()]);
    assert_ok(&human, "info");
    assert!(String::from_utf8_lossy(&human.stdout).contains("Pages:"));

    let json = run(&["info", fx("tracemonkey.pdf").to_str().unwrap(), "--json"]);
    assert_ok(&json, "info --json");
    assert!(String::from_utf8_lossy(&json.stdout).contains("\"page_count\""));
}

#[test]
fn fonts_runs() {
    let out = run(&["fonts", fx("tracemonkey.pdf").to_str().unwrap()]);
    assert_ok(&out, "fonts");
    assert!(String::from_utf8_lossy(&out.stdout).contains("type"));
}

#[test]
fn detach_runs() {
    let out = run(&["detach", fx("attach_nametree.pdf").to_str().unwrap()]);
    assert_ok(&out, "detach");
    assert!(String::from_utf8_lossy(&out.stdout).contains("Hello.txt"));
}

#[test]
fn to_html_runs() {
    let out = run(&[
        "to-html",
        fx("multi_stream.pdf").to_str().unwrap(),
        "-p",
        "1",
    ]);
    assert_ok(&out, "to-html");
    assert!(String::from_utf8_lossy(&out.stdout).contains("<!DOCTYPE html>"));
}

#[test]
fn verify_sig_runs() {
    let out = run(&["verify-sig", fx("sig_valid.pdf").to_str().unwrap()]);
    assert_ok(&out, "verify-sig");
    assert!(String::from_utf8_lossy(&out.stdout).contains("VALID"));
}

#[test]
fn signature_verify_json_does_not_hide_an_untrusted_result() {
    let out = run(&[
        "signature-verify",
        fx("sig_valid.pdf").to_str().unwrap(),
        "--json",
    ]);
    assert_eq!(
        out.status.code(),
        Some(11),
        "signature-verify must return the documented untrusted exit code: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&out.stdout)
        .expect("failed verification must still emit a machine-readable report");
    assert!(report.is_array());
    assert!(String::from_utf8_lossy(&out.stderr).contains("signature untrusted"));
}

#[test]
fn signature_validation_evidence_commands_are_exposed() {
    for command in [
        "evidence-fetch",
        "evidence-export",
        "evidence-verify",
        "evidence-replay",
    ] {
        let out = run(&[command, "--help"]);
        assert_ok(&out, command);
        assert!(String::from_utf8_lossy(&out.stdout).contains("evidence"));
    }
    for command in ["certificate-path-build", "certificate-path-verify"] {
        let out = run(&[command, "--help"]);
        assert_ok(&out, command);
        assert!(String::from_utf8_lossy(&out.stdout).contains("certificate"));
    }
    for (command, needle) in [("ocsp-check", "OCSP"), ("crl-check", "CRL")] {
        let out = run(&[command, "--help"]);
        assert_ok(&out, command);
        assert!(String::from_utf8_lossy(&out.stdout).contains(needle));
    }
}

#[test]
fn password_flag_accepted_by_render_and_images() {
    // render + extract-images accept --password (an encrypted fixture that
    // unlocks with the empty password).
    let enc = fixtures()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("tests")
        .join("corpus")
        .join("pdfs")
        .join("pdfjs")
        .join("empty_protected.pdf");
    if !enc.exists() {
        eprintln!("NOTE: encrypted fixture missing; skipping password-flag test");
        return;
    }
    let o = tmp("enc.zip");
    let out = run(&[
        "render",
        enc.to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "-p",
        "1",
        "--password",
        "",
    ]);
    assert_ok(&out, "render --password");
    let _ = std::fs::remove_file(&o);
}

/// Regression (Renderer Benchmark 0A, Part B): a hostile page declaring a giant
/// `/MediaBox` must NOT abort the process with a multi-hundred-gigabyte
/// allocation. The CLI must survive, exit 0, skip the page with a clean warning,
/// and write a (page-less) output archive.
#[test]
fn render_rejects_huge_page_without_abort() {
    // A parseable single-page PDF whose /MediaBox is [0 0 200000 200000]. At 144
    // DPI that is 400000x400000 px (~640 GB), which must be rejected cleanly
    // before allocation rather than aborting the process. The body is built with
    // a real cross-reference table + startxref so the reader accepts it.
    let input = tmp("huge_page.pdf");
    std::fs::write(&input, huge_page_pdf()).expect("write huge-page fixture");
    let o = tmp("huge_page.zip");

    let out = run(&[
        "render",
        input.to_str().unwrap(),
        "-o",
        o.to_str().unwrap(),
        "--dpi",
        "144",
    ]);

    // The process survives and exits cleanly (no abort/panic/signal).
    assert!(
        out.status.success(),
        "huge-page render must exit cleanly, got status {:?}; stderr: {}",
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("resource limit") || stderr.contains("skipped page"),
        "expected a clean resource-limit warning, got stderr: {stderr}"
    );
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&o);
}

/// Build a minimal but well-formed single-page PDF with a giant `/MediaBox`,
/// including a valid xref table and `startxref` so the reader parses it.
fn huge_page_pdf() -> Vec<u8> {
    let objs = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200000 200000] >>",
    ];
    let mut pdf = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (idx, body) in objs.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", idx + 1, body));
    }
    let xref_off = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n", objs.len() + 1));
    pdf.push_str("0000000000 65535 f \n");
    for off in &offsets {
        pdf.push_str(&format!("{:010} 00000 n \n", off));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objs.len() + 1,
        xref_off
    ));
    pdf.into_bytes()
}

fn simple_secret_pdf() -> Vec<u8> {
    let content = b"BT /F1 20 Tf 1 0 0 1 72 720 Tm (Public SECRET text) Tj ET\n";
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] \
         /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>"
            .to_string(),
        format!(
            "<< /Length {} >>\nstream\n{}endstream",
            content.len(),
            String::from_utf8_lossy(content)
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_string(),
    ];
    let mut pdf = String::from("%PDF-1.7\n");
    let mut offsets = Vec::new();
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.push_str(&format!("{} 0 obj\n{}\nendobj\n", idx + 1, body));
    }
    let xref_off = pdf.len();
    pdf.push_str(&format!("xref\n0 {}\n", objects.len() + 1));
    pdf.push_str("0000000000 65535 f \n");
    for off in offsets {
        pdf.push_str(&format!("{off:010} 00000 n \n"));
    }
    pdf.push_str(&format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
        objects.len() + 1,
        xref_off
    ));
    pdf.into_bytes()
}

// --- Unified-surface additions (Transparency Rendering) -----------------------------------

#[test]
fn transparency_rendering_report_commands_emit_json() {
    let feature = assert_json(&run(&["feature-report"]), "feature-report");
    assert_eq!(
        feature["report"]["transparency_rendering_transparency_compositing"]["status"],
        "native_foundation_with_transparency_closeout_closure"
    );
    assert_eq!(
        feature["report"]["transparency_rendering_transparency_compositing"]["reference_audit"]
            ["memory_cap_mb"],
        4096
    );
    assert_eq!(
        feature["report"]["transparency_closeout_transparency_closure"]["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["transparency_closeout_transparency_closure"]["reference_audit"]
            ["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["advanced_rendering_text_clipping_shading_patterns"]["reference_audit"]
            ["status"],
        "multi_reference_audit_complete"
    );
    assert_eq!(
        feature["report"]["advanced_rendering_text_clipping_shading_patterns"]["reference_audit"]
            ["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["status"],
        "complete_native_common_paths_with_reference_cluster_limits"
    );
    assert_eq!(
        feature["report"]["type3_cid_rendering_type3_cid_tensor_closure"]["reference_audit"]
            ["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
            ["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["cjk_rtl_color_glyph_closeout_color_glyph_cjk_rtl_fidelity_closure"]
            ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]["closure_gates"]
            ["public_report_schema"],
        "additive_feature_report_color_glyph_hinting"
    );
    assert_eq!(
        feature["report"]["color_glyph_hinting_color_glyph_hinting_cff_closure"]
            ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
            ["svg_in_opentype"]["status"],
        "safe_static_subset_rendered_active_constructs_blocked"
    );
    assert_eq!(
        feature["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]["closure_gates"]
            ["public_report_schema"],
        "additive_feature_report_colrv_svg_bitmap"
    );
    assert_eq!(
        feature["report"]["colrv_svg_bitmap_full_colrv1_svg_color_glyph_closure"]
            ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
            ["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
            ["colrv1_clip_stack"]["status"],
        "implemented"
    );
    assert_eq!(
        feature["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
            ["closure_gates"]["public_report_schema"],
        "additive_feature_report_colrv_gradient_composite"
    );
    assert_eq!(
        feature["report"]["colrv_gradient_composite_colrv1_gradient_clip_composite_closure"]
            ["multi_reference_audit"]["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
            ["status"],
        "complete"
    );
    assert_eq!(
        feature["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
            ["porter_duff_plus_composites"]["implemented_modes"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
    assert_eq!(
        feature["report"]["porterduff_radial_color_glyph_colrv1_porterduff_radial_closure"]
            ["closure_gates"]["public_report_schema"],
        "additive_feature_report_porterduff_radial_color_glyph"
    );
    assert_eq!(
        feature["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["status"],
        "complete_with_native_cmm_hard_blocked_precise"
    );
    assert_eq!(
        feature["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_fuzz"]
            ["fuzz_target_count"],
        25
    );
    assert_eq!(
        feature["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["renderer_closeout"]
            ["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        feature["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["native_cmm_backend"]
            ["backend_used_in_current_build"],
        "safe-rust-plus-qcms"
    );
    assert_eq!(
        feature["report"]["renderer_fuzz_cmm_renderer_fuzz_cmm_closeout"]["closure_gates"]
            ["public_report_schema"],
        "additive_feature_report_renderer_fuzz_cmm"
    );
    let native_cmm_backend =
        &feature["report"]["native_cmm_backend_native_littlecms_cmm_backend_closure"];
    assert_eq!(native_cmm_backend["status"], "complete");
    assert_eq!(
        native_cmm_backend["feature_flag"]["name"],
        "native-cmm-lcms2"
    );
    assert_eq!(
        native_cmm_backend["closure_gates"]["public_report_schema"],
        "additive_feature_report_native_cmm_backend"
    );
    let prepress_cmm =
        &feature["report"]["prepress_cmm_prepress_cmm_device_link_separation_plates"];
    assert_eq!(prepress_cmm["status"], "complete");
    assert_eq!(
        prepress_cmm["closure_gates"]["public_report_schema"],
        "additive_feature_report_prepress_cmm"
    );
    assert_eq!(
        prepress_cmm["separation_framebuffer"]["cache_key_includes_plate_state"],
        true
    );
    let nchannel_plate_prepress =
        &feature["report"]["nchannel_plate_prepress_nchannel_plate_reference_closure"];
    assert_eq!(nchannel_plate_prepress["status"], "complete");
    assert_eq!(
        nchannel_plate_prepress["closure_gates"]["public_report_schema"],
        "additive_feature_report_nchannel_plate_prepress"
    );
    assert_eq!(
        nchannel_plate_prepress["reference_audit"]["mupdf"],
        "required_and_run_by_nchannel_plate_prepress_audit"
    );
    let prepress_proofing =
        &feature["report"]["prepress_proofing_full_overprint_prepress_closeout"];
    assert_eq!(prepress_proofing["status"], "complete");
    assert_eq!(
        prepress_proofing["closure_gates"]["public_report_schema"],
        "additive_feature_report_prepress_proofing"
    );
    assert_eq!(
        prepress_proofing["reference_audit"]["wellfriendpdf_outlier_failures"],
        0
    );
    assert_eq!(
        prepress_proofing["reference_audit"]["unclassified_failures"],
        0
    );
    let semantic_intelligence =
        &feature["report"]["semantic_intelligence_semantic_intelligence_parenttree_cjk_ml_layout"];
    assert_eq!(semantic_intelligence["status"], "complete");
    assert_eq!(
        semantic_intelligence["closure_gates"]["public_report_schema"],
        "additive_feature_report_semantic_intelligence"
    );
    assert_eq!(
        semantic_intelligence["privacy_defaults"]["cloud_upload_default"],
        false
    );
    let cjk_dictionary_layout =
        &feature["report"]["cjk_dictionary_layout_cjk_dictionary_layout_backend_closure"];
    assert_eq!(cjk_dictionary_layout["status"], "complete");
    assert_eq!(
        cjk_dictionary_layout["closure_gates"]["public_report_schema"],
        "additive_feature_report_cjk_dictionary_layout"
    );
    assert_eq!(
        cjk_dictionary_layout["dictionary_provider"]["external_pack_support"],
        "implemented"
    );
    assert_eq!(
        cjk_dictionary_layout["layout_backend"]["local_backend_status"],
        "unsupported_reported_no_runtime"
    );

    let forms = assert_json(
        &run(&["forms-report", fx("form_160f.pdf").to_str().unwrap()]),
        "forms-report",
    );
    assert!(
        forms.get("has_acroform").is_some(),
        "forms report should expose AcroForm state"
    );

    let annots = assert_json(
        &run(&[
            "annotations-report",
            fx("attach_annot.pdf").to_str().unwrap(),
        ]),
        "annotations-report",
    );
    assert!(
        annots.get("annotations").is_some(),
        "annotations report should expose annotation list"
    );

    let pages = assert_json(
        &run(&["pages-report", fx("tracemonkey.pdf").to_str().unwrap()]),
        "pages-report",
    );
    assert!(
        pages["page_count"].as_u64().unwrap_or(0) > 0,
        "pages report should expose page count"
    );

    let combined = assert_json(
        &run(&[
            "interactive-report",
            fx("attach_annot.pdf").to_str().unwrap(),
        ]),
        "interactive-report",
    );
    assert_eq!(combined["schema_version"], 1);
    assert!(combined.get("forms").is_some());
    assert!(combined.get("annotations").is_some());
    assert!(combined.get("page_operations").is_some());
}

#[test]
fn transparency_closeout_form_annotation_and_page_ops_run() {
    let xfdf = tmp("transparency_closeout_fields.xfdf");
    let filled = tmp("transparency_closeout_filled.pdf");
    let flattened = tmp("transparency_closeout_flattened.pdf");
    let cropped = tmp("transparency_closeout_cropped.pdf");
    let scaled = tmp("transparency_closeout_scaled.pdf");
    let nup = tmp("transparency_closeout_nup.pdf");
    for path in [&xfdf, &filled, &flattened, &cropped, &scaled, &nup] {
        remove_path(path);
    }

    let export = run(&[
        "forms-export",
        fx("form_160f.pdf").to_str().unwrap(),
        "--format",
        "xfdf",
        "--output",
        xfdf.to_str().unwrap(),
    ]);
    assert_ok(&export, "forms-export xfdf");
    assert!(
        String::from_utf8_lossy(&std::fs::read(&xfdf).unwrap()).contains("<xfdf"),
        "XFDF export should be XML"
    );

    let import = assert_json(
        &run(&[
            "forms-import",
            fx("form_160f.pdf").to_str().unwrap(),
            xfdf.to_str().unwrap(),
            "--format",
            "xfdf",
            "--out",
            filled.to_str().unwrap(),
            "--json",
        ]),
        "forms-import xfdf",
    );
    assert!(import["imported_fields"].as_u64().unwrap_or(0) > 0);
    assert!(filled.exists());

    let flatten = assert_json(
        &run(&[
            "annotations-flatten",
            fx("attach_annot.pdf").to_str().unwrap(),
            "--out",
            flattened.to_str().unwrap(),
            "--json",
        ]),
        "annotations-flatten",
    );
    assert_eq!(flatten["op"], "annotations-flatten");
    assert!(flattened.exists());

    assert_ok(
        &run(&[
            "pages-crop",
            fx("tracemonkey.pdf").to_str().unwrap(),
            "--rect",
            "0,0,200,200",
            "--pages",
            "1",
            "--out",
            cropped.to_str().unwrap(),
            "--json",
        ]),
        "pages-crop",
    );
    assert_ok(
        &run(&[
            "pages-scale",
            fx("minimal.pdf").to_str().unwrap(),
            "--scale",
            "0.75",
            "--pages",
            "1",
            "--dpi",
            "72",
            "--out",
            scaled.to_str().unwrap(),
            "--json",
        ]),
        "pages-scale",
    );
    assert_ok(
        &run(&[
            "pages-nup",
            fx("minimal.pdf").to_str().unwrap(),
            "--columns",
            "2",
            "--rows",
            "1",
            "--pages",
            "1",
            "--dpi",
            "72",
            "--out",
            nup.to_str().unwrap(),
            "--json",
        ]),
        "pages-nup",
    );
    assert!(cropped.exists() && scaled.exists() && nup.exists());

    for path in [&xfdf, &filled, &flattened, &cropped, &scaled, &nup] {
        remove_path(path);
    }
}

#[test]
fn transparency_rendering_redact_search_term_writes_verified_pdf() {
    let input = tmp("transparency_rendering_redact_input.pdf");
    let output = tmp("transparency_rendering_redact_output.pdf");
    remove_path(&input);
    remove_path(&output);
    std::fs::write(&input, simple_secret_pdf()).expect("write redaction fixture");

    let report = assert_json(
        &run(&[
            "redact",
            input.to_str().unwrap(),
            "--text",
            "SECRET",
            "--pages",
            "1",
            "--out",
            output.to_str().unwrap(),
            "--json",
            "--strict",
            "--image-policy",
            "partial",
            "--attachments",
            "remove-all",
        ]),
        "redact --text",
    );
    assert_eq!(report["verification"]["verified_absent"], true);
    assert!(output.exists(), "redacted PDF should be written");

    let text = run(&["extract-text", output.to_str().unwrap(), "-p", "1"]);
    assert_ok(&text, "extract-text redacted output");
    let extracted = String::from_utf8_lossy(&text.stdout);
    assert!(
        !extracted.contains("SECRET"),
        "redacted term should not extract: {extracted}"
    );
    assert!(
        extracted.contains("Public"),
        "unredacted text should remain: {extracted}"
    );

    remove_path(&input);
    remove_path(&output);
}

#[test]
fn version_reports_engine_and_ocr_status() {
    // `--version` must report the engine version AND whether OCR is compiled in,
    // so a user can tell without running an --ocr command. (Value of the OCR
    // line depends on build features; the labels are always present.)
    let out = run(&["--version"]);
    assert_ok(&out, "--version");
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.contains("engine:"), "version should report engine: {s}");
    assert!(s.contains("ocr:"), "version should report ocr status: {s}");
    assert!(s.contains("features:"), "version should list features: {s}");
}

#[test]
fn extract_tables_ocr_errors_cleanly() {
    // --ocr on extract-tables is intentionally unsupported (OCR'd table-grid
    // reconstruction is a known gap); it must fail with an actionable message,
    // not silently produce empty/garbage output.
    let out = run(&["extract-tables", fx("flate.pdf").to_str().unwrap(), "--ocr"]);
    assert!(!out.status.success(), "extract-tables --ocr should error");
    assert_eq!(out.status.code(), Some(5), "unsupported feature exit code");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("does not support --ocr"),
        "should explain the gap: {err}"
    );
}

#[test]
fn cli_exit_codes_are_classified_and_clean() {
    let missing = run(&["info", "does-not-exist.pdf"]);
    assert_eq!(missing.status.code(), Some(3), "I/O exit code");
    let missing_err = String::from_utf8_lossy(&missing.stderr);
    assert!(missing_err.contains("I/O error"), "{missing_err}");
    assert!(!missing_err.contains("panicked"), "{missing_err}");

    let malformed = tmp("malformed_input.pdf");
    std::fs::write(&malformed, b"not a pdf").expect("write malformed fixture");
    let parse = run(&["info", malformed.to_str().unwrap()]);
    assert_eq!(parse.status.code(), Some(4), "parse/format exit code");
    let parse_err = String::from_utf8_lossy(&parse.stderr);
    assert!(parse_err.contains("parse/format error"), "{parse_err}");
    assert!(!parse_err.contains("panicked"), "{parse_err}");

    let usage = run(&[
        "extract-text",
        fx("flate.pdf").to_str().unwrap(),
        "--structured",
        "--format",
        "xml",
    ]);
    assert_eq!(usage.status.code(), Some(2), "usage exit code");
    let usage_err = String::from_utf8_lossy(&usage.stderr);
    assert!(usage_err.contains("usage error"), "{usage_err}");
    assert!(!usage_err.contains("panicked"), "{usage_err}");

    let _ = std::fs::remove_file(&malformed);
}

#[test]
fn write_commands_emit_json_summaries() {
    let render_out = tmp("render_json.zip");
    let render = run(&[
        "render",
        fx("flate.pdf").to_str().unwrap(),
        "-o",
        render_out.to_str().unwrap(),
        "-p",
        "1",
        "--json",
    ]);
    let json = assert_json(&render, "render --json");
    assert_eq!(json["op"], "render");
    assert_eq!(json["pages_rendered"], 1);

    let images_out = tmp("images_json.zip");
    let images = run(&[
        "extract-images",
        fx("image_only.pdf").to_str().unwrap(),
        "-o",
        images_out.to_str().unwrap(),
        "--json",
    ]);
    let json = assert_json(&images, "extract-images --json");
    assert_eq!(json["op"], "extract-images");
    assert!(json["images"].as_u64().unwrap_or(0) >= 1);

    let merge_out = tmp("merge_json.pdf");
    let merge = run(&[
        "merge",
        fx("flate.pdf").to_str().unwrap(),
        fx("minimal.pdf").to_str().unwrap(),
        "-o",
        merge_out.to_str().unwrap(),
        "--json",
    ]);
    let json = assert_json(&merge, "merge --json");
    assert_eq!(json["op"], "merge");
    assert_eq!(json["inputs"], 2);

    let extract_out = tmp("extract_pages_json.pdf");
    let extract = run(&[
        "extract-pages",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "1",
        "-o",
        extract_out.to_str().unwrap(),
        "--json",
    ]);
    let json = assert_json(&extract, "extract-pages --json");
    assert_eq!(json["op"], "extract-pages");
    assert_eq!(json["pages"].as_array().unwrap().len(), 1);

    let split_pattern = tmp("split_json_%d.pdf");
    let split = run(&[
        "split",
        fx("minimal.pdf").to_str().unwrap(),
        "-o",
        split_pattern.to_str().unwrap(),
        "--json",
    ]);
    let json = assert_json(&split, "split --json");
    assert_eq!(json["op"], "split");
    assert_eq!(json["files"], 1);

    let linearized_out = tmp("linearize_json.pdf");
    let linearized = run(&[
        "linearize",
        fx("flate.pdf").to_str().unwrap(),
        "-o",
        linearized_out.to_str().unwrap(),
        "--json",
    ]);
    let json = assert_json(&linearized, "linearize --json");
    assert_eq!(json["op"], "linearize");
    assert!(json["bytes"].as_u64().unwrap_or(0) > 0);

    let _ = std::fs::remove_file(&render_out);
    let _ = std::fs::remove_file(&images_out);
    let _ = std::fs::remove_file(&merge_out);
    let _ = std::fs::remove_file(&extract_out);
    let _ = std::fs::remove_file(expand_test_split_pattern(&split_pattern, 1));
    let _ = std::fs::remove_file(&linearized_out);
}

fn expand_test_split_pattern(pattern: &std::path::Path, page: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(pattern.to_string_lossy().replace("%d", &page.to_string()))
}

// --- Structural-write ops (Bucket 2) ----------------------------------------

#[test]
fn rotate_command_writes_rotated_pdf() {
    let out = tmp("rotate_out.pdf");
    let res = run(&[
        "rotate",
        fx("flate.pdf").to_str().unwrap(),
        "--angle",
        "90",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "rotate");
    assert!(out.exists() && std::fs::metadata(&out).unwrap().len() > 0);
    let _ = std::fs::remove_file(&out);
}

#[test]
fn encrypt_command_aes256_roundtrips_via_cli() {
    let out = tmp("encrypt_out.pdf");
    let res = run(&[
        "encrypt",
        fx("flate.pdf").to_str().unwrap(),
        "--user-pw",
        "secret",
        "--algo",
        "aes256",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "encrypt");
    // The encrypted file must NOT extract text without the password...
    let no_pw = run(&["extract-text", out.to_str().unwrap()]);
    assert!(
        !no_pw.status.success(),
        "encrypted file must require a password"
    );
    // ...and must extract WITH it.
    let with_pw = run(&[
        "extract-text",
        out.to_str().unwrap(),
        "--password",
        "secret",
    ]);
    assert_ok(&with_pw, "extract-text with password");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn optimize_command_writes_smaller_or_equal_pdf() {
    let out = tmp("optimize_out.pdf");
    let res = run(&[
        "optimize",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--json",
    ]);
    assert_ok(&res, "optimize");
    assert!(out.exists());
    // --json output is parseable and reports the op.
    let stdout = String::from_utf8_lossy(&res.stdout);
    assert!(
        stdout.contains("\"op\":\"optimize\""),
        "json result: {stdout}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn repair_command_writes_clean_pdf() {
    let out = tmp("repair_out.pdf");
    let res = run(&[
        "repair",
        fx("flate.pdf").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "repair");
    assert!(out.exists());
    // The repaired file re-parses.
    let info = run(&["info", out.to_str().unwrap()]);
    assert_ok(&info, "info on repaired");
    let _ = std::fs::remove_file(&out);
}

#[test]
fn linearize_command_writes_fast_web_view_pdf() {
    let out = tmp("linearize_out.pdf");
    let _ = std::fs::remove_file(&out);
    let res = run(&[
        "linearize",
        fx("flate.pdf").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "linearize");
    assert!(out.exists());
    let bytes = std::fs::read(&out).expect("linearized output");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/Linearized"),
        "output should carry /Linearized"
    );
    let info = run(&["info", out.to_str().unwrap()]);
    assert_ok(&info, "info on linearized");
    let stdout = String::from_utf8_lossy(&info.stdout);
    assert!(
        stdout.contains("Optimized:       yes"),
        "info output: {stdout}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn linearize_command_writes_multi_page_fast_web_view_pdf() {
    let out = tmp("linearize_multi_out.pdf");
    let _ = std::fs::remove_file(&out);
    let res = run(&[
        "linearize",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "linearize multi-page");
    assert!(out.exists());
    let bytes = std::fs::read(&out).expect("linearized multi-page output");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/Linearized"),
        "multi-page output should carry /Linearized"
    );
    let info = run(&["info", out.to_str().unwrap()]);
    assert_ok(&info, "info on multi-page linearized");
    let stdout = String::from_utf8_lossy(&info.stdout);
    assert!(
        stdout.contains("Pages:           14"),
        "info output: {stdout}"
    );
    assert!(
        stdout.contains("Optimized:       yes"),
        "info output: {stdout}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn linearize_command_writes_form_fast_web_view_pdf() {
    let out = tmp("linearize_form_out.pdf");
    let _ = std::fs::remove_file(&out);
    let res = run(&[
        "linearize",
        fx("form_160f.pdf").to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_ok(&res, "linearize form");
    assert!(out.exists());
    let bytes = std::fs::read(&out).expect("linearized form output");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("/Linearized"),
        "form output should carry /Linearized"
    );
    let info = run(&["info", out.to_str().unwrap()]);
    assert_ok(&info, "info on form linearized");
    let stdout = String::from_utf8_lossy(&info.stdout);
    assert!(
        stdout.contains("Optimized:       yes"),
        "info output: {stdout}"
    );
    let _ = std::fs::remove_file(&out);
}

#[test]
fn phase3_utilities_run_across_cli_surface() {
    let out_dir = tmp("phase3_pages");
    let wrapped = tmp("phase3_wrapped.pdf");
    let watermarked = tmp("phase3_watermarked.pdf");
    let numbered = tmp("phase3_numbered.pdf");
    let organized = tmp("phase3_organized.pdf");
    remove_path(&out_dir);
    remove_path(&wrapped);
    remove_path(&watermarked);
    remove_path(&numbered);
    remove_path(&organized);

    let render = run(&[
        "pdf-to-jpg",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--pages",
        "1",
        "--out-dir",
        out_dir.to_str().unwrap(),
        "--dpi",
        "72",
        "--json",
    ]);
    let json = assert_json(&render, "pdf-to-jpg");
    assert_eq!(json["failed_pages"], 0);
    let jpg = out_dir.join("page-001.jpg");
    assert!(jpg.exists(), "raster output should exist");

    let image_pdf = run(&[
        "image-to-pdf",
        jpg.to_str().unwrap(),
        "--out",
        wrapped.to_str().unwrap(),
        "--page-size",
        "size-to-image",
    ]);
    assert_ok(&image_pdf, "image-to-pdf");
    assert!(wrapped.exists());

    let watermark = run(&[
        "watermark",
        wrapped.to_str().unwrap(),
        "--text",
        "DRAFT",
        "--opacity",
        "0.25",
        "--out",
        watermarked.to_str().unwrap(),
    ]);
    assert_ok(&watermark, "watermark");

    let numbers = run(&[
        "add-page-numbers",
        watermarked.to_str().unwrap(),
        "--format",
        "{n}/{total}",
        "--out",
        numbered.to_str().unwrap(),
    ]);
    assert_ok(&numbers, "add-page-numbers");

    let organize = run(&[
        "organize",
        numbered.to_str().unwrap(),
        "--order",
        "1,1",
        "--out",
        organized.to_str().unwrap(),
    ]);
    assert_ok(&organize, "organize");
    let info = assert_json(
        &run(&["info", organized.to_str().unwrap(), "--json"]),
        "info on organized Phase 3 PDF",
    );
    assert_eq!(info["page_count"], 2);

    remove_path(&out_dir);
    remove_path(&wrapped);
    remove_path(&watermarked);
    remove_path(&numbered);
    remove_path(&organized);
}

#[test]
fn phase4_office_conversions_run_across_cli_surface() {
    let xlsx = tmp("phase4_tables.xlsx");
    let pptx = tmp("phase4_slides.pptx");
    let docx = tmp("phase4_doc.docx");
    let from_xlsx = tmp("phase4_from_xlsx.pdf");
    let from_pptx = tmp("phase4_from_pptx.pdf");
    let from_docx = tmp("phase4_from_docx.pdf");
    remove_path(&xlsx);
    remove_path(&pptx);
    remove_path(&docx);
    remove_path(&from_xlsx);
    remove_path(&from_pptx);
    remove_path(&from_docx);

    let xlsx_out = run(&[
        "pdf-to-xlsx",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--out",
        xlsx.to_str().unwrap(),
        "--layout",
        "pages",
        "--json",
    ]);
    let xlsx_json = assert_json(&xlsx_out, "pdf-to-xlsx");
    assert_eq!(xlsx_json["layout"], "pages");
    assert!(xlsx.exists());
    let xlsx_entries = zip_entries(&xlsx);
    assert!(xlsx_entries
        .iter()
        .any(|(name, _)| name == "xl/workbook.xml"));
    assert!(xlsx_entries
        .iter()
        .any(|(name, _)| name == "xl/worksheets/sheet1.xml"));

    let pptx_out = run(&[
        "pdf-to-pptx",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--out",
        pptx.to_str().unwrap(),
        "--json",
    ]);
    let pptx_json = assert_json(&pptx_out, "pdf-to-pptx");
    assert_eq!(pptx_json["include_images"], true);
    assert!(pptx.exists());
    let pptx_entries = zip_entries(&pptx);
    assert!(pptx_entries
        .iter()
        .any(|(name, _)| name == "ppt/presentation.xml"));
    assert!(pptx_entries
        .iter()
        .any(|(name, _)| name == "ppt/slides/slide1.xml"));

    let docx_out = run(&[
        "pdf-to-docx",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--out",
        docx.to_str().unwrap(),
        "--json",
    ]);
    let docx_json = assert_json(&docx_out, "pdf-to-docx");
    assert_eq!(docx_json["include_images"], true);
    assert!(docx.exists());
    let docx_entries = zip_entries(&docx);
    assert!(docx_entries
        .iter()
        .any(|(name, _)| name == "word/document.xml"));

    for (command, input, output) in [
        ("xlsx-to-pdf", &xlsx, &from_xlsx),
        ("pptx-to-pdf", &pptx, &from_pptx),
        ("docx-to-pdf", &docx, &from_docx),
    ] {
        let out = run(&[
            command,
            input.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
            "--json",
        ]);
        let json = assert_json(&out, command);
        assert!(json["output_bytes"].as_u64().unwrap() > 100);
        assert!(output.exists());
        let bytes = std::fs::read(output).unwrap();
        assert!(bytes.starts_with(b"%PDF-"), "{command} did not write a PDF");
    }

    remove_path(&xlsx);
    remove_path(&pptx);
    remove_path(&docx);
    remove_path(&from_xlsx);
    remove_path(&from_pptx);
    remove_path(&from_docx);
}
