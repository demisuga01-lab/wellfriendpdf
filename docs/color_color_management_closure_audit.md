# Color Management Color/Prepress Closure Audit

Color Management closes the high-value color/prepress leftovers from Decode Scheduler without
claiming complete PDF/A or PDF/X conformance. The closure target is a stronger,
bounded preview and reporting layer: qcms-backed ICC transforms with cache and
fidelity probes, explicit spot/DeviceN preview accounting, common-case CMYK fill
overprint preview, and stricter output-intent color checks.

## Starting Baseline

Starting checkpoint: `1a29d8e Complete Decode Scheduler color management foundation`.
The worktree was clean before Color Management edits.

Anchor benchmark command:

```powershell
cargo build --release -p wellfriendpdf-cli
python renderer-benchmark\scripts\renderer_benchmark.py --manifest target\decode_scheduler-color-baseline-manifest.json --wellfriendpdf-bin target\release\wellfriendpdf.exe --dpi 96 --timeout-sec 30 --max-memory-mb 2048 --max-pages-per-file 1 --output-dir target\color_management-color-before --determinism-sample 4 --threshold-profile renderer
```

Color Management reproduced the Decode Scheduler anchor:

| metric | before Color Management |
| --- | ---: |
| files | 24 |
| visual pages compared | 23 |
| weighted score | 59.0 |
| visual pass | 60.87% |
| file pass | 58.33% |
| peak Wellfriend memory | 19.48 MB |
| determinism | 4/4 stable |
| Poppler | 26.02.0 |
| PDFium | not available |

After the Color Management changes, the same anchor was rerun under
`target\color_management-color-after`:

| metric | before Color Management | after Color Management |
| --- | ---: | ---: |
| files | 24 | 24 |
| visual pages compared | 23 | 23 |
| weighted score | 59.0 | 59.0 |
| visual pass | 60.87% | 60.87% |
| file pass | 58.33% | 58.33% |
| peak Wellfriend memory | 19.48 MB | 19.47 MB |
| determinism | 4/4 stable | 4/4 stable |

The remaining benchmark failures are concentrated in pdf.js color/function/image
fixtures: calculator-function color-space pages, colorkey masks, CMYK JPEG,
function-based shading, and malformed page-label content. Color Management therefore
does not rewrite mesh geometry, color-key masks, or general image decoding.

## Closure Table

| area | Decode Scheduler behavior | Color Management decision | tests/evidence | remaining limit |
| --- | --- | --- | --- | --- |
| Native/accurate CMM backend | safe Rust plus qcms, no LittleCMS FFI | Outcome B: keep default pure Rust/no-unsafe boundary and make qcms transform use explicit, cached, and measured | `srgb_identity_fidelity_vectors_pass`, transform cache tests, color-report backend fields | no device-link or multicolor ICC engine |
| ICC transform fidelity | qcms used for ICCBased preview but no reportable vectors/cache | added transform cache metrics and built-in sRGB identity fidelity probes | report JSON exposes `icc_transform_cache` and `icc_fidelity_vectors` | fidelity proof is compact; not a full ICC suite |
| ICC profile validation and limits | 16 MiB cap, qcms parse diagnostics | retained cap and invalid-profile diagnostics; stricter report fields | `reports_output_intent_and_invalid_icc_profile` | advanced profile classes are fallback/diagnostic only |
| DeviceCMYK preview/proof behavior | deterministic preview fallback | added common-case overprint preview for DeviceCMYK fill paints in CPU renderer/display-list path | `cmyk_overprint_preview_preserves_zero_ink_channels` | RGB framebuffer approximation, not true separations proofing |
| Spot color preview | tint transform to alternate preview, metadata reported | report now counts Separation spaces, tint transforms, and missing transforms | `reports_spot_devicen_overprint_and_intent` | no independent spot plate framebuffer |
| DeviceN preview | tint transform to alternate preview, metadata reported | report now counts DeviceN spaces, tint transforms, and missing transforms | `reports_spot_devicen_overprint_and_intent`, component-cap tests | no arbitrary N-channel compositing |
| Separation tint transforms | parsed/reported through function evaluator | explicit preview model and counters | color-report tests | malformed transforms diagnose; no press-separation output |
| DeviceN tint transforms | parsed/reported through function evaluator | explicit preview model and counters | color-report tests | component cap remains 16 |
| Overprint simulation | OP/op/OPM preserved and diagnosed | DeviceCMYK fill overprint preview preserves zero-ink channels in RGB approximation; display-list carries CMYK paint metadata | render buffer, path, page renderer, display-list tests | stroke overprint and spot/DeviceN plate overprint are diagnostic/approximate |
| Rendering intents | parsed/reported/carried | qcms-supported intent names exposed; backend reports supported intents | report field tests | qcms fallback may not visibly differ across all profiles |
| Black-point compensation | unavailable in default backend | explicit diagnostic/report posture retained | backend decision report fields | no BPC in default qcms path |
| PDF/A output-intent color checks | missing/profile-missing checks | report records color-only standards scope and output-intent checks | output-intent tests | not full PDF/A validation |
| PDF/X output-intent/prepress checks | same as PDF/A subset | adds `/S` check and profile-component warning for non-CMYK output profile when `/N` is known | `pdfx_profile_checks_output_intent_s_and_cmyk_profile` | not full PDF/X validation |
| Image color conversion | existing image conversion and reports | unchanged; color benchmark anchor rerun | benchmark before/after | color-key mask/CMYK JPEG failures remain outside this targeted closure |
| Shading/pattern color integration | existing Decode Scheduler routing/reporting | unchanged; function evaluator/reporting tightened indirectly | benchmark and function tests | function-based shading render gaps remain renderer work |
| Color benchmark coverage | 24-file anchor | rerun before/after with Poppler 26.02.0 | artifacts under `target\color_management-color-before` and `target\color_management-color-after` | benchmark is a bounded slice, not Tier-3 proof |

