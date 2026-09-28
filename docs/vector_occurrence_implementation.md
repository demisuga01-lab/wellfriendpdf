# Vector occurrence isolation — source increment

Status: uncompiled and unexecuted. No Cargo, compiler, build, test, PDF workload,
rendering, benchmark, deployment, commit or push was performed. Formatting,
Rust syntax parsing and whitespace checks are not executable qualification.
The full editor/rendering roadmap remains incomplete.

## Corrected ownership paths

Previously, clone-one edited a new leaf/ancestor Form chain but still rewrote
the original outer page stream. Other pages sharing that stream could execute
the modified `/Do` with different resources. Repeating the same stream in one
page's `/Contents` array also changed every occurrence. Direct page-vector,
grouping and stacking-order writers had the same object-versus-occurrence
problem.

The new `advanced_vector_occurrence.rs` helper stages copy-on-write for exactly
one page-content slot. It moves the pending stream update to a fresh object,
retains the original object, and changes only the selected `/Contents` entry.
It preserves other pending page/resource changes. It rejects stale topology,
duplicate pending object numbers and object-number exhaustion before changing
the supplied update set. Allocations account for both source and pending
objects, including the Forms already cloned by the caller.

The helper now serves direct vector edits, top-level and nested page Form
clone-one, grouping/ungrouping and page-vector stacking-order edits. Explicit
Form `EditAllUses` keeps its intentionally shared semantics. Supplying clone-one
for a direct page vector no longer enters a nonexistent Form invocation path.

Vector IDs now include `content_stream_index`, distinguishing repeated uses
with identical source object/ranges. IDs from an older inventory must be
rediscovered; they are source identities, not cross-revision document IDs.
Grouping and stacking-order sibling selection uses the same exact slot.

Nested page Form cloning validates the ordered invocation chain against the
selected slot and resource bindings. Effective inherited resources are retained
when a supported parent Form omits its own Resources dictionary. The root is
identified by its position in the validated chain, not by membership of its
object number anywhere in the page's Contents array.

## Output identity and accessibility

Vector edit reports now resolve `after` from the saved inventory instead of
copying old stream offsets, resource names or Form paths. Duplication selects
the newly created copy; grouping preserves the selected source ordinal rather
than guessing by a potentially duplicated bounding box. Direct and nested
annotation-appearance clone reports also use the saved occurrence. These checks
are source code in the transaction; they were not executed during this change.

Copying a stream's StructParent/StructParents identity without updating its
owners is unsafe. A bounded, cancellable check detects these keys and reachable
MCR/OBJR Stm, StmOwn and Obj references before cloning. It traverses the `/K`
ownership graph, not cyclic `/P` links or all historical objects. Such cases
still require coordinated ownership migration and return an explicit boundary;
this change does not pretend to implement arbitrary tagged Form cloning.
Page-owned MCIDs stay on the same page when only its Contents reference changes.

## Unexecuted regression source

Fourteen new test functions cover:

- distinct identities for repeated Contents slots;
- top-level/nested Form cloning with both cross-page sharing and repeated slots;
- source object retention and unchanged other-page inventories;
- saved identities and a second edit through the returned identity;
- direct vector edits, deletion and duplication;
- occurrence-local grouping/ungrouping and stacking-order changes;
- cross-occurrence grouping rejection and explicit Form edit-all;
- inherited Form resources and invalid invocation chains;
- pending resource preservation, allocation collisions, stale topology and
  exhausted object numbers;
- stream structure-key guards and reachable ownership checks.

The existing clone graph regression now accounts for the added page-stream
clone receipt. None of these tests establishes pixel fidelity without execution.

## Still open

- This vector isolation helper now also supports the separate Form-text path
  documented in `form_text_implementation.md`; cross-scope text selections and
  general tagged/caller-owned logical migration remain open.
- Stream-owned tagged Form/appearance migration and shared annotation-object
  ownership need their own coordinated transactions.
- This change does not correct all vector paint-state, clipping, cross-stream
  graphics-state, colour, transparency or stacking-order fidelity limitations.
- Indirect Contents-array normalization and general malformed-source repair
  remain outside this helper's accepted source topology.
- Copy-on-write retains older/unreferenced objects; it is neither garbage
  collection nor sanitizing redaction.
- Builds, bindings, save/reopen regressions, independent rendering, difficult
  PDF corpora, memory and cancellation evidence remain pending.

Do not describe this increment as universal editing, a completed roadmap, or
comparative evidence of superiority to Acrobat.
