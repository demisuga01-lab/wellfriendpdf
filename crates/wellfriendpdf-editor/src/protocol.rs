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
/// its page as page 1 (the one-page window the browser edited). `structure`
/// holds the page-structure operations (delete, move, insert blank, insert
/// pages) in the order they were made, numbered as the editor showed the
/// pages at the time; the `k`-th page they insert is window `insert: k`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowedList {
    pub version: u32,
    #[serde(default)]
    pub windows: Vec<WindowEdit>,
    #[serde(default)]
    pub structure: Vec<crate::ops::EditorOperation>,
}

/// One window's operations: a source page (`page`) or an inserted page
/// (`insert`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct WindowEdit {
    #[serde(default)]
    pub page: u32,
    #[serde(default)]
    pub insert: Option<u32>,
    pub operations: Vec<crate::ops::EditorOperation>,
}

impl WindowedList {
    pub fn validate(&self) -> Result<(), EditorError> {
        let invalid = |m: &str| EditorError::new("EDITOR_OPERATION_INVALID", m);
        if self.version != 1 || self.windows.len() > 200 || self.structure.len() > crate::ops::MAX_OPERATIONS {
            return Err(invalid("a windowed list has version 1, at most 200 windows and 500 structure operations"));
        }
        if self.windows.is_empty() && self.structure.is_empty() {
            return Err(invalid("a windowed list edits at least one window or the page structure"));
        }
        let mut keys = std::collections::BTreeSet::new();
        let mut total = self.structure.len();
        for window in &self.windows {
            let key = match (window.page, window.insert) {
                (0, Some(insert)) => (1, insert),
                (page, None) if page > 0 => (0, page),
                _ => return Err(invalid("each window names a page or an inserted page")),
            };
            if !keys.insert(key) || window.operations.is_empty() {
                return Err(invalid("each window names a distinct page and has operations"));
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
            }
        }
        if !self.structure.is_empty() {
            OperationList {
                version: 1,
                operations: self.structure.clone(),
            }
            .validate()?;
            if self.structure.iter().any(|operation| !operation.restructures_pages()) {
                return Err(invalid("structure operations delete, move or insert pages"));
            }
        }
        if total > crate::ops::MAX_OPERATIONS {
            return Err(invalid("at most 500 operations per save"));
        }
        Ok(())
    }

    /// Whether a window redacts: the save is then a full rewrite.
    pub fn redacts(&self) -> bool {
        self.windows
            .iter()
            .any(|w| w.operations.iter().any(|o| matches!(o, crate::ops::EditorOperation::Redact { .. })))
    }
}

/// The saved page order and the fresh window of every inserted page,
/// replayed from the structure operations.
fn replay_structure(
    source_pages: usize,
    structure: &[crate::ops::EditorOperation],
) -> Result<(Vec<crate::windows::Slot>, Vec<Vec<u8>>), EditorError> {
    use crate::ops::EditorOperation as Op;
    use crate::windows::Slot;
    let missing = |page: u32| EditorError::new("EDITOR_PAGE_NOT_FOUND", format!("page {page} does not exist"));
    let mut order: Vec<Slot> = (1..=source_pages as u32).map(Slot::Source).collect();
    let mut inserts: Vec<Vec<u8>> = Vec::new();
    for operation in structure {
        match operation {
            Op::DeletePage { page } => {
                if *page == 0 || *page as usize > order.len() {
                    return Err(missing(*page));
                }
                if order.len() == 1 {
                    return Err(EditorError::new("EDITOR_PAGES_REFUSED", "a PDF must keep at least one page"));
                }
                order.remove(*page as usize - 1);
            }
            Op::MovePage { page, to } => {
                for p in [*page, *to] {
                    if p == 0 || p as usize > order.len() {
                        return Err(missing(p));
                    }
                }
                let slot = order.remove(*page as usize - 1);
                order.insert(*to as usize - 1, slot);
            }
            Op::InsertBlankPage { after, width, height } => {
                if *after as usize > order.len() {
                    return Err(missing(*after));
                }
                order.insert(*after as usize, Slot::Insert(inserts.len()));
                inserts.push(crate::windows::blank_window(*width, *height)?);
            }
            Op::InsertPages { after, pdf, pages } => {
                if *after as usize > order.len() {
                    return Err(missing(*after));
                }
                let windows = crate::windows::inserted_windows(&crate::ops::decode_pdf(pdf)?, pages)?;
                for (offset, window) in windows.into_iter().enumerate() {
                    order.insert(*after as usize + offset, Slot::Insert(inserts.len()));
                    inserts.push(window);
                }
            }
            _ => return Err(EditorError::new("EDITOR_OPERATION_INVALID", "structure operations delete, move or insert pages")),
        }
    }
    Ok((order, inserts))
}

