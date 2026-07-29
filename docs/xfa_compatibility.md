# XFA Compatibility

Prompt 34 inventories packets, extracts bounded data/field evidence, and
records a canonical static-conversion plan. Unrelated source rewrites compare
packet fingerprints so XFA bytes are not silently changed. Approved static
flattening delegates to the XFA runtime and can either retain source packets or
remove them only under an explicit request.

Dynamic XFA, lossy mappings, unsupported scripts/actions, and destructive
conversion without approval return exact typed limits. Prompt 34 does not claim
universal dynamic-XFA conversion.

XFA packets are inventoried and preserved during unrelated canonical edits.
Dynamic XFA conversion is an explicit unsupported boundary; conversion planning
must retain original packets and report loss risk.

## Static datasets import

`xfa_import_datasets` replaces only a resolved, indirect `/datasets` packet in
a parseable static-XFA packet array. The engine reopens the PDF, reparses the
dataset tree, and proves that every non-datasets packet retained its decoded
fingerprint. Dynamic XFA, direct/single-stream XFA, malformed packet arrays,
and unparseable templates remain exact no-change results.
