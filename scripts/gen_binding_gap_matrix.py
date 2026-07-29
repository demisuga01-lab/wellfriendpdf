#!/usr/bin/env python3
"""Generate the roadmap closure 01 binding gap matrix (JSON + Markdown).

Single source of truth for the feature-to-surface status matrix. Edit the
`FEATURES` table below and re-run to regenerate both artifacts:

    python scripts/gen_binding_gap_matrix.py

Outputs:
    target/binding_surface-binding-core/binding-gap-matrix.json
    docs/bindings_binding_surface_gap_matrix.md

Status vocabulary (consumed by later automation):
    implemented_public     public, documented, tested, stable enough
    partial_public         public but incomplete / undocumented / untested
    implemented_internal   engine supports it; not exposed through this surface
    cli_only               behavior exists via CLI, not the library/binding
    unsupported_reported   unsupported but honestly reported via diagnostics
    missing                no implementation and no honest reporting
    deferred               deliberately out of Binding Surface (names later roadmap task)
    blocked                cannot expose safely without a refactor/decision
"""

import json
import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
JSON_OUT = ROOT / "target" / "binding_surface-binding-core" / "binding-gap-matrix.json"
MD_OUT = ROOT / "docs" / "bindings_binding_surface_gap_matrix.md"

# Shorthand statuses.
PUB = "implemented_public"
PART = "partial_public"
INT = "implemented_internal"
CLI = "cli_only"
UNSUP = "unsupported_reported"
MISS = "missing"
DEF = "deferred"
BLK = "blocked"

# Common test/example references used across many rows.
T_SDK = "engine sdk::tests (crates/engine/src/sdk.rs)"
T_PY = "wellfriendpdf-py tests/test_reports.py"
T_C = "wellfriendpdf-capi capi_* tests (crates/wellfriendpdf-capi/src/lib.rs)"
T_PARITY = "cross-surface parity: rust/python/c-abi smoke JSON compared equal"

