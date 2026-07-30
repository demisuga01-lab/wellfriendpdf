# Release-candidate plan

This final pre-release pass uses the newly closed Standard/Research runtime architecture as the starting point.

Scope executed in this pass:

- verify the optimization commit is clean, pushed, and synchronized;
- archive local and remote branch tips before cleanup;
- build a legal compact release-candidate corpus from repository-owned/generated fixtures;
- run same-host Standard-mode measurements for Wellfriend, Poppler, qpdf, and available local tool inventory;
- update README claims from `benchmarks/results/release-candidate/summary.json`;
- document unavailable/documentation-only comparators without treating them as losses;
- preserve raw logs and corpus PDFs outside Git;
- commit only compact summaries, docs, workflow/script checks, and release-candidate evidence;
- after the final source commit is pushed, create an external Git bundle and delete non-main branches.

Boundary:

The large public-corpus target was not reached in the available session. The resulting posture is therefore `ready_for_owner_release_with_documented_limits`, not an unrestricted public performance claim.