/// The text a redaction removes from a window's page 1: each run of
/// consecutive glyphs under the area.
fn redacted_runs(window: &[u8], rect: &crate::ops::Rect) -> Vec<String> {
    let Ok(layout) = wellfriendpdf_engine::PageTextLayout::from_bytes(window, 1) else { return Vec::new() };
    let area = [rect.x, rect.y, rect.x + rect.width, rect.y + rect.height];
    let mut runs = Vec::new();
    let mut run = String::new();
    for glyph in &layout.glyphs {
        let inside = glyph.bbox[0] < area[2] && glyph.bbox[2] > area[0] && glyph.bbox[1] < area[3] && glyph.bbox[3] > area[1];
        if inside {
            run.push_str(&glyph.text);
        } else if !run.trim().is_empty() {
            runs.push(std::mem::take(&mut run));
        } else {
            run.clear();
        }
    }
    if !run.trim().is_empty() {
        runs.push(run);
    }
    runs
}

/// A windowed save, ready to write.
pub struct WindowedSave {
    pub plan: crate::windows::Plan,
    pub scrub: crate::windows::Scrub,
    /// A redaction: written as a full rewrite.
    pub rewrite: bool,
    pub signed: bool,
    pub operation_count: usize,
    pub reports: Vec<OperationReport>,
}

/// Replay page-scoped edits on freshly extracted windows (and the inserted
/// pages' windows) and plan the save.
pub fn plan_windows(
    engine: &wellfriendpdf_engine::ContentEngine,
    list: &WindowedList,
    font_dir: PathBuf,
    allow_signature_invalidation: bool,
) -> Result<WindowedSave, EditorError> {
    use crate::ops::EditorOperation;
    list.validate()?;
    let signed = crate::windows::signed_lazily(engine);
    if signed && !allow_signature_invalidation {
        return Err(EditorError::new(
            "EDITOR_SIGNATURE_CONFIRMATION_REQUIRED",
            "this PDF is signed; confirm that the edit invalidates the signature",
        ));
    }
    let fonts = DirectoryFonts {
        directory: font_dir,
    };
    crate::apply::host_independent();
    let (order, mut inserts) = replay_structure(engine.page_count()?, &list.structure)?;
    let mut edited = Vec::new();
    let mut reports = Vec::new();
    let mut redacted = std::collections::BTreeSet::new();
    let mut runs = Vec::new();
    for window in &list.windows {
        let source = match window.insert {
            Some(insert) => inserts
                .get(insert as usize)
                .cloned()
                .ok_or_else(|| EditorError::new("EDITOR_OPERATION_INVALID", format!("no page {insert} was inserted")))?,
            None => crate::windows::extract(engine, window.page)?,
        };
        let mut bytes = source.clone();
        for operation in &window.operations {
            if let EditorOperation::Redact { rect, .. } = operation {
                runs.extend(redacted_runs(&bytes, rect));
                if window.insert.is_none() {
                    redacted.insert(window.page);
                }
            }
            let (next, mut report) = crate::apply::apply_operation(&bytes, operation, &fonts, allow_signature_invalidation)
                .map_err(|error| error.at(reports.len()))?;
            report.index = reports.len();
            report.page = window.page;
            reports.push(report);
            bytes = next;
        }
        match window.insert {
            Some(insert) => inserts[insert as usize] = bytes,
            None => edited.push(crate::windows::EditedWindow {
                page: window.page,
                before: source,
                after: bytes,
            }),
        }
    }
    for operation in &list.structure {
        reports.push(OperationReport {
            index: reports.len(),
            op: operation.name().to_owned(),
            page: operation.page(),
            ..OperationReport::default()
        });
    }
    let identity = order.iter().enumerate().all(|(i, s)| *s == crate::windows::Slot::Source(i as u32 + 1))
        && order.len() == engine.page_count()?;
    let plan = crate::windows::plan(
        engine,
        &crate::windows::Save {
            windows: &edited,
            inserts: &inserts,
            order: (!identity).then_some(order.as_slice()),
            redacted: &redacted,
        },
    )?;
    let rewrite = list.redacts();
    Ok(WindowedSave {
        plan,
        scrub: if rewrite { crate::windows::Scrub::new(runs) } else { crate::windows::Scrub::default() },
        rewrite,
        signed,
        operation_count: reports.len(),
        reports,
    })
}

/// A writer that hashes and counts what passes through it.
struct Hashing<W: Write> {
    inner: W,
    hasher: Sha256,
    written: u64,
}

