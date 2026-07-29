#![no_main]

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    analyze_semantic_layout, line_break_text, preview_reflow, text_reflow_feature_matrix,
    GeometricReflowRequest, LayoutConstraint,
};

const TEXT_REFLOW_SOURCE_FIXTURE: &[u8] =
    include_bytes!("../../crates/engine/tests/fixtures/multi_stream.pdf");

fuzz_target!(|data: &[u8]| {
    let bounded = &data[..data.len().min(4096)];
    let text = String::from_utf8_lossy(bounded);
    let width = 24.0 + (bounded.len() % 240) as f64;
    let height = 14.0 + (bounded.len() % 160) as f64;
    let _ = line_break_text(&text, width, height, 14.0, Some("und"), None, false);
    let request = GeometricReflowRequest {
        requested_mode: wellfriendpdf_engine::TrueEditingMode::GeometricBlock,
        page: 1,
        source_text: "A".into(),
        replacement_text: text.chars().take(128).collect(),
        region: Some([0.0, 0.0, width, height]),
        allowed_expansion_region: None,
        next_region: None,
        next_column: None,
        downstream_vector_moves: Vec::new(),
        downstream_link_moves: Vec::new(),
        layout_constraints: vec![LayoutConstraint {
            constraint_id: "fuzz-region-height".into(),
            variable: "region_height".into(),
            relation: if bounded.first().is_some_and(|byte| byte & 1 == 0) {
                "ge".into()
            } else {
                "eq".into()
            },
            // This intentionally includes NaN/infinite values generated from
            // fuzz input. The canonical planner must return a typed infeasible
            // report rather than panic or accept invalid geometry.
            value: {
                let mut raw = [0_u8; 8];
                for (index, byte) in bounded.iter().take(8).enumerate() {
                    raw[index] = *byte;
                }
                f64::from_le_bytes(raw)
            },
            priority: if bounded.get(1).is_some_and(|byte| byte & 1 == 0) {
                "required".into()
            } else {
                "weak".into()
            },
        }],
        language: Some(if bounded.get(2).is_some_and(|byte| byte & 1 == 0) {
            "en"
        } else {
            "es"
        }
        .into()),
        direction: bounded.get(3).map(|byte| match byte % 3 {
            0 => "ltr".into(),
            1 => "rtl".into(),
            _ => "vertical".into(),
        }),
        font_policy: "rebuild_subset_or_generated_type0".into(),
        alignment: match bounded.get(4).copied().unwrap_or_default() % 4 {
            0 => "left",
            1 => "right",
            2 => "center",
            _ => "justify",
        }
        .into(),
        justify_last_line: false,
        hyphenation: false,
        allow_page_creation: false,
        allow_font_reduction: false,
        approve_low_confidence_structure: false,
        signature_policy_override: false,
        line_height: 14.0,
        max_downstream_blocks: 2,
    };
    // Exercise the real bounded planner and semantic graph against a legal,
    // repository-owned source. Fuzz data controls only the request, preserving
    // a deterministic parser/writer boundary and avoiding external processes.
    let _ = preview_reflow(TEXT_REFLOW_SOURCE_FIXTURE, &request);
    let _ = analyze_semantic_layout(TEXT_REFLOW_SOURCE_FIXTURE, Some(&request));
    let _ = serde_json::to_string(&request);
    let _ = text_reflow_feature_matrix();
});
