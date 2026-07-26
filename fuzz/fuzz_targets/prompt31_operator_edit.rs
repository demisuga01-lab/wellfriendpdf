#![no_main]

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    edit_text_operator, operator_image_eligibility, operator_text_eligibility,
    operator_text_provenance, OperatorTextEditRequest,
};

fuzz_target!(|data: &[u8]| {
    if data.len() > 64 * 1024 {
        return;
    }
    let mid = data.len() / 2;
    let source: String = String::from_utf8_lossy(&data[..mid])
        .chars()
        .take(32)
        .collect();
    let replacement: String = String::from_utf8_lossy(&data[mid..])
        .chars()
        .take(source.chars().count())
        .collect();
    let request = OperatorTextEditRequest {
        page: 1,
        source_text: source.to_string(),
        replacement_text: replacement,
        signature_policy_override: false,
    };
    let _ = operator_text_eligibility(data, &request);
    let _ = operator_text_provenance(data, 1, &request.source_text, &request.replacement_text);
    let _ = edit_text_operator(data, &request);
    let _ = operator_image_eligibility(data, 1);
});
