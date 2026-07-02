# Phase 3 Cross-Surface API Audit

This audit was written before adding the Phase 3 utilities. It records the
state of the structural/document operations as exposed by the four supported
surfaces: Rust, CLI, Python, and the C ABI.

Status legend:

- Present: exposed under a documented, usable API.
- Partial: present but narrower than another surface, or missing a convenience
  shape expected by the surface.
- Missing: no public surface entry point found.
- N/A: not generally exposed on that surface because it requires provider or
  key material outside Oxide's self-hosted contract.

| Operation | Rust API | CLI | Python | C ABI | Notes / Phase 3 action |
| --- | --- | --- | --- | --- | --- |
| merge | Present: `build_merged` | Present: `merge` | Missing | Missing | Add thin Python/C wrappers. |
| split | Present: `extract_single_page` / page subset writer | Present: `split` | Missing | Missing | Add thin Python/C wrappers for page extraction; CLI already covers split pattern. |
| extract-pages | Present: `ContentEngine::extract_pages` | Present: `extract-pages` | Missing | Missing | Add ordered subset wrappers; this also backs organize. |
| rotate | Present: `rotate_pages` | Present: `rotate` | Missing | Missing | Add thin wrappers. |
| repair | Present: `repair` | Present: `repair` | Missing | Missing | Add thin wrappers. |
| encrypt / protect | Present: `encrypt` | Present: `encrypt` | Missing | Missing | Keep CLI name `encrypt`; Python adds `encrypt_pdf`. No silent rename. |
| decrypt / unlock | Present implicitly through password-aware open and unencrypted rewrites | Partial: no dedicated `decrypt`/`unlock` command | Missing | Missing | Add `decrypt_pdf` wrappers that open with a password and write a normalized unencrypted copy; CLI gets `decrypt` alias. |
| sign | Present: `ContentEngine::sign` | Missing | Missing | Missing | Signing requires certificate/private-key material; leave outside this utility pass and document as Rust-only for now. |
| verify-sig | Present: `verify_signatures` | Present: `verify-sig` | Missing | Missing | Add JSON wrappers where cheap; no signing material required. |
| linearize | Present: `linearize` | Present: `linearize` | Missing | Missing | Add thin wrappers. |
| extract-images | Present: image locator/extractor | Present: `extract-images` | Partial: per-page image objects only | Missing | Python has per-page `images`; C ABI missing. Leave ZIP-style batch extraction CLI/Rust for now. |
| detach | Present: attachment list/extract | Present: `detach` | Missing | Missing | Add read-only JSON list wrapper where cheap; extraction remains CLI/Rust. |
| optimize | Present: `optimize` | Present: `optimize` | Missing | Missing | Add thin wrappers. |
| to-html | Present | Present: `to-html` | Present: `Document.to_html()` | Missing | Add C ABI HTML string wrapper. |
| info | Present: `document_info` | Present: `info` | Present: `Document.metadata` | Present: `oxide_document_info_json` | Already API-clean. |
| fonts | Present: `list_fonts` | Present: `fonts` | Missing | Missing | Add JSON wrappers. |
| render page | Present: PNG and renderer pixel buffer | Present: `render` ZIP | Present: PNG bytes | Present: single-page PNG | Add file-export PDF-to-JPG/PNG utility using existing renderer. |

Vocabulary decisions:

- `encrypt` remains the canonical command/Rust term because it already exists.
  Documentation may mention "protect" as user-facing vocabulary, but the public
  API is not renamed.
- `decrypt` is added alongside password-aware open. It writes an unencrypted
  normalized copy, matching the common "unlock" workflow without introducing a
  second term as the primary API.
- `extract_pages` is the canonical Rust/Python/C operation for ordered page
  subsets. `organize` is the higher-level reorder/delete/duplicate/insert
  workflow built on the same page-copy writer.
