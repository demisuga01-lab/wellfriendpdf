#![no_main]
//! Prompt 11 renderer-oriented routing target.
//!
//! This target complements the narrower parser/codec/font targets by routing
//! structured seeds into renderer-heavy paths: generated valid PDFs, display
//! list capture/replay, font shaping, color/prepress reports, and shading
//! function evaluation.

use libfuzzer_sys::fuzz_target;
use oxide_engine::fuzz::{
    fuzz_color_report, fuzz_font_mapping, fuzz_functions, fuzz_parse_font, fuzz_structured_pdf,
};
use oxide_engine::render::{build_display_list, render_display_list, RenderMode, Viewport};
use oxide_engine::{ContentParser, PageResources};

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let payload = &data[1..];
    match data[0] % 6 {
        0 => fuzz_structured_pdf(payload),
        1 => drive_display_list(payload),
        2 => fuzz_color_report(payload),
        3 => fuzz_functions(payload),
        4 => fuzz_parse_font(payload),
        _ => fuzz_font_mapping(payload),
    }
});

fn drive_display_list(data: &[u8]) {
    let Ok(ops) = ContentParser::parse(data) else {
        return;
    };
    if ops.len() > 512 {
        return;
    }
    let viewport = Viewport::new([0.0, 0.0, 64.0, 64.0], 72);
    let resources = PageResources::default();
    let list = build_display_list(&ops, viewport, &resources);
    std::hint::black_box(&list.stats);
    if list.native_vector_only() {
        let rendered = render_display_list(&list, RenderMode::Compat);
        std::hint::black_box((rendered.width, rendered.height));
    }
}
