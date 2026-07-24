# Prompt 09 Reference Disagreement Policy

Prompt 09B keeps the multi-reference discipline from Prompt 06B.

## Reference Engines

- Poppler from the Prompt 06B manifest
- PDFium from the target-local Prompt 06B wrapper
- MuPDF from the Prompt 06B manifest

If any engine is unavailable, the affected rows are not counted as passed unless the Prompt 06B target-local bootstrap succeeds and records the tool.

## Classification Rules

When references agree, Wellfriend must be within threshold or the row is an Wellfriend outlier. When references disagree, Wellfriend may be accepted only if the row explains the cluster and provides screenshots/diff metrics. Unsupported active or generated behavior must be reported as unsupported; it must not be silently ignored.

Prompt 09B currently records two reference-cluster disagreements:

- `widget_ap_stream`: Poppler and MuPDF paint the widget AP stream; PDFium does not. Wellfriend matches Poppler/MuPDF.
- `ocmd_allon_hidden`: references differ on the synthetic OCMD fixture; Wellfriend remains inside the documented Prompt 09B OCG policy cluster.

Both rows have rendered artifacts and diff metrics in the Prompt 09B artifact root.
