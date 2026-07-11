# Prompt 18 known limits

- Direct partial image rewrite is bounded to safe decoder output and 8-bit Gray/RGB/CMYK samples. Packed stencil and unsafe complex color-space paths remove or fail closed.
- The affected masked clone omits Mask/SMask. Direct preservation and partial rewrite of every possible mask encoding is not claimed.
- Inline DecodeParms dictionaries and unsupported complex color spaces remove or fail closed. Inline XObject promotion is not selected in this bounded implementation.
- Associated-file mutation canonicalizes supported internal payloads into the catalog EmbeddedFiles name tree. Reattaching every non-catalog owner relationship is not yet implemented; those locations remain inventoried.
- Portfolio UI rendering is not supported.
- DocMDP/FieldMDP analysis is structural. Cryptographic validity, trust, certification acceptance, and viewer status remain separate.
- The executable incremental mutation proof currently covers an existing Info dictionary. Secure removal operations require full rewrite.
- Caps: 100,000,000 image/mask pixels, recursion 32, 256 MiB inline bytes, 100,000 inline images, 10,000 associated files, 512 MiB per associated file, 2 GiB total associated bytes, 4,096 signatures/policy references.
