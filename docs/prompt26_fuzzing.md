# Prompt 26 fuzzing

Prompt 26 fuzz targets cover incremental signing planning, CMS insertion boundaries, external
signer response parsing, PDF/A/PDF/UA/PDF/X classification, cross-profile reports, XMP
identifiers, DocMDP/FieldMDP parsing, and post-signature modification classification.

Each target runs alone with the Prompt 25B posture: one build job, disabled incremental builds,
a 4 GiB process cap, no network, no external process spawning, bounded input length, and at
least `-runs=64` for smoke. Build-only output is not a fuzz pass. The closure verdict is closed
only when every listed target both builds and smoke-runs on the current supported toolchain.
