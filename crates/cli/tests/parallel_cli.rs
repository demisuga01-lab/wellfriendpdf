//! CLI contract of the page-parallel conversions: `--threads`, `--pages` on
//! the Office converters, the output facts in JSON results, and incremental
//! rotation.

use std::path::PathBuf;
use std::process::{Command, Output};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../engine/tests/fixtures")
        .join(name)
}

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "wellfriendpdf_parallel_cli_{}_{name}",
        std::process::id()
    ))
}

fn run(args: &[&str]) -> Output {
    let out = Command::new(env!("CARGO_BIN_EXE_wellfriendpdf"))
        .args(args)
        .output()
        .expect("spawn wellfriendpdf");
    assert!(
        out.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).expect("JSON result")
}

fn sha256_hex(bytes: &[u8]) -> String {
    wellfriendpdf_engine::OutputFacts::of(bytes).sha256
}

#[test]
fn conversions_are_identical_for_any_thread_count() {
    let pdf = fixture("tracemonkey.pdf");
    let pdf = pdf.to_str().unwrap();
    for (command, ext, extra) in [
        ("pdf-to-markdown", "md", &[][..]),
        ("pdf-to-json", "json", &[][..]),
        ("pdf-to-docx", "docx", &[][..]),
        ("pdf-to-xlsx", "xlsx", &["--layout", "tables"][..]),
    ] {
        let mut outputs = Vec::new();
        for threads in ["1", "4", "0"] {
            let out = tmp(&format!("{command}-{threads}.{ext}"));
            let mut args = vec![
                command,
                pdf,
                "-o",
                out.to_str().unwrap(),
                "--json",
                "--threads",
                threads,
            ];
            args.extend_from_slice(extra);
            let result = json(&run(&args));
            let bytes = std::fs::read(&out).unwrap();
            assert_eq!(result["output_sha256"], sha256_hex(&bytes), "{command}");
            assert_eq!(result["output_bytes"], bytes.len() as u64, "{command}");
            assert_eq!(result["output_pages"], 14, "{command}");
            outputs.push(bytes);
            let _ = std::fs::remove_file(&out);
        }
        assert!(
            outputs.windows(2).all(|pair| pair[0] == pair[1]),
            "{command}"
        );
    }
}

#[test]
fn office_converters_take_a_page_range() {
    let pdf = fixture("tracemonkey.pdf");
    let pdf = pdf.to_str().unwrap();
    for (command, ext) in [("pdf-to-docx", "docx"), ("pdf-to-xlsx", "xlsx")] {
        let all = tmp(&format!("all.{ext}"));
        let some = tmp(&format!("some.{ext}"));
        run(&[command, pdf, "-o", all.to_str().unwrap()]);
        let result = json(&run(&[
            command,
            pdf,
            "-o",
            some.to_str().unwrap(),
            "-p",
            "2-3",
            "--json",
        ]));
        assert_eq!(result["output_pages"], 2);
        let all_len = std::fs::metadata(&all).unwrap().len();
        let some_len = std::fs::metadata(&some).unwrap().len();
        assert!(some_len < all_len, "{command}: {some_len} >= {all_len}");
        // "all" is the default selection.
        let default = tmp(&format!("default.{ext}"));
        run(&[command, pdf, "-o", default.to_str().unwrap(), "-p", "all"]);
        assert_eq!(
            std::fs::read(&default).unwrap(),
            std::fs::read(&all).unwrap()
        );
        for path in [all, some, default] {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[test]
fn pdf_writers_report_output_facts() {
    let pdf = fixture("tracemonkey.pdf");
    let pdf = pdf.to_str().unwrap();
    let out = tmp("facts.pdf");
    let out_str = out.to_str().unwrap();
    for (args, pages) in [
        (
            vec!["rotate", pdf, "-o", out_str, "--angle", "90", "--json"],
            14,
        ),
        (
            vec!["extract-pages", pdf, "2-4", "-o", out_str, "--json"],
            3,
        ),
        (
            vec!["organize", pdf, "-o", out_str, "--order", "3,1", "--json"],
            2,
        ),
        (vec!["merge", pdf, pdf, "-o", out_str, "--json"], 28),
        (vec!["linearize", pdf, "-o", out_str, "--json"], 14),
    ] {
        let result = json(&run(&args));
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(result["output_sha256"], sha256_hex(&bytes), "{args:?}");
        assert_eq!(result["output_pages"], pages, "{args:?}");
        let reopened = wellfriendpdf_engine::ContentEngine::open_bytes(bytes).unwrap();
        assert_eq!(reopened.page_count().unwrap(), pages, "{args:?}");
    }
    let _ = std::fs::remove_file(&out);
}

#[test]
fn incremental_rotation_appends_to_the_unchanged_input() {
    let input = fixture("tracemonkey.pdf");
    let original = std::fs::read(&input).unwrap();
    let out = tmp("incremental.pdf");
    let result = json(&run(&[
        "rotate",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--angle",
        "90",
        "--relative",
        "--pages",
        "1,5",
        "--incremental",
        "--json",
    ]));
    let bytes = std::fs::read(&out).unwrap();
    assert_eq!(result["incremental"], true);
    assert_eq!(result["baseLength"], original.len() as u64);
    assert_eq!(result["deltaLength"], (bytes.len() - original.len()) as u64);
    assert_eq!(result["output_sha256"], sha256_hex(&bytes));
    assert!(bytes.starts_with(&original));
    let reopened = wellfriendpdf_engine::ContentEngine::open_bytes(bytes).unwrap();
    for page in 1..=14 {
        let expected = if page == 1 || page == 5 { 90 } else { 0 };
        assert_eq!(
            reopened.get_page(page).unwrap().rotate,
            expected,
            "page {page}"
        );
    }
    // Without the flag the command rewrites the file as before.
    let full = json(&run(&[
        "rotate",
        input.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--angle",
        "90",
        "--json",
    ]));
    assert_eq!(full["incremental"], false);
    let _ = std::fs::remove_file(&out);
}
