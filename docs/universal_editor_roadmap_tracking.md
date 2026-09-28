# Full editor/rendering roadmap — active, not completed

This ledger preserves the full requested objective across implementation turns.
Source implementation is not runtime qualification. No build, test, benchmark,
PDF workload or rendering is authorized in the current source-only phase.

| Requirement | Current evidence / remaining implementation | Completion evidence needed later |
|---|---|---|
| Repair audited parser/writer/layout defects | Source patches in advanced editing, shaping, reflow, writer and linked stories. Generated replacements bind selected strings to exact `BT`/`ET` paint slots and refuse ambiguous consolidation unless first/last anchoring is approved. Exact scalar/region partitions can keep arbitrary intervening paint between generated segments; a revision-bound proposal apportions complete graphemes by selected source coverage and requires explicit physical-region/final-layout approval. Horizontal/RTL and automatic/explicit vertical partitions retain paragraph context and must reproduce the unsplit OpenType glyph sequence and positioning exactly. Grapheme-owned leading/trailing/per-segment source text and paint state is retained per partition through one approved Type0 outline program. Unreviewed geometry inference and unrelated source font-family preservation remain open. Regressions are unexecuted | Exact-revision compilation and every regression, independent output checks |
| Unified font resolution/shaping | Contextual fallback, OpenType shaping, persistent fonts; generated vertical source editing uses Unicode 17 orientation, TTB shaping, logical column breaks, explicit W2 metrics and bounded positioned/style-preserving emission; vertical linked stories share multi-font coverage/measurement/emission and logical-axis pagination; story/table paragraphs now retain bounded Japanese-strict/custom break tailoring and protected emergency-wrap controls through saved metadata, review and browser UI, using Unicode 15 line-break data; mixed orthogonal stories, ruby, full Japanese typography and variable/exotic programs remain incomplete | Multilingual/missing-subset/vertical save-reopen-render corpus |
| Linked stories, growth and contraction | Occupied frames, continuation batches, bounded greedy constraints; opt-in pruning now plans/removes empty owned continuations, protects other content/dependencies, preserves surviving labels and projects output page numbering; original/unmarked or uncertain pages stay; global constraints and general dependency-aware page disposal remain incomplete | Cross-width/cross-page fit, backward flow, pruning/reference/label invariants and large-document scaling |
| Accessibility-preserving editing | Owner-indexed repair, annotation OBJR/MCR movement, paragraph/table migration, selected page-owned Figure leaves plus uniquely invoked Form-MCR Figure leaves with separate caption owners, explicitly approved bounded contentless Figure descendant trees with destination-page rebinding, relationship-checked one-shot deletion and reused-Form copy-on-write cloning with internal relationship rewrites plus subtree-wide external split policies over bounded nested direct/indirect array graphs, multi-block paragraph paths with shared-ancestor ordering, row groups and per-owner ClassMap materialization implemented in source; relationship values outside the governed graph, content-bearing/shared reused Figure subtree migration and general tagged flow remain incomplete | MCID/MCR/OBJR ownership, reading order, header relationships, attribute precedence and untouched-tag preservation after edits |
| Anchored images, captions, notes, links, forms | Supported annotation anchors and bounded tag-owner migration; explicit native image/caption blocks reserve space, keep captions whole, avoid exclusions and use atomic source binding/staging/placement through page insertion, with saved ownership, exact-binding deletion, Figure-leaf/MCID migration and browser controls; untagged and same-Figure-owned OCR groups batch text removal with image capture, rebind paragraph ranges and move/delete search with visual content while stripping obsolete source MCIDs from nested search Forms; exact direct sibling P/Span OCR owners can be explicitly assigned disjoint selected spans and merged into a Figure under a strict content-only/no-semantics policy on a page or Form, with approved reused-Form splits retaining ordered residual Figure/OCR leaves; exact untagged owners support in-place one-shot detachment and revision-bound later attachment; standalone and linked-story occurrence-specific nested Form image/OCR groups bind an exact Form-text target, build parallel bounded visual/search capsule chains, patch one cloned leaf atomically and validate Form-MCR ownership, while retaining shared originals; the tagged path atomically consumes old clone MCIDs before page-MCR rebinding and, after explicit approval, preserves residual semantic leaves when splitting one reused Form occurrence; explicitly approved bounded contentless Figure subtrees retain their descendant structure, replace only the root's direct source-content position, rebind explicit descendant pages to the destination, support relationship-checked one-shot deletion and copy-on-write clone descendants plus internal `/Ref` links for a residual reused-Form Figure; outbound `/Ref` ownership applies move/retain/copy across the complete tree, while external incoming owners use batched follow/retarget/reference-both through bounded nested direct/indirect array graphs with topology-preserving copy-on-write and exact transaction checks; a plan-hash-gated two-saved-story transfer supports either two untagged stories or two tagged stories with one page-owned Figure leaf or bounded contentless subtree, moving its exact structure root, preserving descendants and rebuilding ParentTree before target placement; content-bearing/shared reused Figure subtrees, relationship values outside the governed graph, mixed-object cells, mixed tagged/untagged conversion, notes and numbering remain incomplete | Cross-page movements preserve source objects, links, widgets and tags |
| General editable tables | Shared story mode adds spans/fragmentation, row minima, repeated artifact headers, exact values, owned grids, styled/tagged paragraph blocks, nested text-owner paths, row groups and opaque caption preservation; nested/mixed-object content, general relationships and global pagination remain incomplete | Table edit/growth/shrink/save/reopen with independent semantic/render checks |
| Verifiable edit contracts | Declared text/object/stream/page assertions; named/versioned caller-supplied text-oracle hashes bound to both input and reopened output; bounded canonical rooted graph fingerprints that survive deterministic indirect-object renumbering while preserving shared/cyclic topology, with distinct exact-raw and fully-losslessly-decoded stream modes. Decoded mode ignores only `Length`/`Filter`/`DecodeParms`/`DL`, remains byte-budgeted and fails closed on incomplete decoding; optional supplied raster oracles; a bounded exact-revision indirect-object policy that rejects undeclared existing-object changes/removals and excess allocations; and exact same-engine RGBA preservation outside declared device-space edit rectangles before publication | Terminal-codec visual equivalence, semantic resource equivalence and qualification-harness-attested independent extraction/rendering execution |
| Fresh-document authoring layout | Exact font stacks and prepared paragraphs feed fixed-column row fragmentation, horizontal column spans, cross-row span blocks with shaped-line continuation, exact typed decimal/formula evaluation with a reopen-validated catalog registry, source/topology receipts and stable table/cell marked-content owners carrying crop-box-bounded inner content regions; new registry entries retain font/layout plus device-colour paint, header semantics and simple-section continuation geometry; a revision-bound Rust/C/Java/.NET/Python/WASM transaction inspects exact owners/regions and source text state, retains empty-cell insertion carriers through clear/reopen/refill, plans one paragraph across every existing fragment before mutation, injects final lines at the original MCID/source paint position without a page-level overlay, clears obsolete fragment operands, recalculates dependents, rewrites the registry and reopens/rechecks output; a fully typed body row can allocate one canonical page batch for all sibling carriers, preserve per-cell fonts/TH-TD roles/scopes/spans, bind every new MCID to its original owner, rebuild the ParentTree and reproduce an exactly retained repeatable header as an artifact before redistribution; contiguous later rows wholly owned by the target's final page are now removed through exact text/grid source ranges, packed together at retained heights on one following page, rebound to their original TH/TD owners and refilled from retained values/fonts; versioned origin/destination/geometry/font/digest receipts now authorize bounded backward compaction when target continuations empty, retained values still fit their exact original carriers and both page-tail and incoming-reference guards pass; multiple receipts rebind after every rewrite and nested groups compact child-before-parent, verify the parent destination before child mutation and refresh only a digest changed by the verified child-owned transaction; separated downstream rows shift canonically and contraction uses exact table/row/cell-count provenance; every retained typed body grid rectangle has separately inspectable source-range-bound table/row/column/span paint ownership, including row-spanning topology; suffix-feasible cuts, repeated artifact headers and append rollback remain; typed sections own geometry, physical facing-page margins, first/odd/even masters, final page fields, parity blanks and PDF PageLabels; source-range-bound footnotes reserve the shared page bottom, typed endnotes flow from explicit parity, and automatic markers retain exact insertion maps; named PDF destinations plus fixed-capacity forward document/section/anchor page fields preserve line ownership, with optional shaped-range native link annotations, bounded hierarchical document outlines and outline-derived painted TOCs with exact page columns; mixed-object/partially typed cells, split-row relocation, general multi-row compaction, fragment removal, row-height changes/page disposal, section-master continuation, vertical cell redistribution, arbitrary existing-marker renumbering, broad typography, typed managed models and production UI integration remain incomplete | Exact-revision save/reopen, cell-text/note/field/link/outline/TOC partitions, section/master/page-label/destination invariants, page bounds, independent rendering and accessibility checks |
| PDF-native collaboration | Revision-bound text and structural merging now includes typed conflict alternatives, scope-constrained resolution and browser review; causal text histories, dependency retention, deltas and browser draft review; durable checkpoints resume through verified owners with receipt-bound publication and explicit detach/epoch replacement; same-replica selective text/style/structure/inline-style undo preserves original identities and other active edits, explicit 1..=4096-edit groups apply atomically through base or durable sessions, and the supplied browser component coalesces idle typing bursts into one exact-source grapheme-safe operation. Schema-3 paragraph-style operations use fieldwise causal registers. Schema-4 stable paragraph presence/position operations add insert/move/delete, deterministic ordering, delete-versus-concurrent-content conflicts and exact resolution across save/reopen. Schema-5 inline fields target stable scalar atoms, inherit onto observed insertions, expose unequal concurrent values per atom and support exact undoable resolution; supported horizontal/vertical Start/Resume preview/checkpoint, including table cells, now shares one canonical font/style partition with matching horizontal and vertical bidi/orientation itemizers across measurement and native PDF emission, saves the inline projection and verifies frame ownership on reopen. Explicit exact-frontier, plan-hash-approved logical compaction creates a new empty epoch, rejects stale prior-epoch histories and truthfully reports bounded byte-undo retention; it is not automatic/authenticated replica garbage collection or byte sanitization. Arbitrary external rebasing, non-paragraph block operation history and undo, equivalent integration in arbitrary hosts, richer visual conflicts and authenticated host replica lifecycle remain incomplete | Convergence, conflict preservation, offline/rejoin and deterministic output |
| Bidirectional source/logical model | Source owners, opaque-preservation contracts and story metadata exist; the governed object-graph route supports bounded dictionary/stream-dictionary and array replace/insert/remove paths against exact fingerprints, with explicit missing/existing/index preconditions, preserving non-encoding siblings and raw stream bytes, making canonical direct `/Length` normalization explicit, executing exact parsed-model put-get/inverse-preservation laws, and fingerprint-checking every mutated object after reopen. Explicit cross-reference segments bind every traversed indirect owner, make the referenced object the actual write target, verify preserved roots/intermediates again after reopen, and refuse stale fingerprints, cycles, duplicate writes or concurrent mutation of a traversal owner. An explicit stream-encoding lens atomically couples raw bytes with direct Filter/DecodeParms controls, or emits unfiltered/SDK-Flate bytes, clears stale DL, preserves every other sibling, rejects external-file/Crypt routes and executes inverse/reopen checks; unfiltered/Flate output must decode back to the exact requested length/hash. Higher-level semantic/arbitrary lenses remain incomplete | Minimal-write and round-trip invariants across the declared source model |
| Uncertainty-aware scan reconstruction | Visible replacement plus optional calibrated review; standalone and linked-story image/invisible-OCR groups preserve approved source operands, fonts, matrices and complete direct ActualText scopes during movement, with explicit unrelated-text decisions and consumed initial selection metadata; tagged story capture covers same-Figure OCR and explicitly partitioned content-only sibling P/Span owners on a page or Form, including ordered residual owners for approved reused-Form splits; arbitrary backgrounds, orientations and handwriting remain incomplete | Real scan corpus, reviewed uncertainty, OCR-layer/pixel consistency |
| Interactive editor and bindings | Rust/WASM sessions and embeddable worker client; shared retained-session protocol plus C exports and Java/.NET/Python wrappers implement source discovery, receipt-bound checkpoint, history, merges, table drafts and explicit rendering in source; Python detaches native work and guards retained state with a nonblocking mutex; cross-story Figure transfer has operation-specific Rust SDK, C ABI, Python, Java, .NET, WASM and TypeScript-declaration entry points using one request/plan-hash contract; exact-revision generated paint-partition propose/preview/apply/apply-reviewed is likewise wired through Rust SDK, C/header, .NET P/Invoke, Java FFM, Python, WASM/TypeScript and cancellation-scoped HTTP endpoints without duplicating apportionment or validation logic; canonical Rust preview returns bounded same-engine before/candidate PNG evidence without candidate PDF bytes and emits a compact receipt binding input/request/proposal/approval/font/candidate output/display evidence, while reviewed apply recomputes and withholds any mismatch; remote authenticated preview/apply endpoints add a dedicated server-held HMAC-SHA-256 wrapper binding key id, audience, issuance, expiry and the complete content receipt, verify it in constant time, keep API/PDF credentials separate, issue only with the active key and accept at most eight configured verification-only grace keys; trusted native hosts have the same authentication/verification helpers through C/header, .NET, Java, Python and static WASM/TypeScript surfaces, with secret copies zeroized where the owning Rust boundary permits it and browser-key warnings explicit; a browser-safe remote client never accepts the HMAC key, validates exact envelopes/receipts and parses the final multipart PDF without text decoding; .NET and Java now provide canonical typed paint-partition request/options/approval/proposal/preview/receipt models alongside raw JSON compatibility, while TypeScript publishes the same wire shapes; a source-only C-header parity guard covers every explicit Rust export and the seven previously absent declarations are present; its retained browser worker/client and custom element keep the local receipt private and require exact request/proposal/approval/font/revision equality before undoable publication; every Python/WASM SDK-backed document route that exposes an input-password parameter now forwards the retained zeroizing credential, including reflow, document subsystems/security, standards, XFA, signatures, semantics, writer, redaction, associated-file and sanitation paths; every document-dependent C SDK facade route now likewise reuses the retained credential rather than silently reopening encrypted source with an empty password, while the four text-only shaping/substitution utilities remain document-independent and output encryption stays separate; broader generated managed models, independent preview rendering and production UI integration remain incomplete | Exact native symbol/header/FFM/PInvoke/PyO3 parity, binding/server execution, real browser/device behavior and accessibility |
| Incremental performance/cancellation | Prepared paragraphs, caches, bounded history, batch insertion, worker termination and transaction-local image capture context/decoded-page sharing; canonical exact/decoded object-graph contracts now hash incrementally under an exact byte counter instead of retaining a second graph-sized canonical buffer; initial story-image occurrences share one bounded page-state walk with Arc-backed exact capsules, while target balance and owned-fragment inventories are cached once per page; story font ranking parses each approved program once per request and pagination reuses request-local sfnt faces plus prepared CFF2 outline programs across horizontal/vertical width probes and final measurements; further cache/layout optimization remains | RSS/latency/cancellation/load evidence on exact build |
| Universal visual objects/rendering | Existing source/image/vector/object-graph and renderer paths; vector copy-on-write isolates exact Contents slots and nested Form occurrences; scope-bound default colours now feed raw/packed paint, images, shadings and Type 3 replay, with Pattern initial/base-state, non-unit components, bounded palette decoding and cache dependencies; SVG/PS uses reported raster routing for defaults; tagged Form ownership, native vector colour, full ICC/codec/transparency/paint-state limits remain | Difficult PDF corpus plus independent renderer comparisons |
| Security, standards, encryption, signatures | Existing governed policies; broad preservation is not qualified | Exact profiles, permissions, signatures, standards and adversarial documents |
| Better-than-Acrobat claim | No comparative evidence | Named Acrobat version, same tasks/documents, success/failure and fidelity/performance results |

