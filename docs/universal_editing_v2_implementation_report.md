# Universal editing v2: source implementation and VPS qualification plan

Latest uncommitted source continuation on `main`:
[browser sessions, structural merging and annotation anchors](story_workflow_continuation.md).
Further source additions include [table pagination and explicit tagging](paginated_table_implementation.md).
The [full roadmap](universal_editor_roadmap_tracking.md) is not complete. Latest
checks are Rust parsing/formatting, JavaScript syntax and whitespace only; no
compiler/build/tests/PDF workloads/rendering/benchmarks/deployment were run.
The historical branch/phase notes below are not current build or release evidence.

Date: 2026-09-16
Schema: `universal_editing.document-transaction.v2`
Branch: `universal-editing-v2`

## Verdict

The additive v2 source implementation is present. It replaces broad
best-effort editing with a revision-bound analyze, plan, approve, and apply
transaction that can target exact text, image, vector, and semantic-structure
occurrences. Renderer qualification is exposed through the same Rust engine,
CLI, HTTP, C, Python, WASM, .NET, and Java surfaces.

This is **not** a claim that the SDK edits every possible PDF or has Adobe
Acrobat parity. At the owner's direction, no build, test, formatter, benchmark,
fuzzer, reference render, validator, or PDF workload was run for this change.
Every v2 capability therefore reports `source_implementation` separately from
`qualification_status`; renderer status can only become
`source_plan_eligible_for_vps_qualification` until the external campaign is
performed.

## The eight code-level closures

| # | Gap | Implemented source behavior | Principal code |
|---|---|---|---|
| 1 | No canonical occurrence identity | Revision-bound scene and occurrence graphs retain page, owner stream object/generation, decoded byte interval, logical Unicode-scalar range for contiguous multi-run text, invocation path, transform, writing mode, and stable ID. Partial-token and cross-`/Contents` source edits split only at complete CMap scalar boundaries. Selected codes are removed from the current reachable stream revision and replaced by exact numeric `TJ` displacement so following source positions stay fixed. Appended Type0 visuals carry logical `/ActualText`; clipping replacements are instead injected inside the original `BT`/`ET` scope. Inline-image payloads remain opaque during token walking. Content operands preserve PDF `null`, including sparse per-filter `/DecodeParms` arrays, through tokenization, parsing, display-list inspection, rendering, mutation, and serialization. | `crates/engine/src/universal_editing.rs`, `advanced_editing.rs`, `content/tokenizer.rs`, `editing_transactions.rs` |
| 2 | Image editing refused or confused definitions with uses | Exact image occurrence inventory with document-wide definition-sharing counts; page CTM and `q`/`Q` state carried across ordered `/Contents` members; definition-wide edit-all; clone-on-write for one page/Form invocation; selected inline-image promotion to an owned XObject; typed Device/calibrated/ICC/Indexed/Separation/DeviceN resource construction; bounded payload/header and mask validation; explicit one-bit stencil-mask and `/Decode` polarity replacement; and explicit DeviceGray soft-mask replacement for RGBA/JPX-alpha edits. | `universal_editing.rs`, `images/decoder.rs`, `source_editing.rs` |
| 3 | Reflow was too local or heuristic | Grapheme-safe source mapping, Unicode line-break candidates, bidi analysis, shaped advances, bounded paragraph dynamic programming, Cassowary feasibility constraints, region/column/page flow, visible discretionary hyphens with logical `ActualText`, and ordered continuation-page insertion. | `text_reflow.rs`, `advanced_editing.rs`, `writer.rs` |
| 4 | Unsupported fonts caused unconditional refusal or fake substitution | Embedded source OpenType program/metrics recovery where possible; deterministic metric-and-coverage ranking; caller approval of one exact lookup name or exact supplied font asset; approved bytes drive shaping, measurement, generated Type0 resources, and continuation pages. The writer emits GID-preserving TrueType subsets or license-gated full OpenType/CFF1 programs through `FontFile3 /OpenType`; conflicting repeated CID-to-Unicode mappings are collapsed deterministically and logical `/ActualText` remains authoritative. | `advanced_editing.rs`, `editing_transactions.rs`, `text_reflow.rs`, `universal_editing.rs` |
| 5 | Ambiguity was a terminal product error | `preview_and_confirm` returns exact candidates and a revision-bound decision contract. `automatic_exact_only` executes only a unique match and is a typed no-change policy denial for duplicates. Plan-state precedence is monotonic, so later approvals cannot downgrade a denial or irrecoverable result. | `universal_editing.rs`, `source_editing.rs` |
| 6 | Renderer claims were disconnected from executable closure | Qualification compiles the retained plan, enumerates unsupported operators, forbids silent fallback, executes native RGBA rendering, hashes pixels, and can compare independently supplied RGBA references using white-composited visual-channel MAE/RMSE/maximum error, alpha error, exact-pixel ratio, bounded local-window luminance SSIM, and global SSIM diagnostics. | `universal_editing.rs`, `sdk.rs`, C/Python/WASM/.NET/Java bindings, CLI and HTTP route |
| 7 | Reports could imply completion without evidence | Machine-readable capability, input-recovery, signature, conformance, implementation, dirty-region, inverse, and qualification reports distinguish source presence from external proof. | `universal_editing.rs`, `sdk.rs` |
| 8 | Security/release policy was implicit | Two signature modes are explicit; full rewrite requires authorization and invalidation acknowledgement; deterministic repair is commit-ordered and strict-reopened before return; requested standards profiles are checked before bytes are released; optional AES-256 Standard-handler output re-encryption uses apply-only credentials and a credentialed reopen; no-change encrypted inputs return their exact original transport; full-rewrite copying rejects dangling or stale-generation references; dependency and VPS gates remain machine-visible. | `universal_editing.rs`, `secure_mutation.rs`, `structural.rs`, `writer.rs`, canonical writer and capability registry |

