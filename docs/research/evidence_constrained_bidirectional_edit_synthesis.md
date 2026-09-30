# ECBES: evidence-constrained bidirectional edit synthesis

## Abstract

Evidence-Constrained Bidirectional Edit Synthesis (ECBES) is a framework for
editing PDFs when one visible result may correspond to many incompatible source
mutations. Instead of treating editing as a single rewrite heuristic, ECBES
generates a bounded set of revision-bound candidates, classifies their fidelity,
checks route-specific obligations against the actual output, and publishes only
an authenticated candidate that is both admissible and Pareto-optimal under a
deterministic integer cost model.

The framework is designed around a fact that ordinary editors often conceal:
PDF appearance is not a unique inverse of PDF source. Missing Unicode maps,
subset fonts, shared Forms, clipping text, scans, and untagged layout can make
the intended source edit underdetermined. ECBES does not claim to recover
information that is absent. It makes uncertainty explicit, preserves opaque
source outside the approved influence cone, and distinguishes native mutation
from semantic or appearance reconstruction.

## 1. How the idea emerged

ECBES arose from repeated source audits of a growing PDF editor. Each local fix
closed one path—multi-run text, font substitution, scan reconstruction, shared
resources, tagged content—while leaving a deeper problem: different routes could
produce the requested words but satisfy different notions of correctness. An
overlay could look right while leaving stale searchable text; an in-place glyph
rewrite could preserve source locality but fail for missing glyphs; reconstruction
could restore editability while inventing typography or background pixels.

The decisive insight was to stop asking for one universal inverse operation.
The editor should synthesize several explicit candidates and require each to
carry the evidence appropriate to what it claims. This joined three existing
lines of thought:

- bidirectional transformations, which relate a source to an editable view;
- program synthesis, which searches a constrained space of transformations;
- proof-carrying transactions, which bind a decision to checked postconditions.

The resulting framework is project-originated, but academic novelty has not been
established. The contribution currently claimed is a concrete PDF transaction
model and implementation, not a peer-reviewed theorem.

## 2. Problem statement

Let `s` be an immutable PDF revision, `e` a user edit intent, and `P` a policy.
A synthesis route produces candidates

```text
c = (transform, fidelity, influence, cost, obligations, disclosure).
```

Applying a candidate gives `s' = apply(c, s)`. ECBES seeks

```text
c* = argmin C(c)
subject to
  revision(c) = digest(s)
  intent(e, s') = satisfied
  preserve(s, s', outside(influence(c))) = true
  obligations(c, P, s, s') = true
  resources(c, s') ≤ limits(P).
```

The optimization is partial: when no candidate satisfies the constraints, the
correct result is a typed refusal or a request for a specific human decision.
Producing plausible bytes is not success.

## 3. Fidelity lattice

Candidates are classified before cost is considered.

| Class | Meaning | Typical route |
| --- | --- | --- |
| `source_native` | Mutates the exact current-revision source owner while preserving its declared semantics. | Operand rewrite, occurrence-owned resource clone, object lens. |
| `semantic_reconstruction` | Rebuilds editable structure from evidence while preserving logical meaning under a disclosed model. | Linked story, approved font reconstruction, tag repair. |
| `appearance_reconstruction` | Reconstructs visible content when the original source is unavailable or non-editable. | Scan inpainting plus embedded text. |

The ordering is a policy preference, not a statement that every native edit is
visually superior. A candidate may be rejected even at the highest class. Lower
classes require stronger disclosure and independent evidence.

## 4. Bidirectional laws

For a partial lens with source `s`, view `get(s)`, and backward update
`put(s, v)`, ECBES adapts two familiar laws:

```text
GetPut: put(s, get(s)) ≈ s
PutGet: get(put(s, v)) = v.
```

The equivalence relation `≈` is route-specific and versioned. It may mean exact
untouched stream bytes, canonical rooted-object-graph equivalence, decoded stream
equivalence under declared filters, logical text equivalence, or pixel equality
outside an approved mask. A route must state its relation; it cannot silently
substitute a weaker one.

PDF lenses are partial. Preconditions may include one exact owner, supported
encoding, an unshared resource, complete glyph coverage, or approved clone-on-write
scope. A failed precondition moves the candidate to a different fidelity class or
removes it from the set.

## 5. Candidate synthesis

### 5.1 Canonical route grammar

Candidate generation is bounded to registered engine operations:

```text
Route := NativeOperand
       | LogicalMultiRun
       | OccurrenceClone
       | RegionReflow
       | LinkedStory
       | SemanticRepair
       | ScanReconstruction
       | GovernedObjectLens.
```

Each route defines deterministic parameters, source identities, preconditions,
write-set construction, output limits, and required evidence. Arbitrary generated
code is never executed. The grammar is extensible only through a versioned route
registration and its validators.

### 5.2 Influence cone

The candidate declares source owners `W0`. The engine computes a conservative
closure over references, resource ownership, page invocation, structure trees,
annotations, and semantic dependencies:

```text
Wi+1 = Wi ∪ referenced_owners(Wi) ∪ semantic_dependents(Wi)
W* = fixed_point(W).
```

Unknown or cyclic ownership widens the cone or refuses the candidate under
resource limits. Preservation checks apply outside `W*`; changed objects and
pixels inside it must still be justified by edit intent.

### 5.3 Evidence obligations

Every candidate requires:

- `revision_binding`: input, plan, approval, and output identities agree;
- `intent_satisfaction`: reopened output exposes the requested logical change;
- `source_retirement`: obsolete reachable text or image content is removed when
  the route claims replacement;
