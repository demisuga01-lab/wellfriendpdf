# Prompt 27 veraPDF, crypto close-out, release fuzz, and parser campaign audit

Status: in progress until VPS validation, closure commit, and push complete.

Prompt 27 combines:

- 105 - veraPDF corpus parity
- 106 - crypto/standards close-out
- 107 - release fuzz/CI architecture
- 108 - long parser fuzz campaign

Starting checkpoint:

- Repository: `E:\wellpdfsdk`
- GitHub remote: `https://github.com/demisuga01-lab/wellfriendpdf.git`
- Starting commit: `5cf02f27bccba321117e8b8d34e52b25c2386352`
- Starting commit message: `Rename Oxide to Wellfriend PDF SDK`
- Prompt 26 closure ancestor: `8bc83ef5e0b907c67e9c43c7ce81b5a16b856f0d`
- Branch status at start: `main...origin/main`
- Starting worktree: clean

Scope controls:

- Do not start Prompt 28.
- Do not rename the project again.
- Do not revert the Wellfriend PDF SDK rename.
- Do not reset, stash, clean, or discard user work.
- Do not create a partial closure commit.
- Do not deploy.
- Do not touch VPS production services.
- Use the VPS only as an isolated test runner under `/home/demisuga01/wellpdf`.

Prompt 27 source changes are limited to standards close-out status tightening, veraPDF
parity tooling, release fuzz/CI architecture, parser fuzz campaign evidence, docs, and
validation artifacts. Target artifacts are generated under
`target/prompt27-verapdf-crypto-fuzz/` and remain ignored unless repository policy changes.

Final completion is claimable only when the Prompt 27 closure commit exists, is pushed to
`origin/main`, the worktree is clean, and the VPS evidence shows all required gates passed.
