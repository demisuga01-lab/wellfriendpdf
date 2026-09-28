//! Unexecuted source regressions for vector resource ownership and inheritance.
use super::*;
use crate::content::ContentParser;
use crate::engine::ContentEngine;
use crate::render::postscript::render_page_ps_strict;
use crate::render::svg::render_page_svg_strict;
use crate::render::vector_fallback::{
    classify_page_for_postscript_output_with_reader, classify_page_for_svg_output_with_reader,
    load_vector_form_program, VectorFallbackDecision,
};

fn stream(entries: &str, content: &str) -> Vec<u8> {
    format!(
        "<< {entries} /Length {} >>\nstream\n{content}\nendstream",
        content.len()
    )
    .into_bytes()
}

fn fixture(
    page: &str,
    outer: &str,
    outer_resources: &str,
    legacy_resources: &str,
) -> ContentEngine {
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Contents 4 0 R /Resources << /Font << /F1 << /Subtype /Type1 /BaseFont /Helvetica >> >> /ColorSpace << /C1 [/Indexed /DeviceRGB 0 <FF0000>] >> /Pattern << /P 9 0 R >> /XObject << /Outer 5 0 R /Next 6 0 R /Shared 7 0 R >> >> >>".to_vec(),
        stream("", page),
        stream(&format!("/Subtype /Form /BBox [0 0 20 20] {outer_resources}"), outer),
        stream(&format!("/Subtype /Form /BBox [0 0 20 20] {legacy_resources}"), "/Shared Do"),
        stream("/Subtype /Form /BBox [0 0 20 20] /Resources <<>>", "1 0 0 rg 0 0 10 10 re f"),
        stream("/Subtype /Form /BBox [0 0 20 20] /Resources <<>>", "0 0 1 rg 0 0 10 10 re f"),
        stream("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << /XObject << /Next 6 0 R /Shared 8 0 R >> >>", "/Next Do"),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    ContentEngine::open_bytes(bytes).unwrap()
}

const OUTER_RESOURCES: &str = "/Resources << /Font << /F1 << /Subtype /Type1 /BaseFont /Courier >> >> /ColorSpace << /C1 [/Indexed /DeviceRGB 0 <0000FF>] >> /XObject << /Next 6 0 R /Shared 8 0 R >> >>";

fn decisions(engine: &ContentEngine) -> [VectorFallbackDecision; 2] {
    let ops = engine.get_page_content(1).unwrap();
    let resources = engine.get_page_resources(1).unwrap();
    let reader = engine.document().reader();
    [
        classify_page_for_svg_output_with_reader(&ops, &resources, 1.0, reader),
        classify_page_for_postscript_output_with_reader(&ops, &resources, 1.0, reader),
    ]
}