Binding-audit correction: the parity guard now covers all explicit Rust C
exports plus every native symbol imported by .NET and Java. Thirteen missing
public-header declarations were restored in total (the seven explicit exports
noted in the table plus six macro-generated exports found by the broader pass).

Current detailed source reports: `editing_followup_implementation.md`,
`object_stream_encoding_lens_implementation.md`,
`story_workflow_continuation.md`, `linked_story_implementation_status.md`,
`tagged_ownership_implementation.md`, `tagged_story_implementation.md`,
`paginated_table_implementation.md`, `image_fragment_implementation.md`,
`story_figure_implementation.md`, `tagged_figure_implementation.md`,
`ocr_carrier_implementation.md`, `story_ocr_implementation.md`,
`native_story_sessions.md`, `tagged_ocr_implementation.md`, `python_story_sessions.md`,
`story_page_pruning_implementation.md`, `story_annotation_group_implementation.md`,
`story_annotation_identity_implementation.md`, `annotation_identity_unification.md`,
`annotation_relationship_transactions.md`, `annotation_source_promotion.md`,
`annotation_owner_promotion.md`, `annotation_field_materialization.md`,
`annotation_structure_materialization.md`, `vertical_typography_implementation.md`,
`inline_text_position_implementation.md`, `vertical_story_implementation.md`,
`ext_gstate_font_implementation.md`, `vector_occurrence_implementation.md`,
`form_text_implementation.md`, `renderer_resource_scope_implementation.md`,
`vector_export_scope_implementation.md`, `scoped_text_extraction_implementation.md`,
`semantic_stream_scope_implementation.md`, `appearance_text_scope_implementation.md`,
`appearance_text_editing_implementation.md`, `tagged_appearance_clone_implementation.md`,
`governed_scoped_text_implementation.md`, `scoped_candidate_preview_implementation.md`,
`widget_text_transaction_implementation.md`, `story_text_history_implementation.md`,
`story_history_checkpoint_implementation.md`, `story_selective_undo_implementation.md`,
`story_structure_resolution_implementation.md`, `default_colour_rendering_implementation.md`,
`icc_alternate_conversion_implementation.md`, `image_sample_domain_implementation.md`,
`indexed_and_optimized_image_domains.md`, `indexed_paint_domain_implementation.md`,
`shading_color_domain_implementation.md`, `shading_mesh_interpolation_implementation.md`,
`shading_patch_tessellation_implementation.md`, `shading_bounds_background_implementation.md`,
`analytic_shading_geometry_implementation.md`, `function_parameter_semantics_implementation.md`,
`vector_function_semantics_implementation.md`, `sampled_function_interpolation_implementation.md`,
`prepared_function_graph_implementation.md`, `calculator_semantics_implementation.md`,
`retained_colour_function_implementation.md`, `function_memory_accounting_implementation.md`,
`render_owned_function_cache_implementation.md`, `paragraph_line_break_implementation.md`,
`joining_context_implementation.md`, `joining_synopsis_implementation.md`,
`balanced_line_composition_implementation.md`, `hard_line_policy_implementation.md`,
`logical_line_carrier_implementation.md`, `generated_logical_carrier_implementation.md`,
`authoring_paragraph_implementation.md`, `shaped_font_coverage_implementation.md`,
`logical_control_carrier_implementation.md`, `authoring_font_fallback_implementation.md`,
`authored_table_pagination_implementation.md`, `cff_encoding_implementation.md`,
`variable_cmap_implementation.md`, `predefined_cmap_implementation.md`,
`font_collection_preparation_implementation.md`, `cff2_outline_implementation.md`,
`font_variation_core_implementation.md`, `font_instance_metrics_implementation.md`,
`font_glyph_metric_implementation.md`, `font_position_instance_implementation.md`,
`font_baseline_justification_implementation.md`, `font_tuple_variation_implementation.md`,
`truetype_outline_instance_implementation.md`, `truetype_hint_instance_implementation.md`,
`static_truetype_instance_implementation.md`, `cff2_static_instance_implementation.md`,
`cff2_contour_normalization_implementation.md`, `cff2_exact_linear_implementation.md`,
`cff2_checked_preservation_implementation.md`,
`form_feed_pagination_implementation.md`, `authoring_sections_implementation.md`,
`nested_form_ocr_implementation.md`,
`generated_reflow_paint_order_implementation.md`,
`authoring_notes_implementation.md`, `authoring_fields_implementation.md`,
`authoring_outline_implementation.md`, `authoring_toc_implementation.md`,
`authoring_index_implementation.md`, `authoring_front_matter_implementation.md`.

