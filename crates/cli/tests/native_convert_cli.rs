//! CLI contract for the Rust-native converters: `--input`, `--output`,
//! `--options <json>`, one JSON result line on stdout, non-zero exit on error
//! with a code from the server vocabulary.

use std::path::PathBuf;
use std::process::Command;

fn tmp(name: &str) -> PathBuf {
    // Tests run in parallel; every call gets its own file names.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "wellfriendpdf_native_cli_{}_{}_{}",
        std::process::id(),
        id,
        name
    ))
}

fn run(
    command: &str,
    input: &[u8],
    ext: &str,
    options: Option<&str>,
) -> (bool, serde_json::Value, PathBuf) {
    let input_path = tmp(&format!("{command}.{ext}"));
    let output_path = tmp(&format!("{command}.pdf"));
    std::fs::write(&input_path, input).unwrap();
    let _ = std::fs::remove_file(&output_path);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_wellfriendpdf"));
    cmd.arg(command)
        .arg("--input")
        .arg(&input_path)
        .arg("--output")
        .arg(&output_path);
    if let Some(options) = options {
        cmd.arg("--options").arg(options);
    }
    let out = cmd.output().expect("spawn wellfriendpdf");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    assert_eq!(lines.len(), 1, "expected one JSON line, got {stdout:?}");
    let json: serde_json::Value = serde_json::from_str(lines[0]).expect("JSON result line");
    let _ = std::fs::remove_file(&input_path);
    (out.status.success(), json, output_path)
}

fn assert_success(
    command: &str,
    input: &[u8],
    ext: &str,
    options: Option<&str>,
) -> (serde_json::Value, wellfriendpdf_engine::ContentEngine) {
    let (ok, json, output) = run(command, input, ext, options);
    assert!(ok, "{command} failed: {json}");
    assert_eq!(json["ok"], true);
    assert_eq!(json.as_object().unwrap().len(), 3, "{json}");
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(json["bytes"].as_u64().unwrap() as usize, bytes.len());
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(bytes).unwrap();
    assert_eq!(
        engine.page_count().unwrap() as u64,
        json["pages"].as_u64().unwrap()
    );
    let _ = std::fs::remove_file(&output);
    (json, engine)
}

#[test]
fn text_to_pdf_cli_accepts_server_option_names() {
    let (json, engine) = assert_success(
        "text-to-pdf",
        "hello\nмир".as_bytes(),
        "txt",
        Some(
            r#"{"pageSize":"Legal","orientation":"landscape","fontSize":12,"margin":30,"title":"T","author":"A","monospace":true,"line_wrap":"char","unknownKey":true}"#,
        ),
    );
    assert_eq!(json["pages"], 1);
    let media_box = engine.get_page(1).unwrap().media_box;
    assert!((media_box[2] - 1008.0).abs() < 0.5, "{media_box:?}");
    let info = engine.document_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("T"));
    assert_eq!(info.author.as_deref(), Some("A"));
}

#[test]
fn markdown_to_pdf_cli() {
    assert_success("markdown-to-pdf", b"# Hi\n\n- a\n- b\n", "md", None);
}

#[test]
fn csv_to_pdf_cli() {
    let (_, engine) = assert_success(
        "csv-to-pdf",
        b"a;b\n1;2\n",
        "csv",
        Some(
            r#"{"delimiter":";","headerRow":true,"pageSize":"A4","orientation":"portrait","fontSize":9}"#,
        ),
    );
    assert!(engine.get_page_text(1).unwrap().contains('2'));
}

#[test]
fn json_to_pdf_cli() {
    assert_success("json-to-pdf", br#"{"k":[1,2,3]}"#, "json", None);
}

#[test]
fn svg_to_pdf_cli() {
    assert_success(
        "svg-to-pdf",
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="50" height="50"><circle cx="25" cy="25" r="20"/></svg>"#,
        "svg",
        Some(r#"{"page":"fit","pageSize":"Letter"}"#),
    );
}

#[test]
fn failures_print_code_and_exit_non_zero() {
    let (ok, json, _) = run(
        "csv-to-pdf",
        b"a,b,c\n",
        "csv",
        Some(r#"{"max_columns":2}"#),
    );
    assert!(!ok);
    assert_eq!(json["ok"], false);
    assert_eq!(json["code"], "RESOURCE_LIMIT");
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("csv_column_limit_exceeded"));

    let (ok, json, _) = run("text-to-pdf", b"x", "txt", Some("{not json"));
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_OPTIONS");

    let (ok, json, _) = run("text-to-pdf", b"x", "txt", Some(r#"{"fontSize":40}"#));
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_OPTIONS");

    let (ok, json, _) = run("svg-to-pdf", b"<nope>", "svg", None);
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_INPUT");

    let (ok, json, _) = run("json-to-pdf", b"{oops", "json", None);
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_JSON");
}
