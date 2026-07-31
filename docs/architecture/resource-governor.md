# Resource governor

The runtime resource governor provides permits for costly work classes:
metadata, parser/recovery, decoding, rendering, image codecs, shaping, reflow,
OCR, writer/compression, standards/accessibility, redaction/sanitization, and
external providers.

Every expensive operation declares a work estimate with CPU units, memory bytes,
temporary disk bytes, I/O weight, parallelism ceiling, interruptibility, and
optional provider requirements.

Standard mode is the default and adapts to the host. The validated deployment
contract targets 2 vCPU / 6 GiB minimum and 4 vCPU / 8 GiB recommended. Research
adds optional accelerator and provider permits but must fall back to Standard
when those capabilities are inactive.
