# Prepress CMM Separation Framebuffer

Prepress CMM introduced a sparse separation framebuffer side-channel. Nchannel Plate Prepress
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

Nchannel Plate Prepress integrates text, vector, stencil image, shading, and pattern plate
sampling for supported Separation and DeviceN paths. Transparency groups and
soft masks preserve child plate state into the parent report. Prepress Proofing adds
bounded overprint/prepress close-out on top of this Nchannel Plate Prepress baseline.
