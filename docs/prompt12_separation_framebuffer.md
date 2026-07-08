# Prompt 12 Separation Framebuffer

Prompt 12 introduces a sparse separation framebuffer side-channel. It is kept
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

Storage is sparse. The implementation records observed plate contributions
rather than allocating a full-page N-plane buffer for every possible colorant.
The default plate cap is 32 and the default accounting budget is 64 MiB.
Excessive colorant or memory cases degrade to report-only or fail closed with a
diagnostic.

Prompt 12 integrates fill and stroke paths for Separation and DeviceN resources.
Transparency groups and soft masks preserve child plate state into the parent
report, but full overprint blending remains Prompt 13.
