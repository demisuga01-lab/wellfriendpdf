# Fresh-authoring notes - source implementation, not qualification

This increment adds physical footnote reservation, continuation and typed
endnote collections to fresh flow authoring. It changes source only. No compiler,
test, PDF workload, renderer, benchmark, browser, deployment, commit or push was
run.

## Footnote ownership

`FlowFootnote` binds a sorted, disjoint, nonempty UTF-8 range in the exact body
paragraph passed to `add_paragraph_with_footnotes`. The selected text remains in
the body and becomes the visible note label. A reference that crosses a resolved
body line is rejected rather than reassigned heuristically.

The complete note display string is prepared with the same font, fallback,
shaping, bidi and hard-line authoring path used by body paragraphs. Line and byte
indexes are retained. `FootnoteFragmentInfo` reports the source note, reference
range/page, output page, exact display range and continuation state. Private
monotone note identities prevent fragments from separate paragraph calls from
being mistaken for one chain.

For generated markers, `NumberedFootnote` identifies an exact UTF-8 insertion
boundary in the original paragraph. `FootnoteNumbering` supplies decimal,
Roman or alphabetic formatting, configurable affixes/start and section- or
document-scoped continuation. The result returns the enriched paragraph plus
every original offset, assigned number/label and enriched range. Multiple notes
at one boundary remain ordered and disjoint. Counter state participates in the
append rollback, so failed font/layout work cannot consume a number.

## Joint page geometry

Every page owns a checked bottom reservation. A referenced body line and at
least one note line must fit together on a fresh page. The first fragment adds a
separator allowance; later fragments add bounded gaps. Long notes consume the
largest whole-line prefix that fits and continue on later pages. Subsequent
paragraphs, lists, images, spacers and tables use the effective bottom boundary,
so they move or fragment instead of overlapping the note region.

Footnote commands are not appended during layout. Final serialization clones the
builder, verifies every fragment chain and reservation, emits an artifact-marked
separator, and paints the logical note lines top-to-bottom inside the exact
reserved area. The original builder remains reusable. Font planning and the
canonical writer see the resulting commands; there is no overlay or second PDF
serializer.

## Endnotes

`FlowEndnote` carries an explicit single-line label, body, text style and
paragraph style. `add_endnotes` validates the complete collection before page
allocation, begins on the requested next/odd/even physical page, flows long
entries through ordinary pages and returns per-item first/last-page receipts.
An empty collection is a true no-op. The surrounding append transaction restores
commands, pages, note reservations, identities, active page and cursor on any
later error or cancellation.

## Resource and failure policy

The operation bounds note count, source bytes, prepared lines and emitted
fragments; polls cooperative cancellation during preparation, pagination and
materialization; rejects form feeds inside note bodies; and rejects a single
reference/note line pair that cannot fit a fresh page. Failed work publishes no
partial pages or reservations.

## Unexecuted regression source

Sixteen source cases cover private reservation/materialization, following-flow
avoidance, contiguous multi-page note ranges, stable identities across paragraph
calls, invalid/cross-line/overlapping references, table interaction, impossible
fresh-page geometry, idempotence, parity endnotes, empty/invalid collections and
multi-page endnote text. Automatic-marker cases cover source mapping, coincident
insertions, section restart, document continuation, formatting and counter
rollback. They were added but not executed.

## Remaining boundary

Fresh body paragraphs now use `/P`, while every footnote/endnote body owns one
unique-ID `/Note` element across all physical fragments. A bounded balanced
StructTreeRoot `/IDTree` resolves those IDs. Final footnote
materialization emits the note MCIDs without changing pagination. Exact marker
ranges own ordered `/Reference` children beneath the paragraph, ordinary body
ranges own `/Span`, and reciprocal `/Ref` arrays connect markers to notes. The
partition retains whole-line bidi and OpenType positions, emits one logical
carrier per semantic span and rejects a marker boundary that divides a shaped
cluster instead of reshaping substrings.

This is fresh-document note layout. Renumbering arbitrary pre-existing markers,
keep-with-reference backtracking across an
already committed earlier paragraph, side notes, citations, exact `/Reference`
markers for caller-supplied ranges are implemented; automatic reference
inference for unmarked prose, imported-PDF note recognition, linked-story note
movement and managed binding/browser controls
remain open. Executable build, save/reopen, independent extraction/rendering,
accessibility and corpus qualification remain pending the VPS gate. This does not
establish universal editing or a better-than-Acrobat claim.