Latest body-field increment: fresh flow publishes stable named XYZ destinations
and defers current/final document, section and forward-anchor page values until
every page exists. Explicit maximum capacities reserve line breaks; final
resolution reuses whole-paragraph bidi/fallback context, retains unpadded
logical text, preserves command order and rejects unresolved/over-capacity or
geometry-changing output. Anchor-page fields may now opt into native `/Link`
annotations. Their finite page-local hitboxes come from the resolved value's
actual shaped advance cells; pages receive independent annotation objects and
named `/Dest` references plus stable `/NM` identities and owning-page `/P`
back-references, while duplicate/overlapping hitboxes fail closed. The
destination name tree uses encoded PDF key ordering, exact `/Limits` and bounded
64-way indirect leaves/branches. Seventeen regression functions remain
unexecuted. Rich inline fields, exact tab
boxes, TOCs/indexes, semantic cross-reference tags, imported/linked anchors and
all executable qualification remain open. See
`authoring_fields_implementation.md`; this is not roadmap completion.

Latest fresh-outline increment: typed nested outline entries bind bounded titles
and open/closed state to forward-resolvable authored anchors. Final serialization
allocates the root and items atomically, emits exact parent/child/sibling links,
computes visible positive/negative descendant counts and publishes catalog
`/Outlines` plus `/PageMode /UseOutlines`. Explicit outlined headings now bind
the pre-heading anchor, visible text and level-stack entry atomically, with
destination-page-aware anchor capture, skipped-level refusal and complete
rollback on layout failure. Eleven regression functions remain unexecuted.
Styling, non-GoTo actions, inferred heading
capture from arbitrary styling, imported
outline merging, painted TOCs/indexes, semantic structure tags, bindings and all
runtime qualification remain open. See `authoring_outline_implementation.md`;
this is not roadmap completion.

Latest painted-TOC increment: the explicit outline now flattens into one
rollback-capable flow transaction. Prepared titles wrap in checked hierarchy-
indented columns; independently shaped deferred anchor-page values right-align
inside fixed columns, resolve after final pagination and reuse native clickable
links. Measured bounded leaders, combined title/page extents, exact row reports
and multi-page flow are present. Physical left/right page columns plus independent
title/value alignment cover RTL publishing conventions. Exact level overrides
now resolve typography, absolute indentation, spacing, alignment and leader
behavior before mutation; bounded transitive keep-with-next/previous chains move
together when they fit an empty page. Eleven regression functions remain
unexecuted. An explicit back-of-document index now validates nested terms and
cross-references, accepts authored/scalar/explicit locale sort keys, orders
anchor occurrences by physical position, deduplicates same-page hits and emits
clickable deferred page values plus same-section-safe physical page ranges in
one rollback-capable transaction. Six index regressions remain unexecuted.
Generated TOCs may now stage in a distinct front section and splice before an
existing body only after page/section/anchor/note/field preflight. Automatic
suppressed parity padding preserves every original body page's odd/even side,
mirrored margins and master selection. The same isolated transaction now accepts
arbitrary paragraphs, tables, images, section-scoped notes, fields, anchors,
additional sections and generated TOCs, merging staged resources only after
success. Eight front-matter regressions remain unexecuted. The final writer now
derives page-local MCIDs from stable layout identities and emits
`/StructTreeRoot`, `/TOC`/`/TOCI`, `/Index`/`/P`, page `/StructParents`, a
ParentTree and annotation `/StructParent` plus `OBJR` ownership. Wrapped index
rows retain one identity across pages, and prepended front roots precede body
roots. Ordinary/field paragraphs, `/H1`-`/H6` headings, `/L` list hierarchies and
alternate-text `/Figure` content now share the same structure plan; legacy
no-alt images serialize as artifacts. Flowed tables now add `/Table`/`/TR` and
`/TH`/`/TD`, column scope, cross-fragment cell identity and artifact-only repeated
headers. Explicit RFC 5646/BCP 47 document language now publishes catalog `/Lang`.
Fourteen structure regressions and the updated annotation case remain unexecuted.
Document-scoped front-note renumbering, broader orphan constraints,
inferred/imported note references and richer table relationships, full PDF/UA semantics, imported
TOCs/indexes, bindings and runtime qualification remain open. See
`authoring_toc_implementation.md`, `authoring_index_implementation.md`,
`authoring_front_matter_implementation.md` and
`authoring_structure_implementation.md`; this is not roadmap completion.

Latest fresh-note increment: explicit body UTF-8 marker ranges now own physical
footnotes. The joint paginator reserves page-bottom geometry respected by later
paragraphs/images/tables, fragments long notes at prepared-line boundaries,
retains paragraph bidi/font context and reports exact display ranges. Private
final materialization verifies monotone fragment chains and paints an artifact
separator plus logical note text through the canonical writer. Typed endnote
collections validate before allocation and flow from next/odd/even pages.
Sixteen note regression functions remain unexecuted. Fresh note bodies now use
unique-ID `/Note` elements across fragments, with one added shared structure
regression unexecuted. Exact source marker ranges now own ordered `/Reference`
children with reciprocal `/Ref` links to `/Note`; cluster-safe emission retains
whole-line shaping and refuses split clusters. Arbitrary existing-marker
renumbering, imported/linked-story note ownership, side
notes, citations, managed bindings and all executable qualification remain open.
See `authoring_notes_implementation.md`; this is not roadmap completion.

Latest fresh-section increment: `FlowDocument` pages now retain monotone typed
section ownership. Sections control page geometry, margins, first/odd/even
artifact masters, independent label starts and decimal/Roman/alphabetic styles.
Running literal/document/section fields resolve only on a private final-count
clone, fit inside reserved margins and serialize through the existing font/image
plans and canonical writer. Odd/even starts leave suppressed parity blanks in
the preceding section; non-default numbering emits PDF PageLabels. Facing-page
sections swap left/right margins on even physical pages through the same page
allocator used by body, tables and notes. Ten
regression functions remain unexecuted. Imported-section inference, linked-story
section flow, dynamic chapter fields, semantic note/cross-reference relationships,
accessibility structure and all executable qualification remain open. See
`authoring_sections_implementation.md`; this is not roadmap completion.

Latest physical-page increment: linked stories now distinguish next-frame flow
from next/odd/even physical-page policies. U+000C closes its logical line,
skips remaining same-page frames, advances through an approved later frame or
bounded owned continuation pages, and emits a preview-receipt-bound transition.
Trailing and repeated form feeds preserve their logical carriers and materialize
blank/parity pages. Horizontal and vertical stories share the path; metadata
schema 4 and native/browser capability 1 prevent silent old-worker behavior.
Fresh-authoring flow uses the same U+000C meaning and exposes parity breaks;
page-local paragraphs and table cells fail closed where they cannot own page
creation. Ten new regression functions and several extended cases remain
unexecuted. Fresh-document section masters, headers/footers and page-number
fields are extended by the increment above; linked-story section semantics,
imported/linked notes, imported/linked-table page commands and all executable qualification
remain open. See `form_feed_pagination_implementation.md`; this is not roadmap
completion.

Latest authored-table increment: fixed-column text rows now retain prepared
paragraph lines across pages, use backward suffix feasibility for valid line
minima, preserve per-cell fonts/styles and source ranges, repeat whole headers
as artifacts, and append all fragments through a rollback-capable transaction.
Rows may now own next/odd/even physical page commands; parity blanks remain in
the active section, repeated headers paint only at the destination, and typed
receipts bind source row plus from/to page allocation.
The default splits oversized rows; explicit KeepTogether preserves refusal.
Reports disclose each fragment/page/range, and no per-page PDF rewrite occurs.
Fresh tables now also carry visible caption ownership, bounded summaries,
row/column header scopes, stable header IDs, horizontal `/ColSpan` topology and
explicit data-cell associations across every occupied grid column;
caption pagination keeps a feasible first fragment together or refuses.
Forty-one table regression functions, including a small exhaustive partition oracle
and canonical single-cell plus fully typed row continuation/ParentTree transactions,
remain unexecuted. Mixed-object cells and broader accessibility trees,
section-aware grouping, managed UI/bindings and all runtime qualification remain
open. See `authored_table_pagination_implementation.md`.

The latest retained-table increment gives every newly allocated typed-row page
reopen-checked provenance containing the exact table identity, row, expected cell
count and a digest of every decoded byte outside exact cell scopes. Opt-in
contraction requires empty expected carriers, unchanged static paint, no page
features/annotations and no surviving incoming reference; it detaches exact
MCRs, rebuilds the ParentTree, prunes through the shared page writer and projects
changed-page numbering. Original, modified or dependency-bearing pages remain
with machine-readable reasons. Resolvable repeatable headers now retain exact
layout/paint/font identity and repaint as artifacts on typed-row continuation
pages. Fully typed rows in a multi-row table may grow and contract when all
downstream rows already start later. Contiguous same-page downstream rows now
move through exact source deletion, retained-height packing and original-owner
rebinding when they form the final meaningful page tail. A versioned relocation
receipt now retains the origin/destination identity, original geometry/fonts and
static destination digest; opt-in shrink can restore those rows to fresh native
carriers at the original page tail and canonically remove the relocation page.
Multiple receipts rebind after each page rewrite; nested groups compact from the
furthest child back to the parent, verify the stored parent-destination digest
before the child mutates that shared page and refresh only the digest changed by
the verified child-owned restoration. Externally changed parent destinations
now retain both receipts instead of being legitimized by the refresh. Split-row
relocation, later mixed page content and general mixed-cell contraction remain
open.
New row-break provenance also rejects an odd-sized
insertion that would flip a retained next-odd/next-even downstream row.

The retained-table grid contract gives every typed body grid rectangle a
private artifact owner keyed by table and exact row/column/span topology. A
bounded reopen inspector returns exact stream ranges and geometry and reports
complete typed-grid coverage without relying on visual matching. That evidence
now authorizes both separated-page insertion and bounded same-page relocation.

The subsequent cross-row increment adds a table-wide occupancy plan, bounded
row-span validation, occupied-slot skipping, an acyclic minimum-height constraint
solver, one-rectangle painting, atomic safe-boundary pagination, `/RowSpan`
structure state and row-header coverage across spanned rows. A bounded cut planner
selects only measured row/line boundaries and enforces fragment/final-line minima;
page painting, repeated headers, shared semantic ownership and exact receipts now
consume those cuts. Two additional topology/render/serialization/cut regressions
remain unexecuted; executable qualification is still absent.

Fresh authored tables subsequently reuse the canonical typed-value evaluator.
Stable cell identities resolve exact decimal/formula dependency graphs before
layout; checked arithmetic, cycle/missing-reference failures and no-silent-
rounding rules are preserved, and reports bind the evaluated strings. One
additional evaluation/rollback regression remains unexecuted. Durable formula
metadata now serializes in a bounded catalog-owned registry; its loader
independently reevaluates and rejects drift. Post-reopen mutation/recalculation
and checkpoint save remain open.