## Seven post-audit correctness closures

The later source audit found seven concrete defects rather than qualification
gaps. Their source remedies are now present, but remain unexecuted:

1. Appended page text no longer assumes the graphics state left by the prior
   `/Contents` member. A synthetic leading `q` saves page-entry state before
   existing content; the appended member restores that state with `Q` and then
   enters its own balanced `q`/`Q` scope. This isolates page-coordinate text
   from preceding CTMs, clips, alpha, blend modes, and colours on well-formed
   graphics-state stacks.
2. The page-logical text scanner carries direct and named marked-content
   ownership across `/Contents` members. Direct isomorphic `/ActualText`, plus a
   non-isomorphic carrier when its complete glyph-owned source range is selected,
   is retained as a provenance-bound cleanup patch and seeded into every
   destructive, generated-font inline, and source-font inline edit map before
   that map is materialized. The glyph replacement therefore cannot overwrite
   or discard logical-text cleanup. Partial non-isomorphic or shared
   named-property ownership is refused with a typed semantic conflict; the
   legacy single-token writers also refuse these carriers instead of leaving
   stale search/copy text.
3. Page overflow is chunked by the proven line capacity and creates as many
   ordered continuation pages as needed. Every chunk is width-checked and every
   baseline is checked against the target region before `FitAfterPageFlow` is
   reported.
4. Multi-run operands retain `Arc` references to one source object and one
   decoded buffer per content stream. A mutable buffer is cloned only once per
   stream at commit, eliminating the prior stream-size multiplied by operand-
   count memory growth; the selected span count is also bounded.
5. Universal HTTP workers install their request cancellation token in a
   panic-safe engine scope. Analyze, plan, source scans, lossless filter loops,
   predictors, reconstruction, cancellable Flate serialization, native render,
   raster comparison, apply, reopen, and validation poll that token. Third-party
   codecs without an interruption callback still stop at the nearest governed
   boundary rather than through unsafe thread termination.
6. Full justification now adds the computed word and character spacing to each
   subsequent explicit glyph coordinate. Emitting `Tw`/`Tc` alone could not
   affect a following glyph whose position was overwritten by an absolute `Tm`.
   Alignment, including the RTL right-edge anchor, uses the final painted width
   after those spacing contributions rather than the natural glyph width.
7. Visible scan reconstruction detects intersecting invisible text. It requires
   the caller to bind the exact reviewed page-logical range, proves every source
   span covering that range has invisible render mode `Tr 3`, and records the
   revision-bound span identities. After the image clone/write it removes
   selections from highest to lowest logical offset and refreshes each pending
   binding against the current incremental revision immediately before that
   deletion. Stream-byte offsets may therefore move when an earlier metadata
   value changes length. An explicit `/ActualText null` is treated as an absent
   dictionary entry, so clearing a shared carrier does not poison the remaining
   words as unresolved. Each edit report must target the freshly bound invisible
   spans, and the original span content must no longer be reachable before
   visible replacement text is appended. A visible duplicate with identical
   Unicode cannot satisfy this proof. If an intersecting searchable layer is not
   unambiguously identified, the entire operation returns no output.

Scanned-page editing has two typed document-subsystem operations. Searchable
OCR retains the original scan and writes invisible Type0/CID text. Visible OCR
reconstruction takes a reviewed image occurrence plus word rectangles, maps
the rectangles through the inverse occurrence matrix, removes the original
glyph pixels with a bounded four-colour SOR solution of the discrete Laplace
equation, clone-writes the selected image occurrence, and adds visible shaped
Type0 text with `/ToUnicode` and logical `/ActualText`. If an existing invisible
OCR carrier intersects a replacement rectangle, the request must also bind its
exact page-logical scalar range; the carrier and visible pixels are then
mutated in one externally atomic operation. Inline-image decoding
retains `/Decode`, sparse per-filter `/DecodeParms` (including `null` entries),
the complete normalized BI dictionary, and active page/Form named color spaces.
CCITT and JBIG2 decoded samples re-enter the same dictionary-aware image path,
so non-stencil `/Decode` semantics are not bypassed. Recognition remains
provider-owned because text absent from the PDF
cannot be recovered with certainty from source bytes alone.

## Algorithms and invariants

### Exact occurrence graph

For every paint instruction, the walker stores an immutable source identity

`I = H(revision, page, owner object, generation, decoded [start,end), resource, invocation path)`.

Nested Form transforms are composed as affine matrices. If
`M = [a b c d e f]`, a point is mapped to
`(a*x + c*y + e, b*x + d*y + f)`. The four transformed unit-square corners
define the image occurrence bounding box. The invocation path prevents a
shared resource definition from being mistaken for a unique visible use.

### Clone-on-write resource DAG

An edit-all operation replaces the selected indirect image definition. A
clone-one operation allocates a new leaf, rewrites the selected owner content
stream, then clones only the resource/Form/page ancestors needed to reach that
one invocation. This preserves unrelated uses and creates a concrete clone
report for cache invalidation and undo.

### Layout and shaping

The line pipeline is:

1. preserve extended grapheme boundaries;
2. generate legal/tailored Unicode line-break opportunities;
3. resolve paragraph direction and shape uniform font/script/language runs;
4. measure actual positioned glyph advances;
5. minimize bounded paragraph badness over legal candidates;
6. solve required and soft region constraints with an incremental linear
   constraint solver;
7. commit only the approved source occurrence and revalidate postconditions.

