#![no_main]
//! Fuzz the Prompt 04 delimiter scanner invariant.
//!
//! Arbitrary bytes may contain marker-like data inside strings, names, comments,
//! inline images, or binary streams. The accelerated path is only a candidate
//! finder, so it must exactly match the scalar candidate set for every input.

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{scan_pdf_markers_accelerated, scan_pdf_markers_scalar};

fuzz_target!(|data: &[u8]| {
    let scalar = scan_pdf_markers_scalar(data).candidates;
    let accelerated = scan_pdf_markers_accelerated(data).candidates;
    assert_eq!(scalar, accelerated);
});
