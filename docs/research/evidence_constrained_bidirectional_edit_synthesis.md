# Evidence-Constrained Bidirectional Edit Synthesis (ECBES)

## Research and engineering specification

| Field | Value |
| --- | --- |
| Working name | Evidence-Constrained Bidirectional Edit Synthesis |
| Acronym | ECBES |
| Initial specification | 27 September 2026 |
| Repository | WellPDF SDK |
| Algorithm version in source | 1 |
| Specification status | Working research specification |
| Implementation status | Deterministic kernel and canonical universal-edit transaction adapter implemented; runtime and corpus qualification pending |
| Current claim | A new proposed PDF-edit-synthesis framework with source/API/binding integration and deliberately conservative version-1 provenance |
| Claims explicitly not made | Universal correctness, academic novelty, patentability, production qualification, or superiority over another editor |

ECBES is a framework for deciding **how a requested PDF edit may be performed**, not
just whether one hard-coded writer happens to support it. It models the affected
source and render dependencies, generates or receives competing edit constructions,
rejects any construction without the required evidence, and chooses a qualified
construction deterministically while preserving its real fidelity class.

The central rule is:

> Never publish an edit merely because it produced output bytes. Publish it only
> when the chosen construction satisfies the edit contract, covers the complete
> known influence cone, passes the proof obligations required for its fidelity
> class, and discloses any reconstruction.

This document explains where that rule came from, the formal model, the current
algorithm, how it is expected to integrate with WellPDF SDK, and what remains to be
proven.

---

## 1. Why ECBES was needed

### 1.1 A PDF is not a conventional editable document

A PDF page is primarily a painting program. Its visible result may be assembled
from text-showing operators, positioned glyphs, clipping text, vector paths, image
samples, transparency groups, patterns, annotations, Form XObjects, optional
content, or combinations of them. Logical information such as paragraphs, reading
order, Unicode text, table structure, font identity, and authoring constraints may
be incomplete or absent.

The inverse problem is therefore non-unique. Identical pixels can be produced by
different PDF programs, and some information cannot be recovered from the file:

- a font subset may not contain the glyph the user wants to type;
- a glyph code may have no reliable Unicode mapping;
- scanned pixels do not contain the original text or the background hidden beneath
  printed letters;
- a group of positioned glyphs does not uniquely reveal its original paragraph or
  text-frame structure;
- a shared Form or resource does not by itself reveal whether the user meant to
  change every occurrence or only one occurrence; and
- a visually plausible result can still have stale search text, broken tags,
  invalid resource ownership, or incompatible output bytes.

Consequently, “edit anything” cannot safely mean “always force one writer path to
return a PDF.” A safe system needs several edit strategies, explicit reconstruction,
and an evidence-backed method for deciding among them.

### 1.2 The recurring engineering failure pattern

Repeated source reviews of the editing and rendering paths revealed a common
pattern. Fixing an isolated writer defect did not make the whole system universal:

1. An exact operator rewrite could preserve source semantics but fail when a font
   subset lacked new glyphs.
2. Appending replacement content could make the page look correct but change paint
   order, inherit graphics state, or leave stale `/ActualText` or OCR text.
3. Reflow could fit text locally but disturb later frames, page structure,
   annotations, or tagged reading order.
4. Scan reconstruction could produce a visible edit without recovering unknowable
   background pixels or original typography.
5. A same-engine render comparison could miss a defect shared by that engine's
   writer and renderer.
6. A successful save could still fail after reopen, re-edit, independent extraction,
   or rendering in another implementation.

The important observation was that these were not unrelated bugs. They were
instances of one missing abstraction: **the system lacked a governed way to compare
multiple possible source transformations under explicit preservation evidence**.

### 1.3 How the idea was derived

ECBES emerged as an engineering synthesis in six steps:

1. **Replace the universal-editing claim with an edit contract.** The request must
   state the intended change, source revision, authority, postconditions, and what
   must remain unchanged.
2. **Model causality, not only geometry.** A selected glyph can depend on a font,
   graphics state, clip, soft mask, Form, semantic owner, annotation, or page-tree
   relation. These dependencies form an influence graph.
3. **Treat editing as candidate synthesis.** Exact operand replacement, resource
   cloning, local reconstruction, semantic reflow, and scan reconstruction are
   different candidate constructions for the same user intent.
4. **Give every construction a fidelity class.** Reconstruction is legitimate when
   authorized, but it must never be mislabeled as recovery of missing native source.
5. **Make validation an admission gate.** Security, source revision, reopen,
   logical, structural, pixel, renderer, and resource checks are proof obligations,
   not optional diagnostics after publication.
6. **Select reproducibly.** Qualified candidates are compared by fidelity and an
   integer cost vector, with the complete decision hashed into an audit certificate.

This is the sense in which ECBES was “discovered”: a sequence of concrete PDF
editing failures was reduced to a general decision problem, and concepts from
bidirectional transformations, dependency graphs, constraint-based layout,
differential testing, and uncertainty-aware abstention were composed into one
PDF-specific architecture.

It is more precise to call ECBES a **new proposed framework and project-originated
composition** than to claim that every ingredient is new. A defensible academic
novelty claim requires a broader literature and patent search, a precise comparison
against the closest systems, and independent peer review.

---

## 2. Design goals and non-goals

### 2.1 Goals

ECBES is designed to:

