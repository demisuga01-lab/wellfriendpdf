# Paragraph line-break policy — source implementation, unqualified

This increment extends the existing prepared paragraph indexes, story paginator,
table layout, retained-session protocol and browser controls. It does not add a
second PDF writer or change source ownership/approval rules.

## Implemented paths

`StoryParagraph.line_break` has these JSON fields (all optional):

```json
{
  "profile": "japanese_strict",
  "emergency": "preserve_words",
  "prohibit_start": ")",
  "prohibit_end": "("
}
```

- `profile`: `unicode` (default) or `japanese_strict`. The latter layers no-start
  punctuation, small kana, iteration marks, inseparable characters and selected
  Japanese hyphens/quotes over the pinned Unicode opportunities. Opening
  punctuation cannot end a soft line. This is an explicit bounded profile, not
  a claim of complete Japanese typography or CSS conformance.
- `emergency`: `break_word` (default) permits grapheme-boundary breaks inside
  overlong letter/number sequences and preserved space runs; `preserve_words`
  refuses those additional breaks. Word-joiner, nonbreaking space/hyphen and
  trailing zero-width-joiner boundaries are not emergency escape routes. Emoji
  and combining graphemes are not split. Default opportunities still come from
  the pinned Unicode implementation, including its treatment of CJK scripts.
- Custom scalar sets are applied to ordinary and emergency opportunities. Each
  set is limited to 4096 UTF-8 bytes and cannot contain whitespace or control
  characters. A matching scalar constrains its whole indivisible grapheme,
  including explicitly listed combining marks. Significant characters across whitespace are indexed once; the
  algorithm does not repeatedly scan a long space run for every boundary.
- Explicit source breaks and the paragraph end override soft-break restrictions.
  Logical source text, including whitespace, is preserved rather than trimmed
  or normalized by the policy. Nonfinite/negative measurements are rejected.
- `PreparedParagraph` retains the filtered opportunities and emergency index
  alongside paragraph bidi/grapheme data. Horizontal/vertical linked stories,
  table cells, captions and prepared continuations use the same policy. Legacy
  callers of `PreparedParagraph::new` receive the protected default policy;
  their API is not expanded to expose custom settings in this increment.

`LineBreakBatch` additionally distinguishes a measured fitting prefix from a
geometric block at the next paragraph byte. Font/decoder errors, cancellation,
invalid measurements and budgets remain errors, never soft overflow results.
The story paginator can retain this prefix or skip a width-blocked approved
frame. Keep chains treat width blocks as a non-fit, and widow lookahead checks
distinct later frame widths plus the permitted continuation template. It does
not allocate repeated identical pages to retry an impossible first line.
Bounded lookahead is no longer mistaken for the end of a paragraph when applying
widow/keep-with-next constraints. Actual height and exclusions are still checked
at placement; this is not a global search over break combinations.

## Saved editing, review and compatibility

Default settings are omitted from serialized paragraphs, preserving previous
request/fingerprint bytes for omitted defaults. This does **not** promise old
layout equivalence: unsafe emergency split opportunities are now narrower.
Nondefault settings change request/layout-cache hashes and invalidate preview
approval. Checkpoint/reopen and causal-history metadata retain the policy.

Nondefault policies require **saved-story metadata schema 2**, or schema 3 when
the subsequent balanced-composition option is used. Default-only stories still
write schema 1. Current readers accept schemas 1–3 only when the contained policy
meets its version requirements; downgraded and unknown schemas are rejected.
Previous readers that enforce schema 1 refuse new-policy stories instead of
silently discarding their settings. Text-history operation/checkpoint versions
are independent and unchanged by this increment.

Structural merge normalizes omitted defaults only in its comparison view.
Otherwise enumeration of serialized fields would omit the new field entirely.
Independent text/policy edits merge; competing policy structs produce a typed
paragraph-field conflict. Policies are atomic fields, not independently merged
subfields. Conflict resolution compares the same normalized view, preventing a
text-only approval from authorizing an unrelated policy change.

The browser exposes paragraph controls, validates byte/character limits and
invalidates its receipt when settings change. Worker state first checks native
`status.line_break_policy_version === 2` after the balanced-composition update,
through the common retained-session
JSON protocol. The new helper is included in package files. Old/mismatched WASM
assets cannot open an editing session through this worker. Other native hosts
should check the capability before sending new-policy JSON to a deployed SDK;
new client source cannot retroactively change an older binary's deserializer.

## Design references and boundaries

The dependency `unicode-linebreak 0.1.5` declares **Unicode 15.0.0**. This is not
the Unicode 17 vertical-orientation table used elsewhere in the SDK. The design
distinguishes Unicode break opportunities, tailoring and emergency wrapping,
following [UAX #14 revision 49](https://www.unicode.org/reports/tr14/tr14-49.html)
and [CSS Text Level 3](https://www.w3.org/TR/css-text-3/). Neither reference nor
matching type names constitute conformance evidence.

Remaining implementation boundaries:

- No complete JLREQ/CSS engine, loose/normal Japanese profiles, hanging or
  compressed punctuation, ruby, tate-chu-yoko or automatic language dictionaries
  and hyphenation. SA-class script segmentation remains the dependency's bounded
  behavior, not a Thai/Lao/Khmer dictionary segmenter.
- The subsequent `joining_context_implementation.md` carries bounded logical
  neighbours through line/font/orientation shaping and emission;
  `joining_synopsis_implementation.md` preserves a more distant nontransparent
  neighbour across long mark runs for the pinned joining state machine. General
  script/font constraints and broader contextual substitutions remain open;
  this is not complete contextual typography. Unsafe-to-break flags call for
  reshaping, which final-line layout already performs, not blanket rejection.
- Default width probing remains bounded greedy/exponential/binary search.
  Final chosen widths are checked, but non-monotonic shaping can prevent
  discovery of another fitting candidate. The subsequent opt-in `balanced`
  composition evaluates a whole forced-break segment's candidate graph under
  explicit work budgets; see `balanced_line_composition_implementation.md`.
- Story flow has bounded width-block retry; table cells retain strict fixed-width
  line layout. Global backtracking across geometry, exclusions, keep chains and
  pagination costs remains open. A caller can still need to enlarge/relink frames
  or change the policy when the bounded planner cannot find an acceptable fit.
- This is story/table policy, not a new universal source-text edit option or a
  claim that every PDF's original authoring rules can be inferred.

## Unexecuted regression source and allowed checks

Thirty-two new regression functions cover synthetic break behavior, protected
controls/graphemes, custom punctuation, mandatory breaks, space-run progress,
invalid inputs/cancellation, story writing modes, table cells, receipts,
save/reopen, native JSON, durable history, schema downgrade rejection, merge
authorization and worker capability checks. Synthetic widths are not raster
evidence. The browser capability regressions do not execute WASM or a browser.
Additional cases cover prefix recovery, bounded lookahead, propagated errors,
width-blocked horizontal/vertical frames, keep chains, actual widow destinations
and impossible identical continuation widths.

Only Rust formatting/parser checks, JavaScript syntax checks and Git whitespace
checks are permitted here. No compiler, type check, test, PDF workload, render,
benchmark, browser/device QA, deployment, commit or push is performed. Exact
build/binding execution, Unicode break corpus, multilingual output/extraction,
save/reopen, pagination, accessibility and independent-render comparison remain
VPS qualification work. The overall editor/rendering roadmap remains open.