Latest authoring fallback increment: explicitly registered font stacks now use
the shared contextual assignment core with borrowed exact programs. Page,
paragraph, table-cell and flow commands capture physical font assets and final
paragraph-derived glyph runs; mixed-font lines keep one logical-text owner.
Standard-14 equivalents require explicit stack opt-in. Read-only data previews
disclose source ranges, font hashes, choices and geometry. Signed authoring
advances now match measured pen movement. Twelve added regression functions are
unexecuted. Broader font programs, contextual assignment recovery, shaping
options, vertical/ruby/tab layout, authored accessibility,
binding/UI integration and all qualification remain open. See
`authoring_font_fallback_implementation.md`.

Latest logical-control increment: stable carrier codes now preserve standalone
default ignorables under the pinned shaping policy, in addition to hard
separators. Story/table lines, authoring, bounded replacements and source-inline
clipping/tagged replacements use empty outlines with explicit Unicode mappings;
inline spacing and following endpoints remain source-controlled. Story output
checks now bind actual owner-local character operands as well as mappings and
ActualText. Twelve new regressions and two extended cases remain unexecuted.
General glyphless-font semantics, per-control selection geometry, multiline OCR,
inline hard-line layout, the broader roadmap and all qualification remain open.
See `logical_control_carrier_implementation.md`.

Latest font-coverage increment: shared final-cluster outline checks now feed
advanced horizontal/RTL/vertical analysis, final generated conversion, fallback,
authoring and substitution approval. Default-ignorable removal was already
present in the shapers; source-bound blank spacing and normalized decomposition
are distinguished from subset holes. Exact supplied assets cannot inherit
same-name bundled approval, and the older reflow resolver no longer has a
separate cmap-only veto. Twenty-one regressions include plan/approve/apply/reopen
source but remain unexecuted. Supported control-only persistence is extended
above; arbitrary glyphless-font semantics, exotic
fonts, wider layout/rendering and the full roadmap plus qualification remain
open. See `shaped_font_coverage_implementation.md`.

Latest authoring increment: page, table and flow paragraphs share prepared
logical lines, paragraph-derived bidi/joining context and actual font-byte
measurement/emission. Shared immutable custom assets remain bound through
registration, document cloning and command serialization. Exact hard separators,
zero-width blank-line carriers, uncollapsed spaces, checked line geometry and
append-only rollback are implemented in source. Fifteen regressions remain
unexecuted. Authoring fallback is extended above; vertical/ruby, row fragmentation,
arbitrary glyphless logical persistence, wider editor/rendering work and all qualification remain
open. See `authoring_paragraph_implementation.md`.

Latest tab-stop increment: authoring, fallback, flow/list/note/table/field and
horizontal/vertical linked-story paths now share exact U+0009 field planning
with left/right/center/decimal stops, bounded default continuation and the same
font/style/bidi measurement used by emission. Tagged authoring assigns tab
carriers and visible fields to exact semantic owners. Story paint/shape receipts,
saved schema 6 and paragraph-style history retain undecorated plans. Explicit
dot/dash/solid leaders and perpendicular bar rules now bind exact resolved
geometry into the paint model and serialize as artifacts in fresh-authoring and
linked-story writers. Decorated plans require saved schema 7 and durable seed
schema 5. Exact caller-supplied multi-character decimal tokens are bounded,
cached and versioned by saved schema 8 / durable seed schema 6; session
capability version 3 exposes the boundary. Regression source is unexecuted.
User-defined leaders, locale-inferred decimals, arbitrary tab expressions and
all runtime/corpus qualification remain open. See
`tab_stop_layout_implementation.md`.

Latest generated-carrier increment: bounded and multi-run writers, including
generated/exact inherited styles, now stage zero-advance logical text for
hard-separator-only replacements. Source-positioned insertions give only their
generated glyph sequence a nested logical owner; reopened page/Form/appearance
checks bind the actual new carrier tokens and mappings. Inherited mandatory
boundaries, blank justification and shared overflow-safe nominal line capacity
are implemented in source. Generated non-story Type0 fonts now carry a private
version marker, and the shared page/Form writer removes only marker-bearing
resources that a bounded content/Form/pattern/Type3/appearance traversal proves
unreachable. Dormant surviving references and historical incremental objects are
retained. Thirteen new regressions remain unexecuted. Authoring paragraph/font-
metric parity is extended above; mixed-line logical geometry, wider typography/
layout and all qualification remain open. See
`generated_logical_carrier_implementation.md`.

Latest logical-carrier increment: story/table hard-only lines emit zero-advance
Type0 text with explicit ToUnicode and the existing ActualText/tag scopes, using
a private empty-outline subset excluded from font substitution. Owner-local
output checks reject missing/duplicate carriers and damaged mappings; existing
resource retirement applies. The plain-text formatter keeps explicit blank
separators without inventing duplicate boundaries. Thirteen new regressions
remain unexecuted. Bounded/multi-run parity is extended above; authoring parity,
broader text/layout/rendering work,
section masters/headers/footers/numbering and all independent qualification remain open.
See `logical_line_carrier_implementation.md`.

Latest hard-line increment: one BK/CR/LF/NL predicate now feeds generated-text
shaping, font coverage, joining stops, measurement, style-preserving edits and
story/table/vertical emission. Cancellable logical/visible ranges preserve UTF-8
and CRLF; coverage and paragraph shaping retain resolved bidi across LS/VT/FF.
Single-line APIs validate before authoring mutation, caches advance, and native/
worker shaping capability 3 identifies the change. Sixteen new regressions remain
unexecuted. Blank-only story carriers are addressed by the increment above;
section-master/header/footer/numbering semantics,
wider typography/pagination and the full editor/rendering/qualification roadmap
remain open. See `hard_line_policy_implementation.md`.

Latest composition increment: opt-in paragraph-wide DAG selection compares
fully measured line candidates without assuming monotonic width. Natural and
emergency passes share invocation-local metrics and explicit resource budgets;
story/table/vertical requests, receipts, schema-3 persistence, structural review,
history and browser controls retain the chosen policy. The older reflow preview
and optimizer also no longer prune all later candidates after one over-wide
measurement. Twenty-one new regression functions remain unexecuted. Cross-call
composition retention, variable-geometry/global pagination, wider typography,
the broader editor/rendering roadmap and all executable qualification remain
open. See `balanced_line_composition_implementation.md`.

Latest joining-synopsis increment: a retained paragraph index and small source-
derived summaries preserve the nearest nontransparent scalar beyond long mark
runs for the pinned Rustybuzz joining state machine. Font/script/orientation
slices, cache identities and writer plans retain it; native/browser capability
version 2 identifies the change. The upstream transparency table and category
dependency are pinned, with source hash, license and maintenance transcription
script. Sixteen new regression functions remain unexecuted. General font/script
constraints, broader typography/layout, renderer/editor work and all executable
qualification remain open. See `joining_synopsis_implementation.md`.

Latest joining-context increment: paragraph-derived non-emitting neighbours
follow horizontal/vertical font and script runs through line measurement,
preview, caches, authoring and story/table emission. Default and explicit-feature
paragraph shaping share the resolved-line path, and session/worker capabilities
identify the change. Eighteen new regression functions remain unexecuted. The
raw five-scalar joining limitation is addressed by the synopsis increment above
for its pinned state machine. General font/script constraints, broader
typography/layout and all executable qualification remain open. See
`joining_context_implementation.md`.

Latest paragraph increment: persisted Unicode/Japanese-strict profiles, custom
line-edge restrictions and protected emergency word wrapping share prepared
indexes across story/table/vertical layout. Nondefault policies require saved
metadata schema 2; structural review normalizes omitted defaults, and worker
opening checks the native capability before accepting an older SDK. Typed width
blocks retain a fitting prefix and let story flow try later approved geometries;
widow/keep rules distinguish limited lookahead from paragraph completion.
Thirty-two new regression functions remain unexecuted. Full Japanese typography,
general script/font shaping constraints, dictionary segmentation/hyphenation,
general cross-constraint backtracking and variable-geometry global optimization
remain open (fixed-width segment composition is added above), along with the
wider roadmap and all qualification. See `paragraph_line_break_implementation.md`.

Latest function-cache increment: render states now use caller-owned prepared
function retention with reader namespace binding, graph-cache metrics and
aggregate handoff eviction. Independent render budgets do not mutate the
standalone reader policy. Image tint/Indexed/ICC-alternate routes carry the same
explicit cache and live-use function policy; default standalone helpers retain
their bounded reader cache. Twenty new regression functions remain unexecuted.
All-cache admission-time accounting, full codec/parser/image-buffer accounting,
non-shading cumulative work limits, wider roadmap implementation and executable
qualification remain open. See `render_owned_function_cache_implementation.md`.

Latest function-memory increment: cold graph construction, decoded function
output windows and warm graph use now carry nonblocking lifetime-bound memory
reservations. Shadings, scalar soft-mask transfers and nested named-paint tint
conversion receive active temporary/decoded limits; cache hits cannot bypass
those limits. Twenty new regression functions remain unexecuted. Render-owned
retention and image function-policy propagation are added above. Complete
parser/compiler/codec/evaluation accounting, richer public failure diagnostics
and exact-build qualification remain open. See `function_memory_accounting_implementation.md`.

Latest colour-function increment: reader-owned bounded graph caches use exact
typed keys without document-byte hashing or depth-truncated graph identities.
Native evaluation, shading validation/paint, tint conversion and scalar soft-mask
transfer reuse prepared graphs. Tint outputs hold weak identities; scalar LUTs
share one cumulative work budget and reject extra channels. Twenty-five new
regression functions remain unexecuted. Consumer graph reservations are added
above; render-owned graph retention and public metrics/configuration are added
by the latest increment. Full colour-space retention, broader aggregate
accounting, wider rendering coverage and all executable qualification remain open. See
`retained_colour_function_implementation.md`.

Latest calculator increment: a typed decimal scanner, opcode/branch compiler
and integer/real/boolean evaluator replace the floating-only interpreter.
Numeric promotion/conversion, boolean overloads, bitwise shifts, arithmetic
errors, exact final numeric arity and PDF conditional grammar are implemented in
source and shared with editing resource validation. Twenty-one new calculator,
tint and raw/compiled shading regressions remain unexecuted. Independent numeric/
interpreter/corpus qualification, public diagnostics, full colour-space retention and
the wider roadmap remain open. See `calculator_semantics_implementation.md`.