- support multiple real PDF source-editing and reconstruction routes;
- preserve exact source ownership and revision identity;
- identify all known objects and observations an edit can influence;
- fail closed when evidence is missing, failed, or unverified;
- distinguish native edits from semantic and appearance reconstruction;
- minimize unnecessary object, byte, pixel, semantic, layout, and font changes;
- produce deterministic decisions independent of candidate input order;
- retain machine-readable rejection reasons;
- support cancellation and bounded resource use; and
- bind planning, generation, validation, selection, and publication into one
  hash-bound transaction receipt.

### 2.2 Non-goals

ECBES does not:

- infer the unknowable original authoring document;
- guarantee that every PDF is editable without user approval or substitution;
- turn reconstructed scan content into authentic recovered source;
- prove that all PDF viewers render identically;
- make a failed or unverified obligation pass;
- replace the parser, writer, renderer, font engine, OCR engine, or layout engine;
- establish superiority over Acrobat, InDesign, MuPDF, PDFium, Poppler, or another
  implementation without controlled comparative evidence.

---

## 3. Core model

### 3.1 Three connected representations

For input PDF bytes `S`, the complete ECBES architecture uses three connected
representations:

- `G_s`, the source graph: indirect objects, streams, operators, byte ranges,
  resources, ownership, revisions, and encryption authority;
- `G_l`, the logical graph: text, grapheme clusters, styles, paragraphs, linked
  frames, tables, figures, tags, annotations, and relationships; and
- `G_r`, the render-evidence graph: display-list operations, paint order,
  transforms, clips, masks, transparency groups, spatial support, rasters, and
  renderer observations.

No graph is universally canonical. `G_s` is authoritative for existing bytes and
owners, `G_l` is the editable view when logical structure is available or approved,
and `G_r` is the evidence surface for appearance and causal render impact.

### 3.2 Edit contract

An edit is modeled as:

```text
E = (intent, revision, authority, preconditions, postconditions,
     preservation_set, reconstruction_policy, resource_budget)
```

Where:

- `intent` describes the desired logical or visual change;
- `revision` binds the request to exact input bytes or a revision identity;
- `authority` records permission to modify the relevant content and security
  envelope;
- `preconditions` identify selected source owners and expected existing values;
- `postconditions` define the required output text, geometry, semantics, or pixels;
- `preservation_set` defines invariants outside the authorized impact;
- `reconstruction_policy` controls whether semantic or appearance reconstruction
  is permitted; and
- `resource_budget` bounds time, memory, graph size, candidate count, decoded data,
  and output expansion.

The current kernel receives only the graph, seeds, candidate records, evidence, and
selection policy. The richer transaction contract is an integration target.

### 3.3 Influence graph

The typed directed graph is:

```text
G = (V, D)
```

An edge `u -> v` means that changing `u` can affect the rendered or semantic
interpretation of `v`. Version 1 defines these edge kinds:

| Edge kind | Meaning |
| --- | --- |
| `paint_order` | A paint operation can change the composited result of a later or related operation |
| `graphics_state` | Transform, color, opacity, blend, or other state affects a consumer |
| `clip` | A path or text clip constrains later painting |
| `soft_mask` | A mask affects alpha or color contribution |
| `resource` | A font, image, color space, pattern, or other resource is consumed |
| `form_invocation` | A page or Form invokes shared Form content |
| `layout` | A frame, paragraph, anchor, table, or pagination choice affects another node |
| `semantic_owner` | Logical content, marked content, tags, or reading order owns or interprets a node |
| `annotation` | Appearance, action, relationship, or page annotation depends on a node |
| `page_tree` | Page insertion, removal, labels, destinations, or inherited values are affected |

Node identifiers are opaque stable strings to the kernel. The future graph builder
must define a versioned identifier grammar and bind every identifier to source
revision evidence.

### 3.4 Influence cone

For selected seed nodes `A`, the influence cone is the least fixed point:

```text
I_0     = A
I_k+1   = I_k union { v | exists u in I_k and (u -> v) in D }
I(A)    = fixed_point(I_k)
```

The implemented kernel performs a deterministic breadth-first transitive closure.
It is cycle-safe, cancellation-aware, and bounded. It rejects an over-limit query
instead of treating a truncated graph as complete evidence.

A candidate may conservatively cover more graph nodes than the exact cone, but it
may not omit a node in the cone or name a node absent from the graph. The kernel
removes any caller-supplied `influence_closure` assertion and replaces it with its
own computed pass/fail evidence and cone digest.

### 3.5 Candidate construction

A candidate is one complete proposed way to realize the edit. It contains:

- a stable candidate ID;
- a fidelity class;
- exact indirect-object owners;
- one-based affected page numbers;
- the graph nodes it covers;
- one evidence record per proof obligation; and
- an integer cost vector.

Candidate generation is intentionally separate from selection. Expected generator
families, from most source-preserving to most reconstructive, are:

1. **Operator lens**: rewrite exact content operands in their original paint slot,
   preserving advance, marked content, state, and source ownership.
2. **Resource lens**: clone or update a precisely owned font, image, pattern,
   appearance, color-space, or Form resource graph.
3. **Local program reconstruction**: replace a bounded paint partition with newly
   authored, searchable/selectable PDF content.
4. **Semantic region reconstruction**: rebuild an approved frame, story, table,
   figure, or tagged region while preserving opaque siblings.
