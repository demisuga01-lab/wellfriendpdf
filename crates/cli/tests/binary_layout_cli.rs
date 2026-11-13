//! The `wellfriendpdf` executable's own memory cost (round 16): on Linux with
//! glibc it is linked with packed relative relocations (DT_RELR), so the
//! dynamic loader does not read megabytes of RELA entries on every start.

#[cfg(all(target_os = "linux", target_env = "gnu", target_pointer_width = "64"))]
#[test]
fn executable_uses_packed_relative_relocations() {
    if std::env::var_os("WELLFRIENDPDF_NO_PACKED_RELOCS")
        .is_some_and(|value| !value.is_empty() && value != "0")
    {
        return;
    }
    let bytes = std::fs::read(env!("CARGO_BIN_EXE_wellfriendpdf")).expect("read the executable");
    let sections = elf64_sections(&bytes);
    let size_of = |name: &str| {
        sections
            .iter()
            .find(|(section, _)| section == name)
            .map(|(_, size)| *size)
    };
    assert!(
        size_of(".relr.dyn").is_some(),
        "no .relr.dyn section: {sections:?}"
    );
    // Without RELR the RELA table holds every relative relocation (3.8 MB
    // for the release build); packed, only symbolic relocations remain.
    let rela = size_of(".rela.dyn").unwrap_or(0);
    assert!(rela < 256 * 1024, ".rela.dyn is {rela} bytes");
}

/// Names and sizes of an ELF64 little-endian file's sections.
#[cfg(all(target_os = "linux", target_env = "gnu", target_pointer_width = "64"))]
fn elf64_sections(bytes: &[u8]) -> Vec<(String, u64)> {
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let u64_at = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    assert_eq!(&bytes[..4], b"\x7fELF");
    assert_eq!(bytes[4], 2, "ELF64");
    assert_eq!(bytes[5], 1, "little-endian");
    let shoff = u64_at(0x28) as usize;
    let shentsize = u16_at(0x3a) as usize;
    let shnum = u16_at(0x3c) as usize;
    let shstrndx = u16_at(0x3e) as usize;
    let header = |index: usize| shoff + index * shentsize;
    let names_offset = u64_at(header(shstrndx) + 0x18) as usize;
    (0..shnum)
        .map(|index| {
            let at = header(index);
            let name_start = names_offset + u32_at(at) as usize;
            let name_end = name_start
                + bytes[name_start..]
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap();
            (
                String::from_utf8_lossy(&bytes[name_start..name_end]).into_owned(),
                u64_at(at + 0x20),
            )
        })
        .collect()
}
