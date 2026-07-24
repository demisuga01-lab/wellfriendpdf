# Wellfriend PDF SDK rename migration

This repository now uses the public brand **Wellfriend** and the product name
**Wellfriend PDF SDK**. It was formerly developed under the internal name
Oxide.

This is a rename-only migration. It does not intentionally change PDF parsing,
rendering, signing, validation, compression, security, or binding behavior.

## Package and API mapping

| Surface | Old name | New name |
| --- | --- | --- |
| Product brand | Oxide / Oxide PDF | Wellfriend / Wellfriend PDF SDK |
| Technical namespace | `oxide` / `oxide_pdf` | `wellfriendpdf` |
| Rust engine crate | `oxide-engine` | `wellfriendpdf-engine` |
| Rust CLI crate | `oxide-cli` | `wellfriendpdf-cli` |
| Rust C ABI crate | `oxide-capi` | `wellfriendpdf-capi` |
| Rust WASM crate | `oxide-wasm` | `wellfriendpdf-wasm` |
| Python distribution/import | `oxide` / `oxide_pdf` | `wellfriendpdf` |
| CLI binary | `oxide` | `wellfriendpdf` |
| C header | `oxide.h` | `wellfriendpdf.h` |
| C ABI prefix | `oxide_*` | `wellfriendpdf_*` |
| C macros | `OXIDE_*` | `WELLFRIENDPDF_*` |
| WASM package | `oxide-wasm` | `wellfriendpdf-wasm` |
| .NET package/namespace | `Oxide.Sdk` / `Oxide` | `WellfriendPdf` |
| Java group/package | `org.oxidepdf` | `io.wellfriendpdf` |
| Java artifact | `oxide-sdk` | `wellfriendpdf-sdk` |

## Compatibility policy

No backward-compatible public aliases were intentionally kept for the old
Oxide package, CLI, C ABI, Python import, .NET namespace, or Java package names.
This repository was not treated as having a released public compatibility
contract for those names.

## GitHub repository note

The local `origin` remote still points at:

`https://github.com/demisuga01-lab/oxide-parser.git`

The source/package metadata uses Wellfriend PDF SDK naming. Renaming the GitHub
repository itself is an external repository-management action and was not part
of this code-only rename commit.

## Remaining old-name references

Remaining old-name references are limited to:

- historical prompt/audit references;
- old commit messages or branch/remote facts;
- third-party dependency names such as `miniz_oxide`;
- historical test fixture certificate text, for example `Oxide Test Signer`;
- clearance notes documenting names that were rejected and not reused.

No current public package/import/crate/binary/header/API name should remain on
the old Oxide branding.
