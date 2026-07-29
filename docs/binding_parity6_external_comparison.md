# Incremental Signing Standards external comparison

External tools are comparison evidence, not replacements for the engine's clause-mapped rules.
qpdf structural checks verify parse/stream structure only. pyHanko may provide CMS/PAdES sanity
where applicable. veraPDF is the optional PDF/A comparison tool; PDFBox/preflight is optional
structural/preflight evidence.

Every result records the tool version, availability, fixture/artifact, command exit code, and
scope. An unavailable optional tool is recorded as unavailable and is never treated as pass.
Disagreements are retained with a classification rather than silently normalised away.
