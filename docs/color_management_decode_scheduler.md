# Decode Scheduler Color Management

Decode Scheduler makes the existing color layer inspectable, bounded, and usable as a
shared color/prepress surface.

## Architecture

```
PDF ColorSpace / content color operators
  -> ColorSpace model and graphics state
  -> PDF Function evaluator for tint transforms and shadings
  -> render::cmm preview transforms
  -> ColorReport diagnostics and validation profile
  -> renderer / image / shading / pattern consumers
```

Core modules:

- `crates/engine/src/render/cmm.rs`: deterministic DeviceCMYK, CalGray,
  CalRGB, Lab, and qcms ICCBased preview transforms. ICC profile materialization
  is capped at 16 MiB.
- `crates/engine/src/render/colorspace.rs`: Separation, DeviceN, ICCBased,
  Cal, and Lab named-space resolution. DeviceN component count is capped at 16.
- `crates/engine/src/render/function.rs`: PDF Function Types 0, 2, 3, and 4.
  Type 0 sampled functions cap total sample values at 4,194,304. Type 4
  calculator programs cap tokens at 16,384 and stack depth at 1,024.
- `crates/engine/src/content/state.rs`: rendering intent plus OP/op/OPM
  overprint state in the graphics-state stack.
- `crates/engine/src/render/display_list.rs`: rendering intent and overprint
  metadata carried in `DrawState`.
- `crates/engine/src/color_report.rs`: public structured color/prepress report.

## CMM Backend Decision

Outcome B/C: Wellfriend keeps the default pure-Rust/no-unsafe engine boundary and
uses qcms for ICCBased preview transforms. No LittleCMS/native FFI is introduced
in Decode Scheduler. The report exposes this as:

- `default_backend = "safe-rust-plus-qcms"`
- `native_littlecms_integrated = false`
- `default_build_unsafe_ffi = false`

This means screen preview for common ICCBased RGB/Gray/CMYK profiles is
implemented, but device-link transforms, full multicolor profiles, black-point
compensation, and certified prepress conversion are not claimed.

Color Management keeps this as Outcome B and makes it measurable rather than merely
architectural. qcms ICC transforms now flow through a bounded transform cache,
the report exposes cache metrics, and compact sRGB identity probes prove that
the selected backend path is deterministic and within a one-byte channel
tolerance. The default build still has no native LittleCMS dependency and
preserves the pure-Rust/no-unsafe engine policy.

## Public Report API

Rust API:

```rust
use wellfriendpdf_engine::{color_report_bytes, ColorValidationProfile};

let report = color_report_bytes(pdf_bytes, ColorValidationProfile::PdfA)?;
```

CLI:

```powershell
wellfriendpdf parser-report input.pdf --json --include-color --color-profile pdfa
```

The JSON `color` object includes:

- backend decision;
- ICC transform cache and fidelity vectors;
- limits;
- color-space family counts;
- spot colorants;
- DeviceN component sets;
- output intents;
- rendering intents;
- overprint state;
- diagnostics.

Stable diagnostic codes added in Decode Scheduler include:

- `color.output_intent.missing`
- `color.output_intent.profile_missing`
- `color.icc.profile_too_large`
- `color.icc.invalid_profile`
- `color.icc.decode_failed`
- `color.icc.unsupported_components`
- `color.devicen.component_cap`
- `color.devicen.tint_transform_missing`
- `color.separation.tint_transform_missing`
- `color.indexed.invalid_hival`
- `color.indexed.lookup_missing`
- `color.indexed.lookup_malformed`
- `color.overprint.preview_approximation`
- `color.content_stream.scan_cap`

## Support Summary

| feature | Decode Scheduler status | bounded limit |
| --- | --- | --- |
| DeviceGray/RGB | DONE | normal numeric clamping |
| DeviceCMYK | DONE WITH BOUNDED LIMIT | preview fallback, not certified prepress conversion |
| CalGray/CalRGB/Lab | DONE | malformed arrays fall back to defaults/reporting |
| ICCBased | DONE WITH BOUNDED LIMIT | qcms preview; profile bytes capped at 16 MiB |
| Indexed | DONE WITH BOUNDED LIMIT | reports malformed lookup; image path already handles samples |
| Separation | DONE WITH BOUNDED LIMIT | tint transform to alternate preview, spot metadata reported |
| DeviceN | DONE WITH BOUNDED LIMIT | max 16 components; tint transform to alternate preview |
| Rendering intent | DONE WITH BOUNDED LIMIT | parsed/reported/carried; fallback transforms may ignore it |
| Black-point compensation | DEFERRED WITH REASON | no default CMM backend support; surfaced in backend decision |
| Overprint | DONE WITH BOUNDED LIMIT | OP/op/OPM parsed and preserved; Color Management adds DeviceCMYK fill overprint preview in the RGB framebuffer |
| PDF/A color | DONE WITH BOUNDED LIMIT | output-intent color checks only, not full PDF/A validation |
| PDF/X color | DONE WITH BOUNDED LIMIT | output-intent/prepress usage checks only, not full PDF/X validation |

## Benchmark

Decode Scheduler created a deterministic 24-file color/prepress slice under
`target/decode_scheduler-color-baseline-manifest.json` from existing corpus entries:
CMYK synthetic pages, pdf.js DeviceN/color-space/function fixtures, Indexed
samples, CMYK JPEG, shadings, and tiling patterns.

Command:

```powershell
cargo build --release -p wellfriendpdf-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest target\decode_scheduler-color-baseline-manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --dpi 96 --timeout-sec 30 --max-memory-mb 2048 --max-pages-per-file 1 --output-dir target\decode_scheduler-color-after --determinism-sample 4 --threshold-profile renderer
```

Results:

| metric | baseline | after Decode Scheduler |
| --- | ---: | ---: |
| files | 24 | 24 |
| weighted score | 59.0 | 59.0 |
| visual pass | 60.87% | 60.87% |
| file pass | 58.33% | 58.33% |
| peak Wellfriend memory | 19.69 MB | 19.68 MB |
| determinism | 4/4 stable | 4/4 stable |

The score did not move because Decode Scheduler focused on architecture, caps,
diagnostics, and reportability, not new mesh/pattern/color-glyph raster
fidelity. No benchmark regression was observed.

Color Management reran the same 24-file anchor before changes under
`target\color_management-color-before`, then reran after changes under
`target\color_management-color-after`:

| metric | before Color Management | after Color Management |
| --- | ---: | ---: |
| files | 24 | 24 |
| weighted score | 59.0 | 59.0 |
| visual pass | 60.87% | 60.87% |
| file pass | 58.33% | 58.33% |
| peak Wellfriend memory | 19.48 MB | 19.47 MB |
| determinism | 4/4 stable | 4/4 stable |

Color Management's closure details are tracked in
`docs/color_color_management_closure_audit.md`.

## Remaining Bounded Limits

- No native LittleCMS backend or device-link/prepress conversion. Color Management
  records this as a deliberate Outcome B decision with qcms cache/fidelity
  evidence.
- Black-point compensation is reported as unavailable in the fallback backend.
- Overprint is parsed/preserved and diagnosed. DeviceCMYK fill overprint has an
  RGB-framebuffer preview path; stroke, spot, and DeviceN plate overprint are
  not fully simulated.
- DeviceN/Separation preview uses tint transform to alternate color space; spot
  separations are preserved only in reporting metadata.
- PDF/A/PDF/X checks are color/output-intent subset checks, not complete
  standards conformance.
