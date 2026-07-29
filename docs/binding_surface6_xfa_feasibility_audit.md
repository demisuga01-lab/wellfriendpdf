# roadmap closure 16 XFA feasibility audit

Wellfriend now has a bounded XFA foundation, not an Adobe LiveCycle/AEM Forms emulator. The canonical owner is `crates/engine/src/xfa`; AcroForm coexistence remains in `interactive.rs`, ordinary page rendering and writing remain in the existing renderer/editor/writer, and security surfaces remain in `security.rs` and the shared SDK facade.

## Feasibility verdict

| Capability | Status | Boundary |
| --- | --- | --- |
| `/XFA` name/stream array and single XDP stream | `implemented_with_limits` | Ordered, hashed, provenance-bearing, bounded decoding and XML parsing |
| Template/datasets/config/locale/form packet inventory | `implemented_with_limits` | Unknown and ancillary packets remain ordered inventory records |
| connectionSet/sourceSet | `unsupported_reported_security_policy` | Inventoried; never dereferenced; sanitizer can remove them |
| Static template and dataset extraction | `implemented_with_limits` | Common fields, captions, values, choices, geometry, styling hints, hierarchy, SOM/bind provenance |
| Static rendering and page-overlay flattening | `implemented_with_limits` | Ordinary PDF page content through the existing editor/writer and renderer reopen path |
| Minimal dynamic layout | `implemented_with_limits` | Positioned/tb/lr/row, occur, dataset instances, simple page overflow and presence |
| Complex leaders/trailers, keep cycles, arbitrary DOM mutation | `unsupported_reported_exact` | Named in runtime reports; static flatten modes fail closed for dynamic documents |
| FormCalc | `implemented_with_limits` | Pure expressions for calculate/validate only, explicit opt-in |
| JavaScript and proprietary script execution | `unsupported_reported_security_policy` | Inventory and hash only; never executed |
| Full LiveCycle/AEM parity | `not_in_xfa_runtime_scope` | No compatibility claim |

No XFA Runtime-scope row is `blocked`. Exact results are generated at `target/xfa_runtime-xfa-runtime/xfa_runtime-xfa-feasibility-audit.json` by `scripts/xfa_runtime_xfa_runtime_audit.py`.

## Canonical integration points

- Packet/XML/model/runtime/sandbox/flatten/sanitize: `crates/engine/src/xfa/`.
- AcroForm merge and redaction verification: `crates/engine/src/interactive.rs`.
- Security report: `crates/engine/src/security.rs`.
- Page content and reopen rendering: existing `editing.rs`, `writer.rs`, and `render/` modules.
- Versioned reports: `crates/engine/src/sdk.rs`; all language bindings call this facade.

## Security decision

Scripts and events are disabled by default. Opt-in only admits the pure FormCalc expression evaluator for calculate/validate. There is no network, filesystem, process, native, environment, clipboard, UI, timer, random, host, or external-resource API. XML DTDs, entity declarations, unknown entities, invalid UTF-8, non-finite measurements, and cap violations fail closed.

## Mutation decision

`render_preview` may preview the bounded dynamic subset. Static flatten modes reject dynamic XFA. `flatten_and_remove_xfa` additionally rejects any exact unsupported construct before mutation. Every mutation is a full rewrite, is reopened and rendered, and reports signature/DocMDP/FieldMDP risk; existing signature byte ranges are not claimed preserved.
