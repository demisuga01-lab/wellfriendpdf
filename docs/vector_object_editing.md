# Vector object editing

`vector-list` produces stable objects. `vector-edit`, `vector-delete`, and
`vector-duplicate` operate on a selected stable ID. Supported edits include
move, scale, rotate, skew, horizontal/vertical mirror, endpoint/control-point
updates, fill/stroke colors, width, dash, cap/join/miter, opacity posture,
delete, duplicate, four bounded z-order moves, and bounded group/ungroup.

The writer replaces only the provenance operation range. It emits a self-
contained `q`/`Q` block with the retained transform/style/path, deterministically
recompresses the owning stream, appends an incremental object revision, and
verifies reopen plus unaffected decoded prefix and suffix. Marked-content,
clipping, OCG, and external resource references are retained in the model.
Reachable Form paths are inventoried to depth eight. A Form edit defaults to
`reject`; callers must choose `edit_all_uses` or
`clone_edit_one_instance`. Clone-one is bounded to a top-level page Form
invocation: it retains the source Form, creates a deterministic clone, rewrites
only the selected `Do`, updates page resources, reports the clone graph, and
reopens the result. Nested clone-one is rejected exactly.

Z-order moves are bounded to page-owned objects outside clipping,
marked-content, and OCG contexts. Grouping is bounded to contiguous page-owned
vector ranges and uses inert `/OxideGroup BMC ... EMC` ownership markers, so it
does not alter painting. Cross-stream, non-contiguous, and Form-owned groups
fail closed.

Example operation JSON:

```json
{"kind":"rotate","degrees":15.0,"origin":{"x":100.0,"y":100.0}}
```

```text
oxide vector-edit input.pdf --page 1 --id vector-... --operation rotate.json --output edited.pdf
```

For a shared Form, the options JSON includes:

```json
{"signature_policy_override":false,"deterministic":true,"shared_form_policy":"clone_edit_one_instance"}
```