This division follows the upstream responsibilities: Unicode UAX #14 provides
break opportunities but leaves line fitting/optimal selection to higher-level
software; HarfBuzz shapes a uniform run into positioned glyphs but deliberately
does not provide bidi, line breaking, or paragraph layout. Cassowary supplies
an incremental dual-simplex method for required and preferential linear layout
constraints.

### Font substitution ranking

For each governed candidate face, the implementation computes:

`score = 0.40*C + 0.25*exp(-3*abs(ln(A_t/A_c))) + 0.15*V + 0.12*F + 0.08*S`

where `C` is Unicode coverage of the replacement, `A` is mean advance in em,
`V` combines ascender/x-height similarity, `F` is family-class similarity, and
`S` is style similarity. Ties use the stable lookup name. When an embedded
OpenType program is parseable, its own `cmap`, `hmtx`, `OS/2`, and `hhea`
metrics define the target; otherwise the report labels the deterministic family
proxy. Ranking never silently authorizes a face: the chosen lookup name is
bound to the selected text candidate in the approval token and its exact bytes
are reused throughout apply. Identical text occurrences with different source
fonts receive separate metric rankings.
Candidates with less than complete replacement-text coverage remain diagnostic
rankings and cannot be approved.

### Image replacement safety

Raw and Flate sample lengths use PDF row packing:

`expected_bytes = ceil(width * components * bits_per_component / 8) * height`.

JPEG metadata is read before commit. JP2 `ihdr` and raw J2K `SIZ` headers are
parsed under bounded scans. Declared dimensions, components, and bit depth must
agree with the encoded payload. Stale decode arrays/parameters are removed. A
typed descriptor constructs the complete color-space graph and owns any ICC
profile, indexed lookup, tint transform, or DeviceN attributes needed by the
new image. Soft masks with a different raster geometry require an exact
caller-supplied replacement object; color-key masks are channel-validated and
rescaled when bit depth changes.

### Transaction and approval integrity

The plan ID covers input revision, requested operation, executable operation,
and policy. Approval covers plan ID, revision, exact candidate selection, font,
signature mode, and explicit acknowledgements. Apply recomputes the canonical
plan from the requested operation and policy, requires the complete supplied
plan to match it, then recomputes the decision digest; stale, forged, or altered
input is rejected. The content digest prevents accidental/tampered transport
mismatch; it is not itself an authentication credential. The server now adds a
separate, short-lived HMAC-SHA-256 wrapper at the authenticated preview/apply
endpoints. That wrapper binds the complete content receipt, public key id,
audience, issuance and expiry; apply verifies it in constant time before
re-running the complete document-level contract. In-process bindings retain
the unsigned content receipt because their host is already the trust boundary.

### Signatures and recoverable input

PDF incremental updates append changed objects and leave earlier bytes intact,
which is why the preserve mode routes through signature/DocMDP policy and the
incremental writer. “Preserve” means obey the permission and byte-prefix policy;
it does not claim that an arbitrary content change remains cryptographically
valid or acceptable to every signer.

For a permissively readable but strict-open-failing document, repair is allowed
only when `allow_deterministic_repair=true` and mode is `authorized_rewrite`.
The operation mutates in memory, performs deterministic full normalization,
strict-opens the result, and only then returns bytes. Hostile, credential-locked,
resource-exhausting, and irrecoverable byte streams remain terminal non-edits.

## Public v2 surface

| Stage | Rust/JSON operation | Result |
|---|---|---|
| Capabilities | `universal_editing_capabilities_v2` | Machine-readable source and qualification registry |
| Analyze | `universal_editing_analyze_v2` | Revision, scene graph, image occurrences, recovery contract |
| Inspect object | `universal_editing_inspect_object_v2` | Reversible low-level object value plus revision-bound fingerprint for governed object-graph mutation and stream re-encoding |
| Render qualify | `universal_render_qualification_v2` | Retained-plan eligibility, unsupported-op inventory, native RGBA pixels/hashes, and optional independent-reference comparison |
| Plan | `universal_editing_plan_v2` | Immutable operation, candidates, preview requirements, read/write set |
| Approve | `universal_editing_approval_v2` | Revision-bound decision token |
| Apply | `universal_editing_apply_v2` | PDF bytes plus mutation/invalidation/inverse report, or typed no-change |
| Secured apply | `universal_editing_apply_v2_with_output_credentials` | Full-rewrite AES-256 Standard-handler re-encryption using apply-only credentials that are excluded from plans and reports; byte-oriented bindings accept embedded NUL and non-text credentials |