5. **Appearance reconstruction**: alter scan pixels or outlines and add newly
   authored semantics with uncertainty and substitution disclosure.

The integrated adapter invokes the canonical universal-edit planner and applier for
each explicit or deterministically generated candidate from the same immutable
normalized input revision.
That supplies the existing operator, resource, story, structure, image, vector,
object-graph, and document-subsystem routes without introducing a second writer.
Version 1 automatically enumerates operator-preserving, geometric-block, and
semantic-document constructions for text intents, and the canonical construction
for every other universal operation. The caller may also supply explicit candidates
and still owns route-specific approvals. Automatic enumeration is deliberately
limited to real canonical routes; it does not invent unsupported transformations.

### 3.6 Fidelity chain

Version 1 uses a three-level preference chain:

```text
source_native
    > semantic_reconstruction
        > appearance_reconstruction
```

| Fidelity | Meaning | Required disclosure |
| --- | --- | --- |
| `source_native` | Existing source operators or resources are edited through exact owners | Exact owners and lens laws |
| `semantic_reconstruction` | Missing logical structure is reconstructed into real PDF content | Reconstructed structure, substitutions, and independent render evidence |
| `appearance_reconstruction` | Pixels or outlines are reconstructed and paired with new semantics | Visual reconstruction, uncertainty, affected region, and independent render evidence |

This ordering is a selection preference and provenance statement. It does not claim
that a native candidate is always more attractive visually. A reconstruction may
move less layout or produce a closer local appearance, so non-dominated alternatives
remain visible on the Pareto frontier.

---

## 4. Proof obligations

`pass`, `fail`, and `unverified` are separate states. Only `pass` satisfies a
required obligation. Missing evidence is also a rejection.

### 4.1 Base obligations

Every fidelity class requires:

| Obligation | Required evidence |
| --- | --- |
| `security_authority` | The caller has permission to perform this mutation under document and application policy |
| `source_revision` | Candidate owners and ranges belong to the exact input revision |
| `influence_closure` | Candidate covers every node in the kernel-computed cone and no unknown node |
| `reopen` | Serialized output reopens successfully under the declared output policy |
| `logical_postcondition` | Requested text, values, relationships, or other semantics hold after reopen |
| `outside_influence_pixels` | Pixels outside the governed influence support satisfy the declared preservation threshold |
| `structure_integrity` | Object graph, page tree, resources, tags, annotations, and relevant relationships remain valid |
| `resource_budget` | Candidate generation, validation, output, and retained data stay within policy limits |

### 4.2 Fidelity-specific obligations

| Fidelity class | Additional required obligations |
| --- | --- |
| `source_native` | `lens_round_trip` |
| `semantic_reconstruction` | `independent_renderer_agreement`, `reconstruction_disclosure` |
| `appearance_reconstruction` | `inside_edit_intent`, `independent_renderer_agreement`, `reconstruction_disclosure` |

The policy may add more required obligations to every candidate.

### 4.3 Lens laws

For a source `s`, view function `get`, and backward update `put`, the intended
native-edit laws are based on the familiar lens principles:

```text
GetPut: put(s, get(s)) ~= s
PutGet: get(put(s, v)) = v
```

For PDF editing, `~=` cannot always mean byte identity because legal serialization
may renumber objects or normalize representation. The production integration must
declare the exact equivalence relation used by each lens, for example:

- exact untouched stream bytes;
- canonical rooted-object-graph equivalence;
- decoded stream equivalence under declared filters;
- semantic text equivalence; or
- render equivalence under a versioned render contract.

Contract-lens ideas are relevant because many PDF lenses are partial: a source
condition may require an exact owner, supported encoding, and unshared resource;
the view condition may require shaped text that fits a governed region. ECBES does
not turn a partial lens into a total one—it records when a different candidate or an
explicit refusal is necessary.

### 4.4 Evidence trust boundary

The integrated transaction independently computes influence closure, security and
revision binding, strict reopen, logical-route postconditions, structural
resolution, resource budgets, reconstruction disclosure, and qualifying internal
lens results. Whole-document raster-preservation contracts produce outside-cone
pixel evidence. Reference rasters can produce independent-renderer evidence only
when every raster carries a matching named/versioned independent producer and an
artifact digest equal to its exact RGBA bytes. Other evidence remains supplied by
bound external validators. Therefore:

- a decision certificate proves what evidence and policy were considered;
- it does not prove that a dishonest or defective evidence producer reported truth;
- production integration must bind each evidence digest to the actual validator,
  input and output hashes, configuration, and tool identity; and
- high-value obligations should be produced by independent components where
  feasible.

This boundary is essential. Without it, a SHA-256 decision digest could be mistaken
for a correctness proof when it is only a tamper-evident record of supplied data.

---

## 5. Cost model and deterministic selection

### 5.1 Integer cost vector

Each candidate has this ordered vector:

```text
C(c) = (
  changed_indirect_objects,
  rewritten_opaque_bytes,
  outside_mask_changed_pixels,
  semantic_distance_microunits,
  layout_displacement_micropoints,
  font_substitution_penalty,
  reconstruction_uncertainty_ppm
)
```

Integer and fixed-point units prevent cross-platform floating-point tie drift.
Every producer must use a versioned measurement definition; otherwise equal-looking
numbers from different generators are not comparable.

### 5.2 Pareto dominance

Candidate `a` dominates candidate `b` when:

1. `a` is no worse than `b` in fidelity rank and every cost dimension; and
2. `a` is strictly better in at least one of those dimensions.

All qualified, non-dominated candidates form the Pareto frontier. This keeps a
meaningful alternative visible when, for example, it changes fewer objects but uses
a lower fidelity class.

### 5.3 Publication choice

Version 1 selects one frontier member lexicographically by:

```text
(fidelity rank, cost vector field order, stable candidate id)
```

Thus source-native fidelity is preferred first; then the cost fields above are
compared in declaration order; the stable candidate ID breaks a complete tie.
Candidate input order does not control the result.

This is deliberately simple and auditable. A later version may use policy profiles
or constrained optimization, but it must remain versioned and deterministic and
must not silently change the meaning of version 1.

### 5.4 Reconstruction policy

The default policy refuses semantic and appearance reconstruction. The caller must
explicitly authorize each class. Non-native candidates must also remain at or below
the configured uncertainty threshold, which defaults to 50,000 parts per million.

The uncertainty field is evidence—not automatically a calibrated probability.
Probability or coverage language is permitted only after a calibration method,
held-out population, assumptions, and measured validity are documented.

---

## 6. End-to-end algorithm

### 6.1 System pipeline

```text
PDF bytes + edit intent + authority
                |
                v
     parse and bind source revision
                |
                v
 build source/logical/render provenance graph
                |
                v
 select seeds and compute influence cone  <--- integrated, conservative v1
                |
                v
 plan competing universal-edit candidates <--- integrated
                |
                v
 materialize private candidate outputs     <--- integrated
                |
                v
 run internal and bound external validators <--- integrated
                |
                v
 reject invalid/unverified candidates     <--- kernel implemented
                |
                v
 compute Pareto frontier and selection    <--- kernel implemented
                |
                v
 hash decision certificate                <--- kernel implemented
                |
                v
 approval / selection-only publication / receipt <--- integrated
```

### 6.2 Version-1 kernel pseudocode

```text
function ECBES(request):
    validate graph, bounds, identifiers, edges, seeds, and policy
    cone = deterministic_transitive_closure(request.graph, request.seeds)
    cone_digest = SHA256(serialize(cone))

    for candidate in request.candidates:
        remove caller-supplied influence_closure evidence
        covered = set(candidate.influence_nodes)
        missing = cone.nodes - covered
        unknown = covered - graph.nodes
        append computed influence evidence:
            pass iff missing is empty and unknown is empty
            bind cone_digest

    for candidate in candidates:
        validate IDs, unique owners/pages/nodes, evidence shape, and uncertainty
        required = base obligations
                 union fidelity-specific obligations
                 union policy-required obligations
        reject if reconstruction class lacks policy authority
        reject if uncertainty exceeds policy
        reject if any required obligation is missing, failed, or unverified

    qualified = stable_sort(all non-rejected candidates)
    frontier = qualified candidates not dominated by another qualified candidate
    selected = lexicographic_min(frontier by fidelity, costs, ID)

    certificate_input = algorithm name, version, policy, all qualified candidate
                        records, selected record, frontier IDs, rejected candidates
    decision_digest = SHA256(serialize(certificate_input))

    return cone, selected, qualified IDs, frontier IDs, rejections, decision_digest
```

### 6.3 Failure semantics

The API distinguishes invalid requests from rejected candidates:

- malformed IDs, unknown graph nodes, duplicate edges, duplicate candidate IDs,
  repeated evidence obligations, invalid digests, invalid page numbers, and invalid
  limits return an error;
- a valid candidate with missing, failed, or unverified proof is returned in the
  rejected list with reasons;
- if every candidate is rejected, the decision is valid but `selected` is `null`;
- resource-limit overflow fails the request rather than weakening validation; and
- cancellation interrupts influence closure through the shared cancellation scope.

No-candidate-selected is a governed refusal, not an algorithm crash.

---

## 7. Worked example

Suppose the user replaces a word painted inside a clipped Form XObject. Two edit
constructions are available:

- `native_form_clone`: clone the shared Form for one occurrence and rewrite the
  exact text operand with the original font;
- `local_reconstruction`: remove the selected paint partition and author replacement
  text with an approved substitute font.

The seed is the selected operand. The graph connects it to the affected clipping
result, containing Form invocation, and page paint partition. ECBES computes that
full cone. If
`native_form_clone` declares only the operand, it is rejected even when its output
looks correct in one renderer, because its influence coverage is incomplete.

If the native candidate covers the cone and passes every native obligation, it is
preferred. If it fails because the embedded subset lacks the required glyph, that
failure does not authorize reconstruction automatically. The reconstruction
candidate becomes eligible only when:

- policy explicitly allows semantic reconstruction;
- the font substitution is disclosed;
- reopen, logical, structural, pixel, independent-renderer, and resource evidence
  passes; and
- its uncertainty does not exceed policy.

If neither candidate qualifies, ECBES returns a refusal with evidence-backed
reasons. It does not silently append an overlay or claim the word was natively
edited.

---

## 8. Current Rust interface

The selection kernel lives in
`crates/engine/src/research_edit_synthesis.rs` and exposes:

- `EditInfluenceGraph::validate`;
- `EditInfluenceGraph::influence_cone`;
- `synthesize_edit_plan`; and
- `synthesize_evidence_constrained_edit`.

