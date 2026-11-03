//! `wellfriendpdf pdf-editor-apply`: one JSON command on stdin, one JSON
//! envelope on stdout (the `pdf-edit` envelope: `{status: "SUCCESS", result}`
//! or `{status: "ERROR", code, message, retryable}`), exit code 2 on refusal.
//!
//! ```json
//! { "input_path": "/scratch/source.pdf", "output_path": "/scratch/result.pdf",
//!   "font_dir": "/usr/share/wellfriend/editor-fonts",
//!   "operations_path": "/scratch/operations.json",
//!   "allow_signature_invalidation": false }
//! ```

use std::io::{Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::apply::{apply_operations, OperationReport};
use crate::error::EditorError;
use crate::fonts::DirectoryFonts;
use crate::ops::OperationList;

/// Operations carry inline images (24 MiB of image bytes at most, base64).
pub const MAX_COMMAND_BYTES: u64 = 40 * 1024 * 1024;
pub const MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Command {
    input_path: PathBuf,
    output_path: PathBuf,
    font_dir: PathBuf,
    /// The operation list inline, or (for lists with images, larger than a
    /// process argument should be) in a file.
    #[serde(default)]
    operations: Option<OperationList>,
    #[serde(default)]
    operations_path: Option<PathBuf>,
    #[serde(default)]
    allow_signature_invalidation: bool,
    /// `"windows"`: the operations file is a [`WindowedList`] of page-scoped
    /// edits, saved as an incremental update.
    #[serde(default)]
    mode: Option<String>,
}

/// Page-scoped edits of a large document: each window's operations address
/// its page as page 1 (the one-page window the browser edited).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowedList {
    pub version: u32,
    pub windows: Vec<WindowEdit>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowEdit {
    pub page: u32,
    pub operations: Vec<crate::ops::EditorOperation>,
}

impl WindowedList {
    pub fn validate(&self) -> Result<(), EditorError> {
        let invalid = |m: &str| EditorError::new("EDITOR_OPERATION_INVALID", m);
        if self.version != 1 || self.windows.is_empty() || self.windows.len() > 200 {
            return Err(invalid("a windowed list has version 1 and 1-200 windows"));
        }
        let mut pages = std::collections::BTreeSet::new();
        let mut total = 0;
        for window in &self.windows {
            if window.page == 0 || !pages.insert(window.page) {
                return Err(invalid("each window names a distinct page"));
            }
            total += window.operations.len();
            let list = OperationList {
                version: 1,
                operations: window.operations.clone(),
            };
            list.validate()?;
            for operation in &window.operations {
                if operation.restructures_pages() || operation.page() > 1 {
                    return Err(invalid("window operations edit their own page (page 1) only"));
                }
                if matches!(operation, crate::ops::EditorOperation::Redact { .. }) {
                    return Err(EditorError::new(
                        "EDITOR_REDACTION_NEEDS_REWRITE",
                        "redaction must remove content from every revision; save it with a full rewrite",
                    ));
                }
            }
        }
        if total > crate::ops::MAX_OPERATIONS {
            return Err(invalid("at most 500 operations per save"));
        }
        Ok(())
    }
}

/// Replay page-scoped edits on freshly extracted windows and splice the
/// pages back as one incremental update.
pub fn apply_windows(
    input: &[u8],
    list: &WindowedList,
    font_dir: PathBuf,
    allow_signature_invalidation: bool,
) -> Result<(Vec<u8>, ApplyResult), EditorError> {
    list.validate()?;
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(input.to_vec())?;
    let signed = crate::windows::signed_lazily(&engine);
    if signed && !allow_signature_invalidation {
        return Err(EditorError::new(
            "EDITOR_SIGNATURE_CONFIRMATION_REQUIRED",
            "this PDF is signed; confirm that the edit invalidates the signature",
        ));
    }
    let fonts = DirectoryFonts {
        directory: font_dir,
    };
    let mut edited = Vec::new();
    let mut reports = Vec::new();
    let mut count = 0;
    for window in &list.windows {
        let source = crate::windows::extract(&engine, window.page)?;
        let operations = OperationList {
            version: 1,
            operations: window.operations.clone(),
        };
        let outcome = apply_operations(&source, &operations, &fonts, allow_signature_invalidation)?;
        count += window.operations.len();
        for mut report in outcome.reports {
            report.index = reports.len();
            report.page = window.page;
            reports.push(report);
        }
        edited.push(crate::windows::EditedWindow {
            page: window.page,
            before: source,
            after: outcome.bytes,
        });
    }
    let page_count = engine.page_count()?;
    drop(engine);
    let bytes = crate::windows::splice_incremental(input, &edited)?;
    let reopened = wellfriendpdf_engine::ContentEngine::open_bytes(bytes.clone())?;
    if reopened.page_count()? != page_count {
        return Err(EditorError::new(
            "EDITOR_OUTPUT_INVALID",
            "the incremental update changed the page count",
        ));
    }
    let result = ApplyResult {
        operation: "APPLY_EDITOR_OPERATIONS",
        capability: "PDF_EDITOR_OPERATIONS",
        input_sha256: sha256(input),
        output_sha256: sha256(&bytes),
        input_bytes: input.len() as u64,
        output_bytes: bytes.len() as u64,
        page_count,
        operation_count: count,
        save_mode: "INCREMENTAL",
        signature_effect: if signed {
            "INVALIDATES_EXISTING_SIGNATURES"
        } else {
            "NONE"
        },
        source_object_count: 0,
        output_object_count: 0,
        changed_objects: Vec::new(),
        validation: Validation {
            reparsed: true,
            operations_applied: true,
        },
        reports,
    };
    Ok((bytes, result))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowCommand {
    input_path: PathBuf,
    #[serde(default)]
    output_path: Option<PathBuf>,
    #[serde(default)]
    page: Option<u32>,
}

fn read_command<T: serde::de::DeserializeOwned>() -> Result<T, EditorError> {
    let mut payload = Vec::new();
    std::io::stdin()
        .take(64 * 1024)
        .read_to_end(&mut payload)
        .map_err(|_| EditorError::new("EDITOR_IO_FAILED", "the command could not be read"))?;
    serde_json::from_slice(&payload).map_err(|e| {
        EditorError::new(
            "EDITOR_OPERATION_INVALID",
            format!("the command is malformed: {e}"),
        )
    })
}

/// `pdf-editor-index`: page count and page sizes of a (large) document.
pub fn run_index_stdin() -> i32 {
    envelope(read_command::<WindowCommand>().and_then(|command| {
        let engine = wellfriendpdf_engine::ContentEngine::open_path(&command.input_path)?;
        Ok(serde_json::to_value(crate::windows::index(&engine)?).unwrap_or_default())
    }))
}

/// `pdf-editor-window`: write the one-page window of `page`.
pub fn run_window_stdin() -> i32 {
    envelope(read_command::<WindowCommand>().and_then(|command| {
        let page = command
            .page
            .ok_or_else(|| EditorError::new("EDITOR_OPERATION_INVALID", "page is required"))?;
        let output = command
            .output_path
            .ok_or_else(|| EditorError::new("EDITOR_OPERATION_INVALID", "output_path is required"))?;
        let engine = wellfriendpdf_engine::ContentEngine::open_path(&command.input_path)?;
        let bytes = crate::windows::extract(&engine, page)?;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)
            .and_then(|mut file| file.write_all(&bytes))
            .map_err(|_| EditorError::new("EDITOR_IO_FAILED", "the window could not be written"))?;
        Ok(serde_json::json!({ "page": page, "outputBytes": bytes.len(), "outputSha256": sha256(&bytes) }))
    }))
}

fn envelope(result: Result<serde_json::Value, EditorError>) -> i32 {
    match result {
        Ok(value) => {
            println!("{}", serde_json::json!({ "status": "SUCCESS", "result": value }));
            0
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({
                    "status": "ERROR",
                    "code": format!("PDF_{}", error.code),
                    "message": error.to_string().chars().take(1000).collect::<String>(),
                    "retryable": false,
                })
            );
            2
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub operation: &'static str,
    pub capability: &'static str,
    pub input_sha256: String,
    pub output_sha256: String,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub page_count: usize,
    pub operation_count: usize,
    pub save_mode: &'static str,
    pub signature_effect: &'static str,
    pub source_object_count: usize,
    pub output_object_count: usize,
    /// A full rewrite renumbers objects; the per-operation reports say
    /// what changed instead.
    pub changed_objects: Vec<serde_json::Value>,
    pub validation: Validation,
    pub reports: Vec<OperationReport>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Validation {
    pub reparsed: bool,
    pub operations_applied: bool,
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Whether the document carries a signature value dictionary.
pub fn has_signatures(bytes: &[u8]) -> bool {
    let Ok(document) = lopdf::Document::load_mem(bytes) else {
        return false;
    };
    document.objects.values().any(|object| match object {
        lopdf::Object::Dictionary(d) => d.has(b"ByteRange") && d.has(b"Contents"),
        _ => false,
    })
}

pub fn apply_command_bytes(
    input: &[u8],
    operations: &OperationList,
    font_dir: PathBuf,
    allow_signature_invalidation: bool,
) -> Result<(Vec<u8>, ApplyResult), EditorError> {
    let signed = has_signatures(input);
    if signed && !allow_signature_invalidation {
        return Err(EditorError::new(
            "EDITOR_SIGNATURE_CONFIRMATION_REQUIRED",
            "this PDF is signed; confirm that the edit invalidates the signature",
        ));
    }
    let fonts = DirectoryFonts {
        directory: font_dir,
    };
    let outcome = apply_operations(input, operations, &fonts, allow_signature_invalidation)?;
    let reopened = lopdf::Document::load_mem(&outcome.bytes).map_err(|e| {
        EditorError::new(
            "EDITOR_OUTPUT_INVALID",
            format!("the edited PDF does not reopen: {e}"),
        )
    })?;
    let page_count = reopened.get_pages().len();
    let output_object_count = reopened.objects.len();
    let source_object_count = lopdf::Document::load_mem(input)
        .map(|d| d.objects.len())
        .unwrap_or(0);
    let result = ApplyResult {
        operation: "APPLY_EDITOR_OPERATIONS",
        capability: "PDF_EDITOR_OPERATIONS",
        input_sha256: sha256(input),
        output_sha256: sha256(&outcome.bytes),
        input_bytes: input.len() as u64,
        output_bytes: outcome.bytes.len() as u64,
        page_count,
        operation_count: operations.operations.len(),
        save_mode: "FULL_REWRITE",
        signature_effect: if signed {
            "INVALIDATES_EXISTING_SIGNATURES"
        } else {
            "NONE"
        },
        source_object_count,
        output_object_count,
        changed_objects: Vec::new(),
        validation: Validation {
            reparsed: true,
            operations_applied: outcome.reports.len() == operations.operations.len(),
        },
        reports: outcome.reports,
    };
    Ok((outcome.bytes, result))
}

fn run() -> Result<ApplyResult, EditorError> {
    let io = |message: &str| EditorError::new("EDITOR_IO_FAILED", message);
    let mut payload = Vec::new();
    std::io::stdin()
        .take(MAX_COMMAND_BYTES + 1)
        .read_to_end(&mut payload)
        .map_err(|_| io("the command could not be read"))?;
    if payload.len() as u64 > MAX_COMMAND_BYTES {
        return Err(EditorError::new(
            "EDITOR_COMMAND_TOO_LARGE",
            "the command exceeds 40 MiB",
        ));
    }
    let command: Command = serde_json::from_slice(&payload).map_err(|e| {
        EditorError::new(
            "EDITOR_OPERATION_INVALID",
            format!("the command is malformed: {e}"),
        )
    })?;
    if command.mode.as_deref() == Some("windows") {
        let path = command
            .operations_path
            .as_ref()
            .ok_or_else(|| io("the windowed list is unavailable"))?;
        let text = std::fs::read_to_string(path).map_err(|_| io("the windowed list is unavailable"))?;
        let list: WindowedList = serde_json::from_str(&text).map_err(|e| {
            EditorError::new(
                "EDITOR_OPERATION_INVALID",
                format!("the windowed list is malformed: {e}"),
            )
        })?;
        if command.output_path.exists() {
            return Err(io("the output already exists"));
        }
        let input = std::fs::read(&command.input_path).map_err(|_| io("the input is unavailable"))?;
        let (bytes, result) = apply_windows(
            &input,
            &list,
            command.font_dir,
            command.allow_signature_invalidation,
        )?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&command.output_path)
            .map_err(|_| io("the output could not be created"))?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| io("the output could not be written"))?;
        return Ok(result);
    }
    let operations = match (command.operations, &command.operations_path) {
        (Some(operations), None) => operations,
        (None, Some(path)) => {
            let metadata =
                std::fs::metadata(path).map_err(|_| io("the operation list is unavailable"))?;
            if !metadata.is_file() || metadata.len() > MAX_COMMAND_BYTES {
                return Err(EditorError::new(
                    "EDITOR_COMMAND_TOO_LARGE",
                    "the operation list exceeds 40 MiB",
                ));
            }
            let text = std::fs::read_to_string(path)
                .map_err(|_| io("the operation list is unavailable"))?;
            OperationList::parse(&text)?
        }
        _ => {
            return Err(EditorError::new(
                "EDITOR_OPERATION_INVALID",
                "give exactly one of operations and operations_path",
            ))
        }
    };
    operations.validate()?;
    let metadata =
        std::fs::metadata(&command.input_path).map_err(|_| io("the input is unavailable"))?;
    if !metadata.is_file() || metadata.len() > MAX_INPUT_BYTES {
        return Err(io("the input is unavailable or too large"));
    }
    if command.output_path.exists() {
        return Err(io("the output already exists"));
    }
    let input = std::fs::read(&command.input_path).map_err(|_| io("the input is unavailable"))?;
    let (bytes, result) = apply_command_bytes(
        &input,
        &operations,
        command.font_dir,
        command.allow_signature_invalidation,
    )?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&command.output_path)
        .map_err(|_| io("the output could not be created"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| io("the output could not be written"))?;
    Ok(result)
}

/// Returns the process exit code: 0 on success, 2 on a refused edit.
pub fn run_stdin() -> i32 {
    match run() {
        Ok(result) => {
            println!(
                "{}",
                serde_json::to_string(
                    &serde_json::json!({ "status": "SUCCESS", "result": result })
                )
                .expect("result envelope serializes")
            );
            0
        }
        Err(error) => {
            println!(
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "status": "ERROR",
                    // The platform's engine contract admits `PDF_*` codes.
                    "code": format!("PDF_{}", error.code),
                    "message": error.to_string().chars().take(1000).collect::<String>(),
                    "retryable": false,
                }))
                .expect("failure envelope serializes")
            );
            2
        }
    }
}
