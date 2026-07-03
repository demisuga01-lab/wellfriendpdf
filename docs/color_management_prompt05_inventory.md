# Prompt 05 Color Inventory

Starting checkpoint: `16af5ac Complete Prompt 04E final font parity audit`.

Prompt 05 found that Oxide already had a useful internal color layer:
`render::cmm` handled qcms-backed ICC preview transforms and deterministic
DeviceCMYK/Cal/Lab fallback conversion; `render::colorspace` resolved
Separation and DeviceN through tint transforms; `render::function` evaluated PDF
Function Types 0, 2, 3, and 4 for shadings and spot-color transforms.

| color feature | parser support | rendering support | image conversion | shading support | writer support | PDF/A/PDF/X reporting | diagnostics before Prompt 05 | Prompt 05 target |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| DeviceGray | DONE | DONE | DONE | DONE | partial authoring | report usage | scattered | central report/API |
| DeviceRGB | DONE | DONE | DONE | DONE | partial authoring | report usage | scattered | central report/API |
| DeviceCMYK | DONE | preview fallback | DONE | DONE through paint path | CMYK values not preserved for true prepress output | report usage | limited | explicit approximate-preview diagnostics |
| CalGray | DONE | DONE via D50 fallback | DONE | DONE through paint path | not a writer focus | report usage | limited | document/test/report |
| CalRGB | DONE | DONE via D50 fallback | DONE | DONE through paint path | not a writer focus | report usage | limited | document/test/report |
| Lab | DONE | DONE via Lab to XYZ/sRGB | DONE | DONE through paint path | not a writer focus | report usage | limited | document/test/report |
| ICCBased | DONE | qcms to sRGB when valid | qcms to RGB/Gray/CMYK preview | supported through named color path | output intents reported, authoring output intent still subset | profile presence/basic validity | limited | cap profiles, expose backend decision |
| Indexed | DONE | image path and report | DONE | limited by base space | not a writer focus | report lookup issues | limited | structured report |
| Separation | DONE | tint-transform to alternate preview | not broadly image-specific | supported through named color path | spot metadata not preserved in generated prepress output | spot names reported | limited | report/diagnose approximation |
| DeviceN | DONE | tint-transform to alternate preview | not broadly image-specific | supported through named color path | DeviceN metadata not preserved in generated prepress output | component sets reported | limited | component cap and diagnostics |
| Pattern colors | DONE | supported by renderer architecture | n/a | pattern path uses current color behavior | not a writer focus | usage report via resources | limited | documented as not full prepress pattern semantics |
| Shading colors | DONE | functions and current color paths | n/a | axial/radial/mesh infrastructure uses function evaluator | not a writer focus | color-heavy benchmark | limited | cap functions and document |
| OutputIntent | parsed as COS | no CMM destination transform yet | n/a | n/a | partial authoring | new report validation | no color-specific report | expose report and PDF/A/PDF/X color diagnostics |
| RenderingIntent | content state `ri` existed | carried in graphics state | n/a | state available | not writer focus | report usage | no public color report | parse/report and carry through display-list draw state |
| Overprint | not tracked | not simulated | n/a | n/a | not writer focus | no | none | parse OP/op/OPM, preserve, diagnose preview approximation |
| PDF Function Type 0 | DONE | DONE | n/a | DONE | n/a | cap reported | no sample-count cap | sample cap |
| PDF Function Type 2 | DONE | DONE | n/a | DONE | n/a | supported | existing tests | document |
| PDF Function Type 3 | DONE | DONE | n/a | DONE | n/a | supported | existing tests | document |
| PDF Function Type 4 | DONE | DONE | n/a | DONE | n/a | cap reported | depth cap only | token/stack/non-finite caps |

Prompt 05 deliberately did not introduce LittleCMS/native FFI. The engine crate
forbids unsafe code, WASM/default portability matters, and qcms already covers
the preview ICC path used today. True prepress separations and device-link ICC
work remain explicit bounded follow-ups, not hidden behavior.
