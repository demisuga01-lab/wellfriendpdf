# Prompt 11 Native CMM Feasibility and Safety Audit

Prompt 11 audits LittleCMS/lcms2 as the native/accurate CMM candidate and
records a precise hard block for this repository state.

## Decision

LittleCMS is not linked, vendored, or exposed in the default build. The current
engine crate uses `#![forbid(unsafe_code)]`, default and WASM builds must not
gain a silent native dependency, and Prompt 11 does not introduce a separate
audited native boundary crate or package policy.

The reserved future feature flag name is:

```text
native-cmm-lcms2
```

It is documentation-only in Prompt 11 and is not a compiled Cargo feature.

## Current Backend

The current build uses safe Rust plus qcms:

- ICCBased preview transforms to sRGB when qcms accepts the profile
- deterministic DeviceCMYK process-ink preview
- CalRGB, CalGray, and Lab fallback conversion
- rendering intent carried into qcms options where supported
- profile materialization capped at 16 MiB
- bounded transform/profile cache reporting

## Native Boundary Requirements

A future LittleCMS backend must be optional, feature-gated, report-visible,
WASM-disabled, packaging-documented for each binding, covered by fuzzing, and
kept outside the engine default build unless the security policy changes. It
must actually apply transforms before any native color-accuracy claim is made.