Latest prepared-function increment: one immutable evaluator now serves cold
native APIs and retained paint-scoped graphs for all seven shading families.
Aliased children share decoded samples/programs, calculator branches compile
once, metadata/streams have explicit retention/decoder limits, and nested/tensor/
calculator work shares one allowance with cumulative shading debits. Eighteen
new regression functions remain unexecuted. Tint/transfer and cross-paint graph
caching and consumer graph reservations are added above; broader allocation
accounting and all executable qualification remain open. Typed calculator semantics are added above. See
`prepared_function_graph_implementation.md`.

Latest sampled-function increment: Order 3 now uses bounded tensor cubic
interpolation, with linear fallback on short axes, exact sample hits, explicit
endpoint extension and Decode/Range after interpolation. Fixed stencils,
compensated sums, packed byte reads, early table limits, a decoded-stream cap and
cooperative polling replace the allocation-heavy linear-only path. Seventeen new
regression functions, including raw/compiled shading pixel assertions and tint
integration, remain unexecuted. The prepared-function increment above now adds
paint-scoped retention/shared work accounting; broader colour-graph caches and
independent numerical/corpus qualification remain open. See
`sampled_function_interpolation_implementation.md`.

Latest vector-function increment: parent stitching Range survives sampling and
supported exact PS emission; exact interval selection, terminal empty intervals,
analytic child/parent/device clipping stops and bounded/cancellable construction
replace the earlier endpoint-only semantics. Narrow function numbers and stop
colours survive serialization; nonlinear colour conversion without an exact PS
representation uses reported raster routing. Seventeen new regression functions
remain unexecuted. SVG hard transitions, wider native colour/function support,
target numeric limits and all executable qualification remain open. See
`vector_function_semantics_implementation.md`.

Latest function/parameter increment: shared native functions now preserve actual
Type 2 domains/non-unit outputs, apply declared input/output clipping, validate
stitching dimensions/boundaries, and map tiny intervals without absolute cutoffs.
Known shading/function/pattern parameters resolve bounded indirect arrays and
scalars consistently before decode/validation/paint; vector loaders share that
resolution. Twenty-six new regression functions and two revised assertions are
unexecuted. Prepared functions and shared calculator work limits are now added
above; typed calculator semantics are subsequently added above, with numerical/
interpreter and executable proof still open. Bounded
cubic sampled interpolation is also implemented in the increment above. The
subsequent vector-function increment above covers the
bounded stitching/range/stop fixes, not the full roadmap. See
`function_parameter_semantics_implementation.md`.

Latest analytic-shading increment: Type 1/2/3 use locally normalized inverse and
gradient geometry, compensated radial coefficients/discriminants and stable
quadratic roots. Domain interpolation and validation samples avoid opposite-end
overflow; negative radii, zero-radius pairs and coordinate-mapping errors have
explicit behavior. Fourteen new regression functions remain unexecuted.
Ill-conditioned/extreme-ratio arithmetic is not certified; other transform paths,
filtering, broader colour/export behavior and executable qualification remain
open. See `analytic_shading_geometry_implementation.md`.

Latest shading increment: raw/retained shading and shading-pattern calls retain
source domains and selected colour-management options through all seven families.
ICC mesh counts, Type 4 restarts/flags, lattice byte padding, malformed-stream
error propagation and cancellation have source fixes. Meshes now interpolate
source components/function parameters before conversion; patches retain (u,v)
and bilinear source corners. Half-open identical-edge ownership, clipped scratch
compositing, fill/stroke opacity, cumulative mesh work charges and nonblocking
shared temporary-memory reservations are implemented in source. Seventeen new
regression functions supplement the prior twelve; all remain unexecuted. A
further seventeen source regressions accompany device-space derivative estimates,
shared cubic edge/axis negotiation, consistent boundary evaluation, and streaming
two-row patch rasterization. Coons and tensor source order and source-component
colour interpolation remain intact. Collection growth includes old/new allocation
peaks in its reservations. General partial-edge stitching, localized refinement,
aliasing/folded surfaces, broader colour/codec/native export conformance and all
executable qualification remain open. These changes do not bound every native
allocation or guarantee immediate cancellation. Another twenty unexecuted
regression functions accompany target-space BBox polygon clipping, pattern-only
backgrounds, single-composite temporary coverage, original clip-hole sampling
and exact axial/radial cache parameter matching. BBox bounds constrain scratch
allocation; matrix, palette provenance and opacity rules remain explicit.
See `shading_color_domain_implementation.md` and
`shading_mesh_interpolation_implementation.md` and
`shading_patch_tessellation_implementation.md` and
`shading_bounds_background_implementation.md`.

Latest Indexed-paint increment: images and non-image paint share palette-domain
interpretation. Selected fill/stroke and uncoloured pattern bindings retain the
original graph, including saved state, inherited replay and Type 3 cache identity.
ICC/tint alternate conversion carries corresponding source provenance; named
conversion has a cancellation/depth guard. Eleven new regression functions and
two revised metadata assertions are unexecuted. Shading provenance is advanced
by the follow-up above; export/prepress consumers, remaining codecs, native
precision and all executable qualification remain open.
See `indexed_paint_domain_implementation.md`.

Latest Indexed/optimized increment: source colour provenance reaches raw-window
and scaled-JPEG conversion, including scoped aliases. Indexed image pixel Decode
and palette interpretation now share a source-domain-aware path for packed and
normalized samples, with one ICC preparation per palette. Eighteen new regression
functions and revised full/window cache assertions are unexecuted. Non-image
Indexed paint is advanced by the follow-up above; other codec/component-export
integration, native precision and all executable qualification remain open.
See `indexed_and_optimized_image_domains.md`.

Latest sample-domain increment: packed Lab/ICC source samples and explicit Decode
now retain non-unit/signed values and 16-bit low-order input through alternate
conversion. Full image routes carry source colour scope separately from device
remapping and include it in cache identity. Fifteen new regressions are unrun.
Optimized alias/default provenance, Indexed domains, native CMM precision and
all executable qualification remain open. See `image_sample_domain_implementation.md`.

Latest ICC increment: paint, images and vector shading now share metadata-checked
profile/alternate conversion, with declared or implicit alternates, source-range
clipping, no-paint alpha, bounded decoding and cooperative cancellation. Indexed
image palettes preserve requested backend policy. Report counters explicitly
describe thread-local cumulative observations, not document proof. Fifteen new
regression functions and two revised regressions are unexecuted. General image
Decode/domain provenance, precision, full ICC support and all executable
qualification remain open. See `icc_alternate_conversion_implementation.md`.

Latest renderer increment: default colour-space graphs now bind at selection in
the existing raw/packed paths, including underlying Pattern/Indexed/tint spaces,
images, shadings and Type 3 replay. Initial colours, no-paint Pattern state,
Lab/Indexed component domains, ICC range clipping and filtered palette decoding
have source corrections. Tile dependencies include defaults. SVG/PS conservatively
reports native raster routing rather than emitting unremapped device colours.
Twenty-six new regression functions and three revised regressions are unexecuted.
Native vector colour preservation, full ICC domain provenance, general non-RGB
transparency and all executable qualification remain open. See
`default_colour_rendering_implementation.md`.

Latest structural-review increment: typed conflict alternatives and constrained
resolution preserve non-conflicting fields, paragraph membership/order, retained
annotation/Figure dependencies and original tag ownership. Typed table text is
no longer silently overwritten by merge-time recalculation. Review hashes and
complete acknowledgments bind decisions; resolved drafts still need native
layout approval. Shared session commands, worker/client, browser conflict panel
and pending-decision packages are wired in source. Fourteen new regression
functions remain unexecuted. This is same-base snapshot resolution, not inline rich-text
or structural causal history, automatic epoch migration or executable proof.
See `story_structure_resolution_implementation.md`.

Latest selective-undo increment: schema-2 control events toggle original own
text edits without copying atoms or rolling back other events. A grouped route
canonicalizes 1..=4096 unique same-replica edits, requires a uniform exact
activity preimage and appends one causally chained control per target atomically.
Source validation, missing-dependency retention, exact history/activity
preimages and native preview gates remain enforced. Durable save/reopen, generic
native session commands, worker/client and browser single/group operation
selection are wired. The supplied browser component also turns an idle typing
burst into one exact-source grapheme-safe operation. Eighteen regression
functions remain unexecuted. Text, paragraph-style and paragraph-structure
operation-effect undo are implemented in source; this is not authenticated,
automatic grouping for arbitrary host editors, inline rich-text or arbitrary
block-structure undo; replica
lifecycle and all executable qualification remain open. See
`story_selective_undo_implementation.md`.

Latest durable-history increment: original text identities and causal logs are
now saved alongside the native story model. Resume checks model/log hashes and
existing source-owner bindings, then projects the same epoch onto current pages.
Approval binds history as well as native layout; combined bytes publish once
with exact undo. Ordinary saves retain history but can detach it, and replacing
an epoch requires an explicit decision. Shared native commands, worker/client
publication and browser resume/adoption/detachment are wired. Fourteen new
regression functions remain unexecuted. Arbitrary external rebasing,
inline rich-text/non-paragraph block collaboration and undo, and all executable proof remain
open. See `story_history_checkpoint_implementation.md`.

Latest collaboration increment: revision/story-bound logical text operation
histories now use stable scalar identities, causal vectors, explicit missing
dependency retention, deterministic insertion trees and observed deletions.
Delta exchange and compare-and-swap authoring feed the shared native session
protocol and a browser history review/import/export panel. A projection remains
an unsaved draft; the existing native preview/approval/checkpoint still governs
PDF publication. Twelve new regressions remain unexecuted. Same-epoch rejoin is
implemented in source; the later durable-history increment adds constrained
cross-checkpoint continuation, not authenticated transport, arbitrary rebasing,
inline rich-text/non-paragraph block collaboration or executable qualification. See
`story_text_history_implementation.md`.

Latest causal-style increment: schema-3 paragraph formatting/pagination writes
use one causal multi-value register per field. Disjoint or equal concurrent writes
converge; different maximal values remain typed operation-ID candidates and block
publication until an exact history/conflict-hash-bound successor replaces every
conflicting field in one paragraph. Seed schema 2 retains the exact original style
across save/reopen so selective undo cannot treat a saved projection as its base.
Shared protocol/client commands and the supplied browser conflict editor are wired;
core, durable protocol and browser regression sources remain unexecuted. Inline
range marks, non-paragraph block operations, authenticated replica lifecycle and all
runtime qualification remain open. See
`story_paragraph_style_history_implementation.md`.