- `outside_cone_preservation`: governed content outside the influence cone remains
  equivalent under the declared relation;
- `structure_integrity`: page tree, resources, tags, annotations, and references
  resolve within limits;
- `resource_budget`: output size and validation work stay within policy.

Additional obligations follow the fidelity class:

| Fidelity | Additional evidence |
| --- | --- |
| `source_native` | Lens round-trip or route-specific source equivalence. |
| `semantic_reconstruction` | Reconstruction disclosure and independently produced appearance evidence when claimed. |
| `appearance_reconstruction` | Inside-intent evidence, outside-mask preservation, uncertainty disclosure, and independently produced appearance evidence. |

An evidence digest proves that particular bytes and metadata were considered. It
does not prove that an external producer was honest or correct. Producer name,
version, command, configuration, input/output hashes, and artifact hash therefore
belong to the trust boundary.

## 6. Deterministic selection

ECBES uses an integer cost vector to avoid platform-dependent floating-point tie
drift:

```text
C(c) = (
  fidelity_rank,
  changed_indirect_objects,
  rewritten_opaque_bytes,
  outside_mask_changed_pixels,
  semantic_distance_microunits,
  layout_displacement_micropoints,
  font_substitution_penalty,
  reconstruction_uncertainty_ppm
).
```

Candidate `a` dominates `b` when it is no worse in every component and strictly
better in at least one. Non-dominated candidates form the Pareto frontier. The
default publisher selects lexicographically by the vector above and then stable
candidate ID. Policy may require human selection among frontier members, but
input enumeration order never decides the result.

## 7. Transaction algorithm

```text
function ECBES(source, intent, policy):
    revision ← digest(source)
    model ← bounded_source_analysis(source, intent)
    candidates ← []

    for route in registered_routes(policy):
        proposal ← route.plan(model, intent, revision)
        if proposal.preconditions_hold:
            influence ← conservative_closure(proposal.write_set)
            candidate ← route.materialize_privately(source, proposal)
            evidence ← validate(source, candidate.output, influence, policy)
            candidates.append(bind(candidate, evidence, revision))

    admissible ← candidates satisfying all required obligations
    if admissible is empty:
        return typed_refusal(candidates)

    frontier ← pareto_frontier(admissible)
    chosen ← policy_or_lexicographic_choice(frontier)
    approval ← authenticate_required_decisions(chosen, policy)
    receipt ← hash(revision, intent, chosen, approval, evidence)
    publish chosen.output only if receipt and output hash still match
```

Materialization is private: candidate bytes are not published before validation
and approval. Apply recomputes or retrieves the authenticated canonical artifact;
a caller cannot replace its preview, candidate, evidence, or output after review.

If there are `m` candidates and `d` cost dimensions, naive Pareto filtering is
`O(m²d)`. The registered route set is deliberately small and bounded, so clarity
is preferred over a more complex frontier structure. Influence closure is
`O(V + E)` in the bounded dependency graph. Raster evidence dominates cost when
requested and is governed separately.

## 8. Implementation mapping

The source implementation connects ECBES to the universal edit transaction:

- immutable document analysis produces revision-bound candidate identities;
- text, image, vector, linked-story, subsystem, security, and object-lens routes
  provide the synthesis grammar;
- the universal plan carries candidates, selected IDs, approval reasons, read and
  write sets, preview, signature impact, and conformance impact;
- apply authenticates the canonical plan and approval, then publishes only the
  exact validated output;
- the result records affected pages/objects, cloned resources, render
  invalidation, inverse data, issues, and proof evidence;
- C, Python, Java, .NET, WebAssembly, CLI, and service bindings expose the same
  serialized contract rather than implementing separate edit semantics.

RAPTOR is complementary: ECBES decides which candidate may be published; RAPTOR
retains the immutable analysis and execution products needed to make that decision
and mutation efficient.

## 9. Security and failure semantics

Candidates cannot broaden signature authority, bypass document permissions,
silently drop encryption, or claim sanitizing redaction from an incremental edit.
Decompression, object traversal, candidate count, output growth, font assets,
raster dimensions, and evidence size are bounded. Cancellation produces no
published partial output.

Ambiguity is not an exceptional crash. It is represented as one of:

- approval required, with concrete candidates and consequences;
- policy denied, when the requested route is forbidden;
- target not found, when no revision-bound owner exists;
- irrecoverable input, when safe parsing cannot establish a transaction base;
- resource limit, when proof or synthesis exceeds declared bounds.

## 10. Evaluation and limits

Evaluation must use real PDFs and record candidate coverage, fidelity class,
manual decisions, changed-object count, outside-cone preservation, save/reopen
correctness, search/copy behavior, independent raster evidence, latency, memory,
and every refusal. Synthetic fixtures test laws and edge cases but cannot establish
generality.

ECBES cannot recover missing fonts, original scan pixels, author intent, reading
order, or semantics with certainty when the source does not contain them. It can
offer governed reconstruction and calibrated uncertainty. Nor does a bounded
corpus prove universal editability. The framework succeeds when it makes the
strongest supportable edit, proves exactly what it claims, and refuses the rest
without corrupting the document.

Relevant foundations include research on
[bidirectional transformations](https://www.cs.ox.ac.uk/jeremy.gibbons/publications/bx.pdf),
[lenses](https://www.cis.upenn.edu/~bcpierce/papers/lenses-etapsslides.pdf), and
[program synthesis](https://people.csail.mit.edu/asolar/SynthesisCourse/Lecture1.htm).
These references motivate the design; they do not establish novelty for this PDF
specialization.
