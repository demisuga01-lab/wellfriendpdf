# Retained native editing sessions — implemented, qualification bounded

Continuation over `27e62db3a1b84804339e65b6025273fd003b3736` in the existing dirty
`main` checkout. The engine, C ABI, Python and WASM crates compile; the .NET and
Java packages build; the WASM declarations type-check; the password-open engine
and C ABI regressions pass; the .NET and Java story-session suites execute
against the refreshed native DLL; all five Python story-session cases pass from
a freshly installed editable extension; and C-header parity covers the new ABI
symbol. This evidence
does not replace real encrypted-PDF binding, rendering or corpus qualification.

## Shared protocol

`engine::story_session_protocol` delegates to `LinkedStorySession`; it has no
separate parser for PDF content, layout engine, font subsystem or PDF writer.
The caller owns an in-memory session. Opening copies 1..=256 MiB of PDF bytes.
Password-aware open accepts exact permissions/owner credential bytes. A successfully unlocked
Standard-handler input is normalized once into an unencrypted canonical working
revision; every request and receipt binds that revision. Status reports the
original input hash, whether the source was encrypted, whether a decrypted
working copy was created, owner-vs-user authentication, the original permission
mask, whether content modification is permitted, and the invariant that the
password is not retained. User/open passwords are rejected for encrypted story
sessions because their exportable working revision no longer carries the source
encryption envelope. Publication also retains a defense-in-depth authority check.
Output encryption remains an explicit universal-editing publication operation.
JSON commands are UTF-8, at most 32 MiB (including all approved font data).
The command discriminator is `op`; unknown commands/command fields and malformed
JSON are rejected. Request bodies use the existing typed engine models.

| Command | Additional fields | Result |
|---|---|---|
| `status` | none | schema version 1, revision SHA, pages, byte length, bounded history counts/bytes, current receipt or null; line/shaping/pagination/tab-stop versions and saved-story schema maximum describe generated-line capabilities, while paragraph-style/structure/inline-style history and compaction protocol versions gate their newer collaboration routes |
| `saved_stories`, `pages` | none | saved story models or page geometries |
| `source_model`, `image_sources` | `page` (1-based) | exact revision's source spans or image occurrences |
| `annotation_sources`, `tag_sources` | none | existing canonical inventories |
| `inspect_font` | `bytes` (complete font, 1..=4 MiB) | source-hash-bound standalone/collection face catalog; no PDF mutation |
| `prepare_font` | `lookup_name`, `bytes`, `selection: {source_sha256, face_index, allow_signature_removal?}` | `{asset, report}` with standalone bytes for existing request `fonts`; requires explicit face and signature-removal decision; 4-MiB output budget checked before allocation |
| `page_geometry` | `page`, `dpi` | viewport dimensions and PDF-to-device matrix |
| `preview` | `request` | layout `preview`, exact `receipt`, `dirty_regions`; no PDF mutation |
| `synchronize_table_values` | `request` | recalculated draft at the current revision; not approval |
| `checkpoint` | `request`, `receipt` | canonical writer report; publishes only an approved transaction |
| `undo`, `redo` | none | boolean; successful transitions restore exact bytes and invalidate cached approval |
| `merge_text`, `merge_structure` | `request` | existing conservative merge/conflict report, not a checkpoint |
| `review_structure` | `request` | typed conflicts, branch alternatives, provisional/unplaced values and exact review hash; no PDF mutation |
| `resolve_structure` | `request`, `resolution` | acknowledged, scope-constrained logical draft; still requires native preview/checkpoint |
| `text_history_new` | `base` | empty causal text history and unchanged draft |
| `text_history_merge` | `base`, `histories` | canonical history union, missing dependencies and optional complete draft |
| `text_history_edit` | `base`, `history`, `edit` | exact-history/preimage-bound logical edit, not a PDF mutation |
| `text_history_style` | `base`, `history`, `edit` | fieldwise causal paragraph-style write or exact conflict resolution; no PDF mutation |
| `text_history_structure` | `base`, `history`, `edit` | stable paragraph insert/move/delete or exact presence/position conflict resolution; no PDF mutation |
| `text_history_inline_style` | `base`, `history`, `edit` | exact grapheme selection converted to stable atom-targeted field writes; no PDF mutation |
| `text_history_resolve_inline_style` | `base`, `history`, `resolution` | exact one-field resolution over every conflicting atom; no PDF mutation |
| `text_history_delta` | `base`, `history`, `peer` | history operations beyond receiver actor prefixes |
| `text_history_set_active` | `base`, `history`, `change` | same-replica selective undo/redo against an exact history/activity preimage |
| `text_history_set_many_active` | `base`, `history`, `change` | atomic same-replica undo/redo of 1..=4096 unique original edits against one exact preimage |
| `history_resume` | `story_id` | saved causal checkpoint verified against the current story owners |
| `history_prepare` | `source` | normalized Start/Resume source and optional complete logical projection |
| `history_join` | `source`, `histories` | canonical union including all previously persisted events |
| `history_edit` | `source`, `edit` | history/preimage-bound edit retaining original scalar identities |
| `history_style` | `source`, `edit` | durable paragraph-style write retaining the saved seed and explicit conflicts |
| `history_structure` | `source`, `edit` | durable paragraph-list write retaining the saved seed, identities and explicit conflicts |
| `history_inline_style` | `source`, `edit` | inline-style write against a verified Start/Resume seed; supported horizontal/vertical stories and table cells materialize through native preview/checkpoint |
| `history_resolve_inline_style` | `source`, `resolution` | logical exact inline-conflict resolution against a verified Start/Resume seed |
| `history_delta` | `source`, `peer` | bounded event delta after verifying the current checkpoint |
| `history_set_active` | `source`, `change` | selective text-edit activity retaining the saved epoch and other edits |
| `history_set_many_active` | `source`, `change` | atomic grouped activity change retaining the saved epoch and other edits |
| `history_preview` | `source` | native layout plus a receipt binding source, history and layout |
| `history_checkpoint` | `source`, `receipt` | native story and causal metadata published together; one session undo step |
| `history_compaction_plan` | `request` | verifies the exact saved checkpoint/history and complete acknowledged frontier; returns a destructive new-epoch plan hash without publishing |
| `history_compaction_apply` | `request`, `approved_plan_sha256` | recomputes the exact plan, publishes an empty replacement epoch and reports whether bounded exact-byte undo was actually retained |

