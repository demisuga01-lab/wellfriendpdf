# document subsystems Implementation

document subsystems routes tables, approved math and OCR text edits through text reflow,
annotation appearance regeneration through annotation/media redaction, form data through the
canonical form editor, and XFA inspection through the canonical XFA module.
Every supported operation records source links, a transaction report, reopen
evidence, and a reversible preimage.

## Runtime entry points

`document_subsystems_analyze` returns the canonical table projection, born-digital math
trees, OCR layer classification, interactive annotation/form inventory, and
XFA inventory/extraction/runtime evidence. `document_subsystems_plan` accepts the same
typed request consumed by `document_subsystems_apply`; it does not invent a second
document model.

Supported mutations are source-linked table-cell replacement/alignment/padding/border/fill creation, approved table-cell math replacement/movement, reviewed OCR text/link creation, table-linked annotation movement, and text/checkbox/choice form-widget creation in a resolved table cell, plus conservative
ruled-row/column append, approved born-digital math replacement and bracket-matrix
cell, row/column structure, single-fraction-part, source-script, fenced-inner, and radicand replacement, approved correction of an existing searchable text layer and
provider-recorded invisible OCR word batches, supported annotation
create/edit/move/resize/delete/XFDF/flatten operations, AcroForm
text/choice/check/radio values, root text-field creation, terminal
rename/delete, root checkbox creation with named appearances, widget move/resize, reset, and flattening,
root choice/combo creation, source-level text/choice/button default values,
choice option-array and selection updates,
push-button creation with N/R/D states,
single-widget radio creation with synchronized export state,
unsigned signature-field creation with immutable existing-signature policy,
source-linked AcroForm calculation-order updates,
form-data import, and
static-XFA dataset-packet import and approved static-XFA flattening. Each route serializes through the existing
editor or text reflow transaction path and reopens its output before returning.

The same JSON request/report contract is exposed by Rust, the CLI, Python,
C ABI, WASM, .NET, and Java. Bindings are adapters over the core engine and do
not contain binding-specific PDF mutation logic.

## Safety boundary

The original scan, unresolved outlined/raster formulas, unsafe annotation
actions, signature fields, and dynamic XFA are never silently replaced.
Ambiguous source geometry, unapproved reconstruction, invalid field/action
combinations, and unsupported annotation/XFA constructs return typed failures
before writer mutation.