Latest causal-structure increment: schema-4 operations assign stable paragraph
IDs causal presence and predecessor registers. Unique insertions carry their full
paragraph preimage; single-item movement does not drag old successors; equal
concurrent positions converge, incompatible positions and explicit cycles block
publication, and delete-versus-concurrent text/style becomes a typed conflict.
Resolution binds the exact history and complete conflict report. Seed-schema-2
replay now reconstructs the immutable original list, so insert/move/delete and
selective activity survive the canonical native checkpoint/reopen path. Shared
protocol/client commands, capability gating and browser controls are wired.
Core, durable and browser regression source is unexecuted. Inline rich-text,
arbitrary block operations, authenticated replica lifecycle and all runtime
qualification remain open. See
`story_paragraph_structure_history_implementation.md`.

Latest causal-inline-style increment: schema-5 preferred-font, size, colour and
shaping writes target stable scalar atoms instead of mutable byte intervals.
Observed insertions inherit their resolved style; explicit clears restore
paragraph inheritance; equal concurrent values converge and unequal values remain
typed per-atom candidates until an exact one-field/all-target resolution observes
them. Selective activity can re-expose those conflicts. Start/Resume coordinator
commands, worker/client types and a browser selection/conflict panel are wired.
Horizontal and vertical layout, including supported table cells, now intersects effective style and fallback-font
partitions, orders their bidi items once, uses the same shaped runs for prepared
measurement and native emission, paints per-run size/colour and preserves the
whole logical line through ActualText. Durable preview/checkpoint saves and
reopens that projection through verified frame owners. A canonical paint-model
digest binds lines, font/style partitions, bidi context and decorations to each
new owner; an independent shaped-run digest binds exact font programs and every
emitted glyph ID, CID, mapping, advance, offset, orientation and outline metric.
The final transaction rechecks each retained shaped-run receipt after tagging,
image, anchor and page-flow mutations, and persisted metadata is fully
revalidated. General source-object provenance and executable proof remain open.
Core, durable-protocol and
browser regression sources remain unexecuted. See
`story_inline_style_history_implementation.md`.

Latest source-insertion increment: a zero-width leading-style insertion selects
the following provenance-bearing source run, while trailing style selects the
preceding run; document edges use their only adjacent run. The generated glyphs
may target a grapheme boundary inside one decoded source operand as well as an
operand edge, without making PDF producer tokenization part of the public model.
They inherit bounded font size, spacing, scaling, rise, render mode, writing mode and
exact fill/stroke commands, use an embedded or caller/bundled font only after
embedding-rights and shaped-coverage checks, and are written inside the original
text object and marked-content scope. The positioned writer carries resolved
rotated/scaled/skewed source matrices, preserves clipping union before the
original `ET`, and restores the original text matrices through exact numeric
writing-axis displacement without synthesizing an inverse. An existing direct,
isomorphic ActualText owner is spliced at the same logical scalar boundary in
the atomic stream transaction rather than discarded, preserving search/copy for
source fonts that depend on it. Non-isomorphic/shared owners still require an
explicit semantic decision; unresolved source matrix/font history, mismatched
writing modes and multi-line insertion fail closed. Regression sources are
unexecuted.

Latest field increment: ordinary text fields and all their native widget
appearances now form one approved transaction. Exact field/page ownership,
per-widget full display mappings, shared structural ActualText, optional reset
values and explicit viewer-default decisions compose into one published PDF
revision. The shared JSON plan/preview/apply routes and browser plain-field
workflow are wired in source. Twelve regression functions are unexecuted.
Specialized/stateful fields, appearance generation, XFA/NeedAppearances, broader
ownership and all executable qualification remain. See
`widget_text_transaction_implementation.md`.

Latest rendered-review increment: a canonical scoped-text plan can now produce
bounded before/candidate PNGs through the same native render contract. Exact
plan/candidate hashes, unpreviewed affected pages, pixel differences, fonts and
contract telemetry remain explicit; candidate PDF bytes are not published by
preview. Rust SDK, HTTP, C/header, Java, .NET, Python and WASM entry points are
wired in source. The shared browser worker/client now supports source discovery,
planning, preview-bound apply and existing undo/cancellation. A native-occurrence
panel displays both images and separates metadata approval from story drafts.
Nine new regression functions are unexecuted. This is not independent-render,
browser, binding or production proof; broader widget coordination/ownership,
page hit-testing and the other full-roadmap requirements remain open.

Latest scoped-transaction increment: universal analysis optionally exposes exact
Form/AP source inventories to the existing JSON binding routes. A new scoped_text
operation stages the native writer, pins generated-font bytes, binds candidate
output hashes and metadata/tag decisions, and publishes the recomputed private
candidate only after canonical approval and final-output gates. Xref-based
definition inventory includes rewritten ObjStm members without decoding every
unchanged source object; shared/tagged page invalidation is explicitly
conservative. Twenty-one new regression functions remain unexecuted. Native
source coverage boundaries, rendered preview/hit-testing, typed managed models,
performance and all runtime qualification remain open. See
`governed_scoped_text_implementation.md`.

Latest tagged-appearance increment: the native appearance writer now records its
clone map and composes with explicit exclusive-move/shared-split MCR/OBJR
migration, complete ParentTree/IDTree rebuilding and affected structural
ActualText compare-and-swap. Original programs and logical identities are
preserved. All AP states participate in liveness; alternate pattern/font/mask
resource references conservatively retain old carriers. Canonical tag scanning
now shares original-page resource fallback and ignores non-program AP metadata.
Fourteen new regression functions remain unexecuted. Other tagged owner kinds,
MCID remapping, universal approval, additional bindings and all runtime evidence
remain open. See `tagged_appearance_clone_implementation.md`.

Latest appearance-editing increment: an explicit annotation/state/stream/path
target now feeds the existing native source text writer. Copy-on-write isolates
the leaf, selected nested ancestors and normal AP slot, retaining original
programs, other states and other annotation occurrences. Reopen checks the
selected source scope and returns its output-revision target. Metadata policy
either preserves annotation comments or explicitly synchronizes exact root
FreeText Contents and discards rich text under plain-text approval. Rust, JSON
SDK and WASM source interfaces are wired. Thirteen regression functions were
added, not executed. Widget field values, broader tagged ownership migration, direct
appearance materialization, universal plan/approval, further bindings and visual
hit-testing remain open; see `appearance_text_editing_implementation.md`.

Latest appearance increment: selected source normal annotation appearances have
an explicit bounded extraction API with annotation/stream/occurrence provenance.
Forward MCR and ParentTree recovery now bind annotation StmOwn through the real
page/AP graph; repeated streams retain distinct annotation owners. Placement is
shared with the renderer and uses the Matrix-transformed BBox before mapping to
Rect. Owner indexes avoid rescanning every annotation for every marked sequence.
Eighteen regression functions were added, not executed. Legacy page-only
search/redaction selectors do not silently gain appearance targets; geometry-only
structure redaction refuses an appearance-owned selection. Broader appearance-specific
editing/search/redaction, direct owner contexts, non-annotation StmOwn and broader
visual/structure qualification remain open. See `appearance_text_scope_implementation.md`.

Latest semantic increment: Form stream identity now reaches MCR/ParentTree
binding, semantic roles/search, RAG provenance/hashes and reflow evidence.
Recovery uses the Form's StructParents key and reciprocal K context; competing
owners/duplicate keys do not become arbitrary role assignments. Repaired
evidence stays distinct from authored tags, and cross-page content no longer
gets a one-page union box. Twenty-six new regressions are unexecuted. Annotation
StmOwn extraction is extended by the appearance increment above; OBJR text
contexts, other stream owners, broader structure reconciliation, tagged Form mutation,
exact per-page geometry and executable qualification remain open; see
`semantic_stream_scope_implementation.md`.

Latest extraction increment: whole-page extraction now follows nested/repeated
Forms with explicit resource ownership, object-bound selected fonts, accumulated
geometry and scoped ActualText/MCID provenance. Text/search/layout/model inputs
share that traversal; Form edit postconditions remain direct-text-only. Strict
root parsing, error propagation, per-glyph/TJ cancellation, bounded work/output
and parallel cancellation scopes are implemented in source. Twenty-four new
regression functions remain unexecuted. Stream-owned semantic binding, exact
quads/visibility, non-text ActualText and broad execution evidence remain open;
see `scoped_text_extraction_implementation.md`.

Latest vector-export increment: SVG/PostScript and their shared classifier now
carry the original page resource scope through Forms and pattern tiles, keep
explicit local lookup separate from inherited selected fonts/colours/patterns,
and avoid silent Form omissions after scoped replay failures. Form path/clip
interpreter state is isolated/restored. Ten new regression functions remain
unexecuted. Broader default-colour, typography, pattern/compositing and extractor
parity remain; see `vector_export_scope_implementation.md`.

Latest renderer increment: canonical page rendering now separates original-page
lookup from explicit Form/AP/pattern/Type3 scopes and inherited selected objects.
Retained compilation and execution share that rule; offscreen group/mask states
carry the original page and inherited object-bound selections. Type 3 stream
resources precede font/page fallback. Program/raster cache contexts now include
relevant ownership and inherited paint state, negative cache hits remain errors,
and font keys no longer use mutable dictionary addresses as identity. Fifteen
new regression functions remain unexecuted; four previous merging tests were
revised. Broader paint-state/default-colour correctness, other exporters and all
executable qualification remain open. See `renderer_resource_scope_implementation.md`.

Latest Form-text increment: exact page-to-Form occurrence discovery and native
multi-run mutation now share caller font/paint state, Form-local resources and
copy-on-write through the invocation chain. Saved direct-text checks and renewed
targets support subsequent edits in source. Rust JSON and WASM methods are wired;
thirteen new regression functions remain unexecuted. Legacy omitted Resources now
uses page lookup separately from inherited caller state in both Form text and
vector discovery/clone resolution. Caller ActualText and tagged owner migration,
cross-scope selection, other program kinds, broader binding/approval integration
and executable renderer parity remain open. The subsequent renderer source
increment is described above. See `form_text_implementation.md`.