The transaction adapter lives in `crates/engine/src/ecbes_universal.rs`. The engine
SDK exposes both the standalone research kernel and the PDF-producing transaction:

```rust
pub fn evidence_constrained_edit_synthesis_json(
    request_json: &str,
) -> Result<String>

pub fn ecbes_universal_edit_json(
    bytes: &[u8],
    request_json: &str,
    password: Option<&[u8]>,
) -> Result<(Vec<u8>, String)>
```

The kernel wrapper refuses requests larger than 32 MiB. The transaction wrapper
accepts at most 256 MiB of JSON, materializes at most 64 candidates, applies
per-candidate and aggregate output-byte budgets, and returns the shared versioned
report envelope with the selected PDF. When no candidate qualifies it returns the
caller's exact original transport bytes, including an encrypted container that was
normalized only for planning.

The transaction request contains a synthesis policy plus `candidates` and/or
`automatic_candidates`. Each explicit candidate contains a stable ID, declared
fidelity, one canonical `UniversalEditRequestV2`, an optional approval decision,
fixed-point cost hints, and externally produced proof records. An automatic entry
contains an ID prefix and universal request. Text expands to `operator`,
`geometric`, and `semantic` candidates; other operations expand to `canonical`.
The optional `route_approvals` and `route_external_evidence` maps use those suffixes.
Output credentials use separate apply-only entry points and are never serialized
into a plan, report, or receipt.

Independent raster provenance is part of each
`UniversalReferenceRasterV2.producer`: `name`, `version`, `artifact_sha256`, and
`independent`. The artifact digest must equal the exact supplied RGBA bytes. ECBES
creates a passing independent-renderer obligation only when every selected page has
matching producer identity, `independent` is true, and the canonical edit contract
successfully compares all references with the materialized output.

### 8.1 Minimal request shape

The following abbreviated request demonstrates the wire model. Production evidence
must carry meaningful details and digests from bound validators.

```json
{
  "graph": {
    "nodes": ["operand:7:0:42", "font:12:0", "page:1"],
    "edges": [
      {"from":"operand:7:0:42","to":"font:12:0","kind":"resource"},
      {"from":"font:12:0","to":"page:1","kind":"paint_order"}
    ]
  },
  "seeds": ["operand:7:0:42"],
  "max_influence_nodes": 16384,
  "policy": {
    "required_obligations": [],
    "allow_semantic_reconstruction": false,
    "allow_appearance_reconstruction": false,
    "max_candidates": 256,
    "max_reconstruction_uncertainty_ppm": 50000
  },
  "candidates": [{
    "id": "native-operand-rewrite",
    "fidelity": "source_native",
    "owners": [{"object_number":7,"generation":0}],
    "affected_pages": [1],
    "influence_nodes": ["operand:7:0:42", "font:12:0", "page:1"],
    "evidence": [
      {"obligation":"security_authority","status":"pass","detail":"bound authority"},
      {"obligation":"source_revision","status":"pass","detail":"revision matched"},
      {"obligation":"lens_round_trip","status":"pass","detail":"lens laws passed"},
      {"obligation":"reopen","status":"pass","detail":"output reopened"},
      {"obligation":"logical_postcondition","status":"pass","detail":"replacement found"},
      {"obligation":"outside_influence_pixels","status":"pass","detail":"outside mask unchanged"},
      {"obligation":"structure_integrity","status":"pass","detail":"structure valid"},
      {"obligation":"resource_budget","status":"pass","detail":"within limits"}
    ],
    "cost": {
      "changed_indirect_objects": 1,
      "rewritten_opaque_bytes": 0,
      "outside_mask_changed_pixels": 0,
      "semantic_distance_microunits": 0,
      "layout_displacement_micropoints": 0,
      "font_substitution_penalty": 0,
      "reconstruction_uncertainty_ppm": 0
    }
  }]
}
```

The caller omits `influence_closure`; the kernel computes and inserts it.

### 8.2 Implemented bounds

| Item | Version-1 limit |
| --- | ---: |
| Graph nodes | 1,000,000 |
| Graph edges | 4,000,000 |
| Influence-cone nodes | 250,000 hard maximum |
| Default request cone limit | 16,384 |
| Candidates | 4,096 hard maximum |
| Default policy candidate limit | 256 |
| Candidate or graph-node ID | 256 UTF-8 bytes, no control characters |
| Evidence detail | 16 KiB per record |
| JSON SDK request | 32 MiB |
| Integrated transaction JSON request | 256 MiB |
| Integrated materialized candidates | 64 |
| Default candidate output | 512 MiB |
| Default aggregate candidate output | 1 GiB |
| Uncertainty | 0 to 1,000,000 ppm |

### 8.3 Determinism and certificate scope

The kernel uses ordered maps and sets plus stable sorting. The decision digest is
SHA-256 over `serde_json` serialization of a versioned Rust structure containing the
policy, all qualified candidate records, the selected record, frontier IDs, and
rejected candidate IDs, digests, and reasons.

This is deterministic for an identical parsed request under the current schema and
implementation. Nodes, seeds, adjacency, closure edges, owners, pages, obligations,
and candidate order are canonicalized before the decision digest is constructed.
It is not yet an RFC 8785 or other external canonical-JSON standard. Cross-language
certificate reproduction therefore uses the shared native implementation unless a
future byte-level serialization profile is published.

---

## 9. Complexity and resource behavior

