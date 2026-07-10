# Minimal dynamic XFA runtime

The runtime supports repeated subforms, occur min/max/initial, dataset-driven instance counts, positioned and top-to-bottom/left-to-right/row flow, page/content-area dimensions, simple vertical overflow, explicit breaks, value/caption layout, and visible/hidden/inactive presence. Each repeated instance carries its selected dataset node into bounded child name/ref binding; layout records expose the deterministic instance index and indexed SOM path. Event ordering is deterministic.

Default caps are depth 32, 1,024 instances per subform, 50,000 generated nodes, 256 generated pages, 16 relayout iterations (the current runtime performs one deterministic pass), 2 seconds, 128 MiB output, and 64 MiB scheduler admission. Cancellation is checked during discovery and layout.

Complex leader/trailer chains, keep/overflow graphs, layout cycles, arbitrary script DOM mutation, external data/image loading, proprietary extensions, and dynamic signature/barcode engines are `unsupported_reported_exact`. Dynamic documents may use `render_preview`; static flatten/remove modes reject them.
