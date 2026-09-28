# SVG/PostScript resource-scope implementation

Status: source implementation increment, unqualified. Base revision remains
`27e62db3a1b84804339e65b6025273fd003b3736`; the existing uncommitted candidate is
preserved. No builds, compilers, tests, PDF workloads, rasterization, benchmarks,
commits, pushes or deployments were run.

## Implemented

The shared vector classifier, SVG exporter and PostScript exporter now pass the
original page resource dictionary through nested Form and tiling-pattern calls.
An explicit Form dictionary replaces the complete lookup namespace, including
an empty dictionary. Legacy omitted/null Form resources use the original page,
not the enclosing Form or pattern. Pattern streams require an explicit resource
dictionary. These follow the
[PDF resource ownership rules](https://pdf-issues.pdfa.org/32000-2-2020/clause07.html#783-resource-dictionaries).

Changing lookup scope must not change already-selected graphics-state objects.
The shared `vector_resource_scope` module carries selected fonts, named colour
spaces and patterns into private interpreter bindings. These are not PDF source
rewrites or exported PDF resources. The child keeps its own source-visible names:
a local `/F1 Tf` still selects the child's F1, while an inherited selected F1
continues to refer to the caller's font. Paired q/Q within a Form preserves that
distinction. The child's local direct font does not retain an unrelated page
font's indirect-reference metadata.

Private binding allocation reserves content-operand and resource-graph names,
including normalized ExtGState Font names and indirect colour/shading references.
The scan has an explicit name/operand budget and depth bound; indirect cycles
are visited once and raw font/CMap program bytes are not scanned. Selected
colour dependencies are resolved in their original scope, including indexed
bases and tint-space alternates. Shading patterns retain their selected colour
dependency; tiling patterns retain their own resource scope and original page
fallback for nested legacy Forms. Missing selected fonts remain missing rather
than accidentally resolving to a same-named child font.

Form loaders return their effective resources and bound inherited state
together, so classification and emission consume the same context. The old
vector merging helper and the now-unused permissive resource-object parser were
removed. Invalid explicit dictionaries no longer silently become empty/fallback
scopes.

Both exporters report a fatal scoped-validation/resolution/recursion error
instead of silently dropping a Form if replay cannot honor its classification.
Form entry starts a separate path/pending-clip/text-clip context and restores the
caller afterward; PostScript previously did not save these interpreter fields.

## Source regression coverage

Ten new unexecuted functions cover:

- nested legacy Form page fallback in the classifier and both exporters;
- explicit empty and malformed scopes and strict-export refusals;
- inherited font binding versus same-name local font/reference metadata;
- inherited named colours, local selection and q/Q restoration;
- nested legacy Forms within tiling patterns;
- child operand and ExtGState-name collisions with private aliases;
- missing inherited fonts versus coincidentally available local fonts;
- inherited shading-pattern colour dependencies;
- cyclic named-colour dependency refusal.

Some tests call the complete strict SVG/PS API and inspect emitted vector colour
commands. These describe test source, not successful execution or independent
visual evidence. Syntax parsing with rustfmt and whitespace checking passed;
Rust type/ownership checking and all executable validation remain unrun.

## Remaining work

This is not general vector-output exactness or roadmap completion. Default
device-colour spaces, more complex pattern/colour/compositing behavior, arbitrary
Type 3 glyph export, vector transparency eligibility, text extraction parity and
other unsupported constructs require further implementation/review and corpus
qualification. Existing strict-output versus disclosed raster-fallback policies
remain distinct. The private resource model needs performance/cancellation
qualification on deeply nested and resource-heavy real documents.

The entire roadmap, including remaining editing, tagging, bindings, collaboration
and qualification requirements, remains active in
`universal_editor_roadmap_tracking.md`.
