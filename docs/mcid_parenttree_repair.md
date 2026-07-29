# MCID and ParentTree Repair

document security ParentTree repair is source-linked and conservative. It uses existing
marked-content and semantic recovery evidence, then applies supported updates
through the canonical writer.

The operation report records:

- source object evidence;
- affected structure entries;
- whether MCID evidence is exact, recovered, inferred, or unavailable;
- repaired and refused relationships;
- validation status after reopening the output.

When a ParentTree update cannot be represented safely with current canonical
APIs, document security returns a structure-update typed refusal and preserves the input.
