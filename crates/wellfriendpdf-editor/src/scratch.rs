//! Runs a path-based `wellfriendpdf-edit` request on bytes: a private
//! temporary directory natively, the crate's in-memory files on wasm32.

use std::path::Path;

use wellfriend_pdf_edit::PdfEditError;

use crate::error::EditorError;

pub struct Paths<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub extra: Option<&'a Path>,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run<T>(
    input: &[u8],
    extra: Option<&[u8]>,
    edit: impl FnOnce(Paths<'_>) -> Result<T, PdfEditError>,
) -> Result<(T, Vec<u8>), EditorError> {
    let io = |e: std::io::Error| EditorError::new("EDITOR_SCRATCH_FAILED", e.to_string());
    let directory = tempfile::Builder::new()
        .prefix("wellfriendpdf-editor-")
        .tempdir()
        .map_err(io)?;
    let input_path = directory.path().join("input.pdf");
    let output_path = directory.path().join("output.pdf");
    let extra_path = directory.path().join("extra.bin");
    std::fs::write(&input_path, input).map_err(io)?;
    if let Some(extra) = extra {
        std::fs::write(&extra_path, extra).map_err(io)?;
    }
    let result = edit(Paths {
        input: &input_path,
        output: &output_path,
        extra: extra.map(|_| extra_path.as_path()),
    })
    .map_err(EditorError::edit)?;
    let output = std::fs::read(&output_path).map_err(io)?;
    Ok((result, output))
}

#[cfg(target_arch = "wasm32")]
pub fn run<T>(
    input: &[u8],
    extra: Option<&[u8]>,
    edit: impl FnOnce(Paths<'_>) -> Result<T, PdfEditError>,
) -> Result<(T, Vec<u8>), EditorError> {
    use wellfriend_pdf_edit::vfs;
    let input_path = Path::new("/wellfriendpdf-editor/input.pdf");
    let output_path = Path::new("/wellfriendpdf-editor/output.pdf");
    let extra_path = Path::new("/wellfriendpdf-editor/extra.bin");
    vfs::clear();
    vfs::put(input_path, input.to_vec());
    if let Some(extra) = extra {
        vfs::put(extra_path, extra.to_vec());
    }
    let result = edit(Paths {
        input: input_path,
        output: output_path,
        extra: extra.map(|_| extra_path),
    });
    let output = vfs::take(output_path);
    vfs::clear();
    let result = result.map_err(EditorError::edit)?;
    let output = output
        .ok_or_else(|| EditorError::new("EDITOR_SCRATCH_FAILED", "the edit wrote no output"))?;
    Ok((result, output))
}

/// Inspection only (no output file).
pub fn inspect(
    input: &[u8],
    page: Option<u32>,
) -> Result<wellfriend_pdf_edit::PdfInspection, EditorError> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let io = |e: std::io::Error| EditorError::new("EDITOR_SCRATCH_FAILED", e.to_string());
        let directory = tempfile::Builder::new()
            .prefix("wellfriendpdf-editor-")
            .tempdir()
            .map_err(io)?;
        let input_path = directory.path().join("input.pdf");
        std::fs::write(&input_path, input).map_err(io)?;
        wellfriend_pdf_edit::inspect_pdf_for_page(&input_path, page).map_err(EditorError::edit)
    }
    #[cfg(target_arch = "wasm32")]
    {
        use wellfriend_pdf_edit::vfs;
        let input_path = Path::new("/wellfriendpdf-editor/input.pdf");
        vfs::clear();
        vfs::put(input_path, input.to_vec());
        let result = wellfriend_pdf_edit::inspect_pdf_for_page(input_path, page);
        vfs::clear();
        result.map_err(EditorError::edit)
    }
}
