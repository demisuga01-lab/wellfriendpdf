# PDF/UA validation

The PDF/UA report checks discoverable identifiers, MarkInfo, StructTreeRoot, logical structure,
role-map posture, document language, title, image/figure alternative-text posture, annotations,
and form accessibility evidence where represented in the document model.

Missing MarkInfo, a missing structure tree, invalid or absent role/structure evidence, missing
language/title, and missing discoverable alt text produce deterministic diagnostics. Reading
order remains a human judgement; the report states that limit explicitly and does not convert it
into a conformant PDF/UA claim.
