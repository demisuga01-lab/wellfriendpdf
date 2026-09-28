//! Transparency overrides transcribed from the pinned Rustybuzz 0.20.1
//! ot_shaper_arabic_table.rs (MIT; see RUSTYBUZZ_JOINING_LICENSE.txt).
//! Upstream file SHA256: 5b5e3c33bd8743caadc5b3cd72f7608982ce071277569da3322118f5f5001a02
//! X entries use the same unicode-properties 0.1.4 general-category fallback
//! as Rustybuzz; explicit T is true and all other explicit joining types false.
//! Keep both dependency pins and this table synchronized.
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

pub(super) fn transparent(c: char) -> bool {
    let code = c as u32;
    let at = OVERRIDES.partition_point(|&(start, _, _)| start <= code);
    if at != 0 {
        let (_, end, transparent) = OVERRIDES[at - 1];
        if code <= end {
            return transparent;
        }
    }
    matches!(
        c.general_category(),
        GeneralCategory::NonspacingMark | GeneralCategory::EnclosingMark | GeneralCategory::Format
    )
}

#[rustfmt::skip]
const OVERRIDES: &[(u32, u32, bool)] = &[
    (0x600, 0x605, false),
    (0x608, 0x608, false),
    (0x60B, 0x60B, false),
    (0x620, 0x64A, false),
    (0x66E, 0x66F, false),
    (0x671, 0x6D3, false),
    (0x6D5, 0x6D5, false),
    (0x6DD, 0x6DD, false),
    (0x6EE, 0x6EF, false),
    (0x6FA, 0x6FC, false),
    (0x6FF, 0x6FF, false),
    (0x70F, 0x70F, true),
    (0x710, 0x710, false),
    (0x712, 0x72F, false),
    (0x74D, 0x77F, false),
    (0x7CA, 0x7EA, false),
    (0x7FA, 0x7FA, false),
    (0x840, 0x858, false),
    (0x860, 0x86A, false),
    (0x870, 0x88E, false),
    (0x890, 0x891, false),
    (0x8A0, 0x8C8, false),
    (0x8E2, 0x8E2, false),
    (0x1806, 0x1807, false),
    (0x180A, 0x180A, false),
    (0x180E, 0x180E, false),
    (0x1820, 0x1878, false),
    (0x1880, 0x1884, false),
    (0x1885, 0x1886, true),
    (0x1887, 0x18A8, false),
    (0x18AA, 0x18AA, false),
    (0x200C, 0x200D, false),
    (0x202F, 0x202F, false),
    (0x2066, 0x2069, false),
    (0xA840, 0xA873, false),
    (0x10AC0, 0x10AE4, false),
    (0x10AEB, 0x10AEF, false),
    (0x10B80, 0x10B91, false),
    (0x10BA9, 0x10BAF, false),
    (0x10D00, 0x10D23, false),
    (0x10EC2, 0x10EC4, false),
    (0x10F30, 0x10F45, false),
    (0x10F51, 0x10F54, false),
    (0x10F70, 0x10F81, false),
    (0x10FB0, 0x10FCB, false),
    (0x110BD, 0x110BD, false),
    (0x110CD, 0x110CD, false),
    (0x1E900, 0x1E943, false),
    (0x1E94B, 0x1E94B, true),
];
