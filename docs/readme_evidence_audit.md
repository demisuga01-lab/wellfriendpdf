# README evidence audit

Generated: 2026-07-29T18:06:28.4430549Z

## Starting state

- Repository: E:\wellpdfsdk
- Branch: main
- Starting commit: $head
- Expected README baseline: d346915de5125fccf3163847cb3ebec197c49046
- Worktree state at audit: clean and synchronized with origin/main per 	arget/readme-rewrite/readme-starting-state.json.

## Current README problems

The previous README was not suitable for the current true-editing release posture. It mixed older extraction/authoring product language with stale benchmark-style numbers and did not clearly distinguish current Prompt 31-36 evidence from historical or unmeasured claims. Old internal prompt-era reports and historical rename documents still contain legacy pre-Wellfriend branding references, but the rewritten public README must not use those names except for legitimate third-party crate names.

Stale-name scan count in root README: $staleCount.

## Verified current package and API names

- Rust workspace packages: see 	arget/readme-rewrite/code-surface-scan.json.
- CLI binary: wellfriendpdf from wellfriendpdf-cli.
- Engine crate: wellfriendpdf-engine.
- Python package/import: wellfriendpdf.
- C header/prefix: wellfriendpdf.h, wellfriendpdf_*.
- WASM package: @wellfriendpdf/wellfriendpdf-wasm.
- .NET package/namespace: WellfriendPdf.
- Java group/package: io.wellfriendpdf, artifact wellfriendpdf-sdk.
- Server crate: wellfriendpdf-server remains in the workspace.

## Evidence reviewed

- Prompt 36 local evidence directory: 	arget/prompt36-enterprise-validation/.
- Prompt 36 VPS evidence: /home/demisuga01/wellpdf/results/prompt36-20260729T063834Z.
- README competitor VPS evidence: $vps.
- Prompt 36 final verdict: implementation_status=complete, elease_posture=release_ready_with_limits, prompt36_complete=true.
- Prompt 36 maximum observed RSS: 6618920 KiB under a 33554432 KiB budget.
- Prompt 36 fuzz inventory: 43 targets built and smoke-run with 64 runs per target.
- Prompt 36 binding matrix: Rust/CLI/Python/C/WASM/.NET/Java Maven passed; Gradle was classified as an exact VPS host limit.
- Independent tools in Prompt 36: qpdf and Poppler ran; MuPDF, PDFium, veraPDF and some security/package tools were unavailable.

## Comparator evidence added for README

- 	arget/readme-rewrite/comparator-version-inventory.json
- 	arget/readme-rewrite/readme-direct-comparisons-qpdf-clean.json
- 	arget/readme-rewrite/pdfbox-pyhanko-smoke.json
- 	arget/readme-rewrite/pdfcpu-verapdf-attempts.json

These are narrow README-level evidence artifacts, not release certification and not a full corpus benchmark.

## Claim policy applied

README claims are restricted to:

- measured_directly for Wellfriend/comparator operations run on the same VPS fixture.
- alidated_in_repository for Prompt 31-36 code/tests/evidence and repository metadata.
- official_competitor_documentation for current official external documentation.
- inferred_limited only where the README says the claim is inferred.
- unavailable_or_not_measured for unavailable tools, unlicensed commercial SDKs, or non-equivalent operations.

The README must not claim universal Adobe parity, complete PDF support, overall fastest/best ranking, all-viewer appearance parity, universal dynamic-XFA conversion, or verified superiority over unavailable comparators.
