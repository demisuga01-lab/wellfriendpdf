# Differential disagreement classification

Differential mismatches are normalized before they can affect the Malformed Coverage release verdict.

Valid classifications include:

- `wellfriend_bug`
- `external_tool_bug_or_limitation`
- `reference_disagreement`
- `malformed_unspecified_behavior`
- `unsupported_exact`
- `standards_policy_difference`
- `expected_strict_mode`
- `expected_repair_mode`
- `needs_manual_review`
- `deferred_release_readiness_benchmark`
- `unclassified`

Malformed Coverage may close with low-risk manual-review rows only when they are not high-severity Wellfriend regressions. It may not close with an unclassified high-severity Wellfriend failure, crash, hang, OOM, sanitizer failure, or security issue.

The Malformed Coverage differential scorecard records attempted file count, disagreement count, and high-severity unclassified count.
