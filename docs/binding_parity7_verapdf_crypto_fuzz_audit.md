# Crypto Standards Fuzz veraPDF, crypto close-out, release fuzz, and parser campaign audit

Status: in progress until VPS validation, closure commit, and push complete.

Crypto Standards Fuzz combines:

- 105 - veraPDF corpus parity
- 106 - crypto/standards close-out
- 107 - release fuzz/CI architecture
- 108 - long parser fuzz campaign

Starting checkpoint:

- Repository: `E:\wellpdfsdk`
- GitHub remote: `https://github.com/demisuga01-lab/wellfriendpdf.git`
- Starting commit: `5cf02f27bccba321117e8b8d34e52b25c2386352`
- Starting commit message: `Rename Wellfriend to Wellfriend PDF SDK`
- Incremental Signing Standards closure ancestor: `8bc83ef5e0b907c67e9c43c7ce81b5a16b856f0d`
- Branch status at start: `main...origin/main`
- Starting worktree: clean

Scope controls:

- Do not start Fuzz Campaign.
- Do not rename the project again.
- Do not revert the Wellfriend PDF SDK rename.
- Do not reset, stash, clean, or discard user work.
- Do not create a partial closure commit.
- Do not deploy.
- Do not touch VPS production services.
- Use the VPS only as an isolated test runner under `/home/demisuga01/wellpdf`.

Crypto Standards Fuzz source changes are limited to standards close-out status tightening, veraPDF
parity tooling, release fuzz/CI architecture, parser fuzz campaign evidence, docs, and
validation artifacts. Target artifacts are generated under
`target/crypto_standards_fuzz-verapdf-crypto-fuzz/` and remain ignored unless repository policy changes.

Final completion is claimable only when the Crypto Standards Fuzz closure commit exists, is pushed to
`origin/main`, the worktree is clean, and the VPS evidence shows all required gates passed.
