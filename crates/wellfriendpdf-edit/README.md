# Wellfriend native PDF edit core

This isolated Rust service owns structure-preserving PDF edit operations. It is
not a renderer, overlay editor, or replacement document store. Application code
must still authorise a canonical `DocumentVersion`, materialise it into bounded
scratch storage, persist the returned immutable output through the canonical
storage/version authority, and record the edit transaction, audit, provenance,
and usage evidence.

The implemented capabilities remain deliberately bounded:

- `PDF_TEXT_REPLACE`: replace one exact `Tj`, positioned `TJ` array, or
  next-line quote text operation selected by page, content
  stream object, operation index, font resource, and expected decoded text;
- preserve the original font resource and require an exact encode/decode
  round-trip for every replacement glyph;
- modify only the selected content stream object;
- reparse and prove page/object counts, non-target objects, selectable text,
  image-XObject counts, and lack of page rasterisation;
- preserve numeric kerning in unchanged array text, drop obsolete kerning
  within the replaced span, preserve quote spacing/line-advance operands, and
  encode complete characters rather than splitting multibyte CID codes;
- carry font state across content streams and restore it through `q`/`Q`;
- inspect a specific page with `INSPECT.text_page` when the bounded global
  list truncates. The existing authorized session GET accepts `textPage`;
  the dashboard exposes page selection and bounded text-object ranges;
- refuse stale selection, malformed text arrays, unsupported encodings,
  encrypted files, decompression bombs, oversized inputs, and signed files
  without explicit signature-invalidation confirmation;
- `PDF_PAGE_EDIT`: reorder every existing page through the real page tree while
  retaining page/content object identity, or perform one stale-bound `DELETE`,
  `DUPLICATE`, `INSERT_BLANK`, or cross-PDF `IMPORT` mutation. Duplicate/import
  deep-clone bounded page dependencies; unsafe annotations, actions, page
  labels, structural dependencies, external references, and oversized clone
  graphs refuse rather than being discarded;
- `PDF_IMAGE_REPLACE`: replace one exact unmasked image XObject with a fully
  decoded JPEG while preserving every resource reference;
- `PDF_OBJECT_MOVE`: move one exact directly drawn image by changing only the
  translation components of an isolated `q` → `cm` → `Do` → `Q` content-stream
  sequence while preserving its image XObject and every unrelated operation;
- `PDF_IMAGE_TRANSFORM`: replace all six values of that exact placement matrix
  to resize, rotate, or move the image without changing its XObject;
- `PDF_IMAGE_CROP`: insert, update, or remove a normalized `re` → `W` → `n`
  clipping path inside the same isolated graphics state. Cropped placements
  remain inspectable and transformable, and resetting removes only that path;
- `PDF_IMAGE_OPACITY`: create, update, or remove one Wellfriend-owned,
  page-local `ExtGState` and its `gs` operator for the exact isolated image
  placement. The image XObject, matrix, crop, and unrelated graphics state are
  preserved; placements using foreign graphics state remain read-only;
- `PDF_VECTOR_EDIT`: move/resize, recolour, or delete one exact path group only
  when inspection proves it is an isolated bounded `q ... path ... paint ... Q`
  sequence. The stale selector binds the page, content-stream object, operation
  range/hash, transform, colours, and paint operator. Clipping, nested graphics
  state, patterns, shadings, ambiguous operators, and arbitrary artwork remain
  read-only; the engine never rasterises the page as fallback;
- `PDF_LINK_EDIT`: create, update, or delete one exact indirect Link annotation;
  selectors bind the page/annotation collection or annotation object/hash,
  external targets permit only bounded `http`, `https`, and `mailto`, internal
  targets bind an exact page reference, and non-target annotations/objects remain
  unchanged;
- `PDF_BOOKMARK_EDIT`: create, rename, retarget, reparent, reorder, or delete a
  bounded real outline subtree while preserving its linked hierarchy and every
  non-outline semantic object;
- `PDF_FORM_EDIT`: change the real value of an inspected terminal text, choice,
  checkbox, or radio field; move/resize one stale-bound indirect widget by
  changing its actual `/Rect`; change/remove the existing field's alternate
  accessible name (`/TU`); or create a real text, checkbox, radio, dropdown, or
  empty signature field and its page-bound widget objects. Text/choice and new
  button appearance streams are deterministic; existing button/signature
  appearances are preserved. Stale-bound field deletion, simple dropdown
  option updates, and bounded appearance repair are supported. Structural
  form-tree repair and export/display option-pair flattening are not;
- `PDF_PAGE_GEOMETRY_EDIT`: rotate, crop, or resize one exact page through its
  `Rotate`, `CropBox`, and `MediaBox` dictionary entries. Geometry selectors are
  page-object and source-value bound; page content and non-target objects remain
  unchanged.

`PDF_TEXT_REFLOW` wraps one isolated, measurable `Tj` object within an explicit
region using declared font metrics. Neighbour collisions and overflow fail
closed; positioned arrays are replaceable but not automatically reflowable.
`PDF_ANNOTATION_EDIT` creates/updates/deletes supported owned annotations and
their appearances. `PDF_METADATA_EDIT` updates controlled Info fields and
synchronizes owned XMP, or preserves foreign XMP byte-for-byte.

This does not claim arbitrary paragraph/document reflow, image insertion or
deletion, composed/nested image placement transforms, arbitrary vector editing,
page labels, arbitrary malformed form-tree repair, mutation of
pages with unsupported non-link annotations or unsafe structure/interactivity
dependencies, or cryptographic signature validation. Those remain separate
capability gates.

Image-only pages have no editable native text. OCR does not make rewriting
those pixels losslessly possible. A missing glyph requires an explicitly
supported font replacement workflow, not silent fallback. Existing signature
invalidation, exact source/version binding, and non-target preservation are
mandatory for every save. Supporting these operations is not a claim that
every possible PDF object layout can be safely edited.

Verification: `test:native-pdf-edit-core`, `test:native-pdf-edit-core:quality`,
`test:native-pdf-edit-adapter`, `test:native-pdf-edit-persistence`, and
`test:pdf-viewer-interactions:browser`. The CI native-PDF job runs the matrix
against disposable PostgreSQL and a real browser. For an authorized corpus,
`scripts/native-pdf-text-operator-rehearsal.py` checks output text with PDFium,
renders every page, and requires every non-target page to remain identical.

The binary accepts one bounded JSON command on stdin and returns one JSON
success/error envelope on stdout. File paths must point to worker-owned scratch
locations; browser or customer-controlled paths must never be passed directly.