Let `|V|` be graph nodes, `|D|` graph edges, `N` candidates, `P` proof obligations,
and `K` cost dimensions.

- graph validation and adjacency preparation are bounded by graph size, with ordered
  collection overhead;
- influence closure is conceptually `O(|V| + |D|)` after adjacency construction;
- candidate evidence validation is `O(N * P)` apart from ordered-set operations;
- the current Pareto-frontier calculation is `O(N^2 * K)`;
- final stable sorting is `O(N log N)`; and
- retained memory is `O(|V| + |D| + candidate evidence)`.

The quadratic frontier is acceptable under the current default of 256 candidates
and hard maximum of 4,096, but should be measured. A later implementation may use a
more efficient skyline algorithm without changing version-1 decision semantics.

Cancellation is polled during graph closure and at transaction, candidate,
reopen-page, and indirect-object boundaries. Canonical edit routes inherit the
shared cancellation scope; interruption inside a non-cooperative codec remains
bounded by that codec call.

---

## 10. Integration architecture for WellPDF SDK

### 10.1 Integrated adapter and next provenance refinement

The version-1 universal adapter implements the following transaction surfaces:

1. **Revision binder**: hash input bytes, authenticate permissions, and bind all
   source identities to the opened revision.
2. **Plan-derived provenance graph builder**: connect one revision-bound intent to
   canonical request digests, affected indirect owners, and affected pages, with
   typed semantic-owner, resource, layout, and paint-order edges. Operator, Form,
   graphics-state, clip, mask, annotation, and spatial-render edges can refine this
   graph without changing the transaction contract.
3. **Intent-to-seed resolver**: version 1 derives one stable intent seed from the
   revision and canonical candidate request digests.
4. **Candidate adapters**: invoke universal object lenses, source text editing,
   linked stories, table and figure editing, annotation and appearance routes, and
   scan reconstruction without bypassing their typed refusals.
5. **Sandboxed materializer**: create bounded temporary output for each candidate
   without publishing it.
6. **Evidence producers**: reopen, extract, validate structure, compare source
   graphs, render, compare pixels, and measure resources.
7. **Decision-to-transaction bridge**: bind the selected candidate and certificate
   to approval, atomic save, output hash, and history receipt.
8. **Binding surfaces**: expose the same policy, fidelity, evidence, refusal, and
   receipt semantics through C, .NET, Java, Python, WASM, and the HTTP server. A
   dedicated CLI command is not part of version 1.

### 10.2 Transaction rule

Candidate evaluation must not mutate the user's active document. A production flow
should be:

```text
immutable input -> isolated candidates -> validation -> selected candidate
                -> approval -> atomic publication -> reopen -> final receipt
```

If final publication bytes differ from the bytes that passed evidence checks, the
evidence is stale and publication must fail.

### 10.3 Cross-renderer evidence

For reconstruction candidates, the independent-renderer obligation should record:

- renderer name and exact version or build;
- platform and relevant configuration;
- page, crop, resolution, color-management policy, and alpha or background contract;
- input and output raster hashes;
- edit and preservation masks;
- outside-mask exact or thresholded difference;
- inside-mask edit-intent metrics; and
- unsupported operations or renderer failures.

Agreement among WellPDF, MuPDF, PDFium, and Poppler is strong empirical evidence,
not a universal proof. Disagreement must be classified rather than averaged away.

### 10.4 Security considerations

ECBES must not become a policy bypass. The production transaction must:

- verify modification authority before candidate generation;
- bind credentials to the operation without retaining plaintext secrets;
- respect signature and DocMDP policy;
- prevent candidate evidence from referring to a different revision;
- treat active content, external resources, and decompression limits as hostile;
- isolate native codecs and renderers where appropriate;
- never convert a validation timeout into a pass; and
- record whether encryption was preserved, rotated, or removed under explicit
  policy.

---

## 11. Testing and evaluation plan

### 11.1 Kernel verification

The initial focused Rust tests cover:

- deterministic, transitive, cycle-safe influence closure;
- rejection when required evidence is missing;
- stable selection independent of candidate order;
- explicit authorization of reconstruction; and
- enforcement of the computed influence cone.

These tests validate the selection kernel, not real-PDF editing.

### 11.2 Property and adversarial tests still required

Add property-based and fuzz coverage for:

- graph permutation invariance;
- repeated and cyclic edges;
- maximum-length identifiers and evidence;
- cancellation at every closure stage;
- candidate-order invariance;
- dominance antisymmetry and frontier correctness;
- certificate stability under semantically identical ordered input;
- rejection of malformed digests, duplicates, unknown nodes, and numeric extremes;
- empty candidate sets and all-rejected decisions; and
- resource-limit behavior without partial decisions.

### 11.3 Real-PDF corpus design

Use a frozen, licensed, deduplicated corpus stratified by:

- born-digital text and mixed scripts;
- RTL and vertical writing;
- subset, CFF, CFF2, Type 3, and variable fonts;
- shared and nested Forms;
- clipping, masks, transparency, patterns, and shadings;
- annotations, widgets, appearances, links, and destinations;
- tagged PDFs, tables, lists, figures, and reading order;
- linked-frame and cross-page reflow;
- encrypted and signed documents;
- damaged-but-recoverable files;
- scans, outlined text, handwriting, and textured backgrounds; and
- large and adversarial resource graphs.

