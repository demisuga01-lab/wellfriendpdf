use std::path::PathBuf;

use oxide_engine::{ContentEngine, TextSearchOptions, TextSemanticOptions};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("corpus")
        .join("pdfs")
        .join("generated")
        .join(name)
}

#[test]
fn semantic_text_model_serializes_words_and_search_quads() {
    let engine = ContentEngine::open_path(fixture("generated_basic_text.pdf")).unwrap();
    let model = engine
        .extract_text_semantic_model(&[1], TextSemanticOptions::default())
        .unwrap();

    assert_eq!(model.pages.len(), 1);
    assert!(
        model.counters.words >= 10,
        "expected words in semantic model"
    );
    assert!(
        model
            .pages
            .iter()
            .flat_map(|page| &page.blocks)
            .flat_map(|block| &block.lines)
            .flat_map(|line| &line.words)
            .any(|word| word.text == "quick"),
        "expected word-level reconstruction"
    );

    let json = serde_json::to_string(&model).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["pages"][0]["page"], 1);

    let matches = engine
        .search_text(
            &[1],
            "quick brown",
            TextSearchOptions {
                case_sensitive: false,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert!(
        !matches[0].quads.is_empty(),
        "search match has source quads"
    );
}
