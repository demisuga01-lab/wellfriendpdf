# Geometric Text Region Model

`GeometricTextRegion` is the canonical text reflow request analysis result. It
links a page-user-space region to source editing instructions and editing transactions scene
nodes, records writing direction, clipping policy, known neighbor policy,
allowed expansion rectangle, and confidence dimensions.

Supported application is one provenance-resolved source string in one bounded
region. An explicitly supplied expansion must contain that region and stay in
the page box. Unknown neighbors are locked. Polygon clipping, Form coordinate
space reflow, and moving arbitrary neighbors return exact unsupported results.
