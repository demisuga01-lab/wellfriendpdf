#![no_main]

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    analyze_semantic_layout, reading_order_report, GeometricReflowRequest, TrueEditingMode,
};

const SOURCE: &[u8] = include_bytes!("../../crates/engine/tests/fixtures/multi_stream.pdf");

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(2048)];
    let request = GeometricReflowRequest {
        requested_mode: TrueEditingMode::SemanticDocument,
        page: 1,
        source_text: "Hello".into(),
        replacement_text: String::from_utf8_lossy(bounded).chars().take(96).collect(),
        region: Some([50.0, 650.0, 300.0, 730.0]),
        allowed_expansion_region: None,
        next_region: None,
        next_column: None,
        downstream_vector_moves: Vec::new(),
        downstream_link_moves: Vec::new(),
        layout_constraints: Vec::new(),
        language: Some(if bounded.first().is_some_and(|byte| byte & 1 == 0) { "en" } else { "es" }.into()),
        direction: Some(match bounded.get(1).copied().unwrap_or_default() % 3 {
            0 => "ltr", 1 => "rtl", _ => "vertical",
        }.into()),
        font_policy: "rebuild_subset_or_generated_type0".into(),
        alignment: "left".into(),
        justify_last_line: false,
        hyphenation: false,
        allow_page_creation: false,
        allow_font_reduction: false,
        approve_low_confidence_structure: bounded.get(2).is_some_and(|byte| byte & 1 == 0),
        signature_policy_override: false,
        line_height: 14.0,
        max_downstream_blocks: 2,
    };
    let _ = analyze_semantic_layout(SOURCE, Some(&request));
    let _ = reading_order_report(SOURCE);
});
