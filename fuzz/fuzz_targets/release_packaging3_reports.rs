#![no_main]

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    text_reflow_feature_matrix, sdk::text_reflow_confidence_report_json, GeometricReflowRequest,
};

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(4096)];
    let _ = serde_json::from_slice::<GeometricReflowRequest>(bounded);
    let _ = serde_json::from_slice::<serde_json::Value>(bounded);
    let _ = serde_json::to_vec(&text_reflow_feature_matrix());
    if let Ok(request_json) = std::str::from_utf8(bounded) {
        let _ = text_reflow_confidence_report_json(&[], request_json, None);
    }
});