For example: `{"op":"source_model","page":1}`. For saving, embed the exact
request object and the `receipt` object returned by `preview` in a `checkpoint`
command. Do not silently regenerate approval after changing a draft.
The receipt binds intent/revision/layout; the integrating app must authenticate
the user and obtain their decision. It is not an authorization credential.
Structural resolution also preserves automatic changes, retained dependencies
and original tag ownership. It cannot merge source/permission authority or use
a resolution hash as a layout receipt. See
`story_structure_resolution_implementation.md` for the typed scope and review UI.
The history commands share this authority boundary; see
`story_text_history_implementation.md` for causal, Unicode, epoch and resource
limits. They do not merge PDF bytes or bypass preview/checkpoint approval.
Schema-5 inline drafts may be prepared, joined, edited, resolved and exported.
Supported horizontal/vertical stories and table cells use the shared style-run
layout/writer and verified checkpoint/reopen path. See
`story_inline_style_history_implementation.md`.
Saved-story schema 6 retains exact undecorated tab-stop settings and positioned
fields; schema 7 adds artifact-only leaders and bar rules; schema 8 adds exact
multi-character decimal tokens. The status response
advertises `tab_stop_layout_version: 3` and `saved_story_schema_max: 8`.
Nondefault undecorated settings require durable history seed schema 4 and
decorated settings require seed schema 5; multi-character decimal settings
require seed schema 6, advertised as
`history_seed_schema_max: 6`; default-tab stories retain seed schema 3. Clients
must not infer these capabilities from the envelope's independent
`schema_version` field. See
`tab_stop_layout_implementation.md`.
The `text_history_*` API remains bound to its original revision. To continue
across native saves, use `history_*` as specified in
`story_history_checkpoint_implementation.md`. Resume uses verified saved markers,
not old logical ranges or matching strings. Ordinary edits retain but can detach
the saved causal history. A new epoch requires explicit replacement approval.
Selective controls upgrade history to schema 2. Single-target and grouped
controls are not whole-PDF byte undo and do not reset a replica's sequence; see
`story_selective_undo_implementation.md`.

For an explicitly retired saved epoch, call `planHistoryCompaction` with the exact
checkpoint/history hashes and complete actor frontier, review the returned counts
and plan hash, then call `compactHistory` with the unchanged request and exact hash.
The supplied panel requires separate acknowledgements for operation/selective-undo
loss and prior-replica retirement. Compaction starts a new seed with an empty log;
old-epoch histories cannot rejoin it. It is not automatic garbage collection,
authentication, file-size reduction, sanitization or redaction. Exact byte undo is
reported only when the bounded session actually retains the preimage. See
`story_history_compaction_implementation.md`.

The input buffer and exported PDF buffer are independent copies. No API here
opens URLs, writes paths, commits files or deploys anything. A caller can inspect
or download bytes only after the native transaction has published successfully.

The shared story request now has optional `prune_empty_pages` (default false).
Preview/checkpoint reports include `page_pruning` with input removal numbers,
retained reasons and final output page count. It participates in the same exact
receipt; enabling it requires a new preview/approval. See
`story_page_pruning_implementation.md` for the owner and live-reference checks.

## Native and managed ownership

The C header exports `wellfriendpdf_story_session_open`,
`wellfriendpdf_story_session_open_with_password`, `command_json`, `bytes`,
`render_page_png` and `free` with the common `wellfriendpdf_story_session_` prefix.
JSON output is a length-delimited `WellfriendBuffer`, not a NUL-terminated string.
Release outputs with `wellfriendpdf_buffer_free`, errors with
`wellfriendpdf_error_free`, and the session once with `session_free`.
All session use and destruction must be externally serialized. Every pointer
must remain live during its call; raw-pointer validity cannot be checked by C.
Output pointers are validated before mutation and initialized empty on failure.
Do not supply a buffer whose previous allocation has not been freed.