CLI equivalents are `universal-edit-capabilities`,
`universal-edit-analyze`, `universal-edit-inspect-object`,
`universal-render-qualification`, `universal-edit-plan`,
`universal-edit-approve`, and `universal-edit-apply`. Secured CLI apply reads
exact password bytes from `--output-user-password-file` and the optional
`--output-owner-password-file`; credentials are not accepted in plan JSON.
The C ABI also exposes a length-delimited credential entry point. .NET and Java
route their string conveniences through byte overloads and wipe temporary UTF-8
arrays; Python and WASM already pass byte sequences. The original NUL-terminated
C symbol remains only as a source-compatibility surface.
Password-opened C/.NET/Java, Python, and WASM document handles retain only a
zeroizing input credential copy so analyze, inspect, plan, qualify, and apply
can reparse the same immutable encrypted bytes. That input secret is never
promoted to an output credential. Exact binary-password open entry points are
available in C, .NET, Java, Python, and WASM.
The same retained credential is now forwarded by every Python/WASM SDK-backed
document route whose engine facade accepts an input password, including direct
and multi-run text, vector/source, scene transactions, typed tables, reflow,
document subsystems/security, standards, XFA, signatures, semantic reports,
writer operations, redaction, associated files and sanitation. Every
document-dependent C SDK facade route now also forwards it instead of reopening
encrypted source with an implicit empty password. The four remaining users of
the older C report helper are document-independent text mapping, shaping,
subsetting and substitution utilities; they do not parse the handle's PDF.
Output encryption remains an explicit separate credential path. The
revision-bound paint-partition transaction is
source-wired across Rust SDK, C/header, .NET, Java, Python, WASM/TypeScript and
HTTP for propose, canonical private preview, ordinary apply and reviewed apply.
Optional shaping font bytes require an exact SHA-256 in the approval. Preview
returns bounded same-engine PNG evidence without candidate PDF bytes and a
compact publication receipt; reviewed apply recomputes the candidate and
withholds output when any bound input/request/proposal/approval/font/evidence/
output identity differs. For remote use, the additional
`preview-authenticated` and `apply-authenticated` HTTP routes issue and verify a
bounded-lifetime HMAC-SHA-256 wrapper using a dedicated server-only key. The
server always signs with one active key and can retain at most eight explicitly
configured, verification-only grace keys during rotation. Configuration Debug
output reports only key counts/availability and never API or HMAC key bytes.
The same receipt authentication primitives are exposed for trusted native hosts
through the C header, .NET P/Invoke, Java FFM, Python and static WASM/TypeScript
surfaces; browser-delivered code is explicitly instructed not to carry the
server key. The plain receipt remains intentionally unauthenticated, and
the browser-safe remote client accepts only the API credential, validates the
exact envelope/receipt relationship and parses the final multipart PDF as
bytes. The C public header now declares every explicit Rust export and every
native symbol imported by .NET or Java; a source-only parity guard prevents
that ABI surface from silently drifting again. .NET and Java also expose typed paint-partition workflow models
alongside their existing raw JSON methods; TypeScript publishes the canonical
wire shapes.
Neither preview mode
is independent-renderer qualification.
The HTTP route treats input and output password multipart bodies as bounded
opaque bytes and zeroes its owned buffers on drop. The routes are under
`/api/v2/universal-editing/`.

## Deliberate non-universal boundaries still requiring engineering or proof

These are not hidden behind the word "universal":

- No current-build or runtime proof exists for this change because execution
  was prohibited. Syntax, type correctness, binding linkage, and mutation
  behavior are pending the VPS gate.
- Arbitrary Type 3 character programs and every exotic CFF2, variable, color,
  AAT, or Graphite source program cannot be reconstructed as arbitrary new
  Unicode merely from appearance. Text replacement can substitute an approved
  embeddable Type0 font; generated TrueType uses a GID-preserving subset and
  standalone OpenType/CFF1 uses a full `FontFile3 /OpenType` program. The route
  enforces editable/installable OS/2 embedding permissions, but it cannot infer
  unknowable source semantics or manufacture a missing licensed font.
- Page-owned contiguous text is targetable by exact page-logical scalar range,
  including partial string tokens and selections crossing `/Contents`
  streams. A boundary must coincide with a complete source CMap mapping; one
  PDF code that maps to several Unicode scalars cannot be split without
  inventing source bytes. Selected codes are absent from the current reachable
  stream revision; a numeric `TJ` item preserves their horizontal or vertical
  advance. Horizontal, bidi, and upright zero-offset vertical clipping
  replacements stay inside the original text object and restore the source
  writing-axis endpoint, so downstream paint sees the new clipping outlines.
  Existing-font CMaps are reused when exact; missing or ambiguous mappings
  route through approved shaped Type0 substitution.
  Direct `/ActualText` that exactly mirrors its glyph scope, or whose complete
  non-isomorphic glyph-owned source range is selected, is removed from the
  current logical source transaction before replacement, preventing an old
  logical value from overriding edited glyphs on save/reopen. A partial range
  inside a non-isomorphic carrier (for example a ligature or pronunciation
  expansion), and `/ActualText` stored in a shared named `/Properties` object,
  does not provide a unique partial glyph mapping; it requires an explicit
  semantic-range or object-graph decision and otherwise fails closed.
  Tagged replacements remain inside their original `BDC`/`BMC` nesting rather
  than relocating an MCID to appended content. Partial operands, unselected
  sibling text, nested marked content, and distinct owner streams therefore
  retain their existing MCID and ParentTree identity. Generated fonts used by
  source-inline Form edits are installed in each selected Form resource
  dictionary as well as the page dictionary.
  All touched streams are written atomically. Form-owned selections still
  require an explicit occurrence ownership decision because shared Forms can
  paint on multiple pages.
- Replacement images accept typed DeviceGray/RGB/CMYK, CalGray, CalRGB, Lab,
  ICCBased, Indexed, Separation, and DeviceN graphs. This does not infer an
  unknown source profile or tint transform: the caller must supply exact ICC,
  lookup, function, and attribute data when the file does not contain it.
  Replacement alpha can be supplied as an explicit same-size DeviceGray soft
  mask. Visible scan reconstruction splits decoded RGBA/JPX-alpha data into a
  DeviceRGB image plus `/SMask`; it never reinterprets alpha as CMYK.
  One-bit stencil replacements omit `/ColorSpace`, retain their paint-through
  semantics, and require explicit `[0 1]` or `[1 0]` polarity when `/Decode` is
  supplied. Ordinary replacement `/Decode` arrays are finite and sized to the
  declared component count.
- A visual image can be painted through patterns, shadings, transparency groups,
  soft masks, optional content, annotations, or appearance streams. The new
  occurrence editor covers page/Form image XObjects and inline images; the
  vector and appearance subsystems retain their own typed routes.
- Inline-image payloads are kept opaque after the content tokenizer identifies
  them, but ISO PDF uses a delimiter-based `EI` terminator. Binary payloads
  containing delimiter-like byte sequences still need differential corpus and
  malformed-input qualification before this boundary can be considered proven.