impl<W: Write> Write for Hashing<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.written += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Write a planned save: the original followed by the update, or (for a
/// redaction) the full rewrite. `original` copies the source bytes into the
/// writer it is given. Returns the bytes written and their SHA-256.
fn write_windows(
    engine: &wellfriendpdf_engine::ContentEngine,
    save: &WindowedSave,
    base_len: usize,
    newline: bool,
    original: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
    out: impl Write,
) -> Result<(u64, String), EditorError> {
    let io = |e: std::io::Error| EditorError::new("EDITOR_IO_FAILED", format!("the output could not be written: {e}"));
    let mut out = Hashing {
        inner: out,
        hasher: Sha256::new(),
        written: 0,
    };
    if save.rewrite {
        save.plan.write_rewrite(engine.document().reader(), &save.scrub, &mut out)?;
    } else {
        original(&mut out).map_err(io)?;
        save.plan.write_update(base_len, newline, &mut out).map_err(io)?;
    }
    out.flush().map_err(io)?;
    Ok((out.written, format!("{:x}", out.hasher.finalize())))
}

fn windows_result(input_sha256: String, input_bytes: u64, output: (u64, String), save: &WindowedSave) -> ApplyResult {
    ApplyResult {
        operation: "APPLY_EDITOR_OPERATIONS",
        capability: "PDF_EDITOR_OPERATIONS",
        input_sha256,
        output_sha256: output.1,
        input_bytes,
        output_bytes: output.0,
        page_count: save.plan.page_count,
        operation_count: save.operation_count,
        save_mode: if save.rewrite { "FULL_REWRITE" } else { "INCREMENTAL" },
        signature_effect: if save.signed {
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
        reports: save.reports.clone(),
    }
}

fn check_page_count(reopened: &wellfriendpdf_engine::ContentEngine, expected: usize) -> Result<(), EditorError> {
    if reopened.page_count()? != expected {
        return Err(EditorError::new(
            "EDITOR_OUTPUT_INVALID",
            "the saved document does not have the expected pages",
        ));
    }
    Ok(())
}

/// Replay page-scoped edits on freshly extracted windows and save them: an
/// incremental update, or a full rewrite when a window redacts.
pub fn apply_windows(
    input: &[u8],
    list: &WindowedList,
    font_dir: PathBuf,
    allow_signature_invalidation: bool,
) -> Result<(Vec<u8>, ApplyResult), EditorError> {
    let engine = wellfriendpdf_engine::ContentEngine::open_bytes(input.to_vec())?;
    let save = plan_windows(&engine, list, font_dir, allow_signature_invalidation)?;
    let mut bytes = Vec::with_capacity(input.len() + (1 << 16));
    let output = write_windows(
        &engine,
        &save,
        input.len(),
        input.ends_with(b"\n"),
        |out| out.write_all(input),
        &mut bytes,
    )?;
    drop(engine);
    check_page_count(&wellfriendpdf_engine::ContentEngine::open_bytes(bytes.clone())?, save.plan.page_count)?;
    let result = windows_result(sha256(input), input.len() as u64, output, &save);
    Ok((bytes, result))
}

/// [`apply_windows`] from a file into a new file, never holding either in
/// memory: the source is read lazily and the output streamed.
pub fn apply_windows_path(
    input: &std::path::Path,
    output: &std::path::Path,
    list: &WindowedList,
    font_dir: PathBuf,
    allow_signature_invalidation: bool,
) -> Result<ApplyResult, EditorError> {
    let io = |message: &str| EditorError::new("EDITOR_IO_FAILED", message);
    let metadata = std::fs::metadata(input).map_err(|_| io("the input is unavailable"))?;
    let base_len = metadata.len() as usize;
    let (input_sha256, newline) = {
        let mut file = std::fs::File::open(input).map_err(|_| io("the input is unavailable"))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1 << 20];
        let mut last = 0u8;
        loop {
            let n = file.read(&mut buffer).map_err(|_| io("the input could not be read"))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            last = buffer[n - 1];
        }
        (format!("{:x}", hasher.finalize()), last == b'\n')
    };
    let engine = wellfriendpdf_engine::ContentEngine::open_path(input)?;
    let save = plan_windows(&engine, list, font_dir, allow_signature_invalidation)?;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| io("the output could not be created"))?;
    let mut writer = std::io::BufWriter::with_capacity(1 << 22, file);
    let written = write_windows(
        &engine,
        &save,
        base_len,
        newline,
        |out| {
            let mut source = std::fs::File::open(input)?;
            std::io::copy(&mut source, out).map(|_| ())
        },
        &mut writer,
    )?;
    let file = writer.into_inner().map_err(|_| io("the output could not be written"))?;
    file.sync_all().map_err(|_| io("the output could not be written"))?;
    drop(file);
    drop(engine);
    check_page_count(&wellfriendpdf_engine::ContentEngine::open_path(output)?, save.plan.page_count)?;
    Ok(windows_result(input_sha256, base_len as u64, written, &save))
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
        let metadata = std::fs::metadata(path).map_err(|_| io("the windowed list is unavailable"))?;
        if metadata.len() > MAX_COMMAND_BYTES {
            return Err(EditorError::new("EDITOR_COMMAND_TOO_LARGE", "the windowed list exceeds 40 MiB"));
        }
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
        let applied = apply_windows_path(
            &command.input_path,
            &command.output_path,
            &list,
            command.font_dir,
            command.allow_signature_invalidation,
        );
        if applied.is_err() {
            let _ = std::fs::remove_file(&command.output_path);
        }
        return applied;
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
