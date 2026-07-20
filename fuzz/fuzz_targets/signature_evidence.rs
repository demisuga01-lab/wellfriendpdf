#![no_main]
//! Fuzz Prompt 24 evidence bundles and retrieval-policy parsing without I/O.

use libfuzzer_sys::fuzz_target;
use oxide_engine::fuzz::fuzz_signature_evidence;

fuzz_target!(|data: &[u8]| {
    fuzz_signature_evidence(data);
});