## CMM Backend Decision

Outcome B is final for Color Management. Wellfriend keeps the no-unsafe default engine
boundary and uses qcms as the safe Rust ICC transform backend. LittleCMS is not
integrated because it would add native FFI/build/WASM policy work that is not
required to close this bounded Color Management target. The backend decision is now
reportable with supported rendering intents, black-point-compensation posture,
and native-LittleCMS status.

The implementation adds a byte-keyed transform cache for qcms transforms. Cache
keys include source profile bytes, destination profile bytes, source and
destination data type tags, rendering intent, and requested BPC flag. The cache
has a bounded entry count and exposes hits, misses, evictions, invalid profiles,
and unsupported transform counts.

## ICC Fidelity Proof

Color Management adds compact built-in sRGB-to-sRGB identity probes. These are not a
complete ICC conformance suite; they prove that the selected backend path is
active, deterministic, cached, and within tolerance for known RGB vectors.

The report includes each probe name, backend, input/output byte count, max
absolute channel error, tolerance, and pass/fail status. The current tolerance is
1 byte per channel.

## Overprint and Spot Preview

The CPU renderer still stores pixels as RGB. Color Management therefore implements a
common-case preview simulation, not a true separation framebuffer. For
DeviceCMYK fills with fill overprint enabled, the renderer converts the current
destination RGB to an approximate CMYK state, preserves destination separations
when source ink is zero and overprint mode is one, converts back to sRGB, and
then composites through the existing alpha/blend path.

Separation and DeviceN continue to preview through tint transforms into the
alternate color space while preserving colorant/component metadata in the
report. This is honest screen preview, not press-plate output.

## Standards Scope

Color Management strengthens color-only PDF/A/PDF/X reporting:

- PDF/A and PDF/X profiles mark output-intent checks as executed.
- Missing output intents and missing profiles remain diagnostics.
- PDF/A checks warn when output-intent `/S` is not `GTS_PDFA1`.
- PDF/X checks warn when output-intent `/S` is not `GTS_PDFX`.
- PDF/X checks warn when `DestOutputProfile` exposes `/N` and it is not CMYK.

This is not full PDF/A or PDF/X conformance. Metadata schemas, annotations,
fonts, transparency restrictions, compression rules, and full preflight remain
for the standards phase.

## Bounded Limits After Color Management

- No native LittleCMS backend in the default build.
- No device-link, multicolor ICC, or certified proofing transform.
- Black-point compensation is reported but unavailable in the default backend.
- Overprint preview is implemented for DeviceCMYK fills only; stroke, spot, and
  DeviceN overprint remain approximate/report-only.
- Spot and DeviceN preview uses tint-transform-to-alternate, not independent
  plate compositing.
- Color-key image masks, CMYK JPEG blank mismatches, and function-based shading
  renderer gaps remain outside Color Management's targeted closure.
