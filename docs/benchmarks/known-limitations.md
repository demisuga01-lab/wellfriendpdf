# Renderer benchmark known limitations

- The 5,044-PDF corpus is real and large but still domain-skewed toward academic PDFs.
- Raw-hash evidence proves render completion and deterministic bytes for Wellfriend outputs; it is not a full perceptual equivalence proof.
- Compat mode intentionally uses bounded fallbacks for expensive tiling patterns, Type3 fallback text, and image minification. Use high-quality mode for the exact rendering path where supported.
- PDF.js, pypdfium2, and PyMuPDF are wrapper/runtime paths and are labeled that way.
- Commercial SDK behavior is not benchmarked without a legitimate executable/license.
- No cross-purpose overall winner is claimed.
