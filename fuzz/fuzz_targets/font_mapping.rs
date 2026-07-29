#![no_main]
//! Fuzz Codec Boundary font-name recovery, deterministic fallback provider
//! selection, and generated-text shaping.

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::fuzz::fuzz_font_mapping;

fuzz_target!(|data: &[u8]| {
    fuzz_font_mapping(data);
});
