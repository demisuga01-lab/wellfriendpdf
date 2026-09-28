//! Unexecuted glyph-coordinate and PDF resource regression source.
use super::*;
#[test]
fn vertical_story_emits_measured_multifont_origins_and_never_mirrors_lr_glyphs() {
    let fonts = ["Helvetica", "Times-Roman"]
        .into_iter()
        .map(|name| ApprovedFontAsset {
            lookup_name: name.into(),
            bytes: crate::render::get_fallback_font(name).unwrap().to_vec(),
        })
        .collect::<Vec<_>>();
    let text = "AV e\u{301}\u{00A7}";
    let spans = vec![
        FontSpan {
            range: [0, 3],
            font_index: 0,
        },
        FontSpan {
            range: [3, text.len()],
            font_index: 1,
        },
    ];
    let bidi =
        crate::fonts::shaper::resolve_line_bidi(text, 0..text.len(), ShapeOptions::default())
            .unwrap();
    let metrics = fonts
        .iter()
        .map(|font| crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let mut emitted = Vec::new();
    for mode in [WritingMode::VerticalRl, WritingMode::VerticalLr] {
        let line = StoryPaintLine {
            writing_mode: mode,
            text: text.into(),
            x: 80.0,
            baseline: 220.0,
            width: 180.0,
            font_size: 12.0,
            font_index: 0,
            font_spans: spans.clone(),
            style_spans: vec![],
            tab_segments: vec![],
            tab_decorations: vec![],
            rgb: [0.0; 3],
            rtl: false,
            bidi: Some(bidi.clone()),
            shaping: Default::default(),
            tag_owner: None,
            artifact: false,
        };
        let region = [0.0, 0.0, 200.0, 250.0];
        let runs = shape_line(&line, text, &bidi, &spans, &fonts, &metrics, region).unwrap();
        let mut content = String::new();
        let mut pen = [0.0; 2];
        for (index, glyphs) in &runs {
            append_run(
                &mut content,
                &line,
                glyphs,
                &format!("F{index}"),
                line.font_size,
                &mut pen,
            )
            .unwrap();
        }
        let matrices = content
            .lines()
            .filter(|l| l.contains(" Tm "))
            .map(|l| {
                let mut v = l.split_whitespace();
                std::array::from_fn::<_, 6, _>(|_| v.next().unwrap().parse::<f64>().unwrap())
            })
            .collect::<Vec<_>>();
        for (glyph, m) in runs.iter().flat_map(|(_, g)| g).zip(&matrices) {
            assert!((m[0] * m[3] - m[1] * m[2] - 1.0).abs() < 1e-10);
            if let Some(b) = glyph.bounds {
                for x in [b[0], b[2]] {
                    for y in [b[1], b[3]] {
                        let x = x * line.font_size / 1000.0;
                        let y = y * line.font_size / 1000.0;
                        let (x, y) = crate::content::state::transform_point(m, x, y);
                        assert!(
                            x >= region[0] && x <= region[2] && y >= region[1] && y <= region[3]
                        );
                    }
                }
            }
        }
        assert!(content.contains("/WFTextBasisV1"));
        assert!(content.contains("0 -1 1 0"));
        emitted.push(matrices);
    }
    assert_eq!(emitted[0], emitted[1]);
}
#[test]
fn serialization_rejects_a_column_whose_actual_ink_exceeds_frame() {
    let fonts = vec![ApprovedFontAsset {
        lookup_name: "Helvetica".into(),
        bytes: crate::render::get_fallback_font("Helvetica")
            .unwrap()
            .to_vec(),
    }];
    let text = "A";
    let bidi =
        crate::fonts::shaper::resolve_line_bidi(text, 0..1, ShapeOptions::default()).unwrap();
    let spans = vec![FontSpan {
        range: [0, 1],
        font_index: 0,
    }];
    let metrics = fonts
        .iter()
        .map(|font| crate::fonts::line_layout::PreparedFontMetrics::new(&font.bytes).map(Some))
        .collect::<Result<Vec<_>>>()
        .unwrap();
    let line = StoryPaintLine {
        writing_mode: WritingMode::VerticalRl,
        text: text.into(),
        x: 0.0,
        baseline: 20.0,
        width: 100.0,
        font_size: 12.0,
        font_index: 0,
        font_spans: spans.clone(),
        style_spans: vec![],
        tab_segments: vec![],
        tab_decorations: vec![],
        rgb: [0.0; 3],
        rtl: false,
        bidi: Some(bidi.clone()),
        shaping: Default::default(),
        tag_owner: None,
        artifact: false,
    };
    assert!(shape_line(
        &line,
        text,
        &bidi,
        &spans,
        &fonts,
        &metrics,
        [0.0, 0.0, 100.0, 100.0]
    )
    .is_err());
}
