//! CLI contract for `pdfa-convert` and the new-form `pdfa-validate`.

use std::path::PathBuf;
use std::process::Command;

use wellfriendpdf_engine::{AuthorPageSize, PdfBuilder, StandardFont, TextStyle};

fn wellfriendpdf() -> Command {
    Command::new(env!("CARGO_BIN_EXE_wellfriendpdf"))
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wellfriendpdf_pdfa_cli_{}_{}",
        std::process::id(),
        name
    ))
}

fn source_pdf() -> PathBuf {
    let mut doc = PdfBuilder::new();
    doc.set_title("CLI PDF/A source");
    doc.add_page(AuthorPageSize::LETTER)
        .draw_text(
            "Helvetica is not embedded in this source.",
            72.0,
            720.0,
            &TextStyle::standard(StandardFont::Helvetica, 12.0),
        )
        .unwrap();
    let path = tmp("source.pdf");
    std::fs::write(&path, doc.to_bytes().unwrap()).unwrap();
    path
}

fn json_line(out: &std::process::Output) -> serde_json::Value {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let line = stdout.lines().last().unwrap_or_default();
    serde_json::from_str(line).unwrap_or_else(|err| panic!("not JSON ({err}): {stdout}"))
}

#[test]
fn pdfa_convert_contract_and_validate_new_form() {
    let source = source_pdf();
    let output = tmp("out.pdf");
    for options in [
        r#"{"profile":"PDF/A-2b","system_fonts":false}"#,
        r#"{"level":"2u","system_fonts":false}"#,
    ] {
        let out = wellfriendpdf()
            .args(["pdfa-convert", "--input"])
            .arg(&source)
            .arg("--output")
            .arg(&output)
            .args(["--options", options])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let json = json_line(&out);
        assert_eq!(json["ok"], true);
        assert_eq!(json["validated"], true);
        assert_eq!(json["pages"], 1);
        assert!(json["bytes"].as_u64().unwrap() > 0);
        let profile = json["profile"].as_str().unwrap().to_string();
        assert!(profile.starts_with("PDF/A-2"));
        let validate_options = format!(r#"{{"profile":"{profile}"}}"#);

        let out = wellfriendpdf()
            .args(["pdfa-validate", "--input"])
            .arg(&output)
            .args(["--options", validate_options.as_str()])
            .output()
            .unwrap();
        assert!(out.status.success());
        let json = json_line(&out);
        assert_eq!(json["ok"], true);
        assert_eq!(json["compliant"], true, "{json}");
        assert_eq!(json["profile"], profile.as_str());
        assert_eq!(json["failures"].as_array().unwrap().len(), 0);
    }

    // The unconverted source is reported non-compliant with rule ids.
    let out = wellfriendpdf()
        .args(["pdfa-validate", "--input"])
        .arg(&source)
        .args(["--options", r#"{"profile":"PDF/A-2b"}"#])
        .output()
        .unwrap();
    let json = json_line(&out);
    assert_eq!(json["compliant"], false);
    assert!(!json["failures"].as_array().unwrap().is_empty());

    // Old form still works.
    let out = wellfriendpdf()
        .arg("pdfa-validate")
        .arg(&output)
        .args(["--target", "PDF/A-2B", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());

    // Unknown profile: {"ok":false,"code","message"} and non-zero exit.
    let out = wellfriendpdf()
        .args(["pdfa-convert", "--input"])
        .arg(&source)
        .arg("--output")
        .arg(tmp("bad.pdf"))
        .args(["--options", r#"{"profile":"PDF/A-9z"}"#])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let json = json_line(&out);
    assert_eq!(json["ok"], false);
    assert_eq!(json["code"], "invalid_options");
    assert!(json["message"].is_string());
    assert!(!tmp("bad.pdf").exists());

    let _ = std::fs::remove_file(source);
    let _ = std::fs::remove_file(output);
}
