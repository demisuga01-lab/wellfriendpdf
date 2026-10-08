//! Thin benchmark adapter for SDK-only mutation surfaces that are not exposed by
//! the public CLI. It intentionally performs no benchmark aggregation: the
//! external controller owns timing, output validation, and cleanup.

use std::{env, fs, path::PathBuf};

use serde_json::json;
use wellfriendpdf_engine::{
    convert_to_pdfa_checked, edit_paragraph_reflow_pdf, incremental_metadata_update_pdf,
    ContentEngine, ParagraphEditOperation, ParagraphReflowOptions, PdfAProfile, PdfDocument,
};

fn argument(index: usize, label: &str) -> wellfriendpdf_engine::Result<String> {
    env::args().nth(index).ok_or_else(|| {
        wellfriendpdf_engine::WellfriendError::invalid_input(format!(
            "missing {label}; usage: benchmark_sdk_ops <metadata|pdfa-2b|paragraph-reflow> <input> <output> [arguments]"
        ))
    })
}

fn main() -> wellfriendpdf_engine::Result<()> {
    let operation = argument(1, "operation")?;
    let input_path = PathBuf::from(argument(2, "input")?);
    let output_path = PathBuf::from(argument(3, "output")?);
    let input = fs::read(&input_path)?;

    let report = match operation.as_str() {
        "metadata" => {
            let key = env::args().nth(4).unwrap_or_else(|| "Subject".to_string());
            let value = env::args()
                .nth(5)
                .unwrap_or_else(|| "Wellfriend benchmark metadata mutation".to_string());
            let (output, policy) = incremental_metadata_update_pdf(&input, &key, &value, false)?;
            fs::write(&output_path, &output)?;
            json!({
                "operation": "metadata",
                "input_bytes": input.len(),
                "output_bytes": output.len(),
                "key": key,
                "value": value,
                "original_prefix_preserved": output.starts_with(&input),
                "policy": policy,
            })
        }
        "pdfa-2b" => {
            let document = PdfDocument::open_bytes(input)?;
            let (output, conversion) = convert_to_pdfa_checked(&document, PdfAProfile::PdfA2B)?;
            fs::write(&output_path, &output)?;
            json!({
                "operation": "pdfa-2b",
                "output_bytes": output.len(),
                "conversion": conversion,
            })
        }
        "paragraph-reflow" => {
            let source_text = argument(4, "source text")?;
            let replacement_text = argument(5, "replacement text")?;
            let (output, editing) = edit_paragraph_reflow_pdf(
                input,
                &source_text,
                ParagraphEditOperation::Replace {
                    replacement: replacement_text.clone(),
                },
                ParagraphReflowOptions {
                    pages: vec![1],
                    max_edits: 1,
                    ..ParagraphReflowOptions::default()
                },
            )?;
            let reopened_text = ContentEngine::open_bytes(output.clone())?.get_page_text(1)?;
            fs::write(&output_path, &output)?;
            json!({
                "operation": "paragraph-reflow",
                "output_bytes": output.len(),
                "source_text": source_text,
                "replacement_text": replacement_text,
                "replacement_present_after_reopen": reopened_text.contains(&replacement_text),
                "editing": editing,
            })
        }
        other => {
            return Err(wellfriendpdf_engine::WellfriendError::invalid_input(
                format!("unsupported benchmark adapter operation: {other}"),
            ));
        }
    };

    println!("{report}");
    Ok(())
}