An optional existing `WellfriendRenderCancellation` handle is reused as the
engine's general cooperative token. Another thread can signal it while a call
executes, but the handle must not be freed during that call. Checkpoint and
undo/redo check cancellation before publication. There is deliberately no
post-success cancellation exception that could tell the caller a saved edit
failed. Cancellation is not guaranteed to interrupt every third-party codec call.

Caught unwinding panics poison a C session; close it and reopen known bytes.
This does not recover an aborting panic, out-of-memory abort or invalid pointer.
The boundary follows Rust's distinction between unwind and abort behavior;
[Rust FFI guidance](https://doc.rust-lang.org/nomicon/ffi.html) is not evidence
that this implementation has passed runtime validation.

- .NET `StoryEditSession` uses a `SafeHandle`, serializes calls/disposal, copies
  results and accepts `CancellationToken`. Registration is disposed before its
  native cancellation source. Pre-cancel throws `OperationCanceledException`;
  cancellation observed by native code returns the normal `WellfriendPdfException`
  with its native message/status. Successful checkpoints are not reclassified.
  [SafeHandle documentation](https://learn.microsoft.com/en-us/dotnet/fundamentals/runtime-libraries/system-runtime-interopservices-safehandle)
  describes its P/Invoke lifetime role; the class adds the session lock separately.
  `Open(..., string)` wipes its temporary UTF-8 encoding;
  `OpenWithPasswordBytes` preserves exact caller-supplied password bytes.
- Java `WellfriendPdf.StoryEditSession` follows the repository's JDK 25 FFM
  binding and explicitly requires a 64-bit ABI. Session use/close is confined to
  the creator thread. `RenderCancellation.cancel()` may run on a different thread.
  Temporary arenas are closed after native calls and owned outputs copied/freed.
  New session symbols are resolved lazily, so existing document-only calls do
  not acquire a dependency on these symbols. Unpaired UTF-16 surrogates are
  rejected before UTF-8 transport, not replaced with different text.
  `openWithPasswordBytes` is the exact-byte alternative; temporary native
  credential memory is overwritten before its confined arena closes.
  [JDK 25 Arena documentation](https://docs.oracle.com/en/java/javase/25/docs/api/java.base/java/lang/foreign/Arena.html)
  describes the confined-arena lifetime/thread rules used here.
- WASM exports `openWithPassword(bytes, password)`, `commandJson` and shares the same protocol for its named preview,
  checkpoint, table-sync and merge methods. Open and explicit rendering also use
  the common helper. Existing worker cancellation remains worker termination;
  this change does not create shared-memory browser cancellation.
- Python `StoryEditSession(pdf_bytes, password=...)` accepts an exact Python
  `bytes` credential and exposes the same transport and named
  preview/checkpoint/history methods through PyO3. Detached native work uses a
  nonblocking session mutex and the existing shared cancellation token. See
  `python_story_sessions.md` for lifecycle and source-only evidence boundaries.

Rendering occurs only on a host's explicit request. It uses the canonical
render contract, bounded to 300 DPI and 16 million pixels. Exposing this function
is not evidence that its output has been rendered or compared in this change.

## Verification and remaining work

Regression source covers engine receipt rejection/checkpoint/reopen/exact
undo-redo; malformed commands and cancellation; C buffer ownership, output and
length checks, cancellation and panic poisoning; .NET lifecycle/cancellation;
Java lifecycle, Unicode and cross-thread rejection. The password-open engine
case, C ABI case, .NET cases, Java suite and Python suite have executed
successfully against the refreshed local artifacts. These focused cases do not establish a full
managed edit/save/reopen/render workflow; the engine/C cases target that source
path separately. A complete binding acceptance suite is still required.

Owner-credentialed Standard-handler encrypted retained sessions are implemented by
creating an explicitly unencrypted working revision without retaining the
credential. Public-key encrypted story-session open, encryption-envelope
preservation, and automatic output re-encryption are not implied. Existing core
security and signature decisions remain in force; this API is not a bypass. High-level
typed Java/.NET story models, native editing UI,
production application wiring and full binding/runtime/corpus qualification
remain work. Later continuations add same-Figure-owned tagged OCR and exact
per-span partitions across explicitly selected content-only sibling owners
through the existing request protocol; see `tagged_ocr_implementation.md` and
`separate_ocr_owner_implementation.md`. Other editing/rendering limits remain in
`universal_editor_roadmap_tracking.md`.

Checks completed for this continuation include `cargo fmt --all`; `cargo check`
for engine, C ABI, Python and WASM crates; .NET build; Java Maven package; WASM
TypeScript type-check; C-header parity; the focused encrypted engine/C tests;
.NET and Java native story-session tests; and all Python story-session tests.
They establish local source/type/runtime integration and ABI declaration parity—not
browser execution, independent
rendering, encrypted save/reopen interoperability or corpus-wide correctness.
