# Generated text logical carriers - source implementation, not qualification

This extends `logical_line_carrier_implementation.md` to the bounded and
page-logical generated editing writers. The broader roadmap remains active.
No compiler, build, test, PDF workload, rendering, benchmark, deployment, commit
or push was run for this increment.

## Implemented source changes

An all-hard-separator replacement previously produced no glyph-showing operand.
An ActualText dictionary around an empty program does not itself give the SDK's
source selector an encoded text occurrence. Generated replacement and preserved
style writers now share the existing private, empty-outline Type0 carrier. Its
seven zero-advance CIDs preserve CR, LF, VT, FF, NEL, LS and PS exactly; CRLF is
two logical source scalars. Horizontal/RTL and vertical modes use their proper
Identity-H/Identity-V mappings. Empty replacement remains deletion, not a
fabricated blank line.

The carrier is integrated into four existing write routes:

- Bounded single-token paragraph reflow.
- Generated per-segment style reflow.
- Exact-font, positioned per-segment style reflow.
- Ordinary page-logical multi-run generated replacement and insertion.

Font objects and resources are staged in the same transaction as source removal
and source-position anchoring. Form and appearance editing use their existing
occurrence-specific resource and clone transaction, rather than installing a
font in an unrelated page resource dictionary. The private marker is generalized
to `WFLogicalLineCarrier`; the prior story-owned marker remains recognized.
Story font discovery still excludes these private fonts from substitution.

ActualText ownership is explicit. Replacements own the requested logical text.
The later source-insertion increment rewrites the anchor operand at the exact
CMap scalar boundary and gives only the generated inline glyph sequence a nested
nonempty ActualText owner. Prefix and suffix source codes remain in their
original text object. A directly rewritable isomorphic carrier is spliced at the
same logical scalar boundary in the same stream transaction, rather than being
cleared and losing search/copy semantics for a source font that depends on it.
This makes the insertion independently source-selectable without moving it to a
page overlay. It does not infer a partial mapping for non-isomorphic or shared
named ActualText ownership; those cases still require the existing explicit
semantic decision.

After serialization, the new postcondition rebinds the exact page, Form or
appearance program. It checks newly allocated carrier font mappings, writing
mode and zero advances, then scans the source streams for exactly one expected
carrier token with exact encoded text, ordinary text rendering mode, reset text
spacing and the intended ActualText owner. A same-text match in another Form or
appearance cannot satisfy this check. These checks are code, not executed proof.

The bounded writer now retains ActualText for mixed horizontal reflow as well
as RTL/vertical reflow, preserving exact separator scalars instead of relying on
geometric newline inference. The inherited-style route without explicit final
lines uses the shared mandatory-boundary iterator and retains blank lines/CRLF;
it does not silently introduce automatic word wrapping or different fonts.
Justification leaves glyphless lines unexpanded in horizontal and vertical
serializers.

Explicit unpositioned horizontal layouts and exact-style layouts now enforce the
same nominal line-capacity bound as automatic generated layout. One checked
helper clamps the floating-point interval count to the configured 1..=10000
line limit before integer conversion/addition, avoiding overflow on extreme
finite frame sizes. Options reject non-finite or nonpositive derived region
extents/line advance and invalid spacing limits. This is nominal baseline/em
capacity, not proof that every styled glyph outline fits its visible frame.

## Added regression source

Twelve new test functions, not executed:

- Eight separator sequences across bounded horizontal, RTL and vertical edits.
- Multi-run explicit, preserved and leading/trailing inherited style policies.
- Source-code reselection and a second replacement after a blank-only edit.
- Zero-width insertion at beginning/interior/end, using both collectors.
- Inherited mandatory boundaries without explicit layout.
- Blank justification, hard-line overflow and exact mixed-text separators.
- Extreme numeric capacity, overflow/underflow and invalid spacing inputs.
- Same-text-object clipping union, transformed source matrices and retained
  tagged/marked-content scope for source-adjacent insertion.
- Nested/repeated Form and annotation-appearance replacement/reopening while
  retaining original shared objects and unrelated occurrences.

The source is in `advanced_generated_carrier_tests.rs`,
`advanced_form_text_tests.rs` and `advanced_appearance_text_tests.rs`.
Rust formatting/parser checks and Git whitespace inspection are the only
executed checks. They do not establish compilation, type correctness, visible
fidelity, independent extraction or external-viewer compatibility.

## Remaining work

This is not all generated-text parity. The subsequent
`authoring_paragraph_implementation.md` replaces the separate authoring path's
lossy word splitter and bundled-font measurement for custom fonts with a shared
font-asset and paragraph plan. It also adds authoring hard-line carriers. Those
are separate source changes, not qualification of every generated-text route.

Arbitrary glyphless/default-ignorable input, mixed-line logical geometry and
general clipping/tagged multiline mutation remain open. Physical form-feed and
page-parity flow are extended by `form_feed_pagination_implementation.md`;
section masters/headers/footers/numbering remain open. Generated non-story Type0
fonts now carry a private version marker, and the shared page/Form writer removes
only marker-bearing `OxP20F*` resource entries that a bounded traversal proves
unreachable from current content, nested Forms/patterns/Type3 programs or
appearances. Conservative references such as a surviving dormant `Tf` retain
the resource; historical font objects remain in incremental revisions.
Incremental history remains historical-byte preservation, not redaction. All
broader layout/rendering, binding, independent save/reopen, standards and VPS
corpus qualification in the roadmap ledger remain pending. No universal or
better-than-Acrobat claim is supported by this change.