Latest vector increment: page-stream copy-on-write is shared by direct edits,
nested/top-level page Form clone-one, grouping and stacking-order writers.
Original streams remain untouched, repeated Contents slots have distinct IDs,
and saved reports carry live output provenance. Nested paths retain effective
resource ownership and validate their exact root slot. Stream-owned tagging
requires migration instead of silent identity duplication. Fourteen new
regression functions remain unexecuted. Form text integration, broader tagged
cloning and paint-state fidelity remain open; see
`vector_occurrence_implementation.md`.

Latest text increment: source-inline generation now tracks Tm/Tlm and both
writing-axis displacements across page content streams. Rotated/offset vertical
and positioned horizontal replacements restore the source endpoint and line
origin inside the existing paint/tag/clipping scope. Checked per-glyph
layout-origin metadata prevents repeated generation from rotating/offsetting
its own output twice. Source-axis rise and writing-mode conversion are explicit.
Fourteen regression functions were added, not executed. Broader Form integration
and executable qualification
remain open; this is not full vertical typography or roadmap completion.

Latest source-font increment: canonical resource loading now resolves indirect
ExtGState-only fonts to deterministic aliases. Editing discovery, position
tracking, same-width patches and OCR capture follow gs font changes. Page text
writers publish necessary aliases atomically; extraction loads gs-selected fonts
and the raw renderer rebinds same-name font objects. Ten added regressions remain
unexecuted. Malformed/ambiguous font semantics, exotic programs, occurrence-level
Form integration and executable proof remain open; see
`ext_gstate_font_implementation.md`.

Latest story increment: explicit horizontal/vertical-RL/vertical-LR modes use one
logical-axis paginator and the same final multi-font vertical shaping for
measurement/emission. Physical exclusions, supported table grids/cells, upright
image footprints and caption constraints convert together before downstream
ownership/anchoring/pruning. Saved mode and preview hashes retain the decision;
supported changed tags receive current WritingMode attributes. Tagged vertical-LR
requires PDF 2.0 rather than an implicit version upgrade. The browser exposes the
mode; universal plans disclose and bind it. Nineteen new vertical regression
functions remain unexecuted, plus one for the corrected page-removal prefix
preservation report. Mixed orthogonal
stories, ruby/tate-chu-yoko/full Japanese typography, broader typography and all executable
qualification remain open; see `vertical_story_implementation.md`.

Latest annotation increment: complete explicitly approved popup/reply groups,
anonymous-member identity persistence, relative annotation paint-order
preservation, atomic geometry/Annots/tag ownership changes, pruning integration
and browser association controls are implemented in source. Repeated page-local
NM names now receive distinct object identities; selected Unicode identities
persist and disclosed opt-in destination-name collision repair is receipt-bound.
The source identity resolver now also serves XFDF export/import, appearance
generation and native standalone geometry. SDK XFDF is revision-bound and keeps
NM separately; unique external aliases resolve without accepting ambiguous or
stale source selections. Standalone resize and explicit whole-group batches
preserve native dictionaries, AP/actions/field owners and supported tag owners
instead of rebuilding through XFDF. Relative geometry and source/destination
name reservations include direct or geometry-less residents.
XFDF now plans and verifies reply/popup creation, reparenting, explicit detachment,
deletion and closed cross-page moves through the same relationship graph used by
native discovery. Reciprocal-only updates preserve original owner dictionaries;
tagged deletion composes with page migration and rebuilds surviving ownership.
Ordinary direct annotation occurrences now promote atomically for native
geometry, XFDF update/delete/relationships and story staging; identical direct
dictionaries remain distinct source slots, related IDs persist before revision
changes, and output dictionaries, order and graph are checked by the code.
Unique direct widget/tagged ownership now resolves through reachable field/OBJR
carriers: existing authoritative objects are reused, direct owner slots update
with page occurrences, shared arrays copy on write, and final field/ParentTree
ownership is checked. Direct field ancestors now materialize through a planned
dependency closure, preserving stable indirect subtrees and normalizing sibling
Parent/page aliases without moving those siblings. Reports retain normalization
receipts and dependency-page invalidation. Direct structure roots/elements and
their exact ParentTree/IDTree/Ref owner copies now normalize in the same staged
transaction, retaining lookup keys and closing field/tag dependency sets before
mutation. Competing owner identities, missing semantic mappings and direct
non-page OBJR targets remain open.
Direct-source continuation pruning,
extended geometry, malformed-graph repair,
foreign-document persisted-ID remapping, shared tagged appearance cloning and
all runtime qualification remain. Native geometry and generic XFDF sanitization
have different contracts; these increments do not conflate them.
The goal remains active until the full applicable implementation and verification
requirements are met; this table is a tracker, not completion evidence.

Latest font increment: supported authoring/fallback and generated editing now
share standalone TrueType/OpenType-CFF1 classification and native CFF charset
identity. Character codes stay distinct from native CIDs and GIDs; explicit
Encoding CMaps preserve ROS and separate Unicode codes. Cached resolver/render
decoding applies embedded fixed-width CMaps before width/glyph lookup. Font
permissions, no-subsetting rules, real PostScript names and minimum output PDF
versions are retained. Thirty-two added regression functions remain unexecuted.
Predefined CMaps, CFF2/collections, non-default font transforms,
broader font programs and runtime qualification remain open. See
`cff_encoding_implementation.md`; this is not roadmap completion.

Latest character-code increment: Encoding and ToUnicode share canonical tokens,
bounded declarative parsing, inheritance and length-aware one-through-four-byte
keys. A code-space DFA supplies exact byte provenance to rendering, extraction,
source selection, displacement and redaction; reverse encoding retains
multi-scalar ligatures and reports complete-path ambiguity. Ordered ranges,
UTF-16 destination bounds, last-byte increments, native CID widths and encoded
word spacing are handled together. Preserved-style measurements include all Tc
advances, and same-width analysis rejects changed Tw movement. Twenty-five new
regression functions remain unexecuted. Complete predefined mapping packs,
arbitrary executable CMaps, malformed-byte recovery, broader font semantics and
all executable qualification remain open. See `variable_cmap_implementation.md`.

Latest predefined-font increment: 202 pinned Adobe Encoding CMaps and five
CID-to-Unicode resources are bundled offline with original-byte hashes,
licences and a source manifest. Named inheritance, mixed lengths, native CID
selection, declared collection compatibility and synthesized Unicode/reverse
encoding now share the canonical parser and bounded immutable caches. Explicit
ToUnicode wins; unknown resource names do not become identity maps. Metadata
scanning and a truncated-W2 indexing path were also corrected. Twenty-one regression
functions, including complete-resource parsing and saved named-CMap edits, are
unexecuted. Unmapped/private collection semantics, executable CMaps, malformed
recovery, broader font/layout implementation and all runtime gates remain open.
See `predefined_cmap_implementation.md`; the full goal remains active.

Latest collection-font increment: explicit hash-bound TTC/OTC face discovery and
standalone extraction now feed native authoring, registered providers, approved
edit assets and the shared session/browser editor. Source tables and glyph IDs
are retained, collection-relative offsets and rebuilt checksums are handled,
permission bits are enforced, malformed permission tables are refused and DSIG
removal requires a decision. Expansion is bounded before materialization. The
browser has a local face picker and paragraph font selection; session font payloads
are limited to 4 MiB while direct Rust preparation allows 256 MiB. Twenty-four new
regressions are unexecuted. CFF2/non-default variable instantiation, large binary
font transports, broader font semantics and the remaining roadmap/runtime gates
stay open. See `font_collection_preparation_implementation.md`; this is not full
implementation or qualification closure.

Latest CFF2 prerequisite: rendering, glyph bounds and shaped coverage now share
a bounded source decoder with per-glyph FDSelect, Private DICT variation indices,
local/global subroutine ownership, larger variation-region sets and cancellable
blend expansion. Source/static metadata is checked rather than selecting the
first local dictionary. Unknown operators follow CFF2 stack-clearing semantics;
extended stores and real-number edge cases are covered in regression source.
Twenty-five added regression functions remain unexecuted. The transient,
unhinted CFF1 glyph projection is only for outlining and is never saved as a font.
Whole-font static instantiation, variation-dependent metrics/layout, editable
CFF2 preparation, real-font/pixel evidence and the remaining roadmap stay open.
See `cff2_outline_implementation.md`; the full goal remains active.

Latest portable-instancing increment: CFF2 now shares source-bound region/delta
evaluation with the new general ItemVariationStore core, including null item
subtables. Prepared scalars, signed 32-bit rows, index maps and checked rounding
are implemented. A separate internal GSUB/GPOS feature freezer serializes selected
feature rules with stable script/language/feature/lookup identities, extension
wrappers and registered parameter owners. It is not yet the public whole-font
preparer and does not freeze variable positioning values. Twenty-four added
regressions, including reopened-font GSUB/GPOS shaping assertions, are unexecuted.
Whole-font metrics/layout/outline composition, compact large-layout packing,
unknown parameter relocation, public integration and every runtime gate remain.
See `font_variation_core_implementation.md`; the objective is not complete.

Latest coordinate/metric increment: engine and shaping now share ttf-parser
0.25.1 in the manifest/lockfile, without a resolver/build run. Checked instance
selection retains explicit normal descriptor values, validates raw axis/mapping
data and publishes coordinates atomically. Rendering uses that path; COLR stops
now retain instance coordinates and nested transforms preserve parent/child order.
An internal MVAR stage handles registered font-wide fields with checked rounding,
gasp ordering and source immutability; it is not yet the public whole-font preparer.
Twenty-six new regression functions remain unexecuted. Complete outline/metric/
layout composition, newer cross-axis normalization, broad color-font semantics,
public integration and all runtime gates remain open. See
`font_instance_metrics_implementation.md`; the full objective remains active.

Latest per-glyph metric increment: source-bound preparation now combines an
explicit source face, checked coordinates and canonical outline geometry with
HVAR/VVAR, resolved gvar phantom deltas and prior MVAR staging. It rebuilds
horizontal/vertical metric rows, header extrema and CFF vertical origins in one
immutable-source transaction, preserving GID order and reporting conflicting
redundant metrics. Twenty-eight added regression functions remain unexecuted,
including CFF2/HVAR composition and source-bound metric save/reopen assertions.
The entry remains internal, not a completed/public font instancer. Complete
outline/layout/hint/name serialization, empty-composite structural proof, fidelity
decisions, public integration and all runtime gates remain. See
`font_glyph_metric_implementation.md`; the full objective is not complete.

