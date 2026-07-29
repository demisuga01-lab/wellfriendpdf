# Third-Party Licenses & Attribution (NOTICE)

This is the consolidated attribution document for the Wellfriend PDF SDK. It
covers (1) the project's own license, (2) the bundled fonts, and (3) the Rust
crate dependency tree. Generated/verified with `cargo metadata` and the fonts'
embedded license metadata.

## 1. Wellfriend's own license

The Wellfriend PDF SDK (`wellfriendpdf-engine`, `wellfriendpdf-cli`, `wellfriendpdf-server`) is licensed
under **MIT**. See [`LICENSE`](../LICENSE) at the repository root. Each crate
declares `license = "MIT"` in its `Cargo.toml`.

## 2. Bundled fonts (embedded into `wellfriendpdf-engine`)

These font programs are embedded as fallback/substitution fonts (see
[`crates/engine/fonts/README.md`](../crates/engine/fonts/README.md)).

| Font | License | License file |
|---|---|---|
| DejaVu Sans (`DejaVuSans.ttf`) | DejaVu / Bitstream Vera (permissive, MIT-like) + Arev; DejaVu changes public domain | [`crates/engine/fonts/LICENSE-DejaVu.txt`](../crates/engine/fonts/LICENSE-DejaVu.txt) |
| Liberation Sans / Serif / Mono (`Liberation*.ttf`, 12 files) | SIL Open Font License (OFL) 1.1 | [`crates/engine/fonts/LICENSE-Liberation.txt`](../crates/engine/fonts/LICENSE-Liberation.txt) |

Both are permissive (non-copyleft) and compatible with `MIT`.
The full license texts ship in-repo as those licenses require.

## 3. Rust crate dependencies

A `cargo metadata` scan of the **entire resolved dependency tree** (260
third-party crates, excluding Wellfriend's own workspace crates) yields the following
license distribution — **all permissive, no forced copyleft**:

| Count | License (SPDX) |
|---:|---|
| 134 | MIT |
| 47 | Apache-2.0 OR MIT |
| 31 | MIT OR Apache-2.0 |
| 14 | MIT OR Apache-2.0 OR Zlib |
| 6 | Unlicense OR MIT |
| 3 | Apache-2.0 |
| 3 | BSD-3-Clause |
| 3 | MIT OR Zlib |
| 3 | Apache-2.0 WITH LLVM-exception OR MIT |
| 2 | Zlib OR MIT |
| 2 | BSD-2-Clause OR MIT |
| 1 each | 0BSD OR MIT; BSD-2-Clause; CC0-1.0 OR MIT-0 OR Apache-2.0; (MIT) AND BSD-3-Clause; (MIT) AND IJG; MIT AND BSD-3-Clause; MIT OR Zlib OR Apache-2.0; MIT OR LGPL-2.1-or-later; Apache-2.0 OR BSL-1.0; Zlib; (MIT) AND Unicode-3.0 |

### Audit result — no copyleft conflict

- **No GPL/AGPL anywhere.** The only crate whose license string mentions any
  form of GPL is **`r-efi`** (`MIT OR LGPL-2.1-or-later`): a
  tri-licensed UEFI-target crate from which the MIT option can be
  taken (so **no copyleft is forced**), and it is not compiled on the primary
  (Windows/x86-64) target.
- The `IJG` (libjpeg), `Unicode-3.0`, `BSD-*`, and `Zlib` components are all
  permissive and compatible with `MIT`.
- Every crate declares a license (no `(none)` entries in the resolved tree).

**Conclusion:** the dependency tree is fully compatible with the project's
`MIT` license. This is the differentiator vs Poppler's GPLv2 —
Wellfriend can be embedded in proprietary software without copyleft obligations.

## 4. C-toolchain note (pure-Rust status)

The library (`wellfriendpdf-engine`) pulls **no C** at runtime. The `wellfriendpdf-cli` and
`wellfriendpdf-server` binaries pull three C-backed crates (`bzip2-sys`, `lzma-sys`,
`zstd-sys`) **only** via `zip`'s default features — a build-time C dependency
that is a one-line fix (`zip = { default-features = false, features =
["deflate"] }`). No `ring`/`openssl`/`cmake` anywhere. See the positioning doc
§D.4.

## Reproducing this audit

```
cargo metadata --format-version 1   # license field of every resolved package
# fonts: licenses are embedded in each .ttf 'name' table (IDs 0, 13, 14)
```
