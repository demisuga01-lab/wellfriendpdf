#![no_main]
//! Fuzz the PDF document parser end-to-end from raw bytes.
//!
//! Feeds arbitrary bytes to `ContentEngine::open_bytes` — the same in-memory
//! entry point real callers use — exercising the xref/trailer/object-stream
//! parsing, object tokenizer, and recursive object parser. Any panic, abort,
//! OOM, or hang on arbitrary input is a bug: a well-behaved parser must return
//! `Err` for every malformed input, never crash.

use libfuzzer_sys::fuzz_target;
use wellfriendpdf_engine::{
    extract_xfa, xfa_inventory, xfa_runtime_report, ContentEngine, XfaLimits, XfaRuntimeOptions,
};

fuzz_target!(|data: &[u8]| {
    // open_bytes takes ownership of a Vec; the result (Ok or Err) is
    // black-boxed so the call isn't optimized away. We only care that it
    // returns rather than panicking/hanging.
    if let Ok(engine) = ContentEngine::open_bytes(data.to_vec()) {
        let limits = XfaLimits {
            max_xml_bytes: 1 << 20,
            max_packet_decoded_bytes: 1 << 20,
            max_xml_nodes: 4_096,
            max_dataset_nodes: 2_048,
            max_generated_nodes: 2_048,
            max_generated_pages: 16,
            max_script_instructions: 512,
            max_runtime_ms: 25,
            ..XfaLimits::default()
        };
        let _ = std::hint::black_box(xfa_inventory(&engine, &limits));
        let _ = std::hint::black_box(extract_xfa(&engine, &limits));
        let _ = std::hint::black_box(xfa_runtime_report(
            &engine,
            &XfaRuntimeOptions {
                limits,
                ..XfaRuntimeOptions::default()
            },
        ));
    }
});