fn assert_red_outputs(engine: &ContentEngine) {
    let svg = render_page_svg_strict(engine, 1, 72).unwrap();
    assert!(!svg.is_rasterized);
    assert!(svg.svg.contains("#FF0000"));
    assert!(!svg.svg.contains("#0000FF"));
    let ps = render_page_ps_strict(engine, 1, 72).unwrap();
    assert!(!ps.is_rasterized);
    assert!(ps.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(!ps.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
}

#[test]
fn nested_legacy_forms_use_original_page_in_classifier_and_both_sinks() {
    let engine = fixture("/Outer Do", "/Next Do", OUTER_RESOURCES, "");
    for decision in decisions(&engine) {
        assert!(!matches!(
            decision,
            VectorFallbackDecision::WholePageRaster { .. }
        ));
    }
    assert_red_outputs(&engine);
}

#[test]
fn explicit_empty_and_malformed_scopes_never_borrow_caller_resources() {
    for resources in ["/Resources <<>>", "/Resources 42"] {
        let engine = fixture("/Outer Do", "/Next Do", OUTER_RESOURCES, resources);
        for decision in decisions(&engine) {
            assert!(matches!(
                decision,
                VectorFallbackDecision::WholePageRaster { .. }
            ));
        }
        assert!(render_page_svg_strict(&engine, 1, 72).is_err());
        assert!(render_page_ps_strict(&engine, 1, 72).is_err());
    }
}

#[test]
fn explicit_form_scope_isolates_direct_font_reference_metadata() {
    let engine = fixture("", "", OUTER_RESOURCES, "");
    let mut page = engine.get_page_resources(1).unwrap();
    page.font_references.insert("F1".into(), (42, 0));
    let mut gs = GraphicsState::default();
    gs.text.font_name = "F1".into();
    let program =
        load_vector_form_program(&page, &page, engine.document().reader(), "Outer", &gs).unwrap();
    assert_eq!(
        program.resources.fonts["F1"].get_name("BaseFont"),
        Some("Courier")
    );
    assert!(!program.resources.font_references.contains_key("F1"));
    let selected = &program.inherited_gs.text.font_name;
    assert_ne!(selected, "F1");
    assert_eq!(
        program.resources.fonts[selected].get_name("BaseFont"),
        Some("Helvetica")
    );
    assert_eq!(program.resources.font_references[selected], (42, 0));
}

#[test]
fn inherited_named_color_remains_bound_despite_child_shadowing() {
    let engine = fixture(
        "/C1 cs 0 sc /Outer Do",
        "0 0 10 10 re f",
        OUTER_RESOURCES,
        "",
    );
    assert_red_outputs(&engine);
}

#[test]
fn local_color_selection_and_q_restore_use_distinct_bound_scopes() {
    let engine = fixture(
        "/C1 cs 0 sc /Outer Do",
        "q /C1 cs 0 sc 0 0 5 5 re f Q 10 0 5 5 re f",
        OUTER_RESOURCES,
        "",
    );
    let svg = render_page_svg_strict(&engine, 1, 72).unwrap();
    assert!(svg.svg.contains("#FF0000"));
    assert!(svg.svg.contains("#0000FF"));
    let ps = render_page_ps_strict(&engine, 1, 72).unwrap();
    assert!(ps.body.contains("1.0000 0.0000 0.0000 setrgbcolor"));
    assert!(ps.body.contains("0.0000 0.0000 1.0000 setrgbcolor"));
}

#[test]
fn legacy_form_inside_tiling_pattern_keeps_original_page_fallback() {
    let engine = fixture("/Pattern cs /P scn 0 0 10 10 re f", "", OUTER_RESOURCES, "");
    assert_red_outputs(&engine);
}

#[test]
fn private_font_bindings_cannot_be_selected_by_child_operands_or_gs_names() {
    let engine = fixture("", "", OUTER_RESOURCES, "");
    let source = engine.get_page_resources(1).unwrap();
    let ops = ContentParser::parse(b"/WFInheritedVector0 10 Tf").unwrap();
    let mut target = PageResources::default();
    let mut ext = PdfDictionary::empty();
    ext.insert(
        "Font",
        PdfObject::Array(vec![
            PdfObject::Name("WFInheritedVector1".into()),
            PdfObject::Integer(10),
        ]),
    );
    target.ext_g_states.insert("G".into(), ext);
    let mut gs = GraphicsState::default();
    gs.text.font_name = "F1".into();
    bind_inherited_state(
        &mut target,
        &source,
        &mut gs,
        &ops,
        engine.document().reader(),
    )
    .unwrap();
    assert_ne!(gs.text.font_name, "WFInheritedVector0");
    assert_ne!(gs.text.font_name, "WFInheritedVector1");
    assert!(!target.fonts.contains_key("WFInheritedVector0"));
    assert!(!target.fonts.contains_key("WFInheritedVector1"));
}

#[test]
fn missing_inherited_font_cannot_accidentally_bind_child_font_with_same_name() {
    let engine = fixture("", "", OUTER_RESOURCES, "");
    let source = PageResources::default();
    let mut target = engine.get_page_resources(1).unwrap();
    let mut gs = GraphicsState::default();
    gs.text.font_name = "F1".into();
    bind_inherited_state(
        &mut target,
        &source,
        &mut gs,
        &[],
        engine.document().reader(),
    )
    .unwrap();
    assert!(!target.fonts.contains_key(&gs.text.font_name));
    assert!(target.fonts.contains_key("F1"));
}

#[test]
fn inherited_shading_pattern_retains_its_original_named_color_dependency() {
    let engine = fixture("", "", OUTER_RESOURCES, "");
    let mut source = PageResources::default();
    source
        .color_spaces
        .insert("C".into(), PdfObject::Name("DeviceRGB".into()));
    let mut shading = PdfDictionary::empty();
    shading.insert("ColorSpace", PdfObject::Name("C".into()));
    let mut pattern = PdfDictionary::empty();
    pattern.insert("PatternType", PdfObject::Integer(2));
    pattern.insert("Shading", PdfObject::Dictionary(shading));
    source
        .patterns
        .insert("P".into(), PdfObject::Dictionary(pattern));
    let mut target = PageResources::default();
    target
        .color_spaces
        .insert("C".into(), PdfObject::Name("DeviceGray".into()));
    let mut gs = GraphicsState::default();
    gs.fill_pattern_name = Some("P".into());
    bind_inherited_state(
        &mut target,
        &source,
        &mut gs,
        &[],
        engine.document().reader(),
    )
    .unwrap();
    let pattern = target.patterns[gs.fill_pattern_name.as_ref().unwrap()]
        .as_dict()
        .unwrap();
    assert_eq!(
        pattern
            .get("Shading")
            .unwrap()
            .as_dict()
            .unwrap()
            .get_name("ColorSpace"),
        Some("DeviceRGB")
    );
}

#[test]
fn inherited_cyclic_named_color_dependency_is_not_silently_rebound() {
    let engine = fixture("", "", OUTER_RESOURCES, "");
    let mut source = PageResources::default();
    source
        .color_spaces
        .insert("C".into(), PdfObject::Name("C".into()));
    let mut target = PageResources::default();
    target
        .color_spaces
        .insert("C".into(), PdfObject::Name("DeviceGray".into()));
    let mut gs = GraphicsState::default();
    gs.fill_color.space = ColorSpace::Named("C".into());
    assert!(bind_inherited_state(
        &mut target,
        &source,
        &mut gs,
        &[],
        engine.document().reader()
    )
    .is_err());
}
