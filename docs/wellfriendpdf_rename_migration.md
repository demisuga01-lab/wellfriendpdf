# Wellfriend PDF SDK rename migration

This repository now uses the public brand **Wellfriend** and the product name
**Wellfriend PDF SDK**. It was formerly developed under the internal name
Wellfriend.

This is a rename-only migration. It does not intentionally change PDF parsing,
rendering, signing, validation, compression, security, or binding behavior.

## Package and API mapping

| Surface | Old name | New name |
| --- | --- | --- |
| Product brand | Wellfriend / Wellfriend PDF | Wellfriend / Wellfriend PDF SDK |
| Technical namespace | `wellfriend` / `wellfriend_pdf` | `wellfriendpdf` |
| Rust engine crate | `wellfriendpdf-engine` | `wellfriendpdf-engine` |
| Rust CLI crate | `wellfriendpdf-cli` | `wellfriendpdf-cli` |
| Rust C ABI crate | `wellfriendpdf-capi` | `wellfriendpdf-capi` |
| Rust WASM crate | `wellfriendpdf-wasm` | `wellfriendpdf-wasm` |
| Python distribution/import | `wellfriend` / `wellfriend_pdf` | `wellfriendpdf` |
| CLI binary | `wellfriend` | `wellfriendpdf` |
| C header | `wellfriend.h` | `wellfriendpdf.h` |
| C ABI prefix | `wellfriend_*` | `wellfriendpdf_*` |
| C macros | legacy short product prefix | `WELLFRIENDPDF_*` |
| WASM package | `wellfriendpdf-wasm` | `wellfriendpdf-wasm` |
| .NET package/namespace | `Wellfriend.Sdk` / `Wellfriend` | `WellfriendPdf` |
| Java group/package | `org.wellfriendpdf` | `io.wellfriendpdf` |
| Java artifact | `wellfriend-sdk` | `wellfriendpdf-sdk` |

## Compatibility policy

No backward-compatible public aliases were intentionally kept for the old
Wellfriend package, CLI, C ABI, Python import, .NET namespace, or Java package names.
This repository was not treated as having a released public compatibility
contract for those names.

## GitHub repository note

The local `origin` remote still points at:

`https://github.com/demisuga01-lab/wellfriend-parser.git`

The source/package metadata uses Wellfriend PDF SDK naming. Renaming the GitHub
repository itself is an external repository-management action and was not part
of this code-only rename commit.

## Remaining old-name references

Remaining old-name references are limited to:

- historical roadmap task/audit references;
- old commit messages or branch/remote facts;
- third-party dependency names such as `miniz_wellfriend`;
- historical test fixture certificate text, for example `Wellfriend Test Signer`;
- clearance notes documenting names that were rejected and not reused.

No current public package/import/crate/binary/header/API name should remain on
the old Wellfriend branding.
