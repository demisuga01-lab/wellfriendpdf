# Annotation editing identity and destination names

Source-only increment on dirty `main`, base
`27e62db3a1b84804339e65b6025273fd003b3736`. No compiler, Cargo, test,
PDF workload, renderer, benchmark, browser execution, commit, push or deployment
was run. This extends the linked-story annotation path, not every annotation API.

## Corrected source behavior

An annotation's `/NM` is unique within its page, not necessarily the document.
That distinction is explicit in Table 164 of the
[Adobe-hosted ISO 32000-1 specification](https://developer.adobe.com/document-services/docs/assets/35e4369068f86065372c18787171a17e/PDF_ISO_32000-1.pdf).
The previous story inventory rejected the same name on different pages.

Discovery now separates `annotation_id` (opaque selection identity) from `name`
(decoded page-local NM). It resolves indirect names, retains distinct source
references and rejects one annotation object appearing under multiple page
owners. A unique unpersisted name keeps the legacy editing ID when unambiguous.
Repeated names and unnamed annotations get revision/object-bound IDs. A persisted
editing ID always takes precedence; another object's ordinary NM cannot steal it.
Duplicate persisted IDs still fail closed, since they require explicit import
and saved-metadata remapping, not a guess about which object owns the old ID.

Approved checkpoints stamp **every selected** object with `WFStoryAnnotationID`,
including named objects. IDs use valid UTF-16BE PDF text strings, preserving
non-ASCII identifiers and supplementary Unicode characters. Existing NM values
are untouched unless a separately approved destination collision requires repair.
Unselected objects are neither stamped nor renamed. Discovery does not write.

## Destination namespace transaction

- Preview computes names on the actual proposed output pages, including the
  shift from newly inserted continuation pages.
- Names belonging to unchanged residents are reserved first. Arrivals follow
  source annotation order. All arriving existing names are reserved before
  generating repair names, so an early repair cannot steal a later arrival's
  nonconflicting name.
- A collision is not silent permission to change NM. The anchor's optional
  `rename_conflicting_names` flag defaults to false and applies only to that
  anchor's approved members. If it is false and a repair is needed, preview
  rejects before mutation and explains the policy choice.
- With explicit approval, every change is reported in the member's
  `anchor_moves[].name_change = { previous, replacement }`. The normal exact
  preview/checkpoint receipt binds these changes and the request policy.
- The internal writer independently recomputes the final namespace before
  changing NM, rejecting stale or forged name changes. Naming and geometry/tag
  ownership changes are in the same transaction. Reopening checks resulting
  names, member identities, rectangles, pages and group topology.
- Saved anchor IDs survive name repair, canonical object renumbering, page
  insertion/removal and repeated story editing. Geometry receipts rebind to the
  newly written names, but previously approved source ownership does not change.

The browser annotation panel shows both the actual name and opaque ID, and
exposes collision-repair approval for new and existing associations. The native
session JSON protocol carries these fields through the existing bindings.
Changing the policy invalidates the preview receipt and layout cache authority.

## Boundaries and evidence

Actions, scripts and external FDF files may refer to annotation names. They are
**not rewritten by inference**. The user-facing policy explicitly discloses this
tradeoff. Applications needing those references unchanged should not approve NM
repair; they can choose another destination. No document-wide name cleanup is
performed. Already-malformed duplicate names among unchanged residents are not
silently repaired.

Old saved metadata without a persisted ID can become genuinely ambiguous after
external duplication/import; this change does not invent provenance for it.
Multiple objects carrying the same persisted identity likewise need explicit
import remapping. Direct annotation dictionaries, unsupported geometry and
other annotation-group limits remain as documented in
`story_annotation_group_implementation.md`.

Follow-up implementation now shares the source identity resolver with XFDF and
appearance generation, and standalone geometry uses the native transaction.
See `annotation_identity_unification.md` for revision-bound interchange,
page-local aliases, direct-source protections and remaining import/geometry
boundaries. This supersedes the earlier separate-name-lookup limitation, not
the broader interoperability and qualification requirements.

Six new unexecuted regression functions cover repeated page-local names,
unchanged unrelated objects, collision approval/receipt integrity, Unicode and
indirect names, persisted-ID precedence/collisions, output-page namespace
mapping through canonical insertion, and preserving a later arrival's existing
name. They include save/reopen/re-edit and undo/redo paths in source.

Allowed validation was limited to rustfmt formatting/parser checks, JavaScript
syntax checks and whitespace checks. These do not establish type correctness,
runtime correctness, visual fidelity or browser behavior. The full roadmap and
VPS qualification remain open.
