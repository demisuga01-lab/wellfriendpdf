# Prompt 12B Prepress N-Channel Plate Closure Audit

Starting checkpoint:

- expected commit: `829d570 Complete combined prompt 12 prepress cmm separation plates`
- starting worktree: clean
- artifact root: `target/prompt12-prepress-cmm`
- audit script: `scripts/prompt12b_prepress_nchannel_plate_closure.py`

Prompt 12B closes the remaining Prompt 12 prepress scope without claiming full
overprint simulation or certification-grade PDF/X validation.

Closure classifications:

| Item | Status | Evidence |
| --- | --- | --- |
| device-link transform path | implemented_with_limits | Native LittleCMS builds expose the legal device-link transform posture; fallback/WASM report preview-only. |
| device-link output channel handling | implemented_with_limits | Transform keys and n-channel sample metadata carry input/output channel counts and fail closed above the safe cap. |
| multicolor ICC 2CLR through FCLR transform path | implemented_with_limits | Inventory covers 2CLR through FCLR; safe n-channel output representation exists, with unsupported native wrapper cases reported precisely. |
| arbitrary/high-channel n-color output representation | implemented | Bounded dynamic channel samples support 1 through 15 channels with labels, process/named distinction, alpha, profile context, intent, BPC, and cache fingerprinting. |
| n-channel image/intermediate pixel format | implemented | `NChannelSample` is recorded from plate contributions and accounted in the separation framebuffer report. |
| text plate writing | implemented_with_limits | Fill/stroke text modes and supported Type3 path geometry record plate contributions; resource-heavy recursive Type3 charprocs remain exact limited cases. |
| vector fill/stroke plate writing | implemented | Fill, stroke, fill-stroke, even-odd, dash/cap/join geometry paths record named/process plate contributions. |
| image plate writing | implemented_with_limits | Stencil masks and named Separation/DeviceN image color-space samples write plate contributions; unsafe high-channel packed image layouts fail closed/report-only. |
| shading plate writing | implemented_with_limits | Named Separation/DeviceN shading resources and shading patterns produce plate samples where the color-space resource is resolvable. |
| tiling pattern plate writing | implemented_with_limits | Colored tiling and caller-color plate samples are recorded, with recursion limits preserved. |
| spot plate preview | implemented | Plate preview hashes and per-plate contribution summaries are emitted in Prompt 12 and 12B artifacts. |
| DeviceN process/named component separation | implemented | Cyan/Magenta/Yellow/Black remain process plates; other DeviceN names remain named component plates. |
| tint transform interaction | implemented_with_limits | Bounded PDF functions provide alternate preview; malformed or excessive functions fail closed. |
| BPC/rendering intent with native backend | implemented_with_limits | Intent and BPC state are propagated into transform/cache keys; fallback reports BPC unsupported. |
| PDFium reference availability | implemented | Target-local Prompt 06B PDFium wrapper is required and run by the Prompt 12B audit. |
| MuPDF reference availability | implemented | Target-local Prompt 06B `mutool` is required and run by the Prompt 12B audit. |
| Poppler reference preservation | implemented | Existing Poppler reference path is preserved and included in all Prompt 12B reference results. |
| native/fallback report parity | implemented | Feature and color reports expose the same additive Prompt 12B envelope with native/fallback posture. |
| public binding report parity | implemented | Rust, CLI, Python, C ABI, WASM, .NET, Java Maven, and Java Gradle smokes assert the new section. |

The audit artifacts report zero Oxide outlier failures and zero unclassified
failures. Reference renderer disagreements remain classified because spot and
DeviceN previews are often flattened differently by external tools, while Oxide
plate data is verified through internal artifacts.
