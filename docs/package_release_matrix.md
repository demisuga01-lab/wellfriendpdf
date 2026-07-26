# Package release matrix

| Surface | Package / artifact | Release evidence |
| --- | --- | --- |
| Rust | `wellfriendpdf-engine`, `wellfriendpdf-cli` | metadata, package dry-run, workspace tests |
| Python | `wellfriendpdf` wheel | fresh VPS build, clean install, tests |
| C ABI | `wellfriendpdf.h`, `wellfriendpdf_capi` | header parity, ownership/runtime tests |
| WASM | `wellfriendpdf-wasm` | wasm target and wasm-pack smoke |
| .NET | `WellfriendPdf` | test, pack, native-load smoke |
| Java | `io.wellfriendpdf:wellfriendpdf-sdk` | Maven and Gradle test/package |

No command in this matrix publishes to a registry. Registry release requires a
separate authorized release operation after the final gate is reviewed.
