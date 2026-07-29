# roadmap closure 01 — Binding Gap Matrix

Human-readable view of `target/binding_surface-binding-core/binding-gap-matrix.json`. Regenerate both with `python scripts/gen_binding_gap_matrix.py`.

**Features:** 180  
**Headline tally (best of rust/python/c_abi per feature):** cli_only=1, implemented_internal=29, implemented_public=112, missing=1, partial_public=37

Statuses: `implemented_public`, `partial_public`, `implemented_internal`, `cli_only`, `unsupported_reported`, `missing`, `deferred`, `blocked`.

## Opening, parser, COS, xref, and repair

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| document open options (`open.options`) | implemented_public | implemented_public | implemented_public | implemented_public | ContentEngine::open_bytes[/with_password]; sdk::open; py Document(...); capi open_from_bytes. wellfriendpdf-py tests/test_reports.py |
| byte-source abstraction (`open.byte_source`) | implemented_public | implemented_public | partial_public | implemented_public | Rust open_bytes/open_path; py bytes+path; capi bytes only (file input via caller read). Action: capi memory-only is intentional; path input is a caller concern. |
| xref table and xref stream access (`parser.xref`) | partial_public | implemented_public | implemented_public | implemented_public | Surfaced via parser_report (linearization/source_metrics/xref recovery). sdk::parser_report_json; wellfriendpdf-py tests/test_reports.py; wellfriendpdf-capi capi_* tests (crates/wellfriendpdf-capi/src/lib.rs). Raw xref entry walking stays Rust-only (reader::XrefEntry). |
| trailer and document ID reporting (`parser.trailer_id`) | implemented_public | implemented_public | implemented_public | implemented_public | document_info (ids/producer) + parser_report. sdk::document_info_json/parser_report_json. |
| incremental revision enumeration (`parser.revisions`) | implemented_public | implemented_public | implemented_public | implemented_public | parser_report.revision_history. sdk::parser_report_json. |
| object lookup and typed object access (`parser.object_lookup`) | implemented_internal | missing | missing | implemented_internal | reader::get_object / PdfObject at Rust root only. Action: deferred to a later low-level-access prompt; not exposed to bindings by design (unstable object model). |
| page tree traversal (`parser.page_tree`) | implemented_public | implemented_public | implemented_public | implemented_public | page_count + page access; document_info page_sizes. py Document len/iter/page; capi page_count. |
| stream length and offset diagnostics (`parser.stream_offsets`) | partial_public | partial_public | partial_public | implemented_public | parser_report source_metrics + decode budget report. Action: per-stream offset table stays Rust-only. |
| repair-mode diagnostics (`parser.repair`) | implemented_public | implemented_public | implemented_public | implemented_public | parser_report mode=repair/audit repair_summary. wellfriendpdf-capi capi_* tests (crates/wellfriendpdf-capi/src/lib.rs) capi_parametrized_reports. |
| linearization status (`parser.linearization`) | implemented_public | implemented_public | implemented_public | implemented_public | parser_report.linearization. sdk::parser_report_json. |
| encryption status discovery (`parser.encryption_status`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report.encrypted/encryption + document_info. sdk::security_report_json. |
| object cycle detection (`parser.object_cycle`) | partial_public | partial_public | partial_public | partial_public | Reported via parser_report diagnostics when hit; no standalone report. Action: dedicated cycle report deferred. |
| malformed object recovery (`parser.malformed_recovery`) | implemented_public | implemented_public | implemented_public | implemented_public | parser_report repair mode reports recovered/failed objects. |
| Arlington validation hooks (`parser.arlington`) | implemented_public | implemented_public | implemented_public | implemented_public | parser_report.arlington + standards_profile arlington_status. sdk::standards_profile_json. |
| parser memory-limit reporting (`parser.memory_limits`) | partial_public | partial_public | partial_public | implemented_public | decode budget report surfaces limits; parser open honors engine limits. Action: explicit parser memory budget option not yet a binding param (partial). |

## Decode, filters, images, and low-level safety

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| filter-chain diagnostics (`decode.filter_chain`) | partial_public | partial_public | partial_public | implemented_public | parser_report decode section (include_decode) + decode budget report. sdk::decode_budget_report_json. |
| Flate and predictor decode limits (`decode.flate_predictor`) | implemented_public | implemented_public | implemented_public | implemented_public | decode budget report + DecodeLimits (MAX_FLATE_DECOMPRESSED_BYTES). |
| DCT image decode reporting (`decode.dct`) | implemented_public | implemented_public | implemented_public | implemented_public | decode_budget_report(filter='DCTDecode',...). sdk::decode_budget_report_json. |
| JPX image decode reporting (`decode.jpx`) | implemented_public | implemented_public | implemented_public | implemented_public | decode_budget_report(filter='JPXDecode',...). |
| JBIG2 safety reporting (`decode.jbig2`) | implemented_public | implemented_public | implemented_public | implemented_public | decode_budget_report(filter='JBIG2Decode',...); risky policy in security_report. |
| CCITT decode reporting (`decode.ccitt`) | implemented_public | implemented_public | implemented_public | implemented_public | decode_budget_report(filter='CCITTFaxDecode',...). |
| image inventory extraction (`decode.image_inventory`) | implemented_public | implemented_public | implemented_internal | implemented_public | engine.find_all_images; py Page.images; Rust root ImageLocator. Action: capi image inventory JSON deferred (bytes-heavy). |
| stream decode cancellation (`decode.cancellation`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | engine CancelToken exists; bindings do not yet pass a cancel token. Action: cancel-token binding param deferred; reported as unsupported in feature notes. |
| decode cache status (`decode.cache`) | implemented_internal | missing | missing | implemented_internal | DecodeCache/DecodeCacheMetrics at Rust root only. Action: metrics report deferred. |
| decode scheduler limits (`decode.scheduler`) | implemented_internal | missing | missing | implemented_internal | DecodeMemoryBudget/DecodeSchedulerMetrics at Rust root only. Action: deferred. |
| decompression bomb detection (`decode.bomb`) | implemented_public | implemented_public | implemented_public | implemented_public | decode_budget_report exceeds-limit diagnostics; security_report findings. |
| sandbox policy reporting (`decode.sandbox_policy`) | partial_public | partial_public | partial_public | partial_public | codec sandboxing documented; surfaced via decode diagnostics. Action: standalone policy report deferred. |
| raw stream versus decoded stream access (`decode.raw_vs_decoded`) | implemented_internal | missing | missing | partial_public | filters::decode_stream[_lossless] at Rust root. Action: raw/decoded stream fetch not exposed to bindings (unstable object handles). |
| unsupported filter diagnostics (`decode.unsupported_filter`) | implemented_public | implemented_public | implemented_public | implemented_public | DecodeReport diagnostics + WellfriendError::UnsupportedFeature; honest reporting. |
| decode performance counters (`decode.perf_counters`) | partial_public | partial_public | partial_public | partial_public | DecodeMetrics inside decode reports. Action: full perf-counter export deferred. |

## Rendering and visual output

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| page raster rendering (`render.raster`) | implemented_public | implemented_public | implemented_public | implemented_public | render_page_png_fast/jpeg. py Document/Page.render; capi render_page_png/jpeg. |
| display-list extraction (`render.display_list`) | implemented_internal | missing | missing | partial_public | render::DisplayList at Rust root. Action: display-list JSON export deferred (large/unstable). |
| render options (`render.options`) | partial_public | partial_public | partial_public | implemented_public | DPI/format exposed; full RenderQuality/RenderMode subset. Action: extended render options deferred. |
| DPI and scale handling (`render.dpi_scale`) | implemented_public | implemented_public | implemented_public | implemented_public | dpi param on all render entry points. |
| tile rendering (`render.tile`) | implemented_internal | missing | missing | implemented_internal | render::RenderTile at Rust root. Action: deferred to a render-binding roadmap task. |
| band rendering (`render.band`) | implemented_internal | missing | missing | implemented_internal | renderer band path Rust-only. Action: deferred. |
| progressive rendering state (`render.progressive`) | implemented_internal | missing | missing | implemented_internal | Rust-only. Action: deferred. |
| render cancellation (`render.cancellation`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | CancelToken exists in engine; not a binding param yet. Action: deferred. |
| annotation appearance rendering (`render.annot_appearance`) | partial_public | partial_public | partial_public | implemented_public | annotation_report appearance status; render includes annots. sdk::annotation_report_json. |
| optional-content visibility reporting (`render.optional_content`) | implemented_internal | missing | missing | partial_public | OCG handling in renderer Rust-only. Action: OCG report deferred. |
| render diagnostics (`render.diagnostics`) | partial_public | partial_public | partial_public | partial_public | UnsupportedRenderOp/DisplayListStats at Rust root. Action: render diagnostics JSON deferred. |
| visual hash reporting (`render.visual_hash`) | implemented_internal | missing | missing | partial_public | versioning simhash / render compare Rust-only. Action: deferred. |
| render memory budget (`render.memory_budget`) | implemented_public | partial_public | partial_public | implemented_public | max_render_pixels/DEFAULT_MAX_RENDER_PIXELS. Action: per-call render budget param deferred for bindings. |
| color-managed render options (`render.color_managed`) | implemented_internal | missing | missing | partial_public | color-managed render Rust-only; color_report exposes color state. Action: deferred. |
| image output encoding (`render.image_output_encoding`) | implemented_public | implemented_public | implemented_public | implemented_public | png/jpeg output selection on render entry points. |

## Fonts, glyphs, text shaping

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| font inventory (`fonts.inventory`) | implemented_public | implemented_public | implemented_public | implemented_public | list_fonts. sdk::font_report_json; py font_report; capi fonts_json. |
| font substitution diagnostics (`fonts.substitution`) | partial_public | partial_public | partial_public | implemented_public | FontInfo embedded flag + substitution notes. Action: dedicated substitution report deferred. |
| Type0 and CID font reporting (`fonts.type0_cid`) | implemented_public | implemented_public | implemented_public | implemented_public | FontInfo type/subtype in font_report. |
| CMap diagnostics (`fonts.cmap`) | partial_public | partial_public | partial_public | partial_public | Surfaced via font_report + text provenance. Action: standalone CMap report deferred. |
| glyph positioning data (`fonts.glyph_positioning`) | implemented_internal | partial_public | partial_public | partial_public | text_semantic spans carry geometry. Rust ShapedGlyph deeper. Action: raw glyph positions deferred. |
| font subsetting reports (`fonts.subsetting`) | partial_public | partial_public | partial_public | implemented_public | FontInfo subset flag; writer subsetting. Action: dedicated subset report deferred. |
| font embedding status (`fonts.embedding_status`) | implemented_public | implemented_public | implemented_public | implemented_public | FontInfo embedded field in font_report. |
| color glyph status (`fonts.color_glyph`) | implemented_internal | missing | missing | partial_public | COLR/CPAL handling Rust-only. Action: color-glyph report deferred. |

## Text extraction

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| text extraction spans (`text.spans`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic model + words. sdk::text_semantic_json; py text_semantic/words. |
| char-level provenance (`text.char_provenance`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic chars + provenance flags. |
| word and line grouping (`text.word_line_grouping`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic words/lines; extract_page_words. |
| CJK segmentation status (`text.cjk_segmentation`) | partial_public | partial_public | partial_public | implemented_public | TextSemanticOptions cjk mode; default in text_semantic. Action: cjk mode not yet a binding param (partial). |
| RTL and vertical-writing diagnostics (`text.rtl_vertical`) | partial_public | partial_public | partial_public | partial_public | bidi handled in extraction; SemanticTextDirection at Rust root. Action: direction report field deferred. |
| text search (`text.search`) | implemented_public | partial_public | partial_public | implemented_public | engine.search_text (Rust); used inside redact facade. Action: standalone search binding method deferred; redaction uses it. |
| quad and bbox reporting (`text.quad_bbox`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic quads/bboxes; Page.words bbox. |

## Color management and prepress reports

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| ICC profile inventory (`color.icc_inventory`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.color_spaces/output_intents. sdk::color_report_json. |
| output intent reporting (`color.output_intent`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.output_intents. |
| DeviceCMYK reporting (`color.device_cmyk`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.color_spaces. |
| DeviceN and Separation reporting (`color.devicen_sep`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.devicen_components/spot_colorants. |
| spot color inventory (`color.spot`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.spot_colorants. |
| overprint status (`color.overprint`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.overprint. |
| black-point compensation status (`color.bpc`) | partial_public | partial_public | partial_public | partial_public | color_report backend/limits. Action: explicit BPC field deferred. |
| rendering intent reporting (`color.rendering_intent`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.rendering_intents. |
| prepress warning report (`color.prepress_warning`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.diagnostics (ColorSeverity). |
| color conversion diagnostics (`color.conversion_diag`) | implemented_public | implemented_public | implemented_public | implemented_public | color_report.icc_fidelity_vectors/diagnostics. |
| PDF/X validation report (`color.pdfx`) | implemented_public | implemented_public | implemented_public | implemented_public | standards_profile(profile=pdfx) + color_report(profile=pdfx). |
| proofing mode options (`color.proofing`) | implemented_internal | missing | missing | partial_public | proofing render Rust-only. Action: deferred. |
| color-managed image extraction (`color.managed_image_extract`) | implemented_internal | partial_public | partial_public | partial_public | image extraction present; color-managed variant Rust-only. Action: deferred. |
| shading color diagnostics (`color.shading_diag`) | partial_public | partial_public | partial_public | partial_public | shading color surfaced in color_report color_spaces. Action: dedicated shading report deferred. |
| profile hash reporting (`color.profile_hash`) | partial_public | partial_public | partial_public | partial_public | icc transform cache; resource_digest available. Action: explicit profile hash field deferred. |

## Semantic model, structure tree, search, and RAG

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| StructTree access (`sem.structtree`) | implemented_public | implemented_public | implemented_public | implemented_public | extract_semantic_document + text_semantic structure. sdk::semantic_document_json. |
| MCID mapping (`sem.mcid`) | partial_public | partial_public | partial_public | implemented_public | text_semantic include_structure maps MCID; SemanticMcid at Rust root. Action: raw MCID map deferred. |
| ParentTree diagnostics (`sem.parenttree`) | partial_public | partial_public | partial_public | partial_public | structure context in text_semantic. Action: dedicated ParentTree report deferred. |
| reading-order blocks (`sem.reading_order`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic blocks/paragraphs in reading order. |
| semantic paragraphs (`sem.paragraphs`) | implemented_public | implemented_public | implemented_public | implemented_public | text_semantic paragraphs. |
| semantic tables (`sem.tables`) | implemented_public | implemented_public | partial_public | implemented_public | extract_tables + semantic tables; py Page.tables. Action: capi tables JSON deferred (present via chunks). |
| figure and caption detection (`sem.figure_caption`) | implemented_public | implemented_public | implemented_public | implemented_public | canonical Document blocks (Figure/Caption) via chunk/model. |
| heading detection (`sem.headings`) | implemented_public | implemented_public | implemented_public | implemented_public | document model headings; chunk section_path. |
| provenance graph (`sem.provenance_graph`) | partial_public | partial_public | partial_public | partial_public | text provenance flags/summary. Action: full provenance graph export deferred. |
| semantic search (`sem.search`) | partial_public | partial_public | partial_public | implemented_public | engine.search_text via semantic model. Action: standalone binding method deferred. |
| RAG chunk export (`sem.rag_chunk`) | implemented_public | implemented_public | implemented_public | implemented_public | chunk(). sdk::chunk_report_json; py chunks; capi chunks_json. |
| JSON model export (`sem.json_model`) | implemented_public | implemented_public | implemented_public | implemented_public | Document.to_json; document_model. py document_model; capi parse_json. |
| CJK dictionary hooks (`sem.cjk_dictionary`) | implemented_internal | missing | missing | partial_public | CJK segmentation dictionary Rust-only. Action: deferred. |
| layout confidence scores (`sem.layout_confidence`) | partial_public | partial_public | partial_public | partial_public | text_semantic confidence fields. Action: aggregate layout-confidence report deferred. |
| tagged-PDF accessibility hints (`sem.a11y_hints`) | implemented_public | implemented_public | implemented_public | implemented_public | validate_pdfua report. sdk::pdfua_validation_json. |

## Tables, forms, annotations, and page operations

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| table extraction grid (`tables.grid`) | implemented_public | implemented_public | partial_public | implemented_public | extract_tables grid; py Page.tables. Action: capi standalone tables JSON deferred (chunks covers). |
| table confidence reports (`tables.confidence`) | partial_public | partial_public | partial_public | implemented_public | table detection confidence in extract_tables. Action: dedicated confidence report deferred. |
| AcroForm field inventory (`forms.acroform_inventory`) | implemented_public | implemented_public | implemented_public | implemented_public | forms_report. sdk::forms_report_json; py forms_report; capi forms_report_json. |
| field value read/write (`forms.field_rw`) | partial_public | partial_public | partial_public | implemented_public | forms_report reads; apply_form_data_pdf writes (Rust root). Action: binding write method deferred to a forms roadmap task. |
| FDF import and export (`forms.fdf`) | implemented_internal | missing | missing | implemented_public | form_exchange (FDF) at Rust root + CLI. Action: binding FDF methods deferred. |
| XFDF field import and export (`forms.xfdf`) | implemented_internal | missing | missing | implemented_public | form_exchange (XFDF) at Rust root + CLI. Action: binding XFDF methods deferred. |
| annotation inventory (`annot.inventory`) | implemented_public | implemented_public | implemented_public | implemented_public | annotation_report. sdk::annotation_report_json. |
| annotation appearance status (`annot.appearance_status`) | implemented_public | implemented_public | implemented_public | implemented_public | annotation_report appearance fields. |
| page insert/delete/reorder/rotate (`page.ops`) | implemented_public | implemented_public | implemented_public | implemented_public | organize/rotate; page_operations_report. py organize_pdf/rotate_pdf; capi organize/rotate. |
| page box read/write (`page.boxes`) | partial_public | partial_public | partial_public | implemented_public | pages_report reads boxes; crop_pdf writes. Action: box write binding method deferred. |
| embedded file inventory (`attach.inventory`) | implemented_public | implemented_public | partial_public | implemented_public | list_attachments (Rust) + security_report risky embedded files. Action: capi attachments JSON deferred. |
| attachment policy reporting (`attach.policy`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report risky_content embedded_files + sanitize policy. |
| XFA status reporting (`forms.xfa`) | implemented_public | implemented_public | implemented_public | implemented_public | forms_report XFA status; security_report xfa_packets. |
| rich media policy reporting (`annot.rich_media`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report risky_content rich_media_annotations. |
| annotation flattening (`annot.flatten`) | implemented_internal | missing | missing | implemented_public | PdfEditor flatten path + CLI annotations-flatten. Action: binding flatten method deferred. |

## Redaction, sanitizer, and safe output

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| redaction planning (`redact.plan`) | partial_public | partial_public | partial_public | implemented_public | search_text derives redaction geometry inside redact facade. Action: standalone plan API deferred. |
| redaction apply (`redact.apply`) | implemented_public | implemented_public | implemented_public | implemented_public | sdk::redact_terms_json (search+apply+verify). wellfriendpdf-py tests/test_reports.py test_redact_removes_and_verifies; wellfriendpdf-capi capi_* tests (crates/wellfriendpdf-capi/src/lib.rs) capi_redact_terms. |
| text redaction proof (`redact.text_proof`) | implemented_public | implemented_public | implemented_public | implemented_public | redaction_verification_report embedded in redact report (verified_absent). |
| image redaction proof (`redact.image_proof`) | partial_public | partial_public | partial_public | implemented_public | ImageRedactionPolicy applied; report notes policy. Action: pixel-level image proof deferred. |
| partial image redaction status (`redact.partial_image`) | partial_public | partial_public | partial_public | implemented_public | ImageRedactionPolicy::Partial default. Action: per-image status report deferred. |
| sanitizer policy options (`sanitize.policy`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize(policy). sdk::sanitize_json; py sanitize; capi sanitize_json. |
| JavaScript removal report (`sanitize.js`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize report removed map + security_report javascript_actions. |
| Launch action removal report (`sanitize.launch`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize report removed map + security_report launch_actions. |
| URI and external action policy (`sanitize.uri`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report uri_actions/submit_form_actions; sanitize removal. |
| embedded file removal (`sanitize.embedded_removal`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize removed embedded files. |
| metadata scrubbing (`sanitize.metadata`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize + redact scrub_metadata; canonicalize. |
| safe output proof (`sanitize.safe_output_proof`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize report strict_passed + output rescan. |
| post-sanitize rescan (`sanitize.rescan`) | implemented_public | implemented_public | implemented_public | implemented_public | sanitize_pdf rescans output (output_risky_total). |
| visual diff evidence (`sanitize.visual_diff`) | implemented_internal | missing | missing | partial_public | render-compare Rust-only. Action: deferred. |
| security risk classification (`security.risk_class`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report findings severity; risky_content_report. |

## Editing, conversion, and writer APIs

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| editable model export (`edit.model_export`) | partial_public | partial_public | partial_public | implemented_public | build_editable_document (Rust root) + CLI. document_model exposed via py/capi. Action: full editable model binding method deferred. |
| paragraph reflow edit (`edit.paragraph_reflow`) | implemented_internal | missing | missing | implemented_public | edit_paragraph_reflow_pdf (Rust root) + CLI. Action: binding edit method deferred. |
| insert and delete text (`edit.insert_delete_text`) | implemented_internal | missing | missing | implemented_public | replace_text_pdf (Rust root) + CLI. Action: binding edit method deferred. |
| page-faithful DOCX export (`conv.docx_faithful`) | partial_public | partial_public | partial_public | implemented_public | pdf_to_docx (layout). py/capi to_docx. Action: layout=page-faithful option not yet a binding param. |
| flowing DOCX export (`conv.docx_flow`) | implemented_public | implemented_public | implemented_public | implemented_public | pdf_to_docx flowing. py pdf_to_docx; capi to_docx. |
| PPTX export (`conv.pptx`) | implemented_public | implemented_public | implemented_public | implemented_public | pdf_to_pptx. py/capi to_pptx. |
| XLSX export (`conv.xlsx`) | implemented_public | implemented_public | implemented_public | implemented_public | pdf_to_xlsx. py/capi to_xlsx. |
| HTML export (`conv.html`) | implemented_public | implemented_public | implemented_public | implemented_public | to_html. py/capi to_html. |
| Markdown export (`conv.markdown`) | implemented_public | implemented_public | implemented_public | implemented_public | to_markdown. py to_markdown; capi parse_markdown. |
| JSON export (`conv.json`) | implemented_public | implemented_public | implemented_public | implemented_public | Document.to_json. py document_model; capi parse_json. |
| Office-to-PDF conversion status (`conv.office_to_pdf`) | implemented_public | implemented_public | implemented_public | implemented_public | docx/xlsx/pptx_to_pdf. py/capi *_to_pdf. |
| incremental save (`writer.incremental`) | implemented_internal | missing | missing | implemented_public | EditMode::Incremental + DeterministicSaveOptions (Rust root) + CLI. Action: binding save method deferred; canonicalize covers full rewrite. |
| full rewrite save (`writer.full_rewrite`) | implemented_public | implemented_public | implemented_public | implemented_public | canonicalize (full rewrite) + redact/sanitize output. sdk::canonicalize_json. |
| deterministic writer options (`writer.deterministic`) | implemented_public | implemented_public | implemented_public | implemented_public | canonicalize fixed_source_date_epoch → deterministic bytes (test asserts equality). |
| resource dedup report (`writer.resource_dedup`) | implemented_public | implemented_public | partial_public | implemented_public | resource_dedup_report. sdk::resource_dedup_report_json; py resource_dedup_report. Action: capi dedup fn deferred (needs resource buffers). |

## Security, encryption, signatures, and standards

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| security report (`sec.report`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report. engine sdk::tests (crates/engine/src/sdk.rs); wellfriendpdf-py tests/test_reports.py; wellfriendpdf-capi capi_* tests (crates/wellfriendpdf-capi/src/lib.rs); cross-surface parity: rust/python/c-abi smoke JSON compared equal. |
| permissions report (`sec.permissions`) | implemented_public | implemented_public | implemented_public | implemented_public | document_info.permissions + security_report permissions_note. |
| AES-256 encryption status (`sec.aes256`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report encryption; encrypt (Rust/py/capi).  |
| public-key security handler status (`sec.pubkey_handler`) | implemented_public | implemented_public | implemented_public | implemented_public | security_report.public_key_security_handler_detected (honest unsupported note). |
| ByteRange signature report (`sig.byterange`) | implemented_public | implemented_public | implemented_public | implemented_public | signature_report coverage/ByteRange. sdk::signature_report_json. |
| CMS or PKCS7 signature status (`sig.cms_pkcs7`) | implemented_public | implemented_public | implemented_public | implemented_public | signature_report sub_filter/validity/certificate. |
| PAdES status (`sig.pades`) | implemented_public | implemented_public | implemented_public | implemented_public | signature_report PadesLevel. |
| DSS and LTV status (`sig.dss_ltv`) | implemented_public | implemented_public | implemented_public | implemented_public | signature_report.ltv (LtvReport). |
| timestamp status (`sig.timestamp`) | implemented_public | implemented_public | implemented_public | implemented_public | signature_report checks/timestamp. |
| signature-preservation warning (`sig.preservation_warning`) | implemented_public | implemented_public | implemented_public | implemented_public | canonicalize signature_impact + DeterministicSaveReport warning. |
| PDF/A validation report (`std.pdfa`) | implemented_public | implemented_public | implemented_public | implemented_public | validate_pdfa + standards_profile(pdfa). sdk::pdfa_validation_json. |
| PDF/UA validation report (`std.pdfua`) | implemented_public | implemented_public | implemented_public | implemented_public | validate_pdfua + standards_profile(pdfua). sdk::pdfua_validation_json. |
| PDF/X validation report (`std.pdfx`) | implemented_public | implemented_public | implemented_public | implemented_public | standards_profile(pdfx). sdk::standards_profile_json. |
| canonicalize output (`std.canonicalize`) | implemented_public | implemented_public | implemented_public | implemented_public | canonicalize. sdk::canonicalize_json; py canonicalize; capi canonicalize_json. |
| threat-model report (`std.threat_model`) | partial_public | partial_public | partial_public | implemented_public | security_report + risky_content_report cover threat surface. Action: consolidated threat-model doc report deferred. |

## Diagnostics, reports, errors, and limits

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| structured diagnostics schema (`diag.schema`) | implemented_public | implemented_public | implemented_public | implemented_public | versioned JSON envelope (schema_version/kind/report). REPORT_ENVELOPE_VERSION. |
| error code taxonomy (`diag.error_taxonomy`) | implemented_public | implemented_public | implemented_public | implemented_public | ErrorKind.code(); py WellfriendError; capi int status + error string. |
| warning severity model (`diag.warning_severity`) | implemented_public | implemented_public | implemented_public | implemented_public | SecuritySeverity/ColorSeverity/ValidationSeverity in reports. |
| JSON report versioning (`diag.report_versioning`) | implemented_public | implemented_public | implemented_public | implemented_public | envelope schema_version + inner report schema_version. report_schema_versioning doc. |
| human report formatting (`diag.human_format`) | cli_only | missing | missing | implemented_public | CLI pretty/human output. Action: bindings return structured data; human formatting is a caller concern (documented). |
| progress callbacks (`diag.progress`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | no engine progress callback seam yet. Action: deferred; reported honestly as unavailable. |
| cancellation tokens (`diag.cancellation`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | CancelToken in engine; not a binding param. Action: deferred. |
| memory budget options (`diag.memory_budget`) | partial_public | partial_public | partial_public | implemented_public | decode/render pixel budgets exposed; DecodeLimits. Action: per-call budget binding param deferred. |
| recursion limit options (`diag.recursion_limit`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | engine enforces internal recursion limits. Action: configurable binding param deferred. |
| object limit options (`diag.object_limit`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | engine enforces object limits. Action: configurable binding param deferred. |
| timeout options (`diag.timeout`) | implemented_internal | unsupported_reported | unsupported_reported | partial_public | ocr_timeout exists; general timeout not a binding param. Action: deferred. |
| thread-safety guarantees (`diag.thread_safety`) | implemented_public | implemented_public | implemented_public | implemented_public | ContentEngine Send+Sync (compile assert); capi header documents concurrent read. |
| log redaction policy (`diag.log_redaction`) | implemented_internal | missing | missing | partial_public | log crate used; no secret logging. Action: explicit policy report deferred. |
| trace correlation IDs (`diag.trace_ids`) | missing | missing | missing | missing | no trace-id seam. Action: deferred to an observability roadmap task. |
| feature availability reporting (`diag.feature_availability`) | implemented_public | implemented_public | implemented_public | implemented_public | feature_report (capabilities). sdk::feature_report_json; py feature_report; capi feature_report_json/version. |

## Testing, packaging, examples, and release surfaces

| Feature | Rust | Python | C ABI | CLI | Note / action |
| --- | --- | --- | --- | --- | --- |
| Rust integration tests (`test.rust`) | implemented_public | implemented_public | implemented_public | implemented_public | sdk::tests downstream-style facade tests (12). |
| Python integration tests (`test.python`) | implemented_public | implemented_public | implemented_public | implemented_public | tests/test_reports.py (12) + test_smoke.py (6). |
| C ABI integration tests (`test.capi`) | implemented_public | implemented_public | implemented_public | implemented_public | capi_* inline tests (report/version/output) + compiled examples/sdk_reports.c. |
| cross-language golden fixtures (`test.cross_lang_golden`) | implemented_public | implemented_public | implemented_public | implemented_public | rust/python/c-abi smoke JSON in target/binding_surface-binding-core; parity asserted equal. |
| snapshot JSON schemas (`test.snapshot_schema`) | implemented_public | implemented_public | implemented_public | implemented_public | report_schema_versioning_binding_surface.md + smoke JSON snapshots. |
| API documentation (`doc.api`) | implemented_public | implemented_public | implemented_public | implemented_public | public_api_rust/python_sdk/c_abi binding_surface docs + rustdoc. |
| example programs (`doc.examples`) | implemented_public | implemented_public | implemented_public | implemented_public | sdk_reports.{rs,py,c} + binding_examples_binding_surface.md. |
| package metadata (`pkg.metadata`) | implemented_public | implemented_public | implemented_public | implemented_public | Cargo.toml / pyproject.toml / wellfriendpdf.h; honest capabilities via feature_report. |
| versioning and semver (`pkg.semver`) | implemented_public | implemented_public | implemented_public | implemented_public | ENGINE_VERSION + REPORT_ENVELOPE_VERSION + wellfriendpdf_version/wellfriendpdf_abi_version. |
| feature flags (`pkg.feature_flags`) | implemented_public | implemented_public | implemented_public | implemented_public | cargo features surfaced in feature_report capabilities. |
| platform matrix (`pkg.platform_matrix`) | partial_public | partial_public | partial_public | partial_public | builds on win-msvc validated here; full platform matrix is CI. Action: platform matrix is a CI/release-roadmap task concern. |
| CI packaging smoke (`pkg.ci_smoke`) | partial_public | partial_public | partial_public | partial_public | local maturin build + cargo build validated. Action: CI wiring is a release roadmap task. |
| ABI compatibility checks (`pkg.abi_compat`) | implemented_public | implemented_public | implemented_public | implemented_public | wellfriendpdf_abi_version + hand-maintained header; opaque handles keep ABI stable. |
| memory leak checks (`pkg.memory_leak`) | implemented_public | partial_public | implemented_public | partial_public | capi tests free every allocation; py returns owned objects. Action: valgrind/asan run is a CI concern (bounded). |
| release artifact manifest (`pkg.release_manifest`) | partial_public | partial_public | partial_public | partial_public | smoke JSON artifacts serve as manifest evidence. Action: formal release manifest is a release roadmap task. |
