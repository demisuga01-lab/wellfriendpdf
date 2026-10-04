# Prepared Render Pipeline

## Objective

The Prepared Render Pipeline (PRP) reduces repeated page-render latency without
changing the PDF rendering contract. It separates document-cold rendering,
retained-resource rendering, and final-raster cache hits so measurements and
product claims cannot mix unlike work.

PRP applies to arbitrary PDFs. No corpus filename, object number, page checksum,
or benchmark-specific threshold selects a fast path.

## Latency classes

| Profile | Included work | Final-raster reuse |
|---|---|---|
| Document cold | Open identical bytes, resolve page, parse content, compile plan, rasterize, normalize RGB | No |
| Retained resources | Reuse an open document, parsed display list, compiled plan, decoded resources, glyph/path masks, and scratch buffers; rasterize again | No |
| Final-raster hit | Validate the complete render-contract key and return an existing raster | Yes |

Only the first two profiles measure renderer execution. The final profile
measures application cache latency and always appears separately.

## Contract identity

Every prepared artifact uses this logical key:

```text
document revision
+ page identity and page box
+ DPI and output dimensions
+ device transform and tile window
+ optional-content visibility
+ annotation and form policy
+ print, proof, overprint, and color-management policy
+ rendering intent
+ smoothing, subpixel, and backend policy
+ prepress/resource fingerprint
+ exactness and resource limits
```

A key mismatch is a cache miss. Transaction invalidation removes artifacts whose
source-object dependency closure intersects the edit. Unknown dependency state
resets the cache instead of publishing stale pixels.

## Preparation graph

PRP represents one page as an immutable dependency graph:

```text
source objects
  -> decoded content operations
  -> typed display list
  -> packed render plan + spatial index
  -> resolved fonts/images/forms/shadings/patterns
  -> ordered tile replay
  -> deterministic tile assembly
  -> raw RGB publication
```

Preparation deduplicates object decoding, resource lookup, plan compilation,
font program parsing, glyph outlines, image decoding/scaling, clipping masks,
soft masks, shading meshes, Form programs, pattern programs, and annotation
appearance programs. Raster replay still executes for every renderer timing.

## Execution scheduler

The scheduler divides a page into deterministic horizontal bands or rectangular
tiles when the render plan supports independent replay. Every tile replays the
same ordered operation stream against a tile-local surface and shared immutable
prepared resources. Transparent groups, filters, backdrop-dependent blend
modes, and operations with unresolved global bounds expand the tile by the
required halo or select the serial exact path.

Parallel execution never reorders painting operations within a tile. Assembly
uses fixed coordinates and contains no blending step, so worker completion order
does not affect pixels. Cancellation stops unpublished work; only a complete
contract-matching raster reaches callers.

## Tail-latency controls

- Prepared artifacts use byte and entry limits with deterministic eviction.
- Oversized pages remain executable through bounded streaming/tile fallbacks.
- Image decoders receive source-region and reduction requests only when the
  selected codec reports native support.
- Scratch buffers return to bounded per-document pools.
- A page-level stage trace reports content decode, display-list build, plan
  compile, text, vector, image, transparency, annotation, flattening, and RGB
  conversion time.
- Slow-document reports use document medians and retain every raw observation.

## Quality invariant

Optimization does not reduce DPI, skip annotations, change antialiasing, replace
color management, flatten transparency early, omit unsupported operators, or
silently substitute a font. The optimized path either matches the selected
contract or returns a typed refusal. Differential quality metrics remain
diagnostic; strict fixtures and PDF feature invariants provide the correctness
oracle where one exists.

## Anti-rigging rules

1. The corpus manifest and benchmark configuration have hashes.
2. Adapter sources, linked-library versions, and binaries have hashes.
3. A reproducible-build manifest binds source revision, compiler, flags,
   dependencies, and adapter binary.
4. Every engine receives the same operation, DPI, CPU set, repetition count,
   randomized block order, and warm-up policy.
5. Primary percentiles use one median per document. Raw observations remain
   visible as scheduler and tail diagnostics.
6. Repeated output dimensions and raster hashes must remain stable.
7. Document-cold, retained-resource, and final-raster-hit results never share a
   table or performance claim.
8. Failures, timeouts, dimension disagreements, and unsupported files remain in
   the denominator.

## Current source boundary

The renderer exposes a caller-owned `RenderDocumentCache`, revision-aware
dependency invalidation, retained display lists, packed plans, tile replay, and
bounded resource caches. `FinalRasterCachePolicy::ResourcesOnly` makes retained
resource measurements execute rasterization on every call. The cache retains a
bounded compiled plan under the complete render-contract fingerprint and the
document revision. Page/source invalidation removes the corresponding plan.
The PEBQ v2 report format labels document-cold semantics, uses document-median
primary statistics, retains raw observations, discloses slowest documents, and
checks repeated-raster determinism.

Stage telemetry, parallel tile scheduling, and reproducible-build attestation
remain separate deliverables until source and VPS evidence exist. A
single-digit document-cold median is not a present claim: the qualified median
is 114.979 ms, so the target requires at least a 12.8x median reduction while
preserving the same output contract.
