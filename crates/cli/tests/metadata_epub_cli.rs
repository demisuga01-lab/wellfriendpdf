//! CLI contract tests for `edit-metadata` and `pdf-to-epub`: `--input`,
//! `--output`/`--output-dir`, `--options <json>`, one JSON line on stdout, and a
//! non-zero exit status on failure.

use std::io::Read;
use std::path::PathBuf;
use std::process::Command;

fn wellfriendpdf() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wellfriendpdf"))
}

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("engine")
        .join("tests")
        .join("fixtures")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wellfriendpdf_meta_epub_{}_{}",
        std::process::id(),
        name
    ))
}

fn run(args: &[&str]) -> (bool, serde_json::Value) {
    let out = wellfriendpdf()
        .args(args)
        .output()
        .expect("spawn wellfriendpdf");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout
        .lines()
        .last()
        .unwrap_or_else(|| panic!("no stdout; stderr={}", String::from_utf8_lossy(&out.stderr)));
    let value: serde_json::Value = serde_json::from_str(line)
        .unwrap_or_else(|err| panic!("stdout is not one JSON line ({err}): {stdout}"));
    (out.status.success(), value)
}

#[test]
fn edit_metadata_single_file() {
    let output = tmp("single.pdf");
    let input = fx("minimal.pdf");
    let (ok, json) = run(&[
        "edit-metadata",
        "--input",
        input.to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--options",
        r#"{"title":"CLI Title","author":"CLI Author","creationDate":"2024-01-02","fields":{"Batch":"no"},"unknownKey":1}"#,
    ]);
    assert!(ok, "{json}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["pages"], 1);
    assert!(json["bytes"].as_u64().unwrap() > 0);
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(&std::fs::read(&input).unwrap()));
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(bytes).unwrap();
    let info = engine.document_info().unwrap();
    assert_eq!(info.title.as_deref(), Some("CLI Title"));
    assert_eq!(info.author.as_deref(), Some("CLI Author"));
    let (raw, xmp) = wellfriendpdf_engine::metadata_edit::read_metadata_views(&engine);
    assert_eq!(raw["Batch"], "no");
    assert_eq!(raw["CreationDate"], "D:20240102000000");
    let xmp = xmp.unwrap();
    assert!(xmp.contains(">CLI Title</rdf:li>"));
    assert!(xmp.contains("<xmp:CreateDate>2024-01-02T00:00:00</xmp:CreateDate>"));
    let _ = std::fs::remove_file(output);
}

#[test]
fn edit_metadata_batch_and_failures() {
    let dir = tmp("batch");
    let _ = std::fs::remove_dir_all(&dir);
    let a = fx("minimal.pdf");
    let b = fx("flate.pdf");
    let (ok, json) = run(&[
        "edit-metadata",
        "--input",
        a.to_str().unwrap(),
        "--input",
        b.to_str().unwrap(),
        "--output-dir",
        dir.to_str().unwrap(),
        "--options",
        r#"{"subject":"Batch subject"}"#,
    ]);
    assert!(ok, "{json}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["files"].as_array().unwrap().len(), 2);
    assert_eq!(json["files"][0]["ok"], true);
    assert!(json["files"][0]["code"].is_null());
    assert!(json["files"][0]["pages"].as_u64().unwrap() >= 1);
    for name in ["minimal.pdf", "flate.pdf"] {
        let engine =
            wellfriendpdf_engine::ContentEngine::open_bytes(std::fs::read(dir.join(name)).unwrap())
                .unwrap();
        assert_eq!(
            engine.document_info().unwrap().subject.as_deref(),
            Some("Batch subject")
        );
    }

    // A missing input fails that file, reports per-file errors, exits non-zero.
    let missing = dir.join("does-not-exist.pdf");
    let (ok, json) = run(&[
        "edit-metadata",
        "--input",
        a.to_str().unwrap(),
        "--input",
        missing.to_str().unwrap(),
        "--output-dir",
        dir.join("out2").to_str().unwrap(),
        "--options",
        r#"{"subject":"x"}"#,
    ]);
    assert!(!ok);
    assert_eq!(json["ok"], false);
    assert_eq!(json["code"], "INVALID_INPUT");
    assert_eq!(json["files"][0]["ok"], true);
    assert_eq!(json["files"][1]["ok"], false);

    // Unknown keys are ignored; nothing left to change is INVALID_OPTIONS.
    let (ok, json) = run(&[
        "edit-metadata",
        "--input",
        a.to_str().unwrap(),
        "--output",
        dir.join("x.pdf").to_str().unwrap(),
        "--options",
        r#"{"titel":"typo"}"#,
    ]);
    assert!(!ok);
    assert_eq!(json["ok"], false);
    assert_eq!(json["code"], "INVALID_OPTIONS");

    let (ok, json) = run(&[
        "edit-metadata",
        "--input",
        a.to_str().unwrap(),
        "--output",
        dir.join("x.pdf").to_str().unwrap(),
        "--options",
        "{not json",
    ]);
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_JSON");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pdf_to_epub_contract() {
    let output = tmp("paper.epub");
    let (ok, json) = run(&[
        "pdf-to-epub",
        "--input",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--options",
        r#"{"split":"pages","pagesPerChapter":4}"#,
    ]);
    assert!(ok, "{json}");
    assert_eq!(json["ok"], true);
    assert_eq!(json["pages"], 14);
    assert!(json["bytes"].as_u64().unwrap() > 0);
    assert_eq!(json["chapters"], 4);
    let file = std::fs::File::open(&output).unwrap();
    let mut zip = zip::ZipArchive::new(file).unwrap();
    {
        let mut first = zip.by_index(0).unwrap();
        assert_eq!(first.name(), "mimetype");
        assert_eq!(first.compression(), zip::CompressionMethod::Stored);
        let mut text = String::new();
        first.read_to_string(&mut text).unwrap();
        assert_eq!(text, "application/epub+zip");
    }
    assert!(zip.by_name("OEBPS/nav.xhtml").is_ok());

    let (ok, json) = run(&[
        "pdf-to-epub",
        "--input",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--options",
        r#"{"split":"chapters"}"#,
    ]);
    assert!(!ok);
    assert_eq!(json["ok"], false);
    assert_eq!(json["code"], "INVALID_OPTIONS");

    let (ok, json) = run(&[
        "pdf-to-epub",
        "--input",
        fx("tracemonkey.pdf").to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
        "--options",
        r#"{"pageRange":"2-3","split":"pages","pagesPerChapter":1}"#,
    ]);
    assert!(ok, "{json}");
    assert_eq!(json["pages"], 2);
    assert_eq!(json["chapters"], 2);

    let (ok, json) = run(&[
        "pdf-to-epub",
        "--input",
        fx("does-not-exist.pdf").to_str().unwrap(),
        "--output",
        output.to_str().unwrap(),
    ]);
    assert!(!ok);
    assert_eq!(json["code"], "INVALID_INPUT");
    let _ = std::fs::remove_file(output);
}
