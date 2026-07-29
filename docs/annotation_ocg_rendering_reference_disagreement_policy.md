# Annotation Ocg Rendering Reference Disagreement Policy

Renderer Validation keeps the multi-reference discipline from Reference Renderer.

## Reference Engines

- Poppler from the Reference Renderer manifest
- PDFium from the target-local Reference Renderer wrapper
- MuPDF from the Reference Renderer manifest

If any engine is unavailable, the affected rows are not counted as passed unless the Reference Renderer target-local bootstrap succeeds and records the tool.

## Classification Rules

When references agree, Wellfriend must be within threshold or the row is an Wellfriend outlier. When references disagree, Wellfriend may be accepted only if the row explains the cluster and provides screenshots/diff metrics. Unsupported active or generated behavior must be reported as unsupported; it must not be silently ignored.

Renderer Validation currently records two reference-cluster disagreements:

- `widget_ap_stream`: Poppler and MuPDF paint the widget AP stream; PDFium does not. Wellfriend matches Poppler/MuPDF.
- `ocmd_allon_hidden`: references differ on the synthetic OCMD fixture; Wellfriend remains inside the documented Renderer Validation OCG policy cluster.

Both rows have rendered artifacts and diff metrics in the Renderer Validation artifact root.
