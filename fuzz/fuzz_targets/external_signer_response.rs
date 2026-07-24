#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    wellfriendpdf_engine::fuzz::fuzz_external_signer_response(data);
});