- Visible scanned-word reconstruction is an approved deterministic restoration,
  not recovery of unknowable original pixels. Harmonic inpainting propagates
  the observed boundary into reviewed word masks; document texture, handwriting,
  or graphics crossing a rectangle may require a richer caller-supplied mask or
  replacement image for acceptable visual fidelity. A pre-existing invisible
  OCR layer is never silently left behind: an intersecting layer requires the
  exact reviewed logical range and is source-deleted before visible replacement;
  ambiguous or overlapping searchable ranges are refused without output.
- Semantic overflow can reuse existing regions, columns, and following pages.
  When continuation pages are required, the line-capacity loop inserts every
  required page at the ordered boundary, checks each line against its region,
  preserves the source page geometry, keeps existing page-leaf references
  stable, updates every page-tree ancestor count, and shifts page-label
  number-tree indices. Meaning-level workflows that store page
  numbers outside standard page references still require an explicit
  document-subsystem or object-graph mutation.
- `structure_correction` validates a current semantic node, governs the
  source-linked reflow, and can execute ParentTree/StructParents repair. The
  low-level object-graph operation can replace exact custom structure objects,
  but it deliberately does not guess author intent, reading order, or alternate
  text that is absent from the source.
- Requested PDF/A, PDF/UA, and PDF/X profiles now execute built-in final-byte
  validators and withhold failed or inconclusive edited bytes. External
  accredited validators and human meaning-level accessibility review remain
  release evidence; the built-in subset is not certification.
- A v2 plan can explicitly request Standard-handler output encryption and bind
  its algorithm, permissions, and metadata policy. Apply-only credentials drive
  edit, full-rewrite encryption, credentialed reopen, and report finalization
  without serializing passwords. Public-key recipient rotation remains the
  separate PubSec workflow; PDF/A and PDF/X correctly conflict with encryption.
  RC4-128 and legacy AES-128 output are deliberately policy-denied until their
  known crypt-filter interoperability deviations are removed and independently
  qualified; they are not advertised as safe universal output modes. AES-256
  user and owner passwords longer than the ISO 127-byte effective limit are
  rejected instead of being silently truncated beneath the plan contract.
- Password-opened Standard-handler input is carried across analyze, inspect,
  plan, qualify, and apply by zeroizing handle state or an explicit request
  credential. Public-key encrypted input is not silently treated as password
  encryption: universal mutation still needs a dedicated retained recipient
  provider path. The separate PubSec APIs can open those documents, but that
  alone does not make the byte-reparse universal transaction provider-aware.
- The RustCrypto `rsa` dependency remains affected by
  `RUSTSEC-2023-0071`, for which the upstream advisory lists no patched release.
  Blinding and uniform outward errors reduce oracle quality but do not prove a
  fix. Do not expose in-process private-key decryption to remotely timed,
  attacker-controlled requests; use a separately reviewed native/HSM/service
  provider or keep that release feature disabled.
- “Any PDF at any level” is not a defensible software contract. The implemented
  contract is ISO-valid or deterministically recoverable PDFs under declared
  credentials, resource limits, policies, and source ownership.

## Static source closeout performed in this change

This was source implementation plus static inspection, not executable
verification. The source pass closed
the previously identified move-after-use return expression by computing the
output digest before moving the byte vector. It also removed saturating PDF
object allocation from the affected text, OCR-layer, Form-clone, appearance,
and ink paths; those operations now reserve checked object-number blocks or
return a resource-limit error.

Full-rewrite copying now rejects unreadable dependencies, dangling references,
stale generations, and surviving references omitted by a subset remap. The
subset-remap check occurs after its declared source mutation so intentional
catalog removals remain possible without allowing an undeclared reference to
become `null` silently.

Credential transport was traced end to end. Standard-handler input passwords
survive document-handle reparsing only in zeroizing memory. Output credentials
have a length-delimited C ABI, managed byte overloads, zeroed temporary string
encodings, and bounded opaque HTTP fields. The compatibility NUL-terminated C
entry point remains but is no longer the path used by the managed bindings.

The final source-only editing pass additionally replaced non-painting source
carriers with destructive current-revision string rewrites plus exact `TJ`
advance compensation; added shaped, per-grapheme style replay for RTL, vertical,
missing-code, and ambiguous-code replacements; injected horizontal/bidi and
upright zero-offset vertical clipping replacements at their original source
position; retained partial and nested MCID ownership by rewriting inside the
original marked-content scopes; added collision-safe generated-font resources
to rewritten Form owners; added license-gated OpenType/CFF1 embedding; and added
visible scanned-word reconstruction with occurrence-matrix masks and harmonic
inpainting. Text/font/paint/marked-content state and image CTM/`q`/`Q` state now
continue across ordered page `/Contents` members rather than silently resetting
at each stream. Exact inline-image occurrence decoding, the native renderer,
and the SVG/PostScript regional fallback retain the normalized BI dictionary,
named page/Form color-space resources, `/Decode`, sparse per-filter
`/DecodeParms` arrays including `null`, and CCITT/JBIG2 post-decode semantics
instead of reducing the image to dimensions and a family name.

The post-audit pass then isolated all generated page content from inherited
graphics state, synchronized or fail-closed logical `/ActualText`, paginated
overflow across repeated continuation pages, changed selected operands to
per-stream shared buffers, installed cooperative HTTP-to-engine cancellation,
applied justification spacing and final-width anchoring to absolute glyph
coordinates, and synchronized visible scan changes with an exact, render-mode-
and-source-span-bound pre-existing invisible OCR occurrence. Focused regression
tests for atomic inline `/ActualText` cleanup, RTL final-width anchoring, and
visible-versus-invisible duplicate OCR binding were added but not executed. A
further full-path fixture now drives a real image occurrence plus two invisible
words sharing one `/ActualText` carrier through image mutation, per-deletion
provenance refresh, source removal, postcondition checks, and reopen; it also
remains unexecuted pending the VPS gate.

