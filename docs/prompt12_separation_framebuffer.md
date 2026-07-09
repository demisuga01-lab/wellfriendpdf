# Prompt 12 Separation Framebuffer

Prompt 12 introduced a sparse separation framebuffer side-channel. Prompt 12B
extends it into a sampled n-channel plate contribution surface. It remains
separate from the RGB preview framebuffer so spot and DeviceN state can survive
without claiming full press simulation.

The model records:

- process and named plate identity
- spot colorants
- DeviceN component names
- tint values
- alternate preview RGB
- alpha and coverage posture
- operation provenance
- page and tile identity
- enabled/disabled plate state
- deterministic plane order
- memory accounting
- per-sample n-channel plate contributions
- operation-kind inventory
- cache fingerprint fields for backend, profile, intent, BPC, output intent,
  and plate state

Storage is still bounded. The implementation records observed plate
contributions as tile-local sparse planes plus sampled n-channel contribution
records rather than allocating an unbounded full-page N-plane buffer for every
possible colorant. The default plate cap is 32, the n-channel cap is 15, and the
default accounting budget is 64 MiB. Excessive colorant, channel, or memory
cases degrade to report-only or fail closed with a diagnostic.

Prompt 12B integrates text, vector, stencil image, shading, and pattern plate
sampling for supported Separation and DeviceN paths. Transparency groups and
soft masks preserve child plate state into the parent report. Prompt 13 adds
bounded overprint/prepress close-out on top of this Prompt 12B baseline.
