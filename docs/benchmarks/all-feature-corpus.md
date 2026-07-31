# Wellfriend 5,044-PDF all-feature corpus evidence

This file is generated from compact VPS stage summaries. Raw PDFs and raw logs stay on the VPS.

- Corpus PDFs: 5044
- Overall status: pass
- Result directory: `evidence\all-feature-corpus\raw-summaries`
- VPS raw-summary source: `/mnt/wellpdf-block/results/all-feature-corpus-current`

| Stage | Files | Successes | Failures | Median ms | P95 ms | Artifact SHA256 |
|---|---:|---:|---:|---:|---:|---|
| info | 5044 | 5044 | 0 | 86.234 | 368.973 | `880a5e1b914ec6514370f4002acccb3901529529fb0b4563f120d6790027635a` |
| parser_report | 5044 | 5044 | 0 | 57.501 | 499.277 | `e0ca0716494adab063619c09e893dd2b6524183845c50782ee5f96955fbbe50b` |
| security_report | 5044 | 5044 | 0 | 25.046 | 72.022 | `a309d7a642f73365f30da42f39ca1fb71dac8313c51bf3eb63ff0ab76b3b4a18` |
| validate | 5044 | 5044 | 0 | 36.836 | 103.356 | `08fd9a33c236b3a7fd738f90a39749ceeee2b1596814c2c3b7100a346de9e7ae` |
| fonts | 5044 | 5044 | 0 | 24.337 | 55.416 | `c2514bf38b30eb62d159f297c45469f4ed611a8c7b5c558e2a5b386e48fcd446` |
| extract_text_structured | 5044 | 5044 | 0 | 67.512 | 175.205 | `2ff129584c12324484158b720454376deb8f7476257a5d296f431d519d2ee073` |
| parse_json | 5044 | 5044 | 0 | 151.544 | 507.433 | `6d99ec992bfa4ed01a1ce2165abcb75753e6938ac5132863908d5c25304aa83f` |
| extract_tables | 5044 | 5044 | 0 | 100.313 | 315.007 | `4c8047d5588f3b3102fd765a640ed3dacb8db8740c25dd8ed0435a30a6d6f47f` |
| forms_report | 5044 | 5044 | 0 | 18.607 | 37.391 | `5ec79869861a583bdd5879e8b25f076c7fd0bb1fe57646b1f76d3acb38fa78ef` |
| annotations_report | 5044 | 5044 | 0 | 19.533 | 45.504 | `b8d700d2f9509523dfe4d59e2db58e8805a2eed8a45e832f02bace75ffac96e6` |
| document_subsystems_report | 5044 | 5044 | 0 | 14.039 | 42.562 | `36e801f1d86b4c087bce0c36e3b438b76c3f2d1945679222d7584440f79707cf` |
| document_security_report | 5044 | 5044 | 0 | 13.641 | 42.179 | `1ac6d59d7e783a48e6bee463ef304adf84d61bd52979748a162727aedb84b4c6` |
| layout_analyze_page1 | 5044 | 5044 | 0 | 389.791 | 991.115 | `2bf5e3f5658e8a52a56b03eb6c9712bdc9ff4308225751d03c46af2f1d52fa7d` |
| reading_order_report | 5044 | 5044 | 0 | 972.761 | 5357.27 | `f16c79219f0b65a0df284d1bd1f3880bbba35b089341411f2ccd7d3e9b7fe98e` |
| flow_graph_report | 5044 | 5044 | 0 | 995.009 | 5388.723 | `7f3694391a77963ea56ba06e096feadd1d3e38841943f9d591f646daab563ac2` |
| document_subsystems_analyze | 5044 | 5044 | 0 | 1194.613 | 5464.166 | `252dbe4b19c8d140238d1d1e5d129605a3c785b2801acda5d0bbad5b0935cbaa` |
| document_security_analyze | 5044 | 5044 | 0 | 1305.186 | 5557.55 | `5d01dd049530f616cfa40ca01d250abf08156c434c3256ba84110c93a3ae7ecc` |
| render_compare_page1 | 5044 | 5044 | 0 | 100.243 | 341.289 | `537237d33581cd5154d7a73586856f073b58d8183bab57d4bb4c5177ec755751` |
| editing_smoke | 5044 | 5044 | 0 | see nested | see nested | `fc6b575c5019eac03a8a7b8ec41e19353f283944d6ffd7cb40b890b59a73a051` |

## Editing smoke scope

The editing smoke is read-only against the source corpus. It extracts page-1 text, builds a scene report, checks operator-preserving edit eligibility, and runs GeometricBlock reflow planning/report surfaces with temporary outputs. It does not overwrite corpus PDFs.

## Scope notes

- Visual rendering has a separate all-pages corpus campaign and a separate page-1 render-compare smoke in this file.
- Public semantic/document-subsystem report commands use bounded document-report scope so real-corpus runs return typed evidence instead of hanging on very large documents.
- Unsupported or unavailable edit targets are expected to return typed refusals; unclassified panics/timeouts/nonzero exits count as failures.