Keep a held-out adversarial set separate from development. Every test must preserve
failed cases and typed refusals; excluding them would invalidate completion-rate
claims.

### 11.4 Metrics

For each edit contract, record:

- qualified completion rate by fidelity class;
- refusal and false-success rates;
- exact source and object preservation;
- changed object and byte counts;
- text, search, and copy correctness;
- tag, annotation, relationship, and page-tree integrity;
- outside-cone pixel delta per renderer;
- inside-region edit-intent satisfaction;
- save, reopen, and re-edit success;
- latency, peak memory, output growth, and cancellation latency; and
- manual correction required after publication.

### 11.5 Research hypotheses

ECBES should test, not assume:

- **H1:** exact influence cones reduce render-validation work without increasing
  outside-cone regressions;
- **H2:** candidate synthesis plus evidence-gated Pareto selection completes more
  valid edits than a single fixed route at equal false-success rate;
- **H3:** mandatory fidelity labels prevent reconstruction from being silently
  presented as native editing;
- **H4:** contract-lens checks reduce repeat edit, save, and reopen failures;
- **H5:** independent-renderer evidence catches writer defects that same-engine
  comparison misses; and
- **H6:** deterministic refusal reasons reduce manual diagnosis time compared with
  generic unsupported-edit failures.

Use preregistered acceptance thresholds and confidence intervals. If conformal
uncertainty is later used for OCR or reconstruction abstention, document the
calibration population and exchangeability assumptions; do not transfer a coverage
claim to arbitrary future PDFs without evidence.

### 11.6 Ablation study

Measure the complete system and variants with one control removed:

- no influence closure;
- no lens-law requirement;
- no independent renderer;
- no fidelity preference;
- no reconstruction disclosure;
- no uncertainty gate; and
- single-candidate routing instead of synthesis.

The value of ECBES is established only if the complete system improves the declared
outcomes over these baselines.

---

## 12. Known limitations and open research questions

### 12.1 Current implementation limitations

- The integrated graph is derived from the intent, canonical request digests,
  affected owners, and affected pages. It is materially narrower than a whole-file
  cone, but its completeness is bounded by the planner/apply reports; future
  operator/resource/render provenance can refine it further.
- Text intents automatically enumerate all three canonical fidelity routes; other
  operation families expose their canonical route. Automatic invention of an
  operation that the typed universal API cannot represent remains forbidden.
- Security authority, revision binding, strict reopen, logical postconditions,
  structural resolution, budgets, and reconstruction disclosure are generated by
  the transaction. Lens and independent-renderer evidence still require either an
  exact internal route report or an externally retained artifact.
- A passing external proof is hash-bound to the exact candidate output and must
  identify its producer and artifact. An independent-renderer proof additionally
  requires `independent: true`; the SDK cannot establish organizational
  independence merely from that declaration.
- C, .NET, Java, Python, WASM, and HTTP server surfaces are source-integrated. Their
  build/runtime parity remains subject to the verification record for the exact
  revision. No dedicated CLI command is part of version 1.
- No real-PDF corpus, cross-renderer, performance, or adversarial qualification has
  been completed for the framework.
- The decision serialization is implementation-deterministic, not yet an external
  canonicalization standard.
- The quadratic Pareto calculation has not been benchmarked at the hard limit.

### 12.2 Research questions

- How can the source, logical, and render graphs be related without making an
  unproved logical reconstruction authoritative?
- What is the smallest sound influence graph for PDF painting programs with shared
  resources, transparency, optional content, and annotation appearances?
- Which lens equivalence is correct for each PDF object family?
- How should cost dimensions be normalized across heterogeneous candidate
  generators without hiding policy in arbitrary weights?
- Can independent-renderer disagreement be localized to a viewer defect, writer
  defect, undefined behavior, or implementation-dependent behavior?
- How should evidence survive incremental revisions, object renumbering, linearized
  output, and encryption changes?
- Can uncertainty-aware abstention be calibrated across document domains without
  overclaiming distribution-free validity under dataset shift?
- What formal properties can be proved about candidate composition and atomic
  multi-object edits?

---

## 13. Relationship to prior work

ECBES combines established ideas but applies them to a distinct PDF editing problem:

- **Bidirectional transformations and lenses** provide the source and view update
  model and round-trip laws.
- **Contract lenses** add conditions for partial bidirectional transformations,
  matching the fact that a PDF rewrite is valid only under source and view
  preconditions.
- **Effectful lenses** motivate reasoning about failure, state, I/O, and other
  effects rather than assuming pure transformations.
- **Incremental bidirectional transformation** motivates updating only the affected
  dependency region for large documents.
- **Constraint and factor graphs** motivate explicit higher-order relationships in
  layout synthesis.
- **Constrained decoding and backtracking** motivate pruning invalid constructions
  before selection rather than repairing them after publication.
- **Differential testing** motivates comparing independent PDF implementations to
  expose disagreements hidden by a single engine.
- **Conformal prediction** is a possible future tool for calibrated abstention, but
  only under its stated data assumptions.

Primary starting points:

