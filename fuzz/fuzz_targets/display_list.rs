#![no_main]
//! Fuzz the bounded display-list capture and native-vector replay path.
//!
//! Arbitrary bytes are parsed as a PDF content stream, captured into a tiny
//! display list, and replayed only when the list is natively vector-compatible.
//! Compatibility runs can carry text/images/forms through the existing renderer
//! in normal code, but this target deliberately avoids invoking that broader
//! semantic path on arbitrary bytes.

use libfuzzer_sys::fuzz_target;
use oxide_engine::render::{build_display_list, render_display_list, RenderMode, Viewport};
use oxide_engine::{ContentParser, PageResources};

fuzz_target!(|data: &[u8]| {
    let Ok(ops) = ContentParser::parse(data) else {
        return;
    };
    if ops.len() > 256 {
        return;
    }

    let viewport = Viewport::new([0.0, 0.0, 32.0, 32.0], 72);
    let resources = PageResources::default();
    let list = build_display_list(&ops, viewport, &resources);
    std::hint::black_box(&list.stats);

    if list.native_vector_only() {
        let buffer = render_display_list(&list, RenderMode::Compat);
        std::hint::black_box((buffer.width, buffer.height));
    }
});
