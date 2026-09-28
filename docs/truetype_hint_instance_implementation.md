# TrueType hint-instance staging and cross-table point checks

Follow-up: `static_truetype_instance_implementation.md` connects supported stages
to public TrueType publication with names/style/STAT handling. It rejects the
unresolved review conditions described here. The historical internal-only status
below is superseded for that declared subset, not for general font instancing.

Uncommitted over `27e62db3a1b84804339e65b6025273fd003b3736`. This increment
extends the internal source-bound font transaction. It is not a completed public
font instancer, universal-editor release or runtime qualification.

## Hint program handling

`fonts/tt_bytecode.rs` walks instruction boundaries, including every fixed and
variable-length byte/word push. Immediate payload bytes are never interpreted as
opcodes. It inventories function/instruction definitions, variation/information
queries, calls, relative jumps and unknown instructions. Definition/conditional
scopes, truncated payloads, definition sizes and glyph-program ownership are
checked. Literal identifiers/selectors are recognized only immediately after a
push; they are not invented through dynamic stack operations or function calls.

`fonts/tt_hint_instance.rs` appends a GETVARIATION instruction definition after the
original preparation program. Its body pushes the selected normalized signed
coordinates in axis order. Original instruction offsets and glyph programs are
unchanged. The stage updates instruction-definition and conservative stack
capacities using the outline writer's rebuilt maxp, not the old source profile.
Both stages publish identical maxp replacements, avoiding merge-order loss.

This follows the static-instance fallback recommended by the
[OpenType instruction specification](https://learn.microsoft.com/en-us/typography/opentype/spec/tt_instructions#GETVARIATION).
The recommendation is not a guarantee of equivalent hint behavior on every
rasterizer. The implementation does not execute a TrueType interpreter.

The outline writer now supplies exact, GID-ordered instruction ranges in its
generated glyf table. The hint stage checks those ranges against staged locations
without reparsing/copying all outline points. It uses the same face and selected
coordinates as the metric, CVT and layout stages.

Explicit review receipts cover variation queries executed during initialization,
initialization calls when definitions contain such queries, relevant/unknown
GETINFO selectors, legacy variation queries, dynamic instruction definitions,
initialization relative control flow and unknown instructions. The final prep
definition does not repair those earlier execution paths. An empty review list
is not a proof of stack safety, termination or visual equivalence.

## Layout-to-outline ownership

The source-bound TrueType preparation path now passes its exact expanded explicit
point counts to the layout resolver. GPOS cursive/mark anchors and JSTF embedded
positioning use coverage-order glyph ownership; ligature component rows use the
whole ligature glyph's point domain, not an invented component-local index.
GDEF attachment/caret records use their own coverage owners. BASE format-2
coordinates use their explicit reference glyph. The resolver checks glyph and
point bounds and records checked versus unchecked references.

Shared anchor records are checked before relocation-cache reuse, so a record
valid for one glyph cannot silently authorize the same index on another glyph.
Coverage ranges are expanded once per owner graph rather than rescanned for each
point. Native source preparation supplies the outline authority automatically;
isolated layout and CFF preparation without that authority report unchecked
references, not a successful contour-identity validation.

Indices remain source identities. No implied points are inserted and no
contour reference is rewritten as a guessed coordinate. This binds the formats
described in [GPOS](https://learn.microsoft.com/en-us/typography/opentype/spec/gpos#anchor-tables),
[GDEF](https://learn.microsoft.com/en-us/typography/opentype/spec/gdef) and
[BASE](https://learn.microsoft.com/en-us/typography/opentype/spec/base) to the
generated TrueType point domains. It does not implement their device-hinted
positioning or validate every glyph/lookup relationship in an arbitrary font.

## Bounds and evidence

Hint scanning limits each program to 4 MiB and aggregate scanned bytes to 64 MiB,
with four million decoded instructions, 262144 recorded events and conditional
depth 256. Definition bodies must fit the instruction format. Publication and
scan/copy loops poll cancellation. Layout checks share the existing resolver work
budget; point-domain storage is at most one u16 per glyph. These are component
budgets, not measured memory/latency guarantees.

Thirty-six new regression functions are unexecuted: 24 for bytecode/fallback
handling and 12 for point ownership. They cover push-data isolation, signed axis
order, malformed scopes, budgets/cancellation, profile composition, saved-font
fallback preservation, shared anchors with unequal glyph domains, coverage ranges,
mark/ligature ownership, GDEF/BASE/JSTF references and explicit unchecked reporting.
The preceding outline save/reopen regression is also extended to check actual
point-bound GDEF preparation and reject a phantom index before publication.

Only static source review, rustfmt formatting/parser checks and whitespace checks
were performed. No Cargo, compiler, build, typecheck, test, font/PDF workload,
rendering, benchmark, browser QA, commit, push or deployment was run.

## Still open

General early-query/control-flow hint freezing, full dynamic hint semantics,
phantom-point component attachment, device-dependent geometry, complete CFF2
serialization, names/style metadata, broader variation/color formats and the
public whole-font transaction remain open. Point checks here cover the declared
layout owners; they are not a general font sanitizer. The wider editor/rendering
roadmap and all executable/corpus qualification remain unfinished.