# Each feature: (id, category, name, rust, python, capi, cli, note)
# rust/python/capi/cli are status strings. `note` gives action or evidence.
FEATURES = [
    # ── Opening, parser, COS, xref, repair ──────────────────────────────────
    ("open.options", "parser", "document open options", PUB, PUB, PUB, PUB,
     f"ContentEngine::open_bytes[/with_password]; sdk::open; py Document(...); capi open_from_bytes. {T_PY}"),
    ("open.byte_source", "parser", "byte-source abstraction", PUB, PUB, PART, PUB,
     "Rust open_bytes/open_path; py bytes+path; capi bytes only (file input via caller read). Action: capi memory-only is intentional; path input is a caller concern."),
    ("parser.xref", "parser", "xref table and xref stream access", PART, PUB, PUB, PUB,
     f"Surfaced via parser_report (linearization/source_metrics/xref recovery). sdk::parser_report_json; {T_PY}; {T_C}. Raw xref entry walking stays Rust-only (reader::XrefEntry)."),
    ("parser.trailer_id", "parser", "trailer and document ID reporting", PUB, PUB, PUB, PUB,
     "document_info (ids/producer) + parser_report. sdk::document_info_json/parser_report_json."),
    ("parser.revisions", "parser", "incremental revision enumeration", PUB, PUB, PUB, PUB,
     "parser_report.revision_history. sdk::parser_report_json."),
    ("parser.object_lookup", "parser", "object lookup and typed object access", INT, MISS, MISS, INT,
     "reader::get_object / PdfObject at Rust root only. Action: deferred to a later low-level-access prompt; not exposed to bindings by design (unstable object model)."),
    ("parser.page_tree", "parser", "page tree traversal", PUB, PUB, PUB, PUB,
     "page_count + page access; document_info page_sizes. py Document len/iter/page; capi page_count."),
    ("parser.stream_offsets", "parser", "stream length and offset diagnostics", PART, PART, PART, PUB,
     "parser_report source_metrics + decode budget report. Action: per-stream offset table stays Rust-only."),
    ("parser.repair", "parser", "repair-mode diagnostics", PUB, PUB, PUB, PUB,
     f"parser_report mode=repair/audit repair_summary. {T_C} capi_parametrized_reports."),
    ("parser.linearization", "parser", "linearization status", PUB, PUB, PUB, PUB,
     "parser_report.linearization. sdk::parser_report_json."),
    ("parser.encryption_status", "parser", "encryption status discovery", PUB, PUB, PUB, PUB,
     "security_report.encrypted/encryption + document_info. sdk::security_report_json."),
    ("parser.object_cycle", "parser", "object cycle detection", PART, PART, PART, PART,
     "Reported via parser_report diagnostics when hit; no standalone report. Action: dedicated cycle report deferred."),
    ("parser.malformed_recovery", "parser", "malformed object recovery", PUB, PUB, PUB, PUB,
     "parser_report repair mode reports recovered/failed objects."),
    ("parser.arlington", "parser", "Arlington validation hooks", PUB, PUB, PUB, PUB,
     "parser_report.arlington + standards_profile arlington_status. sdk::standards_profile_json."),
    ("parser.memory_limits", "parser", "parser memory-limit reporting", PART, PART, PART, PUB,
     "decode budget report surfaces limits; parser open honors engine limits. Action: explicit parser memory budget option not yet a binding param (partial)."),

    # ── Decode, filters, images, low-level safety ───────────────────────────
    ("decode.filter_chain", "decode", "filter-chain diagnostics", PART, PART, PART, PUB,
     "parser_report decode section (include_decode) + decode budget report. sdk::decode_budget_report_json."),
    ("decode.flate_predictor", "decode", "Flate and predictor decode limits", PUB, PUB, PUB, PUB,
     "decode budget report + DecodeLimits (MAX_FLATE_DECOMPRESSED_BYTES)."),
    ("decode.dct", "decode", "DCT image decode reporting", PUB, PUB, PUB, PUB,
     "decode_budget_report(filter='DCTDecode',...). sdk::decode_budget_report_json."),
    ("decode.jpx", "decode", "JPX image decode reporting", PUB, PUB, PUB, PUB,
     "decode_budget_report(filter='JPXDecode',...)."),
    ("decode.jbig2", "decode", "JBIG2 safety reporting", PUB, PUB, PUB, PUB,
     "decode_budget_report(filter='JBIG2Decode',...); risky policy in security_report."),
    ("decode.ccitt", "decode", "CCITT decode reporting", PUB, PUB, PUB, PUB,
     "decode_budget_report(filter='CCITTFaxDecode',...)."),
    ("decode.image_inventory", "decode", "image inventory extraction", PUB, PUB, INT, PUB,
     "engine.find_all_images; py Page.images; Rust root ImageLocator. Action: capi image inventory JSON deferred (bytes-heavy)."),
    ("decode.cancellation", "decode", "stream decode cancellation", INT, UNSUP, UNSUP, PART,
     "engine CancelToken exists; bindings do not yet pass a cancel token. Action: cancel-token binding param deferred; reported as unsupported in feature notes."),
    ("decode.cache", "decode", "decode cache status", INT, MISS, MISS, INT,
     "DecodeCache/DecodeCacheMetrics at Rust root only. Action: metrics report deferred."),
    ("decode.scheduler", "decode", "decode scheduler limits", INT, MISS, MISS, INT,
     "DecodeMemoryBudget/DecodeSchedulerMetrics at Rust root only. Action: deferred."),
    ("decode.bomb", "decode", "decompression bomb detection", PUB, PUB, PUB, PUB,
     "decode_budget_report exceeds-limit diagnostics; security_report findings."),
    ("decode.sandbox_policy", "decode", "sandbox policy reporting", PART, PART, PART, PART,
     "codec sandboxing documented; surfaced via decode diagnostics. Action: standalone policy report deferred."),
    ("decode.raw_vs_decoded", "decode", "raw stream versus decoded stream access", INT, MISS, MISS, PART,
     "filters::decode_stream[_lossless] at Rust root. Action: raw/decoded stream fetch not exposed to bindings (unstable object handles)."),
    ("decode.unsupported_filter", "decode", "unsupported filter diagnostics", PUB, PUB, PUB, PUB,
     "DecodeReport diagnostics + WellfriendError::UnsupportedFeature; honest reporting."),
    ("decode.perf_counters", "decode", "decode performance counters", PART, PART, PART, PART,
     "DecodeMetrics inside decode reports. Action: full perf-counter export deferred."),

    # ── Rendering ───────────────────────────────────────────────────────────
    ("render.raster", "render", "page raster rendering", PUB, PUB, PUB, PUB,
     "render_page_png_fast/jpeg. py Document/Page.render; capi render_page_png/jpeg."),
    ("render.display_list", "render", "display-list extraction", INT, MISS, MISS, PART,
     "render::DisplayList at Rust root. Action: display-list JSON export deferred (large/unstable)."),
    ("render.options", "render", "render options", PART, PART, PART, PUB,
     "DPI/format exposed; full RenderQuality/RenderMode subset. Action: extended render options deferred."),
    ("render.dpi_scale", "render", "DPI and scale handling", PUB, PUB, PUB, PUB,
     "dpi param on all render entry points."),
    ("render.tile", "render", "tile rendering", INT, MISS, MISS, INT,
     "render::RenderTile at Rust root. Action: deferred to a render-binding roadmap task."),
    ("render.band", "render", "band rendering", INT, MISS, MISS, INT,
     "renderer band path Rust-only. Action: deferred."),
    ("render.progressive", "render", "progressive rendering state", INT, MISS, MISS, INT,
     "Rust-only. Action: deferred."),
    ("render.cancellation", "render", "render cancellation", INT, UNSUP, UNSUP, PART,
     "CancelToken exists in engine; not a binding param yet. Action: deferred."),
    ("render.annot_appearance", "render", "annotation appearance rendering", PART, PART, PART, PUB,
     "annotation_report appearance status; render includes annots. sdk::annotation_report_json."),
    ("render.optional_content", "render", "optional-content visibility reporting", INT, MISS, MISS, PART,
     "OCG handling in renderer Rust-only. Action: OCG report deferred."),
    ("render.diagnostics", "render", "render diagnostics", PART, PART, PART, PART,
     "UnsupportedRenderOp/DisplayListStats at Rust root. Action: render diagnostics JSON deferred."),
    ("render.visual_hash", "render", "visual hash reporting", INT, MISS, MISS, PART,
     "versioning simhash / render compare Rust-only. Action: deferred."),
    ("render.memory_budget", "render", "render memory budget", PUB, PART, PART, PUB,
     "max_render_pixels/DEFAULT_MAX_RENDER_PIXELS. Action: per-call render budget param deferred for bindings."),
    ("render.color_managed", "render", "color-managed render options", INT, MISS, MISS, PART,
     "color-managed render Rust-only; color_report exposes color state. Action: deferred."),
    ("render.image_output_encoding", "render", "image output encoding", PUB, PUB, PUB, PUB,
     "png/jpeg output selection on render entry points."),

    # ── Fonts, glyphs, text ─────────────────────────────────────────────────
    ("fonts.inventory", "fonts", "font inventory", PUB, PUB, PUB, PUB,
     "list_fonts. sdk::font_report_json; py font_report; capi fonts_json."),
    ("fonts.substitution", "fonts", "font substitution diagnostics", PART, PART, PART, PUB,
     "FontInfo embedded flag + substitution notes. Action: dedicated substitution report deferred."),
    ("fonts.type0_cid", "fonts", "Type0 and CID font reporting", PUB, PUB, PUB, PUB,
     "FontInfo type/subtype in font_report."),
    ("fonts.cmap", "fonts", "CMap diagnostics", PART, PART, PART, PART,
     "Surfaced via font_report + text provenance. Action: standalone CMap report deferred."),
    ("fonts.glyph_positioning", "fonts", "glyph positioning data", INT, PART, PART, PART,
     "text_semantic spans carry geometry. Rust ShapedGlyph deeper. Action: raw glyph positions deferred."),
    ("text.spans", "text", "text extraction spans", PUB, PUB, PUB, PUB,
     "text_semantic model + words. sdk::text_semantic_json; py text_semantic/words."),
    ("text.char_provenance", "text", "char-level provenance", PUB, PUB, PUB, PUB,
     "text_semantic chars + provenance flags."),
    ("text.word_line_grouping", "text", "word and line grouping", PUB, PUB, PUB, PUB,
     "text_semantic words/lines; extract_page_words."),
    ("text.cjk_segmentation", "text", "CJK segmentation status", PART, PART, PART, PUB,
     "TextSemanticOptions cjk mode; default in text_semantic. Action: cjk mode not yet a binding param (partial)."),
    ("text.rtl_vertical", "text", "RTL and vertical-writing diagnostics", PART, PART, PART, PART,
     "bidi handled in extraction; SemanticTextDirection at Rust root. Action: direction report field deferred."),
    ("fonts.subsetting", "fonts", "font subsetting reports", PART, PART, PART, PUB,
     "FontInfo subset flag; writer subsetting. Action: dedicated subset report deferred."),
    ("text.search", "text", "text search", PUB, PART, PART, PUB,
     "engine.search_text (Rust); used inside redact facade. Action: standalone search binding method deferred; redaction uses it."),
    ("text.quad_bbox", "text", "quad and bbox reporting", PUB, PUB, PUB, PUB,
     "text_semantic quads/bboxes; Page.words bbox."),
    ("fonts.embedding_status", "fonts", "font embedding status", PUB, PUB, PUB, PUB,
     "FontInfo embedded field in font_report."),
    ("fonts.color_glyph", "fonts", "color glyph status", INT, MISS, MISS, PART,
     "COLR/CPAL handling Rust-only. Action: color-glyph report deferred."),

    # ── Color / prepress ────────────────────────────────────────────────────
    ("color.icc_inventory", "color", "ICC profile inventory", PUB, PUB, PUB, PUB,
     "color_report.color_spaces/output_intents. sdk::color_report_json."),
    ("color.output_intent", "color", "output intent reporting", PUB, PUB, PUB, PUB,
     "color_report.output_intents."),
    ("color.device_cmyk", "color", "DeviceCMYK reporting", PUB, PUB, PUB, PUB, "color_report.color_spaces."),
    ("color.devicen_sep", "color", "DeviceN and Separation reporting", PUB, PUB, PUB, PUB,
     "color_report.devicen_components/spot_colorants."),
    ("color.spot", "color", "spot color inventory", PUB, PUB, PUB, PUB, "color_report.spot_colorants."),
    ("color.overprint", "color", "overprint status", PUB, PUB, PUB, PUB, "color_report.overprint."),
    ("color.bpc", "color", "black-point compensation status", PART, PART, PART, PART,
     "color_report backend/limits. Action: explicit BPC field deferred."),
    ("color.rendering_intent", "color", "rendering intent reporting", PUB, PUB, PUB, PUB,
     "color_report.rendering_intents."),
    ("color.prepress_warning", "color", "prepress warning report", PUB, PUB, PUB, PUB,
     "color_report.diagnostics (ColorSeverity)."),
    ("color.conversion_diag", "color", "color conversion diagnostics", PUB, PUB, PUB, PUB,
     "color_report.icc_fidelity_vectors/diagnostics."),
    ("color.pdfx", "color", "PDF/X validation report", PUB, PUB, PUB, PUB,
     "standards_profile(profile=pdfx) + color_report(profile=pdfx)."),
    ("color.proofing", "color", "proofing mode options", INT, MISS, MISS, PART,
     "proofing render Rust-only. Action: deferred."),
    ("color.managed_image_extract", "color", "color-managed image extraction", INT, PART, PART, PART,
     "image extraction present; color-managed variant Rust-only. Action: deferred."),
    ("color.shading_diag", "color", "shading color diagnostics", PART, PART, PART, PART,
     "shading color surfaced in color_report color_spaces. Action: dedicated shading report deferred."),
    ("color.profile_hash", "color", "profile hash reporting", PART, PART, PART, PART,
     "icc transform cache; resource_digest available. Action: explicit profile hash field deferred."),

    # ── Semantic / structure / RAG ──────────────────────────────────────────
    ("sem.structtree", "semantic", "StructTree access", PUB, PUB, PUB, PUB,
     "extract_semantic_document + text_semantic structure. sdk::semantic_document_json."),
    ("sem.mcid", "semantic", "MCID mapping", PART, PART, PART, PUB,
     "text_semantic include_structure maps MCID; SemanticMcid at Rust root. Action: raw MCID map deferred."),
    ("sem.parenttree", "semantic", "ParentTree diagnostics", PART, PART, PART, PART,
     "structure context in text_semantic. Action: dedicated ParentTree report deferred."),
    ("sem.reading_order", "semantic", "reading-order blocks", PUB, PUB, PUB, PUB,
     "text_semantic blocks/paragraphs in reading order."),
    ("sem.paragraphs", "semantic", "semantic paragraphs", PUB, PUB, PUB, PUB, "text_semantic paragraphs."),
    ("sem.tables", "semantic", "semantic tables", PUB, PUB, PART, PUB,
     "extract_tables + semantic tables; py Page.tables. Action: capi tables JSON deferred (present via chunks)."),
    ("sem.figure_caption", "semantic", "figure and caption detection", PUB, PUB, PUB, PUB,
     "canonical Document blocks (Figure/Caption) via chunk/model."),
    ("sem.headings", "semantic", "heading detection", PUB, PUB, PUB, PUB, "document model headings; chunk section_path."),
    ("sem.provenance_graph", "semantic", "provenance graph", PART, PART, PART, PART,
     "text provenance flags/summary. Action: full provenance graph export deferred."),
    ("sem.search", "semantic", "semantic search", PART, PART, PART, PUB,
     "engine.search_text via semantic model. Action: standalone binding method deferred."),
    ("sem.rag_chunk", "semantic", "RAG chunk export", PUB, PUB, PUB, PUB,
     "chunk(). sdk::chunk_report_json; py chunks; capi chunks_json."),
    ("sem.json_model", "semantic", "JSON model export", PUB, PUB, PUB, PUB,
     "Document.to_json; document_model. py document_model; capi parse_json."),
    ("sem.cjk_dictionary", "semantic", "CJK dictionary hooks", INT, MISS, MISS, PART,
     "CJK segmentation dictionary Rust-only. Action: deferred."),
    ("sem.layout_confidence", "semantic", "layout confidence scores", PART, PART, PART, PART,
     "text_semantic confidence fields. Action: aggregate layout-confidence report deferred."),
    ("sem.a11y_hints", "semantic", "tagged-PDF accessibility hints", PUB, PUB, PUB, PUB,
     "validate_pdfua report. sdk::pdfua_validation_json."),

    # ── Tables, forms, annotations, page ops ────────────────────────────────
    ("tables.grid", "forms", "table extraction grid", PUB, PUB, PART, PUB,
     "extract_tables grid; py Page.tables. Action: capi standalone tables JSON deferred (chunks covers)."),
    ("tables.confidence", "forms", "table confidence reports", PART, PART, PART, PUB,
     "table detection confidence in extract_tables. Action: dedicated confidence report deferred."),
    ("forms.acroform_inventory", "forms", "AcroForm field inventory", PUB, PUB, PUB, PUB,
     "forms_report. sdk::forms_report_json; py forms_report; capi forms_report_json."),
    ("forms.field_rw", "forms", "field value read/write", PART, PART, PART, PUB,
     "forms_report reads; apply_form_data_pdf writes (Rust root). Action: binding write method deferred to a forms roadmap task."),
    ("forms.fdf", "forms", "FDF import and export", INT, MISS, MISS, PUB,
     "form_exchange (FDF) at Rust root + CLI. Action: binding FDF methods deferred."),
    ("forms.xfdf", "forms", "XFDF field import and export", INT, MISS, MISS, PUB,
     "form_exchange (XFDF) at Rust root + CLI. Action: binding XFDF methods deferred."),
    ("annot.inventory", "forms", "annotation inventory", PUB, PUB, PUB, PUB,
     "annotation_report. sdk::annotation_report_json."),
    ("annot.appearance_status", "forms", "annotation appearance status", PUB, PUB, PUB, PUB,
     "annotation_report appearance fields."),
    ("page.ops", "forms", "page insert/delete/reorder/rotate", PUB, PUB, PUB, PUB,
     "organize/rotate; page_operations_report. py organize_pdf/rotate_pdf; capi organize/rotate."),
    ("page.boxes", "forms", "page box read/write", PART, PART, PART, PUB,
     "pages_report reads boxes; crop_pdf writes. Action: box write binding method deferred."),
    ("attach.inventory", "forms", "embedded file inventory", PUB, PUB, PART, PUB,
     "list_attachments (Rust) + security_report risky embedded files. Action: capi attachments JSON deferred."),
    ("attach.policy", "forms", "attachment policy reporting", PUB, PUB, PUB, PUB,
     "security_report risky_content embedded_files + sanitize policy."),
    ("forms.xfa", "forms", "XFA status reporting", PUB, PUB, PUB, PUB,
     "forms_report XFA status; security_report xfa_packets."),
    ("annot.rich_media", "forms", "rich media policy reporting", PUB, PUB, PUB, PUB,
     "security_report risky_content rich_media_annotations."),
    ("annot.flatten", "forms", "annotation flattening", INT, MISS, MISS, PUB,
     "PdfEditor flatten path + CLI annotations-flatten. Action: binding flatten method deferred."),

    # ── Redaction, sanitizer, safe output ───────────────────────────────────
    ("redact.plan", "security", "redaction planning", PART, PART, PART, PUB,
     "search_text derives redaction geometry inside redact facade. Action: standalone plan API deferred."),
    ("redact.apply", "security", "redaction apply", PUB, PUB, PUB, PUB,
     f"sdk::redact_terms_json (search+apply+verify). {T_PY} test_redact_removes_and_verifies; {T_C} capi_redact_terms."),
    ("redact.text_proof", "security", "text redaction proof", PUB, PUB, PUB, PUB,
     "redaction_verification_report embedded in redact report (verified_absent)."),
    ("redact.image_proof", "security", "image redaction proof", PART, PART, PART, PUB,
     "ImageRedactionPolicy applied; report notes policy. Action: pixel-level image proof deferred."),
    ("redact.partial_image", "security", "partial image redaction status", PART, PART, PART, PUB,
     "ImageRedactionPolicy::Partial default. Action: per-image status report deferred."),
    ("sanitize.policy", "security", "sanitizer policy options", PUB, PUB, PUB, PUB,
     "sanitize(policy). sdk::sanitize_json; py sanitize; capi sanitize_json."),
    ("sanitize.js", "security", "JavaScript removal report", PUB, PUB, PUB, PUB,
     "sanitize report removed map + security_report javascript_actions."),
    ("sanitize.launch", "security", "Launch action removal report", PUB, PUB, PUB, PUB,
     "sanitize report removed map + security_report launch_actions."),
    ("sanitize.uri", "security", "URI and external action policy", PUB, PUB, PUB, PUB,
     "security_report uri_actions/submit_form_actions; sanitize removal."),
    ("sanitize.embedded_removal", "security", "embedded file removal", PUB, PUB, PUB, PUB,
     "sanitize removed embedded files."),
    ("sanitize.metadata", "security", "metadata scrubbing", PUB, PUB, PUB, PUB,
     "sanitize + redact scrub_metadata; canonicalize."),
    ("sanitize.safe_output_proof", "security", "safe output proof", PUB, PUB, PUB, PUB,
     "sanitize report strict_passed + output rescan."),
    ("sanitize.rescan", "security", "post-sanitize rescan", PUB, PUB, PUB, PUB,
     "sanitize_pdf rescans output (output_risky_total)."),
    ("sanitize.visual_diff", "security", "visual diff evidence", INT, MISS, MISS, PART,
     "render-compare Rust-only. Action: deferred."),
    ("security.risk_class", "security", "security risk classification", PUB, PUB, PUB, PUB,
     "security_report findings severity; risky_content_report."),

    # ── Editing, conversion, writer ─────────────────────────────────────────
    ("edit.model_export", "editing", "editable model export", PART, PART, PART, PUB,
     "build_editable_document (Rust root) + CLI. document_model exposed via py/capi. Action: full editable model binding method deferred."),
    ("edit.paragraph_reflow", "editing", "paragraph reflow edit", INT, MISS, MISS, PUB,
     "edit_paragraph_reflow_pdf (Rust root) + CLI. Action: binding edit method deferred."),
    ("edit.insert_delete_text", "editing", "insert and delete text", INT, MISS, MISS, PUB,
     "replace_text_pdf (Rust root) + CLI. Action: binding edit method deferred."),
    ("conv.docx_faithful", "editing", "page-faithful DOCX export", PART, PART, PART, PUB,
     "pdf_to_docx (layout). py/capi to_docx. Action: layout=page-faithful option not yet a binding param."),
    ("conv.docx_flow", "editing", "flowing DOCX export", PUB, PUB, PUB, PUB,
     "pdf_to_docx flowing. py pdf_to_docx; capi to_docx."),
    ("conv.pptx", "editing", "PPTX export", PUB, PUB, PUB, PUB, "pdf_to_pptx. py/capi to_pptx."),
    ("conv.xlsx", "editing", "XLSX export", PUB, PUB, PUB, PUB, "pdf_to_xlsx. py/capi to_xlsx."),
    ("conv.html", "editing", "HTML export", PUB, PUB, PUB, PUB, "to_html. py/capi to_html."),
    ("conv.markdown", "editing", "Markdown export", PUB, PUB, PUB, PUB, "to_markdown. py to_markdown; capi parse_markdown."),
    ("conv.json", "editing", "JSON export", PUB, PUB, PUB, PUB, "Document.to_json. py document_model; capi parse_json."),
    ("conv.office_to_pdf", "editing", "Office-to-PDF conversion status", PUB, PUB, PUB, PUB,
     "docx/xlsx/pptx_to_pdf. py/capi *_to_pdf."),
    ("writer.incremental", "editing", "incremental save", INT, MISS, MISS, PUB,
     "EditMode::Incremental + DeterministicSaveOptions (Rust root) + CLI. Action: binding save method deferred; canonicalize covers full rewrite."),
    ("writer.full_rewrite", "editing", "full rewrite save", PUB, PUB, PUB, PUB,
     "canonicalize (full rewrite) + redact/sanitize output. sdk::canonicalize_json."),
    ("writer.deterministic", "editing", "deterministic writer options", PUB, PUB, PUB, PUB,
     "canonicalize fixed_source_date_epoch → deterministic bytes (test asserts equality)."),
    ("writer.resource_dedup", "editing", "resource dedup report", PUB, PUB, PART, PUB,
     "resource_dedup_report. sdk::resource_dedup_report_json; py resource_dedup_report. Action: capi dedup fn deferred (needs resource buffers)."),

    # ── Security, encryption, signatures, standards ─────────────────────────
    ("sec.report", "standards", "security report", PUB, PUB, PUB, PUB,
     f"security_report. {T_SDK}; {T_PY}; {T_C}; {T_PARITY}."),
    ("sec.permissions", "standards", "permissions report", PUB, PUB, PUB, PUB,
     "document_info.permissions + security_report permissions_note."),
    ("sec.aes256", "standards", "AES-256 encryption status", PUB, PUB, PUB, PUB,
     "security_report encryption; encrypt (Rust/py/capi). "),
    ("sec.pubkey_handler", "standards", "public-key security handler status", PUB, PUB, PUB, PUB,
     "security_report.public_key_security_handler_detected (honest unsupported note)."),
    ("sig.byterange", "standards", "ByteRange signature report", PUB, PUB, PUB, PUB,
     "signature_report coverage/ByteRange. sdk::signature_report_json."),
    ("sig.cms_pkcs7", "standards", "CMS or PKCS7 signature status", PUB, PUB, PUB, PUB,
     "signature_report sub_filter/validity/certificate."),
    ("sig.pades", "standards", "PAdES status", PUB, PUB, PUB, PUB, "signature_report PadesLevel."),
    ("sig.dss_ltv", "standards", "DSS and LTV status", PUB, PUB, PUB, PUB, "signature_report.ltv (LtvReport)."),
    ("sig.timestamp", "standards", "timestamp status", PUB, PUB, PUB, PUB, "signature_report checks/timestamp."),
    ("sig.preservation_warning", "standards", "signature-preservation warning", PUB, PUB, PUB, PUB,
     "canonicalize signature_impact + DeterministicSaveReport warning."),
    ("std.pdfa", "standards", "PDF/A validation report", PUB, PUB, PUB, PUB,
     "validate_pdfa + standards_profile(pdfa). sdk::pdfa_validation_json."),
    ("std.pdfua", "standards", "PDF/UA validation report", PUB, PUB, PUB, PUB,
     "validate_pdfua + standards_profile(pdfua). sdk::pdfua_validation_json."),
    ("std.pdfx", "standards", "PDF/X validation report", PUB, PUB, PUB, PUB,
     "standards_profile(pdfx). sdk::standards_profile_json."),
    ("std.canonicalize", "standards", "canonicalize output", PUB, PUB, PUB, PUB,
     "canonicalize. sdk::canonicalize_json; py canonicalize; capi canonicalize_json."),
    ("std.threat_model", "standards", "threat-model report", PART, PART, PART, PUB,
     "security_report + risky_content_report cover threat surface. Action: consolidated threat-model doc report deferred."),

    # ── Diagnostics, reports, errors, limits ────────────────────────────────
    ("diag.schema", "diagnostics", "structured diagnostics schema", PUB, PUB, PUB, PUB,
     "versioned JSON envelope (schema_version/kind/report). REPORT_ENVELOPE_VERSION."),
    ("diag.error_taxonomy", "diagnostics", "error code taxonomy", PUB, PUB, PUB, PUB,
     "ErrorKind.code(); py WellfriendError; capi int status + error string."),
    ("diag.warning_severity", "diagnostics", "warning severity model", PUB, PUB, PUB, PUB,
     "SecuritySeverity/ColorSeverity/ValidationSeverity in reports."),
    ("diag.report_versioning", "diagnostics", "JSON report versioning", PUB, PUB, PUB, PUB,
     "envelope schema_version + inner report schema_version. report_schema_versioning doc."),
    ("diag.human_format", "diagnostics", "human report formatting", CLI, MISS, MISS, PUB,
     "CLI pretty/human output. Action: bindings return structured data; human formatting is a caller concern (documented)."),
    ("diag.progress", "diagnostics", "progress callbacks", INT, UNSUP, UNSUP, PART,
     "no engine progress callback seam yet. Action: deferred; reported honestly as unavailable."),
    ("diag.cancellation", "diagnostics", "cancellation tokens", INT, UNSUP, UNSUP, PART,
     "CancelToken in engine; not a binding param. Action: deferred."),
    ("diag.memory_budget", "diagnostics", "memory budget options", PART, PART, PART, PUB,
     "decode/render pixel budgets exposed; DecodeLimits. Action: per-call budget binding param deferred."),
    ("diag.recursion_limit", "diagnostics", "recursion limit options", INT, UNSUP, UNSUP, PART,
     "engine enforces internal recursion limits. Action: configurable binding param deferred."),
    ("diag.object_limit", "diagnostics", "object limit options", INT, UNSUP, UNSUP, PART,
     "engine enforces object limits. Action: configurable binding param deferred."),
    ("diag.timeout", "diagnostics", "timeout options", INT, UNSUP, UNSUP, PART,
     "ocr_timeout exists; general timeout not a binding param. Action: deferred."),
    ("diag.thread_safety", "diagnostics", "thread-safety guarantees", PUB, PUB, PUB, PUB,
     "ContentEngine Send+Sync (compile assert); capi header documents concurrent read."),
    ("diag.log_redaction", "diagnostics", "log redaction policy", INT, MISS, MISS, PART,
     "log crate used; no secret logging. Action: explicit policy report deferred."),
    ("diag.trace_ids", "diagnostics", "trace correlation IDs", MISS, MISS, MISS, MISS,
     "no trace-id seam. Action: deferred to an observability roadmap task."),
    ("diag.feature_availability", "diagnostics", "feature availability reporting", PUB, PUB, PUB, PUB,
     "feature_report (capabilities). sdk::feature_report_json; py feature_report; capi feature_report_json/version."),

    # ── Testing, packaging, examples, release ───────────────────────────────
    ("test.rust", "release", "Rust integration tests", PUB, PUB, PUB, PUB,
     "sdk::tests downstream-style facade tests (12)."),
    ("test.python", "release", "Python integration tests", PUB, PUB, PUB, PUB,
     "tests/test_reports.py (12) + test_smoke.py (6)."),
    ("test.capi", "release", "C ABI integration tests", PUB, PUB, PUB, PUB,
     "capi_* inline tests (report/version/output) + compiled examples/sdk_reports.c."),
    ("test.cross_lang_golden", "release", "cross-language golden fixtures", PUB, PUB, PUB, PUB,
     "rust/python/c-abi smoke JSON in target/binding_surface-binding-core; parity asserted equal."),
    ("test.snapshot_schema", "release", "snapshot JSON schemas", PUB, PUB, PUB, PUB,
     "report_schema_versioning_binding_surface.md + smoke JSON snapshots."),
    ("doc.api", "release", "API documentation", PUB, PUB, PUB, PUB,
     "public_api_rust/python_sdk/c_abi binding_surface docs + rustdoc."),
    ("doc.examples", "release", "example programs", PUB, PUB, PUB, PUB,
     "sdk_reports.{rs,py,c} + binding_examples_binding_surface.md."),
    ("pkg.metadata", "release", "package metadata", PUB, PUB, PUB, PUB,
     "Cargo.toml / pyproject.toml / wellfriendpdf.h; honest capabilities via feature_report."),
    ("pkg.semver", "release", "versioning and semver", PUB, PUB, PUB, PUB,
     "ENGINE_VERSION + REPORT_ENVELOPE_VERSION + wellfriendpdf_version/wellfriendpdf_abi_version."),
    ("pkg.feature_flags", "release", "feature flags", PUB, PUB, PUB, PUB,
     "cargo features surfaced in feature_report capabilities."),
    ("pkg.platform_matrix", "release", "platform matrix", PART, PART, PART, PART,
     "builds on win-msvc validated here; full platform matrix is CI. Action: platform matrix is a CI/release-roadmap task concern."),
    ("pkg.ci_smoke", "release", "CI packaging smoke", PART, PART, PART, PART,
     "local maturin build + cargo build validated. Action: CI wiring is a release roadmap task."),
    ("pkg.abi_compat", "release", "ABI compatibility checks", PUB, PUB, PUB, PUB,
     "wellfriendpdf_abi_version + hand-maintained header; opaque handles keep ABI stable."),
    ("pkg.memory_leak", "release", "memory leak checks", PUB, PART, PUB, PART,
     "capi tests free every allocation; py returns owned objects. Action: valgrind/asan run is a CI concern (bounded)."),
    ("pkg.release_manifest", "release", "release artifact manifest", PART, PART, PART, PART,
     "smoke JSON artifacts serve as manifest evidence. Action: formal release manifest is a release roadmap task."),
]