No claim in this section establishes syntax, type, linker, runtime, PDF output,
pixel, performance, interoperability, or corpus correctness. Those remain the
VPS gate below because execution was explicitly prohibited.

## Follow-up source hardening and linked-story work (2026-09-16)

See [the current implementation status](linked_story_implementation_status.md)
for the new parser, bidi, font, source-anchor, linked-frame and session changes.
That report explicitly separates implemented code from partial fixes, remaining
code-level work and unexecuted qualification. It supersedes any implication here
that the complete universal-editor roadmap is implemented. Source parsing and
diff whitespace checks were performed for this follow-up; no builds, tests, PDF
workloads or rendering qualification were performed.

The subsequent continuation adds persistent frame ownership and saved-story
rebinding, shared page-stream cloning, paragraph-context line shaping and feature
settings, glyph-ink fit checks, subset-outline coverage checks, incremental layout
convergence, bounded lookahead and exact undo/redo. These are also uncompiled,
unexecuted source changes. The [next follow-up](editing_followup_implementation.md)
adds the ten reported source fixes, contextual multi-font runs, per-edit contracts,
logical text merge, typed fixed-grid values and uncertainty review. Automatic
table/anchor/tag migration, full vertical/source-paint preservation and generated-page
pruning remain code-level gaps documented in the linked status report.

Story-image batching now replaces the former per-figure state-prefix walk with
one bounded operator/state pass per selected source page. Every requested image
range captures the same exact capsule as the single-selection algorithm, but
Arc-backed capsules are reused by validation and staging. Complete-page balance
validation and owned-fragment discovery are cached once per page inside the
immutable capture transaction. Aggregate decoded-page/capsule budgets,
cancellation polling and revision/policy binding remain fail-closed. The focused
batch-versus-single source regression is present but unexecuted.

The latest contract hardening adds an optional exact-revision indirect-object
change policy. A caller can enumerate existing object identities permitted to
change or disappear and place an explicit upper bound on newly allocated
objects. Verification inventories the reopened input and output and withholds
output on any undeclared delta. This is a bounded source-preservation contract,
not semantic object matching across renumbering or independent visual proof;
its focused regression remains unexecuted in the source-only phase.
The contract can now additionally map input/output pages and declare bounded
top-left-origin device-pixel rectangles where visual change is permitted. Both
revisions render through fully supported exact native display lists at one DPI;
the verifier normalizes overlapping rectangles, hashes every outside byte and
withholds output if even one outside RGBA pixel differs. Pixel and rectangle
budgets, one-to-one page mappings and cancellation checks bound the scan. This
is same-renderer preservation evidence, so an independent supplied raster oracle
is still required for cross-renderer fidelity claims. Its regression is also
present but unexecuted.
Object preservation can also bind a bounded rooted indirect-object graph rather
than one literal object number. A deterministic encoder assigns first-encounter
indices along ordered array/dictionary paths, follows shared and cyclic references,
and incrementally hashes tagged exact scalar values, topology and raw stream
bytes without retaining a second graph-sized canonical buffer. Equivalent
canonical renumbering therefore retains the fingerprint while changed values or
edges do not. Depth, object, byte and cancellation guards fail closed. This does
not equate independently recompressed streams or attempt unconstrained graph
isomorphism; the added renumbering/value-change regression remains unexecuted.
A separate decoded-object-graph assertion now retains the same scalar, dictionary,
array and reference-topology rules while fully losslessly decoding each stream and
excluding only `Length`, `Filter`, `DecodeParms` and `DL` from its canonical form.
It therefore accepts a raw versus Flate-reencoded stream only when decoded bytes
and every semantic dictionary value remain exact. The graph byte budget is also
the canonical hash byte counter and remaining per-stream decode ceiling;
incomplete decoding fails closed, and
external-file streams are refused rather than treating absent inline bytes as
content. The exact-raw assertion remains available. This is not pixel
equivalence for terminal JPEG/JPX/JBIG2 encodings or semantic equivalence
between different resource constructions; the focused
reencoding/changed-payload regression is present but unexecuted.
Named/versioned caller-supplied text oracles can now bind an exact extracted-text
hash for both the input revision and expected reopened output page. The input
hash is checked before mutation and output disagreement withholds publication;
multiple distinct extractors may cover the same mapping under bounded unique
identities. Reports repeat only hashes and provenance metadata, not page text.
The SDK deliberately labels the extractor identity as caller-asserted: only the
future qualification harness can attest that an independent executable actually
produced it. The focused pass/fail regression remains unexecuted.

