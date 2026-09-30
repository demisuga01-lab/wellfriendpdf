# PEBQ native adapters

PEBQ compares PDF engines only when they execute the same declared contract.
The native adapters accept identical in-memory input and expose two profiles:

- `page-count`: open the PDF and return the resolved page count.
- `render`: perform `page-count`, then rasterize page one to RGB at the exact
  requested DPI. Optional PPM writing occurs after the timed raster stage.

Each request reports file-read, parse, render, RGB-normalization/write and full
request timings separately. `--server` retains only process/library startup;
it opens a fresh document for every request. `--request` is the corresponding
fresh-process contract.

The adapters do not make qpdf a renderer. qpdf participates in parsing only.
The qualification harness must reject mismatched page counts and failed
renders before calculating qualified speed rankings.
