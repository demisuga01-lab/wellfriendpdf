//! Lightweight source guards for renderer correctness dispositions.
//!
//! These guards deliberately inspect only stable production-path markers. They
//! do not substitute for semantic render tests, but prevent accidental removal
//! of explicit safety classifications during future refactors.

#[test]
fn renderer_safety_dispositions_remain_explicit() {
    let display_list = include_str!("../src/render/display_list.rs");
    assert!(display_list.contains("named shading resource /{name} is missing"));
    assert!(!display_list.contains("missing_named_shading_stays_native_and_replays_canonical_noop"));
    assert!(display_list.contains("document_revision"));
    assert!(display_list.contains("contract_fingerprint"));

    let postscript = include_str!("../src/render/postscript.rs");
    assert!(postscript.contains("\"Do\" | \"sh\" | \"gs\""));

    let progressive = include_str!("../src/render/progressive.rs");
    for state in ["Created", "Paused", "Cancelled", "Closed"] {
        assert!(
            progressive.contains(state),
            "missing lifecycle state {state}"
        );
    }
}
