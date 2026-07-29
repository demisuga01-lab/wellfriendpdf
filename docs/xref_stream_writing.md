# Xref Stream Writing

writer history xref-stream output is provided by `PdfWriter` in modern writer modes. `XrefStreamWithObjStm` writes:

| Item | Policy |
| --- | --- |
| `/Type /XRef` stream | Writer-generated final xref stream. |
| Type 0 entries | Free object head. |
| Type 1 entries | Direct objects and object-stream containers. |
| Type 2 entries | Members packed inside `/ObjStm`. |
| `/W`, `/Index`, `/Size`, `/Root`, `/Info`, `/ID`, `/Encrypt` | Deterministically serialized from writer state. |

Incremental signature-preserving updates do not repack existing objects. Linearized inputs are not claimed to remain linearized after packing.
