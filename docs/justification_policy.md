# Justification Policy

Horizontal `left`, `right`, `center`, `start`, `end`, and `justify` are
serialized through Prompt 20. Full justification uses bounded PDF `Tw` then
`Tc`, records each line adjustment, and fails before mutation if the residual
cannot fit within those bounds. Final lines are not justified by scaling
outlines.

Arabic requests for full justification now return `shaping_failed` before
mutation: the SDK will not substitute generic character spacing for kashida.
CJK-specific punctuation policy and vertical full justification are also exact
unsupported results. CJK text may use the ordinary bounded horizontal spacing
path only when the caller has not requested a script-specific justification
policy.
