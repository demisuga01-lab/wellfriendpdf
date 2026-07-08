# Prompt 11 CMM Security Policy

ICC profiles are untrusted input. Prompt 11 keeps native CMM out of the default
engine and requires fail-closed behavior for invalid or oversized profiles.

## Policy

- Default and WASM builds must not depend on native CMM libraries.
- ICC profile bytes must be size-capped before parsing or transform creation.
- Invalid profiles must return diagnostics rather than silently falling back to
  a claimed accurate transform.
- Transform/profile caches must be bounded and keyed by source profile,
  destination profile or target role, rendering intent, black-point-compensation
  posture, and color-space role.
- Any future native CMM backend must be feature-gated, package-visible,
  fuzz-covered, documented, and disabled where unsupported.

Prompt 11 keeps qcms preview transforms but does not claim certified prepress
accuracy.
