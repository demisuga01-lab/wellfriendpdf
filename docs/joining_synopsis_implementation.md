# Long transparent joining context — source implementation, not qualified

This extends `joining_context_implementation.md`. Five nearby combining marks
could fill Rustybuzz's context buffer and hide the joining letter beyond them.
Generated text on the next line could then receive an isolated form despite
retaining paragraph bidi levels. The source now retains the actual nearest
nontransparent scalar separately, rather than guessing a joiner or storing a
whole paragraph with every line.

## Implementation and its exact scope

`ShapingContext` retains the existing five-scalar `before` and `after` edges,
plus optional `joining_before` and `joining_after` scalars. The extra scalar is
used only when its raw edge contains five transparent characters. It must itself
be nontransparent and must not be a forced break. During buffer setup it replaces
the farthest transparent carrier, leaving the other four in order. Context is
not inserted into the glyph input, ActualText, or ToUnicode.

This is a sufficient summary for the **pinned Rustybuzz 0.20.1 joining state
machine**: its pre/post-context loops skip transparent scalars, consume the first
nontransparent scalar, then stop. The summary preserves that scalar's actual
joining type, including nonjoining controls, ZWJ, and Syriac distinctions.
The other inspected context consumer uses context presence for dotted-circle
handling; the synopsis preserves presence and length. This source reasoning is
not runtime or independent typography evidence and is not a general theorem
about arbitrary font GSUB/GPOS rules or future shaping-library versions.

`ParagraphBidi` builds a reusable `JoiningIndex` alongside bidi preparation.
It stores `u32` byte offsets of nontransparent scalars and hard boundaries.
Each context lookup uses two binary searches and at most five raw scalars per
edge, rather than scanning every preceding mark again for every width probe.
Local font/script/orientation slices compose their own text with the outer
summary; their scans stop at the nearest effective scalar or forced break and
poll cancellation every 1,024 scalars. The existing 4 MB text budget applies.
Index entries occupy four bytes each, excluding Vec capacity and allocator
overhead; this is not a bound on total shaping/layout memory or measured RSS.

BK/CR/LF/NL stop both local scans and indexed context. Horizontal fallback,
vertical orientation, prepared line measurement, story/table paint lines and
shared authoring retain the summary through their existing `LineBidi` paths.
The shape-cache domain is bumped and includes serialized context with length
framing. Authoring plan equality/hashing includes the new fields too: identical
visible text and identical raw edges with different distant neighbours cannot
alias these keys.

## Versioned data and maintenance

- Rustybuzz is pinned to `=0.20.1`; `unicode-properties` to `=0.1.4` with its
  general-category feature. The latter was already present transitively in
  `Cargo.lock`; its direct engine dependency was added without running Cargo.
- `fonts/joining_properties.rs` combines 49 explicit override ranges with the
  same NonspacingMark/EnclosingMark/Format fallback as the pinned joining code.
  It was transcribed from 1,434 entries and 11 offset dispatches in upstream
  `ot_shaper_arabic_table.rs`, SHA256
  `5b5e3c33bd8743caadc5b3cd72f7608982ce071277569da3322118f5f5001a02`.
- The upstream MIT notice is retained in `fonts/RUSTYBUZZ_JOINING_LICENSE.txt`.
  `scripts/generate_joining_properties.mjs` prints the override table from that
  exact source, rejecting a different hash or dispatch shape. It performs no
  file writes, package downloads or SDK execution. Review its output and apply
  any source change explicitly. The checked-in maintenance script itself has
  only received a JavaScript syntax check, not an execution pass.
- General-category data here is Unicode 17 from `unicode-properties 0.1.4`.
  Paragraph line-break data remains Unicode 15 from its separate pinned
  dependency. This patch does not claim unified Unicode-version conformance.
- Before changing either pin, re-audit context consumers, transparency rules,
  hard-boundary treatment and glyph output, not just the generated table.

## Wire and persistence behavior

Native status and the browser worker now use `line_shaping_context_version: 3`
after the shared hard-line update;
`line_break_policy_version` is now 2 after the balanced-composition update. The browser guard requires both exact
versions. Deploy guard, worker and rebuilt WASM together. Native JSON hosts can
inspect the same status. Types and capability regression source are updated.

Older JSON without the optional synopsis fields still reads as raw-only context;
it cannot recover missing distant neighbours. Invalid supplied summaries are
rejected before cache lookup and authoring command insertion. Structural
validation does not establish that arbitrary caller-supplied context originated
in a particular PDF; canonical story layout derives it from current text.

Saved story requests store paragraph text, not stale shaping summaries. Layout
recomputes them after reopening. Preview and authoring records do contain small
source-derived context strings/scalars; these are not redaction-safe records.
Original source-glyph editing is not silently converted into Unicode reshaping.

## Break semantics and remaining work

HarfBuzz's `UNSAFE_TO_BREAK` flag calls for reshaping both sides of a break; it
does not mean every flagged position must be rejected. These line paths already
reshape final lines rather than slicing a cached visual glyph array. A new
ligature regression guards against introducing a blanket refusal.
[HarfBuzz glyph flags](https://harfbuzz.github.io/harfbuzz-hb-buffer.html#hb-glyph-flags-t)

Unicode also distinguishes permitted breaks from script-specific shaping and
hyphenation decisions. The summary does not implement dictionaries, spelling
changes at hyphenation, full contextual substitutions across fragments, or every
script/font's required cluster constraints.
[Unicode line breaking](https://www.unicode.org/reports/tr14/)

Global line/pagination optimization, complete Japanese typography, ruby,
mixed orthogonal stories, variable/exotic fonts, broader editor/rendering work,
and comparative Acrobat qualification remain open. The full roadmap is active.

## Verification boundary

Sixteen new regression functions are **unexecuted**: fourteen synopsis/shaping
cases, one authoring case and one story checkpoint/reopen case. They cover
transparency overrides, indexed/scanned equivalence, nested context, hard breaks,
joining controls, contextual forms, cache isolation, malformed/legacy JSON,
cancellation/budgets, ligatures, fallback/vertical propagation, grapheme wrapping,
CID/Unicode isolation and recomputation after saving. Native/browser capability
regressions are also updated, not run.

Only source inspection, source-data transcription, Rust parsing/formatting,
JavaScript syntax and Git whitespace checks were performed. No Cargo, compiler,
type check, test, PDF workload, rasterization, benchmark, browser/device QA,
deployment, commit or push was run. This does not establish a compiling build,
visual correctness, universal editing, or superiority to Acrobat.
