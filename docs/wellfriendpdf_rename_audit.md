# Wellfriend PDF SDK rename audit

Status: rename in progress after preliminary clearance.

Starting baseline:

- Commit: `8bc83ef5e0b907c67e9c43c7ce81b5a16b856f0d`
- Message: `Close combined prompt 26 incremental signing pdfa pdfua pdfx validation`
- Branch: `main`
- Remote: `https://github.com/demisuga01-lab/oxide-parser.git`
- Starting worktree: clean

Scope controls:

- Work only in `E:\wellpdfsdk`.
- Do not touch the Wellfriend website repo.
- Do not touch the separate compression-engine repo.
- Do not touch any GitHub repository other than the current `wellfriendpdf-parser` remote.
- Do not start Prompt 27.
- Do not implement PDF features.
- Do not deploy or touch VPS production services.

Rename target:

- Public brand: Wellfriend
- Public product name: Wellfriend PDF SDK
- Technical namespace: `wellfriendpdf`
- .NET namespace/package casing: `WellfriendPdf`
- Java package namespace: `io.wellfriendpdf`

Owner note:

`wellfriend.dev` is treated as first-party based on the user's explicit statement that it is their own site and this SDK is part of that open-source website project.

GitHub repository note:

The code/package rename does not rename the GitHub repository object. The local `origin` remote remains `https://github.com/demisuga01-lab/oxide-parser.git` for this commit and push.