The governed indirect-object route accepts an optional bounded path of dictionary
keys and array indexes. An explicit dereference segment can cross a selected
indirect reference only when the target object's exact planning fingerprint is
supplied. Planning binds every traversed owner, reports the actual referenced
write target and refuses cycles, repeated owners, duplicate write targets or a
batch that also mutates another lens's traversal owner. Apply rebuilds the same
resolution from the original revision before changing the final owner. Direct
and referenced leaves retain every non-encoding opaque sibling and raw stream
byte before the canonical incremental write. An explicit stream-encoding lens
is the only exception to ordinary stream-control refusal: it atomically replaces
raw bytes together with a direct `Filter`/`DecodeParms` graph, removes those
controls for unfiltered bytes, or deterministically emits level-6 Flate bytes.
It clears stale `DL`, preserves every other dictionary sibling, refuses
external-file streams and `/Crypt`, repeats fingerprint-bound traversal at
apply, checks inverse preservation on the parsed owner and reopens the result.
For unfiltered and SDK-Flate modes, the reopened stream must also decode
completely to the exact requested length and SHA-256; raw caller-encoded mode
can only assert the exact supplied encoded bytes/control graph.
Canonical direct `/Length`
normalization is applied up front, and every mutated indirect object must
reproduce its expected post-mutation fingerprint after reopen; every unmodified
root and intermediate traversal owner must also reproduce its planning
fingerprint. Before serialization, replace/insert also execute an exact
put-get check and every action applies its inverse to a clone to prove the
complete parsed owner returns to its pre-edit value. These are executable laws
over the exact parsed model, not semantic equivalence for independently
re-encoded objects. Paths are
approval-bound and capped by segment/key bytes. Stream `Length`, `Filter` and
`DecodeParms` paths remain refused for ordinary replace/insert/remove because
changing them without the coupled stream operation would reinterpret opaque
data. Insert requires an absent dictionary
key or an in-range array gap, while remove requires an existing leaf plus an
explicit null placeholder so no unused replacement payload is ambiguous. Seven
focused source regressions cover sibling preservation, ordered insertion/removal,
state preconditions, direct-crossing and ordinary stream-control refusal, atomic
re-encoding/sibling preservation, governed cross-reference mutation, stale
fingerprints, cycles and competing resolved write targets; they remain
unexecuted.
See `object_stream_encoding_lens_implementation.md` for the request contract and
the exact raw/unfiltered/Flate boundaries.

Saved untagged story figures also have an explicit ownership-release
transaction. The request binds the exact current native owner, preserves its
paint occurrence in place, removes it only from the saved story model, consumes
the one-shot command after publication and admits that owner to a different
story on a later exact revision. The source regression covers preservation,
command consumption, standalone admission and subsequent target-story
validation; it is not executed here. This standalone detachment remains
untagged-only rather than silently deleting semantic ownership.

A dedicated Rust transfer route composes that release with an exact target-story
rebind under one approval hash. It accepts two distinct already-saved untagged
stories or two supported tagged stories and one existing target caption
paragraph, retains the Figure's geometry/style, withholds the final byte vector
unless the source model releases the key, the target model owns it and exactly
one native owner remains, and records that the private intermediate revision is
still historical PDF data. In the tagged route it first validates both exact
source selections, moves the existing page-owned Figure leaf between the two
approved sibling intervals, preserves its leaf dictionary, rewrites `/P` and
story identity, recomputes both insertion positions and rebuilds ParentTree.
The target story transaction then binds that same leaf to the relocated image
MCID, followed by final ParentTree and source/target selection validation.
Mixed tagged/untagged conversion, content-bearing/shared Figure subtrees and
tagged table-cell objects are refused. An explicitly preserved bounded
contentless subtree now moves with its root, retains descendant order/metadata
and has valid descendant page bindings rebound by final target placement.
The end-to-end source regression creates two saved stories, rejects a stale
approval and checks all three postconditions; it remains unexecuted. The route
is also a tagged universal-editing JSON operation, making it transportable by
the existing C/Python/Java/.NET/WASM universal plan/apply surfaces. Dedicated
operation-specific Rust SDK, C ABI, Python, Java, .NET and WASM preview/apply
entry points use the same serialized request and exact plan hash; the TypeScript
declaration is updated. Generated managed request classes, executed binding
parity and product UI remain open.

Story pagination no longer reparses the same approved font for every paragraph,
candidate ranking pass, width probe and final line measurement. Ranking retains
one borrowed `ttf_parser` face per immutable program for the request. Layout
prepares only the actually selected fonts, retaining one borrowed face and one
validated CFF2 outline program for horizontal and vertical shaping metrics.
Standalone callers keep their original API and construct an ephemeral prepared
face. Focused equivalence regressions compare standalone/prepared horizontal
bounds and vertical shaping/metrics; they remain unexecuted, and no speed or RSS
claim is made before VPS measurement.

Standalone native image relocation now admits a bounded occurrence-specific
nested Form path. Planning captures the selected leaf image plus the minimum
required state for each ancestor and binds the whole chain to the exact input
revision. Apply creates a private visual capsule with the original Form matrices
and effective resource scopes, and separately clones the leaf-to-page source
chain so only the selected outer invocation reaches the image-free leaf clone.
Collision-free private XObject names prevent mutation of shared resource
bindings; the original image, Forms and page stream remain unchanged for other
occurrences. The source regression covers a two-level shared chain and remains
unexecuted. Untagged linked-story batches additionally build one bounded prefix
tree across all selected nested paths, share private clones for common prefixes,
combine leaf removals and child redirects before serialization, and rewrite
each affected page stream once. A two-image shared-prefix story regression is
present but unexecuted. A tagged story may now admit a uniquely invoked nested
Form image when one content-only Figure leaf owns exactly the image's complete
Form-MCR set. The image stage records exact live-clone/source pairs; tag prepare
removes only those old MCID delimiters and consumes every private marker before
the existing final page-MCR/ParentTree rebuild. Reused Form semantics remain
refusal-by-default; an explicit split decision now retains a residual Figure
leaf on the shared Form MCR while the original owner follows the selected paint
to its final page MCR. Outbound `/Ref` ownership requires and verifies an
explicit move/retain/copy policy. Incoming `/Ref` owners similarly require
follow/retarget/reference-both authority and are rewritten in one batch. Bounded
nested direct/indirect array graphs retain topology and shared aliases with
copy-on-write rewrites; cyclic, excessive, non-array-indirect or non-structure
relationship containers and nested OCR/text owners still refuse.
Focused tagged regressions are
present but unexecuted. Transparency-group migration
remains explicit follow-up work rather than being silently flattened.

