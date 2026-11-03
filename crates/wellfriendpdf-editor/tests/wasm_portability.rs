//! QF-2564: the browser editor runs the engine on `wasm32-unknown-unknown`,
//! where `std::time::Instant::now()` and `SystemTime::now()` panic. The
//! modules an editor operation runs through must use
//! `wellfriendpdf_engine::portable_time` instead.

const EDITING_PATH: &[(&str, &str)] = &[
    ("advanced_editing.rs", include_str!("../../engine/src/advanced_editing.rs")),
    ("source_editing.rs", include_str!("../../engine/src/source_editing.rs")),
    ("universal_editing.rs", include_str!("../../engine/src/universal_editing.rs")),
    ("text_reflow.rs", include_str!("../../engine/src/text_reflow.rs")),
    ("text_editor.rs", include_str!("../../engine/src/text_editor.rs")),
    ("editing.rs", include_str!("../../engine/src/editing.rs")),
    ("editing_transactions.rs", include_str!("../../engine/src/editing_transactions.rs")),
    ("authoring.rs", include_str!("../../engine/src/authoring.rs")),
    ("input_identity.rs", include_str!("../../engine/src/input_identity.rs")),
];

#[test]
fn editing_path_modules_use_the_portable_clock() {
    for (name, source) in EDITING_PATH {
        for (number, line) in source.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            let direct = code.contains("std::time::Instant")
                || (code.contains("use std::time::") && code.contains("Instant"))
                || code.contains("SystemTime::now");
            assert!(!direct, "{name}:{}: {line}", number + 1);
        }
    }
}
