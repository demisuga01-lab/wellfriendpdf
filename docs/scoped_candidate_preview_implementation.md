# Rendered native scoped-text candidate review

Status: source implementation only, on the accumulated uncommitted `main`
candidate based on `27e62db3a1b84804339e65b6025273fd003b3736`. This does not
complete the editor roadmap, prove compilation, certify visual fidelity, or
establish universal/Acrobat-beating editing.

## Transaction boundary

`universal_editing::scoped_preview::preview_scoped_candidate` accepts the whole
existing scoped-text plan and recomputes it against the supplied PDF. Every
serialized plan field must match; the recomputed private candidate's hash must
match the planner-owned execution hash. Denied, stale, changed or non-scoped
plans do not produce a preview. Preview never calls apply, creates approval, or
returns candidate PDF bytes.

Source and candidate pages render through the same native contract, including
annotation appearances and forms, at CropBox geometry and the default visible
optional-content state. Page count, boxes, rotation, UserUnit and output
dimensions must agree. All requested dimensions are budgeted before any raster
allocation. Output contains two opaque RGB PNGs, hashes of the PNGs and canonical
premultiplied RGBA buffers, font-substitution/contract telemetry, and pixel
differences (changed pixel count, maximum channel delta and top-left-origin
exclusive bounds). Difference tolerance affects the summary, not displayed PNGs.

Limits are eight distinct one-based pages, 24–600 DPI, 16 million combined
before/after pixels by default and an absolute 32 million ceiling, 16 MiB of
combined PNG bytes, and a 72 MiB SDK JSON limit. Empty page selection previews
the selected occurrence's page only. The output lists affected pages not shown;
conservative document-wide invalidation is not silently called full review.
Cancellation is polled during planning, rendering, differences and transport
hashes; PNG compression has bounded input and cancellation checks around the
codec call, not an immediate native-code interruption guarantee.

Compatibility mode can use renderer fallbacks. `require_exact: true` requests
the existing HighQualityExact contract, without retrying under a weaker mode.
Neither mode is an independent-render comparison. The preview is of the native
candidate **before** final normalization, encryption and conformance/edit-contract
gates. Apply still re-plans and checks approval; final gates can refuse. Hidden
appearance/optional-content states and unselected pages remain unqualified.

## Exposed routes

| Surface | Additive entry point |
|---|---|
| Rust SDK | `universal_editing_scoped_preview_v2_json(bytes, plan_json, options_json, password)` |
| HTTP | `POST /api/v2/universal-editing/scoped-preview`, existing multipart `file`, `plan_json`, optional `options_json` and `password` |
| C | `wellfriendpdf_document_universal_editing_scoped_preview_v2_json` |
| Java | `Document.universalEditingScopedPreviewV2Json` |
| .NET | `WellfriendPdfDocument.UniversalEditingScopedPreviewV2Json` |
| Python | `Document.universal_editing_scoped_preview_v2` |
| WASM | `WellfriendPdf.universalEditingScopedPreviewV2Json` |
| Worker client | `scopedSources`, `planScopedText`, `previewScopedText`, `applyScopedText` |

The report envelope kind is `universal_editing_scoped_preview_v2`; callers pass
the raw plan under the preceding planning envelope's `report`, not the envelope.
Native JSON transports carry PNG byte arrays. The SDK serializes the typed
envelope directly, avoiding an intermediate `serde_json::Value` per PNG byte.
The worker converts arrays to transferable `Uint8Array`s. HTTP uses the existing
timeout/cancellation/semaphore and output-size policy. No new deployment route
or separate authentication authority is introduced.

## Browser integration

`wellfriend-scoped-text-editor` is embedded in the story editor and is also an
exported standalone custom element. Both share the same `StoryWorkerClient` and
published PDF; the panel is not a second document store. It exposes native
occurrence selection, Unicode-scalar source ranges, replacement text, source
rectangle/mode/size, font-substitution permission and explicit annotation
metadata policy. The default clone-one workflow does not infer tagged semantic
migration or rewrite signatures. A subsequent explicit ordinary text-field
workflow coordinates all widget displays and the field value; see
`widget_text_transaction_implementation.md` for its bounded contract.

Planning and preview are read-only. Before apply, the worker and client both
require an exact plan/revision match to a completed preview in the current worker
epoch. The panel waits for both browser PNG decodes before enabling approval.
This is binding and display plumbing, **not proof that a human reviewed pixels**.
Apply creates the standard explicit decision/token and uses the canonical SDK
apply route. Only a successfully reopened session is published. Changed outputs
enter the existing bounded undo history; no-change outcomes add no undo entry.
Cancellation terminates the WASM worker and restores the last published bytes.

A loaded logical story draft blocks native apply in the combined editor. The
new explicit Clear story draft action confirms discarding the draft while
retaining saved PDF/stories. Hosts using the standalone panel must coordinate
their own unsaved drafts and authorization. The panel provides source-local
selection, not page-coordinate hit-testing or inferred reading order.

## Evidence and remaining work

Nine added regression functions remain **unexecuted**: three bounded
comparison/options helpers; three native AP plan/preview/apply, stale-plan,
combined-budget and SDK-envelope cases; and three worker-client receipt,
snapshot, cancellation, stale-queue, undo and no-change cases. The AP fixture
checks that replacing one shared appearance changes pixels only within that
occurrence and leaves the other appearance untouched. That assertion has not
been run and is not visual evidence. Browser tests use a fake worker, not WASM.

Rustfmt syntax/formatting and Git whitespace checks passed. No compiler,
typecheck, build, test, PDF workload, render, browser session, benchmark, generated
binding/header check, deployment, commit or push was executed. Native FFM,
P/Invoke, PyO3, C/header and WASM paths have source wiring, not runtime proof.

The full roadmap still includes broader tagged Form/vector ownership,
specialized/stateful field/AP coordination, direct appearance ownership, cross-program selections,
page hit-testing, mixed-object/global layout, richer typography and collaboration,
and exact-revision runtime/corpus qualification. See
`universal_editor_roadmap_tracking.md`; this increment is not closure of those
requirements.
