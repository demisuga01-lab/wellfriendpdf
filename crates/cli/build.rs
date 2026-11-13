//! Link settings for the `wellfriendpdf` executable.
//!
//! On Linux with glibc the executable is linked with packed relative
//! relocations (`-z pack-relative-relocs`, DT_RELR). The binary has about
//! 160,000 relative relocations (vtables, panic locations, static tables);
//! as plain RELA entries they take 3.8 MB that the dynamic loader reads on
//! every start, which is 3.8 MB of resident memory for every command. Packed,
//! they take a few kilobytes. The output of every command is unchanged.
//!
//! DT_RELR needs glibc 2.36 or newer at run time and a linker that supports
//! the option (LLD, which Rust uses by default on x86_64 Linux, or GNU ld
//! 2.38+). Set `WELLFRIENDPDF_NO_PACKED_RELOCS=1` to build without it, for
//! example to run the binary on an older glibc.
//!
//! On x86_64 Linux the code the CLI runs for common commands is also placed
//! together (`link/hot-functions.txt`, an LLD `--symbol-ordering-file`).
//! The kernel maps an executable's code 64 KiB around each page it first
//! runs (fault-around), and the compiler scatters a render's 1,200 or so
//! code pages over the 53 MB of code, so a one-page render kept about 14 MB
//! of code resident. Grouped, the same render keeps a fraction of that. The
//! list names functions as the current toolchain and `Cargo.lock` mangle
//! them; names the linker does not find are ignored, so a stale list only
//! loses the saving. `scripts/hot-functions/generate.sh` regenerates it.
//! Set `WELLFRIENDPDF_NO_HOT_ORDER=1` to link without it.
fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WELLFRIENDPDF_NO_PACKED_RELOCS");
    println!("cargo:rerun-if-env-changed=WELLFRIENDPDF_NO_HOT_ORDER");
    println!("cargo:rerun-if-changed=link/hot-functions.txt");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let opted_out =
        |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0");
    if target_os == "linux" && target_env == "gnu" && !opted_out("WELLFRIENDPDF_NO_PACKED_RELOCS") {
        println!("cargo:rustc-link-arg-bins=-Wl,-z,pack-relative-relocs");
    }
    // Rust links x86_64 Linux GNU binaries with LLD, which takes the order.
    let order = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("link")
        .join("hot-functions.txt");
    if target_os == "linux"
        && target_env == "gnu"
        && target_arch == "x86_64"
        && order.is_file()
        && !opted_out("WELLFRIENDPDF_NO_HOT_ORDER")
    {
        println!(
            "cargo:rustc-link-arg-bins=-Wl,--symbol-ordering-file={}",
            order.display()
        );
        println!("cargo:rustc-link-arg-bins=-Wl,--no-warn-symbol-ordering");
    }
}
