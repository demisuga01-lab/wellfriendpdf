# Wellfriend 5,044-PDF all-feature corpus evidence

This file is generated from compact VPS stage summaries. Raw PDFs and raw logs stay on the VPS.

- Corpus PDFs: 5044
- Overall status: pass
- Result directory: `evidence\all-feature-corpus\raw-summaries`
- VPS raw-summary source: `/mnt/wellpdf-block/results/all-feature-corpus-current`

| Stage | Files | Successes | Failures | Median ms | P95 ms | Artifact SHA256 |
|---|---:|---:|---:|---:|---:|---|
| info | 5044 | 5044 | 0 | 205.769 | 591.462 | `ea5ebc54f357be46d01c30daa322dab23199b26816fb9480a9840d617e455442` |
| parser_report | 5044 | 5044 | 0 | 156.225 | 844.899 | `ac3bce980c9eea88989203964c5aabd78461c8fc32ad6c4b0f6dcac5d2ea0aca` |
| security_report | 5044 | 5044 | 0 | 186.717 | 651.857 | `7b6fb6b97c181ccb1bee938baba73aaac058e5f8ed1009ce75f913f4b24e28f2` |
| validate | 5044 | 5044 | 0 | 34.639 | 157.136 | `c01d3d635031f023abeff565869492f2b217b6d81dcd356fff8343a6a6371f17` |
| fonts | 5044 | 5044 | 0 | 16.277 | 42.614 | `35becff19a70cd7f26c90a41b1803bbd1d539185972029b0062fa8cfcdf47f4e` |
| extract_text_structured | 5044 | 5044 | 0 | 68.878 | 187.937 | `0032c344634051e0120529c27473d2586929e6a2858db5863d1825d2ca039e31` |
| parse_json | 5044 | 5044 | 0 | 157.736 | 525.566 | `6c6a534ddb33970bb1b92b7a4505ed7beb76720f26703f9a7fa6bf0210fdb3b5` |
| extract_tables | 5044 | 5044 | 0 | 107.421 | 334.1 | `3626fa70b6027b8a28eb98d2d5b5a150d5d85fa5ddffcd3a74c9eab06f9db245` |
| forms_report | 5044 | 5044 | 0 | 10.777 | 24.188 | `e4fa8e22b6a5104884f48bef4b689ee0a198f5c714614279a1085afca1f24e9e` |
| annotations_report | 5044 | 5044 | 0 | 11.105 | 25.131 | `f03686d7a8408cd59800a73cf8b710292ba418426186191444c947c8d015c630` |
| document_subsystems_report | 5044 | 5044 | 0 | 9.146 | 29.686 | `60df2185728e0aec848cd7351b88c3bed1e0efa140ebab0611b92274b71c29ee` |
| document_security_report | 5044 | 5044 | 0 | 9.342 | 31.253 | `70b339448b5807959b0d9acbd3721898b70164f71d99fda3f38910e75f28f9c4` |
| layout_analyze_page1 | 5044 | 5044 | 0 | 413.538 | 1053.217 | `c16f54fae3f8ef2cdda262a4d5780b87d225b8c133037b340a513017785a894c` |
| reading_order_report | 5044 | 5044 | 0 | 154.243 | 578.285 | `056600fc0a6ef957ac80ed78f42642eaa4bcd9d1fc41cd7a17c8bd85afd7f8d0` |
| flow_graph_report | 5044 | 5044 | 0 | 167.808 | 587.753 | `f3c5e71f24db56cc7b1467e0a436a91373f0e3fc4601586b543e9231ece3af23` |
| document_subsystems_analyze | 5044 | 5044 | 0 | 284.71 | 947.766 | `6788b2e322be3ccf96fbddf0b62c541fe3ebc83052a2c49631f06b1e37876e60` |
| document_security_analyze | 5044 | 5044 | 0 | 395.073 | 1228.329 | `7de0b11a0d21b2f76ecefb9b30e941b5e1bd98d9f47409a3b7b0c60dee9181f8` |
| render_compare_page1 | 5044 | 5044 | 0 | 69.913 | 200.334 | `e89638295e33af1c68e6fe0287ccb0d854112a8f8f8ce22ead3dcbe39c491770` |
| editing_smoke | 5044 | 5044 | 0 | see nested | see nested | `fc97226ccb0062860d509afb5652b4fb64e95be7794ddefaf5df855fba8b8ff1` |
| source_operator_apply | 5044 | 5044 | 0 | 308.56 | 687.43 | `6c0f365c3448862b7a5464772d5c295862a59fa7c8a4a91d5144d920638e5351` |

## Editing smoke scope

The editing smoke does not overwrite corpus PDFs. It extracts page-1 text, builds a scene report, checks operator-preserving edit eligibility, runs GeometricBlock reflow planning/report surfaces, and attempts temporary output-producing edit paths where source evidence permits.

The `source_operator_apply` stage is a separate temporary-output corpus pass for operator-preserving text edits. Successful rows write edited PDFs in a temporary directory and record output/report sizes; unsupported source mappings remain typed refusals.

## Scope notes

- Visual rendering has a separate all-pages corpus campaign and a separate page-1 render-compare smoke in this file.
- Public semantic/document-subsystem report commands use bounded document-report scope so real-corpus runs return typed evidence instead of hanging on very large documents.
- Unsupported or unavailable edit targets are expected to return typed refusals; unclassified panics/timeouts/nonzero exits count as failures.
