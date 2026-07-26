# API stability policy

The supported public product surfaces are the `wellfriendpdf-*` Rust packages, the
`wellfriendpdf` CLI, Python `wellfriendpdf`, the `wellfriendpdf.h` C ABI, the
`wellfriendpdf-wasm` package, .NET `WellfriendPdf`, and Java `io.wellfriendpdf`.

Until a 1.0 release, additions are preferred to breaking changes. A breaking public
API, CLI option/exit-code, report-envelope schema, C ABI, or binding change requires
a release-note entry, an inventory diff, and an intentional version decision. Internal
modules and explicitly unsupported capability reports are not stable APIs.
