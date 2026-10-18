# Changelog

All notable changes to Wellfriend are recorded here. The project follows the
stability policy in `docs/stability.md`.

## Unreleased

- Added Rust-native conversions to PDF: `text-to-pdf`, `markdown-to-pdf`
  (CommonMark + GFM), `csv-to-pdf` (repeated headers, column cap error instead
  of truncation), `json-to-pdf` and `svg-to-pdf` (vector, via `svg2pdf`), on
  the CLI, HTTP server and C API.
- Added `pdf-to-epub` (EPUB 3 with navigation, chapters and images) and
  `edit-metadata` (Info and XMP kept in sync, batch mode, incremental update
  that keeps existing signatures valid).
- Added native PDF/A conversion (`pdfa-convert`: PDF/A-1b, 2b, 2u, 3b, 3u)
  with font substitution and subsetting that preserves text positions, an
  OutputIntent, XMP, forbidden-feature removal, and a gate that returns a file
  only when the built-in PDF/A validator passes it. `pdfa-validate` gains a
  JSON `--input/--options` form; the old form keeps working.
- The CLI conversion commands print one JSON result line
  (`{"ok":true,"pages","bytes"}` or `{"ok":false,"code","message"}`).
- Windows builds no longer need the `sha2` assembly feature; it stays enabled
  on other targets. The Tesseract crate is no longer a default workspace member.
- Added the main CI gate, scheduled/manual sanitizer gate, and tag-driven
  release pipeline with checksummed CLI artifacts and crate publish dry-runs.
- Added the documented public API overview and stability policy.
- Added `ErrorKind` plus `WellfriendError::kind()`, `WellfriendError::code()`, and
  `WellfriendError::is_input_error()` for programmatic error handling.
- Added RSA/SHA-256 PDF signing through incremental update and the
  `sign_document` example.
