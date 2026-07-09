# ML Layout Backend Runtime Policy

Prompt 14B does not bundle ONNX Runtime, Torch, LayoutParser, DocLayNet,
TableTransformer, or model weights. That is intentional.

Reasons:

- model weights require explicit license and redistribution proof;
- ML runtimes add heavyweight optional dependencies;
- core semantic extraction must remain deterministic and offline by default;
- Prompt 14's roadmap item was the hook/schema/template layer, not mandatory
  bundled ML document understanding.

Applications can integrate a real local runtime by converting model output into
`LayoutProposalSet`, then merging through the deterministic Prompt 14 merge
policy. The local runtime must use user-supplied model paths, enforce
timeout/memory/page limits, avoid network access, and report availability.
