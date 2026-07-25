# Prompt 27 release verdict

Prompt 27 final release status is written to:

- `target/prompt27-verapdf-crypto-fuzz/prompt27-final-release-verdict.json`

The only closure-pass state is `complete`.

Prompt 27 remains `not_complete` until all of these are true:

- veraPDF tool and corpus are available on the VPS.
- veraPDF parity has zero unclassified mismatches for the supported scope.
- PDF/A-4 status is implemented or exact unsupported with evidence.
- Crypto/standards close-out has zero unclassified security failures.
- Release fuzz inventory, CI policy, runner smoke, crash triage, and seed promotion policy
  exist and are reproducible.
- Parser fuzz targets build and smoke-run.
- The long parser campaign satisfies the Prompt 27 duration policy.
- No unclassified crash, hang, timeout, or OOM remains.
- Full workspace, binding/package, security, performance/memory, and historical gates pass
  on the VPS.
- The closure commit exists with the required message and is pushed to `origin/main`.
- Worktree is clean.
- No deployment or VPS production action occurred.
