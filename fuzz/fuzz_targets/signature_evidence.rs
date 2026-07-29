#![no_main]
//! Fuzz Signature Validation evidence bundles and retrieval-policy parsing without I/O.

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::fuzz::fuzz_signature_evidence;

fuzz_target!(|data: &[u8]| {
    fuzz_signature_evidence(data);
});
