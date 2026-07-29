# Nested Form clone-one

Vector provenance carries an ordered page-to-leaf `form_invocation_path`. With `clone_edit_one_instance`, advanced editing closeout clones the edited leaf Form, then clones each selected parent Form while replacing only that invocation's `Do` resource, and finally rewrites only the outer page invocation. Unselected source Forms and invocations remain reachable and byte-stable.

The operation requires lossless stream decoding, an acyclic Form graph within the recursion cap, and direct or indirect resource dictionaries. Missing or malformed ownership chains fail closed. `edit_all_uses` remains explicit.

advanced editing closeout focused fixtures cover depth two and depth three invocation paths,
including deterministic clone graphs and undo/redo through
`AdvancedEditingMutationSession`. The audit harness records invocation-path, clone
graph, resource-chain, selected-instance, unaffected-instance, determinism, and
signature-impact artifacts.
