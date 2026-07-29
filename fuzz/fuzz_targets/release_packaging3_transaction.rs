#![no_main]

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    apply_reflow_region, undo_reflow_from_replay, validate_reflow_output, GeometricReflowRequest,
    TrueEditingMode,
};

const SOURCE: &[u8] = include_bytes!("../../crates/engine/tests/fixtures/multi_stream.pdf");

fuzz_target!(|data: &[u8]| {
    let suffix = data.iter().take(48).map(|byte| char::from(b'a' + byte % 26)).collect::<String>();
    let request = GeometricReflowRequest {
        requested_mode: TrueEditingMode::GeometricBlock,
        page: 1,
        source_text: "Hello".into(),
        replacement_text: format!("Fuzz{suffix}"),
        region: Some([50.0, 650.0, 300.0, 730.0]),
        allowed_expansion_region: None,
        next_region: None,
        next_column: None,
        downstream_vector_moves: Vec::new(),
        downstream_link_moves: Vec::new(),
        layout_constraints: Vec::new(),
        language: Some("en".into()),
        direction: Some("ltr".into()),
        font_policy: "rebuild_subset_or_generated_type0".into(),
        alignment: "left".into(),
        justify_last_line: false,
        hyphenation: false,
        allow_page_creation: false,
        allow_font_reduction: false,
        approve_low_confidence_structure: false,
        signature_policy_override: false,
        line_height: 14.0,
        max_downstream_blocks: 2,
    };
    if let Ok((output, _)) = apply_reflow_region(SOURCE, &request) {
        let _ = validate_reflow_output(SOURCE, &output, &request);
        let _ = undo_reflow_from_replay(SOURCE, &output, &request);
    }
});