1. [Contract Lenses: Reasoning about Bidirectional Programs via Calculation](https://doi.org/10.1017/S0956796823000059)
2. [Effectful Lenses: There and Back with Different Monads](https://doi.org/10.1145/3747523)
3. [Incremental Updates for Efficient Bidirectional Transformations](https://doi.org/10.1145/2034773.2034825)
4. [Generative Layout Modeling Using Constraint Graphs](https://openaccess.thecvf.com/content/ICCV2021/html/Para_Generative_Layout_Modeling_Using_Constraint_Graphs_ICCV_2021_paper.html)
5. [LayoutFormer++: Conditional Graphic Layout Generation via Constraint Serialization and Decoding Space Restriction](https://openaccess.thecvf.com/content/CVPR2023/html/Jiang_LayoutFormer_Conditional_Graphic_Layout_Generation_via_Constraint_Serialization_and_Decoding_CVPR_2023_paper.html)
6. [Constrained Layout Generation with Factor Graphs](https://openaccess.thecvf.com/content/CVPR2024/html/Dupty_Constrained_Layout_Generation_with_Factor_Graphs_CVPR_2024_paper.html)
7. [Topological Differential Testing](https://arxiv.org/abs/2003.00976)
8. [A Gentle Introduction to Conformal Prediction and Distribution-Free Uncertainty Quantification](https://arxiv.org/abs/2107.07511)

These sources support ingredients of the design. They do not establish that the
ECBES composition is academically novel or empirically superior.

---

## 14. Paper intake and claim-control protocol

When a research paper is supplied for ECBES:

1. Record the exact file SHA-256, title, authors, venue, version or date, and
   licensing status.
2. Extract definitions, assumptions, algorithms, formal claims, datasets,
   baselines, metrics, limitations, and threats to validity.
3. Map each claim to an ECBES component and classify it as:
   - directly reusable;
   - compatible but requiring adaptation;
   - contradictory;
   - unsupported by supplied evidence; or
   - potentially novel relative to the current design.
4. Separate mathematical guarantees from empirical results.
5. Record implementation dependencies and licensing constraints.
6. Add or revise code only through a versioned design decision.
7. Add a regression or evaluation that would fail if the adopted claim is false.
8. Do not promote a paper claim into an SDK capability until the implementation and
   bound evidence exist.

---

## 15. Claim language

### Acceptable now

> We developed and source-integrated the first version of ECBES, a new proposed
> PDF-edit-synthesis framework. Its deterministic kernel and universal-edit adapter
> materialize bounded candidates from one revision, enforce fidelity-specific
> evidence gates, retain a Pareto frontier, publish only the selected candidate, and
> emit a hash-bound decision and publication receipt. Runtime and corpus
> qualification remain pending unless separately reported for the exact revision.

### Acceptable after full integration, if demonstrated

> WellPDF uses ECBES to generate and validate multiple PDF edit constructions,
> selects a qualified construction under an explicit preservation contract, and
> discloses whether the result is source-native or reconstructed.

### Not acceptable without future evidence

- “ECBES can edit every PDF perfectly.”
- “ECBES mathematically proves arbitrary PDF correctness.”
- “ECBES is universally novel or patentable.”
- “ECBES is better than Adobe.”
- “A decision hash proves that every evidence producer was correct.”
- “Appearance reconstruction recovered the original hidden pixels or font.”

---

## 16. Versioning and change control

Any change to these items requires a new algorithm version or an explicitly
compatible schema revision:

- fidelity classes or their rank;
- mandatory proof obligations;
- influence-edge semantics;
- dominance relation;
- cost-vector field order or units;
- selection tie-break order;
- certificate serialization input; or
- reconstruction authorization semantics.

Documentation, code, tests, binding schemas, and capability reporting must identify
the same version. Historical decision certificates must remain interpretable after
new versions are introduced.

---

## 17. Implementation map and present truth

| Component | Location | Present state |
| --- | --- | --- |
| Deterministic kernel | `crates/engine/src/research_edit_synthesis.rs` | Implemented |
| Universal transaction adapter | `crates/engine/src/ecbes_universal.rs` | Implemented; plan-derived intent/owner/page cone |
| Engine module registration | `crates/engine/src/lib.rs` | Implemented |
| JSON SDK wrappers | `crates/engine/src/sdk.rs` | Kernel, unsecured transaction, and secured-output transaction implemented |
| Universal capability record | `crates/engine/src/universal_editing.rs` | Registered as `Unverified` |
| Real PDF provenance graph builder | `crates/engine/src/ecbes_universal.rs` | Plan-derived intent/request/affected-owner/affected-page graph |
| Candidate-generator adapters | Canonical universal planner/applier | Explicit candidates plus deterministic text-route and canonical-route enumeration |
| Bound validator and evidence pipeline | ECBES adapter plus universal edit contracts | Integrated with producer/output digest checks |
| Selection-only publication | ECBES adapter | Implemented; non-selected bytes are not returned |
| C ABI | `crates/wellfriendpdf-capi` | Source-integrated |
| .NET binding | `bindings/dotnet/WellfriendPdf` | Source-integrated |
| Java binding | `bindings/java` | Source-integrated |
| Python binding | `crates/wellfriendpdf-py` | Source-integrated |
| WASM binding | `crates/wellfriendpdf-wasm` | Source-integrated |
| HTTP API | `POST /api/v2/universal-editing/ecbes` | Source-integrated |
| Corpus and cross-renderer qualification | Not run for ECBES | Required |

The current code is therefore a source-integrated **research transaction and
selection framework**, not proof of universal editing. That distinction is part of
ECBES itself: the framework rejects unsupported proof, and its documentation must do
the same.
