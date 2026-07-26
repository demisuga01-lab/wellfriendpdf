# Prompt 29 release verdict

The Prompt 29 final release verdict is generated in `target/prompt29-malformed-differential-coverage/prompt29-final-release-verdict.json`.

The verdict may be `complete` only when all of the following are true:

- start state is the pushed Prompt 28 baseline or a clean descendant;
- heavy testing ran on VPS `35.185.176.47`;
- the 32 GiB Wellfriend budget was respected;
- malformed corpus and differential runs completed with zero unclassified crash, hang, OOM, sanitizer, or high-severity Wellfriend regression;
- crash minimization and bug triage artifacts exist;
- coverage and sanitizer reports exist;
- full workspace gates passed;
- binding/package regression gates passed;
- historical impact and secret-scan artifacts passed or were honestly classified;
- no deployment or VPS production-service action occurred.

The final JSON verdict must be exactly `complete` or `not_complete`. Chat summaries must not overrule the machine verdict.