Within one tagged story, an existing Figure may retain an explicitly approved
bounded descendant tree instead of being flattened to a leaf. Every descendant
must be uniquely parented, contentless and free of OBJR; the Figure's
first direct source-content slot is replaced by the generated page MCR while
descendant order and metadata remain unchanged. Valid explicit descendant `/Pg`
bindings follow the destination page. A separate one-shot removal approval
deletes the complete validated contentless tree only when no surviving structure
relationship or effective attribute references any member. An explicitly
approved reused-Form split now preserves the original tree with the selected
Figure and copy-on-write clones its contentless descendants for the residual
Figure. Clone `/P`, `/K` and internal Figure/subtree `/Ref` links are rebuilt,
while copied `/ID`, `/Pg` and private story keys are removed. External links on
any member use the same explicit outbound/incoming split policies as the root;
the complete relationship map is receipt-checked after reopen. Content-bearing
or externally shared reused subtrees still refuse. Multiple Figure trees split
in one request share a coordinated source-to-clone relationship map. The preservation,
deletion, cloning and refusal regressions are source-only and remain unexecuted.

## ECBES transaction integration (source complete, qualification not run)

Evidence-Constrained Bidirectional Edit Synthesis is connected to the canonical
universal planner/applier rather than a second PDF writer. The transaction accepts
explicit candidates and deterministic automatic route generation: text intents
expand to operator-preserving, geometric-block, and semantic-document candidates;
other operation families receive their canonical route. Every candidate starts
from the same immutable normalized revision, stays private through selection, and
is subject to post-plan fidelity classification, approval binding, byte budgets,
strict page/object reopen, logical postconditions, structural checks, and required
proof obligations.

The influence graph is derived from the revision-bound intent, canonical request
digests, affected indirect owners, and affected pages. It no longer obtains an
outside-pixel pass from a whole-document/vacuous cone. Such a pass requires either
a raster-preservation edit contract that covers every input and output page or
exact output-bound external evidence. Independent-renderer evidence additionally
requires every supplied reference raster to carry matching named/versioned
producer metadata, `independent: true`, and an artifact SHA-256 equal to its exact
RGBA bytes. Planning that escalates a nominal source-native candidate to a
reconstruction route is rejected unless the candidate's fidelity label discloses
that escalation.

The engine SDK, C ABI, .NET, Java, Python, WASM, and HTTP server expose the same
transaction. The detailed algorithm and trust boundary are specified in
`docs/research/evidence_constrained_bidirectional_edit_synthesis.md`. No build or
runtime qualification is claimed for this final source increment until the user
starts the VPS gate below.

## VPS qualification gate (not run)

The future VPS campaign must record exact commit, toolchain, OS/CPU/RAM,
dependency graph, corpus manifest and hashes, commands, raw artifacts, and all
failures. A release may move from source-present to qualified only after:

1. format, workspace build, Clippy, docs, all targets/features, and binding
   compilation pass with warnings denied;
2. unit/integration/property tests cover plan-state precedence, stale plans,
   approval tampering, duplicates, nested Forms, inline binary boundaries,
   clone-one/edit-all, masks, font binding, repair, signatures, undo, graphics-
   state isolation, direct/named/non-isomorphic `ActualText`, repeated
   edit-save-reopen for ordinary/RTL/ligature text, searchable scan layers,
   multi-page overflow, and justification geometry;
3. parser/writer differential and malformed/fuzz campaigns run with ASan or
   equivalent native instrumentation and bounded-memory assertions;
4. edited outputs strict-open in this engine and independent parsers and pass
   qpdf-style structural checks;
5. pre/post pages render through Wellfriend plus independent reference engines
   under normalized color/alpha/DPI policy, with per-page pixel/SSIM thresholds
   and manually classified disagreements;
6. a diverse multilingual/font/layout/image/forms/signature/standards corpus
   includes difficult production files, not only generated fixtures;
7. veraPDF and applicable PDF/UA/PDF/X tooling validate claimed profiles, with
   human review for semantic accessibility;
8. `cargo audit`, `cargo deny`, secret/config review, hostile-input limits, and
   RSA provider isolation are signed off; unresolved applicable advisories keep
   the release gate closed;
9. C, Python, WASM, .NET, Java, CLI, and HTTP execute the same fixtures and
   compare canonical report semantics and output hashes where deterministic;
10. latency, peak RSS, allocation, output growth, and cancellation are measured
    at P50/P95/P99 on the declared VPS class, including a dense multi-operand
    stream and proof that a timed-out worker releases its permit promptly.

## Primary technical references

- Adobe, *PDF Reference 1.5*, sections 3.4.5 and content/resource chapters:
  <https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/pdfreference1.5_v6.pdf>
- HarfBuzz manual, shaping contract and layout exclusions:
  <https://harfbuzz.github.io/harfbuzz-hb-shape.html> and
  <https://harfbuzz.github.io/what-harfbuzz-doesnt-do.html>
- Unicode Standard Annex #9, Bidirectional Algorithm:
  <https://www.unicode.org/reports/tr9/>
- Unicode Standard Annex #14, Line Breaking Algorithm:
  <https://www.unicode.org/reports/tr14/>
- Badros, Borning, and Stuckey, *The Cassowary Linear Arithmetic Constraint
  Solving Algorithm*:
  <https://badros.com/greg/papers/cassowary-tochi.pdf>
- RustCrypto advisory GHSA-c38w-74pg-36hr / RUSTSEC-2023-0071:
  <https://github.com/RustCrypto/RSA/security/advisories/GHSA-c38w-74pg-36hr>
