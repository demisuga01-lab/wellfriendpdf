//! Cross-language SDK facade demo (Rust side).
//!
//! Runs the [`wellfriendpdf_engine::sdk`] report facade over a PDF and prints a compact
//! summary. This is the Rust counterpart of the Python `sdk_reports.py` and the
//! C `sdk_reports.c` examples — all three call the SAME facade and receive the
//! SAME versioned-JSON envelopes.
//!
//! ```sh
//! cargo run -p wellfriendpdf-engine --example sdk_reports -- input.pdf [out.json]
//! ```
//!
//! With a second argument, writes the raw report envelopes to that path as a
//! single JSON object (used to generate the Prompt-01 smoke artifact).

use std::path::PathBuf;

use wellfriendpdf_engine::sdk;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().unwrap_or_else(|| {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/multi_stream.pdf"
        )
        .to_string()
    });
    let out: Option<PathBuf> = args.next().map(PathBuf::from);

    let bytes = std::fs::read(&input)?;

    // One representative call per facade area. Each returns a versioned JSON
    // envelope; a real integration would parse the "report" field.
    let reports: Vec<(&str, String)> = vec![
        ("feature", sdk::feature_report_json()?),
        ("document_info", sdk::document_info_json(&bytes, None)?),
        ("security", sdk::security_report_json(&bytes, None)?),
        (
            "parser",
            sdk::parser_report_json(&bytes, Some("audit"), None)?,
        ),
        ("color", sdk::color_report_json(&bytes, Some("generic"))?),
        ("fonts", sdk::font_report_json(&bytes, None)?),
        ("signatures", sdk::signature_report_json(&bytes, None)?),
        ("forms", sdk::forms_report_json(&bytes, None)?),
        ("annotations", sdk::annotation_report_json(&bytes, None)?),
        ("pages", sdk::page_operations_report_json(&bytes, None)?),
        ("interactive", sdk::interactive_report_json(&bytes, None)?),
        (
            "standards",
            sdk::standards_profile_json(&bytes, Some("all"), None)?,
        ),
        (
            "pdfa",
            sdk::pdfa_validation_json(&bytes, Some("pdfa2b"), None)?,
        ),
        ("pdfua", sdk::pdfua_validation_json(&bytes, None)?),
        ("chunk", sdk::chunk_report_json(&bytes, None)?),
        ("text_semantic", sdk::text_semantic_json(&bytes, &[], None)?),
        (
            "decode_budget",
            sdk::decode_budget_report_json("DCTDecode", 4096, 4096, 3)?,
        ),
    ];

    println!("wellfriendpdf SDK facade — {input}");
    for (name, json) in &reports {
        let v: serde_json::Value = serde_json::from_str(json)?;
        println!(
            "  {name:<14} kind={} schema={}",
            v["kind"], v["schema_version"]
        );
    }

    if let Some(path) = out {
        let mut map = serde_json::Map::new();
        map.insert(
            "envelope_version".into(),
            serde_json::json!(sdk::REPORT_ENVELOPE_VERSION),
        );
        map.insert("source".into(), serde_json::json!(input));
        for (name, json) in reports {
            let v: serde_json::Value = serde_json::from_str(&json)?;
            map.insert(name.to_string(), v);
        }
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::Value::Object(map))?,
        )?;
        println!("wrote {}", path.display());
    }

    Ok(())
}