CATEGORY_TITLES = {
    "parser": "Opening, parser, COS, xref, and repair",
    "decode": "Decode, filters, images, and low-level safety",
    "render": "Rendering and visual output",
    "fonts": "Fonts, glyphs, text shaping",
    "text": "Text extraction",
    "color": "Color management and prepress reports",
    "semantic": "Semantic model, structure tree, search, and RAG",
    "forms": "Tables, forms, annotations, and page operations",
    "security": "Redaction, sanitizer, and safe output",
    "editing": "Editing, conversion, and writer APIs",
    "standards": "Security, encryption, signatures, and standards",
    "diagnostics": "Diagnostics, reports, errors, and limits",
    "release": "Testing, packaging, examples, and release surfaces",
}


def main() -> int:
    rows = []
    for (fid, cat, name, rust, py, capi, cli, note) in FEATURES:
        rows.append({
            "id": fid,
            "category": cat,
            "feature": name,
            "surfaces": {"rust": rust, "python": py, "c_abi": capi, "cli": cli},
            "note": note,
        })

    # Tallies over the primary binding surfaces this roadmap task implements (rust,
    # python, c_abi). Report the max-maturity status per row for a headline.
    def headline(r):
        order = [PUB, PART, CLI, INT, UNSUP, MISS, DEF, BLK]
        vals = [r["surfaces"]["rust"], r["surfaces"]["python"], r["surfaces"]["c_abi"]]
        for s in order:
            if s in vals:
                return s
        return MISS

    tally = {}
    for r in rows:
        h = headline(r)
        tally[h] = tally.get(h, 0) + 1

    matrix = {
        "schema_version": 1,
        "feature_area": "combined-01-binding-core",
        "envelope_version": 1,
        "status_vocabulary": [PUB, PART, INT, CLI, UNSUP, MISS, DEF, BLK],
        "surfaces": ["rust", "python", "c_abi", "cli"],
        "feature_count": len(rows),
        "headline_tally": tally,
        "features": rows,
    }

    JSON_OUT.parent.mkdir(parents=True, exist_ok=True)
    JSON_OUT.write_text(json.dumps(matrix, indent=2), encoding="utf-8")

    # Markdown.
    lines = []
    lines.append("# roadmap closure 01 — Binding Gap Matrix")
    lines.append("")
    lines.append("Human-readable view of "
                 "`target/binding_surface-binding-core/binding-gap-matrix.json`. "
                 "Regenerate both with `python scripts/gen_binding_gap_matrix.py`.")
    lines.append("")
    lines.append(f"**Features:** {len(rows)}  ")
    lines.append("**Headline tally (best of rust/python/c_abi per feature):** "
                 + ", ".join(f"{k}={v}" for k, v in sorted(tally.items())))
    lines.append("")
    lines.append("Statuses: `implemented_public`, `partial_public`, "
                 "`implemented_internal`, `cli_only`, `unsupported_reported`, "
                 "`missing`, `deferred`, `blocked`.")
    lines.append("")
    by_cat = {}
    for r in rows:
        by_cat.setdefault(r["category"], []).append(r)
    for cat, title in CATEGORY_TITLES.items():
        if cat not in by_cat:
            continue
        lines.append(f"## {title}")
        lines.append("")
        lines.append("| Feature | Rust | Python | C ABI | CLI | Note / action |")
        lines.append("| --- | --- | --- | --- | --- | --- |")
        for r in by_cat[cat]:
            s = r["surfaces"]
            note = r["note"].replace("|", "\\|")
            lines.append(
                f"| {r['feature']} (`{r['id']}`) | {s['rust']} | {s['python']} "
                f"| {s['c_abi']} | {s['cli']} | {note} |"
            )
        lines.append("")
    MD_OUT.write_text("\n".join(lines), encoding="utf-8")

    print(f"wrote {JSON_OUT}")
    print(f"wrote {MD_OUT}")
    print(f"features={len(rows)} tally={tally}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
