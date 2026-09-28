# Structure-owner materialization for annotation editing

Source implementation, not a qualified release. No compiler, build, test,
PDF workload or renderer was run for this increment. The entire roadmap remains
active and incomplete.

## Implemented transaction path

`annotation_structure_materialization.rs` extends the shared annotation promoter
used by native geometry, XFDF and linked-story staging. It handles recoverable
direct structure roots/elements and direct owner copies without reconstructing
the document's logical meaning:

1. Inventory reachable structure occurrences, OBJR carriers, ParentTree values,
   IDTree values and structural Ref relationships. Require exact, unique source
   ownership; dictionary similarity or annotation names alone are not selectors.
2. Reuse an existing indirect structure owner only when a standard owner link
   identifies one exact copy. An unreachable equal object is not authoritative.
   Competing indirect copies, contradictory P pointers and shared element
   occurrences are errors, not automatic merges.
3. Close annotation relationship, field-sibling and structure dependencies to a
   fixed point before mutation. A tag dependency can discover a widget requiring
   direct field-ancestor normalization, which can discover further siblings.
   This does not geometrically move those dependencies.
4. Bind all original slots and dictionaries, allocate identities, stage annotation
   aliases and field repairs, then materialize structure nodes bottom-up from the
   staged overlay. The final direct-root replacement uses the staged root value,
   so earlier OBJR edits are not discarded or rejected as stale source bytes.
5. Update P, K, Ref and lookup owner values together. Retain reading order, roles,
   alternate text, IDs, MCIDs, original ParentTree keys/arrays/null holes,
   ParentTreeNextKey, IDTree names and unrelated dictionary entries. Lookup entry
   arrays are copied on change, preserving unrelated aliases to the old arrays.
   Equivalent indirect Type/S names are normalized for the canonical validator.
6. Reopen the unpublished output and run the existing annotation identity/order,
   relationship, exact-object and complete ParentTree checks. Failure produces no
   published intermediate PDF.

Indirect page annotations with direct OBJR copies also trigger preparation.
Direct OBJR carriers needed by the final ownership check join the dependency set
even when every structure element is already indirect.
Field and structure owner plans share one object-update transaction and preserve
their respective changes to the shared catalog dictionary. Referenced lookup nodes/arrays and
name scalars are revision-bound before rewriting. Traversal, alias bytes, depth
and cancellation checks bound the work; this is not a performance measurement.

The owner-value distinction follows the PDF specification: object items map to
their owning element; marked-content scopes map to owner arrays indexed by MCID.
See the PDF Association's published [ISO 32000-2 clause 14 corrections](https://pdf-issues.pdfa.org/32000-2-2020/clause14.html),
tables 354/355 and section 14.7.5.4. This is a constrained normalization policy,
not certification of standards conformance or recovery of missing semantics.

## Reports

`AnnotationPromotionReport` adds defaulted fields:

- `materialized_structure_nodes`: direct root/element occurrences given indirect
  identities, including reuse of an already-declared equivalent indirect owner;
- `repaired_structure_parents`: P links normalized to those identities;
- `normalized_structure_links`: changed ParentTree/IDTree/Ref owner values;
- `dependent_tagged_annotation_ids`: tagged annotation occurrences included in
  the ownership preparation, including the requested source where applicable.

Native geometry and XFDF receipts already retain this report. Dependency pages
join invalidation sets. The normalization hash describes the private preparation
revision; the enclosing edit receipt identifies the final edited output.

## Explicit remaining boundaries

This does not infer missing tags, roles, reading order, owner indexes or intent.
Direct non-page OBJR targets, conflicting or non-unique owner copies, dangling
owners, structural dictionary reference chains, hidden tagged field objects and
arbitrary semantic subtree repair remain separate work. The global validator can
reject unrelated malformed tagged content. Ref arrays require non-null element
owners; null ParentTree MCID holes retain their different meaning.

This is not general accessibility-preserving document repagination, sanitizing
redaction, signature preservation or encrypted incremental writing. Other
roadmap implementation gaps and all executable qualification remain.

## Unexecuted regression source

Twelve cases in `annotation_structure_materialization_tests.rs` cover direct owners,
direct roots, declared shadow reuse, repeat normalization, indirect annotations
with direct OBJR copies, nested ancestors, MCID arrays, private array aliases,
stale lookup metadata, competing shadows, cross-page field/tag dependency closure,
semantic Ref links, indirect names, changed name scalars and contradictory owners.
They also cover multiple direct OBJR carriers under already-indirect owners.
They exercise transaction entry points and output assertions when later run;
their presence is not a passing result.

Only rustfmt parser/format checks and Git whitespace checks were performed.
These do not establish Rust type correctness, runtime success, accessible reading
order, rendering fidelity or compatibility with external PDF readers.