Latest positioning increment: internal source-bound preparation now stages GDEF,
GPOS and selected GSUB/GPOS features with the same coordinates as the metric
stage. Single/pair/cursive/mark positioning, per-owner devices, variable anchors
and ligature carets are rebuilt; classic device hints and contour-point identities
are retained separately. Cached variation evaluation, aggregate work limits and
cancellation bound the stage. Twenty-seven added regression functions, including
reopened shaping and joint metric/layout assertions, remain unexecuted. BASE/JSTF,
point-identity reconciliation, large-layout offset packing, complete static-font
serialization, public integration and all wider roadmap/runtime gates remain.
See `font_position_instance_implementation.md`; the full goal remains active.

Latest baseline/justification increment: the internal layout transaction now
rebuilds BASE scripts, languages, baseline and feature extents with its separate
store, and JSTF priorities, external lookup lists and embedded positioning with
the shared GDEF resolver. Shared-store retirement occurs only after GPOS/JSTF
rewrite; source point identities and pixel hints remain explicit. Relocation
cache lookup now uses binary search. Twenty-two added regression functions are
unexecuted, including combined saved-font byte/shaping assertions. Complete font
outline/hint/name serialization, point identity, general offset packing, wider
cross-table validation, public integration, application-level baseline/justification
behavior and every remaining roadmap/runtime gate stay open. See
`font_baseline_justification_implementation.md`; the full objective is not complete.

Latest tuple/CVT increment: a shared bounded tuple decoder now preserves duplicate
point references, signed/zero runs, shared/private point lists and per-original-
contour inferred deltas. A source-owned gvar directory checks glyph/axis identities
and payload extents; CVT freezing retains indices and rounds accumulated deltas
once. Internal source-bound metric preparation stages CVT and retains the gvar
directory with the same face/coordinates. Forty-one added regression functions
are unexecuted. The new evaluator has not replaced the production outline backend;
point-preserving glyf/loca/maxp serialization, complete hint/name/CFF2 handling,
public integration and every wider roadmap/runtime gate remain open. See
`font_tuple_variation_implementation.md`; the full objective remains active.

Latest TrueType-outline increment: internal font preparation now serializes
source-order explicit points/components through the native tuple decoder and
uses those rounded points and phantom deltas for metrics. It rebuilds glyf,
loca, head and maxp; preserves GIDs, contour identities and instruction bytes;
handles transforms, explicit-point attachments, empty composites and bounded
dependency expansion; and reports default-offset normalization. Twenty-five
new regressions, including edited-font save/reopen/reprepare assertions, remain
unexecuted. Phantom attachments, instruction-semantic freezing, cross-table point
validation, complete CFF2/name handling, public/renderer integration and every
wider roadmap/runtime gate remain open. See
`truetype_outline_instance_implementation.md`; this is not full completion.

Latest TrueType hint/identity increment: internal source-bound preparation now
appends a selected-coordinate GETVARIATION fallback, inventories instructions
without scanning push payloads as code, preserves original program offsets and
synchronizes hint/profile capacity with rebuilt glyph maxima. Early-query,
capability and dynamic-control cases produce unresolved review receipts. GPOS,
JSTF, GDEF and BASE contour references are bound to exact generated TrueType point
domains, including shared-record owners; isolated/CFF preparation without that
authority explicitly records unchecked references. Thirty-six added regressions
remain unexecuted, including saved-font fallback and cross-owner cases. Full hint
semantics, phantom attachments, CFF2/name/color completion, the public whole-font
transaction, wider editor/rendering work and every runtime gate remain open. See
`truetype_hint_instance_implementation.md`; the full goal remains active.

Latest public-font increment: selected variable TrueType outlines, metrics,
CVT, layout, point checks and hint fallback now publish through one static-font
transaction with explicit names/style links and STAT name-owner relocation.
Source hashes, permissions, signature decisions, metric discrepancies, table
owners and exact saved table receipts gate output. Editing assets, provider and
authoring registration, retained-session JSON and the browser font picker share
the same bytes. Thirty-one added regressions remain unexecuted. This closes the
earlier missing public path only for the declared supported TrueType subset;
CFF2 publication is extended below; wider font/hint/color semantics, broader editor/rendering work
and every executable/corpus qualification remain open. See
`static_truetype_instance_implementation.md`; the full objective is not complete.

Latest CFF2 publication increment: the public static-font transaction now emits
native CFF1 with unchanged GIDs/CIDs, actual source FD/subroutine ownership,
selected private hints/blends, masks, flex and widths from frozen metrics.
Identical private semantics coalesce, crossing stem-snap sets normalize, and
whole-font metric/layout/name stages share exact saved-table receipts and
structural reopen checks. Editing/authoring/session paths and browser capability
2 use the same output. Twenty-four new regression functions remain unexecuted.
Contour overlaps are retained in default mode; opt-in normalization is extended
below. Exact hint preservation remains implementation work, alongside wider font owners,
the complete editor/rendering roadmap and every executable/corpus gate. See
`cff2_static_instance_implementation.md`; this is not whole-roadmap completion.

Latest contour increment: explicit CFF2 normalization now resolves nonzero-filled
regions with pinned curve-preserving topology, quantizes emitted points, checks
saved boundaries and numerical area differences, and rebases metrics from source
owners. Hint loss requires separate consent and reports identify affected glyphs.
Complexity preflights and cooperative boundaries surround synchronous solver
calls. Native/browser capability 3 shares the option through the existing public
transaction. Sixteen new regressions remain unexecuted. Dependency resolution,
builds, corpus and independent geometry/pixel evidence remain unverified; exact
hint-preserving union, rehinting, hard solver cancellation and all wider roadmap
requirements remain open. See `cff2_contour_normalization_implementation.md`.

Latest selective-hint increment: opt-in normalization now retains compatible
glyph programs and frozen private hints instead of discarding hints across the
whole font. Numerical boundary classification accounts for merged/cancelled
source edges and crossing vertices. Rewritten glyphs use a separate unhinted
owner with explicit consent; shared private hints remain on compatible glyphs.
Output FD allocation follows these decisions, with bounded shared-byte interning
and separate retained/bypassed/removed receipts. Browser copy/typings and older
receipt decoding follow the same policy. Eight additional regression functions
remain unexecuted, including retained-story save/reopen/edit. Exact hint transfer
on rewritten outlines, rehinting, independent topology/raster qualification,
hard solver cancellation and every wider roadmap requirement remain open. See
`cff2_contour_normalization_implementation.md`; the full objective stays active.

Latest exact-linear increment: explicit CFF2 normalization now uses bounded
16.16 integer/i128 contact, orientation and winding predicates for straight-edge
outlines. Compatible programs retain hints and bytes without an epsilon solve;
one-quantum contacts cannot be overridden by the numerical compatibility path.
Exact-work budgets, cancellation and additive receipts carry through existing
native/session/browser contracts. Fourteen new regression functions remain
unexecuted. Curved geometry is still numerical, default preserve-source mode
rejects unchecked overlaps, while hint transfer and independent qualification
and every broader editor/rendering requirement remain open. See
`cff2_exact_linear_implementation.md`; the full objective is not complete.

Latest checked-publication increment: default CFF2-to-CFF1 preparation no longer
publishes unchecked overlapping contours. It preserves exact expanded programs
and hints only after exact-linear or bounded curved compatibility checks, emits
an additive preservation receipt, and otherwise refuses with the explicit
normalization/hint-decision route. Flat zero-ink flex programs remain preserved.
Native/browser font capability 4 prevents an older worker from silently applying
the former contract. Four new regression functions and extended cases remain
unexecuted. Curved checks remain numerical, rewritten hint transfer and all
independent qualification remain open, as do the broader roadmap requirements.
See `cff2_checked_preservation_implementation.md`.

Latest separate-OCR-owner increment: a Figure may explicitly consume one or
more selected direct sibling P/Span leaves whose complete page-owned or Form
content is partitioned by exact invisible-OCR span ID. The one-shot
`merge_into_figure` decisions are source-bound, reject semantic attributes,
relationships, subtrees, extra content, repeated owners or inference, remove
the old owners with the selected sibling interval, and let the final Figure page
MCR own the native visual/search group. Explicit reused-Form splitting preserves
an ordered residual for every consumed OCR owner. Browser review and request
typing expose one assignment per exact span. Additional regression source covers
page multi-owner merge/reopen, partition refusal cases and unique/reused Forms,
but remains unexecuted. Realistic scan reconstruction and every runtime/corpus
gate remain open. See
`separate_ocr_owner_implementation.md`; the full objective stays active.

Latest Figure-relationship increment: reused-Form split policies accept bounded
nested graphs of direct and indirect `/Ref` arrays whose leaves are structure
references. Incoming follow/retarget/reference-both and outbound
move/retain/copy retain array topology, preserve repeated-container aliasing in
each result, allocate private indirect paths and leave shared source containers
untouched. Resolution is depth/item/visit bounded and rejects cycles,
non-array indirect objects, non-structure leaves, empty/excessive containers and
malformed mixed arrays. New outbound/incoming/shared/nested/malformed-container
regression functions are present but unexecuted. Relationship representations
outside this governed graph, content-bearing/shared reused Figure subtrees and
every runtime/corpus gate remain open.

Latest Figure semantic-subtree increment: an existing Figure can opt into
preserving a bounded descendant structure tree while its direct source-content
items are replaced by the generated page MCR at their original structural
position. Preflight requires unique acyclic parentage, a content-owning root and
descendants with no MCID/MCR/OBJR, paint or text; valid explicit descendant
`/Pg` bindings follow the Figure destination, while malformed bindings refuse.
Metadata and descendant order survive save/reopen. A distinct one-shot approval
deletes the complete validated contentless tree only after the structural
relationship scanner proves that no surviving element or effective attribute
targets any member. A separate one-shot approval lets a reused-Form split retain
the original descendants on the selected Figure and copy-on-write clone the
contentless tree for the residual Figure. The clone rebuilds `/P`, `/K` and
internal Figure/subtree `/Ref` links, removes copied identity/page/story bindings,
and applies the root's explicit outbound/incoming policies to external links on
any member. Multiple trees split in one request share a coordinated map so their
selected and residual graphs remain separate. The complete relationship map is checked against an exact
receipt after reopen. Positive preservation/deletion/cloning/reopen regressions,
page-rebinding/malformed-binding coverage, missing-policy refusal, subtree-wide
external relationship routing and nested relationship-container copy-on-write
are present but unexecuted. Content-bearing or externally shared reused-Form
subtree migration, relationship representations outside the bounded governed
graph and every runtime/corpus gate remain open. See
`tagged_figure_implementation.md`; the full objective stays active.
