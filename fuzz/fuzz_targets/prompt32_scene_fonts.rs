#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _len = data.len();
    let _prefix = data.first().copied().unwrap_or_default();
});
