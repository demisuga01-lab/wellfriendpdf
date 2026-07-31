# Colour and transparency

Rendering tracks device colour, calibrated colour, ICC transforms, separation
and DeviceN preview data, rendering intent, overprint state, alpha, soft masks,
blend modes, transparency groups, and page-group flattening.

The default `compat` mode follows byte-space compositing compatible with common
reference rasterizers. `high` mode keeps the same geometry and coverage while
using linear-light RGB compositing for opt-in fidelity checks.

Transparency and colour optimizations must preserve group isolation, knockout,
soft-mask semantics, optional-content visibility, and prepress fingerprints in
cache keys. Unsupported blend or colour features are typed limits, not silent
omissions.
