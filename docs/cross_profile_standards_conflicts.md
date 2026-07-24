# Cross-profile standards conflicts

`validate_all_standards` runs PDF/A, PDF/UA, and PDF/X then computes stable conflict records.
Examples include encrypted documents that violate archival/print expectations, PDF/A and PDF/X
output-intent posture conflicts, accessible structure with invalid archival metadata, and
inconsistent profile identifiers.

Each conflict names the involved profiles and rule IDs, carries severity/evidence, and affects
the aggregate verdict. The result is deterministic and intentionally does not infer a successful
profile from a different profile's pass.
