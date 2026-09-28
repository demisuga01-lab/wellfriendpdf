//! Source regressions only: these have not been compiled or executed.
use super::*;
use crate::editing_transactions::ApprovedFontAsset;
use std::collections::BTreeSet;

fn subset(text: &str) -> Vec<u8> {
    let bytes = get_fallback_font("Symbol").unwrap();
    let shaped = TextShaper::shape(bytes, text, ShapeOptions::default()).unwrap();
    let gids = shaped
        .glyphs
        .iter()
        .map(|g| g.glyph_id)
        .collect::<BTreeSet<_>>();
    subset_glyf_preserving_gids(bytes, &gids).unwrap().bytes
}

fn fixture() -> (PdfBuilder, FontFace, FontFace, FontFace) {
    let mut builder = PdfBuilder::new();
    let latin = builder
        .register_font_bytes("LatinSubset", subset("ABC *"))
        .unwrap();
    let hebrew = builder
        .register_font_bytes("HebrewSubset", subset("אבג "))
        .unwrap();
    let stack = builder.register_font_stack(&[latin, hebrew]).unwrap();
    (builder, stack, latin, hebrew)
}

fn logical_commands(page: &PdfPageBuilder) -> String {
    page.commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::TextGroup { logical_text, .. } => Some(logical_text.as_str()),
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::LogicalBreak { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn reopened_text(builder: &PdfBuilder) -> String {
    let engine = crate::ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    (1..=builder.pages.len())
        .map(|page| {
            engine
                .collect_page_text_chunks(page)
                .unwrap()
                .into_iter()
                .map(|chunk| chunk.text)
                .collect::<String>()
        })
        .collect()
}

#[test]
fn mixed_subsets_share_measurement_exact_glyphs_and_logical_line_ownership() {
    let (mut builder, stack, latin, hebrew) = fixture();
    let style = TextStyle::new(stack, 14.0);
    let text = "ABC אבג ABC";
    let page = builder.add_page(PageSize::LETTER);
    let preview = page.preview_font_stack(text, 400.0, &style).unwrap();
    assert_eq!(page.commands.len(), 0);
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].width, page.text_width(text, &style).unwrap());
    assert!(preview[0]
        .runs
        .iter()
        .any(|r| r.font == latin && !r.fallback));
    assert!(preview[0]
        .runs
        .iter()
        .any(|r| r.font == hebrew && r.fallback && r.right_to_left));
    assert!(preview[0].runs.iter().all(|r| r.font_sha256.len() == 64));
    page.draw_text(text, 30.0, 700.0, &style).unwrap();
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    let PageCommand::TextGroup { logical_text, runs } = &builder.pages[0].commands[0] else {
        panic!()
    };
    assert_eq!(logical_text, text);
    let mut pen = 30.0;
    for command in runs {
        let PageCommand::Text {
            text,
            style,
            x,
            bidi,
            font_asset: Some(asset),
            shaped: Some(shaped),
            ..
        } = command
        else {
            panic!()
        };
        assert!((*x - pen).abs() < 1e-9);
        let FontFace::Custom(id) = style.font else {
            panic!()
        };
        assert!(Arc::ptr_eq(
            asset,
            &builder.custom_fonts[id.0 as usize].bytes
        ));
        let emitted = plan
            .shaped_run_resolved(style.font, text, bidi.as_ref())
            .unwrap();
        assert_eq!(emitted.glyphs.len(), shaped.glyphs.len());
        for (a, b) in emitted.glyphs.iter().zip(&shaped.glyphs) {
            assert_eq!(
                (a.advance, a.offset_x, a.offset_y),
                (b.advance, b.offset_x, b.offset_y)
            );
        }
        pen += shaped.glyphs.iter().map(|g| g.advance).sum::<f64>() * style.size / 1000.0;
    }
    assert_eq!(reopened_text(&builder), text);
}

#[test]
fn wrapped_fallback_preserves_source_ranges_bidi_context_and_hard_breaks() {
    let mut builder = PdfBuilder::new();
    let full = builder
        .register_font_bytes("Context", get_fallback_font("Symbol").unwrap())
        .unwrap();
    let stack = builder.register_font_stack(&[full]).unwrap();
    let style = TextStyle::new(stack, 12.0);
    let text = "ABC \u{2067}אבג 123 ABC\u{2069} ABC\r\n\r\nאבג ABC";
    let page = builder.add_page(PageSize::LETTER);
    let preview = page.preview_font_stack(text, 65.0, &style).unwrap();
    assert!(preview.len() > 2);
    let paragraph = PreparedParagraph::new(text, ShapeOptions::default()).unwrap();
    let lines = layout::prepare(page, text, 65.0, &style).unwrap();
    let mut end = 0;
    for (line, disclosed) in lines.iter().zip(&preview) {
        assert_eq!(disclosed.logical_utf8_range[0], end);
        end = disclosed.logical_utf8_range[1];
        assert_eq!(
            disclosed.logical_text,
            text[disclosed.logical_utf8_range[0]..end]
        );
        assert!(disclosed.width <= 65.0 + 1e-7);
        let plan = line.fallback.as_ref().unwrap();
        let visible_end = plan.range.start + line.visual.len();
        let bidi = paragraph.bidi.line(plan.range.start..visible_end).unwrap();
        for run in &plan.runs {
            let local = run.range.start - plan.range.start..run.range.end - plan.range.start;
            let expected = bidi.slice(&line.visual, local, run.bidi.rtl).unwrap();
            assert_eq!(run.bidi, expected);
            let shaped = TextShaper::shape_resolved(
                page.font_program(run.font).unwrap().unwrap(),
                &run.text,
                &expected,
                &Default::default(),
            )
            .unwrap();
            assert_eq!(run.shaped.as_ref(), &shaped);
        }
    }
    assert_eq!(end, text.len());
    page.draw_paragraph(text, 10.0, 700.0, 65.0, &style, &ParagraphStyle::default())
        .unwrap();
    assert_eq!(logical_commands(page), text);
    assert_eq!(reopened_text(&builder), text);
}

#[test]
fn standard_fonts_require_explicit_embedded_equivalence_and_nested_stacks_reuse_it() {
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::LETTER);
    let standard = FontFace::Standard(StandardFont::Helvetica);
    let stack = builder.register_font_stack(&[standard, standard]).unwrap();
    let members = builder.font_stack_members(stack).unwrap().to_vec();
    assert_eq!(members.len(), 1);
    assert!(matches!(members[0], FontFace::Custom(_)));
    assert_eq!(
        builder.register_font_stack(&[stack, standard]).unwrap(),
        stack
    );
    assert_eq!(builder.custom_fonts.len(), 1);
    assert_eq!(builder.font_stacks.len(), 1);
    assert!(Arc::ptr_eq(
        &builder.pages[0].font_stacks,
        &builder.font_stacks
    ));
    assert!(Arc::ptr_eq(
        &builder.pages[0].custom_fonts,
        &builder.custom_fonts
    ));
    assert_eq!(TextStyle::default().font, standard);
}

#[test]
fn invalid_or_cancelled_registration_changes_no_registry() {
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::LETTER);
    let fonts = Arc::clone(&builder.custom_fonts);
    let stacks = Arc::clone(&builder.font_stacks);
    assert!(builder
        .register_font_stack(&[FontFace::default(), FontFace::Custom(CustomFontId(999))])
        .is_err());
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(token
        .scope(|| builder.register_font_stack(&[FontFace::default()]))
        .is_err());
    assert!(Arc::ptr_eq(&fonts, &builder.custom_fonts));
    assert!(Arc::ptr_eq(&stacks, &builder.font_stacks));
    assert!(Arc::ptr_eq(&fonts, &builder.pages[0].custom_fonts));
    assert!(Arc::ptr_eq(&stacks, &builder.pages[0].font_stacks));
}

#[test]
fn unknown_stack_and_uncovered_contextual_word_do_not_append_partial_commands() {
    let (mut builder, stack, _, _) = fixture();
    let style = TextStyle::new(stack, 12.0);
    let mut standalone = PdfPageBuilder::new(PageSize::LETTER);
    assert!(standalone.draw_text("ABC", 1.0, 2.0, &style).is_err());
    assert!(standalone.commands.is_empty());
    let page = builder.add_page(PageSize::LETTER);
    page.draw_text("ABC", 1.0, 2.0, &style).unwrap();
    assert!(page
        .draw_paragraph(
            "ABC\nDEF",
            1.0,
            700.0,
            200.0,
            &style,
            &ParagraphStyle::default()
        )
        .is_err());
    assert_eq!(page.commands.len(), 1);
    assert_eq!(logical_commands(page), "ABC");
    assert!(page
        .draw_paragraph("ABC", 1.0, 700.0, 0.001, &style, &ParagraphStyle::default())
        .is_err());
    assert_eq!(page.commands.len(), 1);
}

#[test]
fn one_joining_unit_is_never_assembled_from_complementary_character_fonts() {
    let mut builder = PdfBuilder::new();
    let a = builder.register_font_bytes("A", subset("A ")).unwrap();
    let b = builder.register_font_bytes("B", subset("B ")).unwrap();
    let font = builder.register_font_stack(&[a, b]).unwrap();
    let page = builder.add_page(PageSize::LETTER);
    assert!(page
        .draw_text("AB", 10.0, 10.0, &TextStyle::new(font, 12.0))
        .is_err());
    assert!(page.commands.is_empty());
}

#[test]
fn captured_fallback_assets_cannot_be_rebound_by_transferring_a_page() {
    let (mut source, stack, _, _) = fixture();
    source
        .add_page(PageSize::LETTER)
        .draw_text("ABC אבג", 10.0, 700.0, &TextStyle::new(stack, 12.0))
        .unwrap();
    let mut destination = PdfBuilder::new();
    destination.pages.push(source.pages[0].clone());
    destination
        .register_font_bytes("OtherA", get_fallback_font("Courier").unwrap())
        .unwrap();
    destination
        .register_font_bytes("OtherB", get_fallback_font("Symbol").unwrap())
        .unwrap();
    assert!(FontBuildPlan::from_builder(&destination).is_err());
    FontBuildPlan::from_builder(&source).unwrap();
}

#[test]
fn late_registration_and_builder_clones_keep_stack_snapshots_and_shared_assets() {
    let (mut builder, stack, _, _) = fixture();
    builder
        .add_page(PageSize::LETTER)
        .draw_text("ABC אבג", 10.0, 700.0, &TextStyle::new(stack, 12.0))
        .unwrap();
    let mut copy = builder.clone();
    let extra = copy
        .register_font_bytes("Another", get_fallback_font("Courier").unwrap())
        .unwrap();
    copy.register_font_stack(&[stack, extra]).unwrap();
    assert_eq!(builder.font_stacks.len(), 1);
    assert_eq!(copy.font_stacks.len(), 2);
    assert!(Arc::ptr_eq(
        &builder.custom_fonts[0].bytes,
        &copy.custom_fonts[0].bytes
    ));
    assert!(Arc::ptr_eq(&copy.pages[0].font_stacks, &copy.font_stacks));
    FontBuildPlan::from_builder(&builder).unwrap();
    FontBuildPlan::from_builder(&copy).unwrap();
}

#[test]
fn flow_and_table_cells_use_the_same_fallback_and_keep_transactional_failures() {
    let (mut builder, stack, _, _) = fixture();
    let style = TextStyle::new(stack, 12.0);
    let mut flow = FlowDocument::new(PageSize::custom(120.0, 80.0), Margins::all(10.0));
    builder.add_page_with_margins(flow.page_size, flow.margins);
    flow.builder = builder;
    let text = "ABC אבג\nABC אבג\nABC אבג\nABC אבג\nABC אבג";
    flow.add_paragraph(text, &style, &ParagraphStyle::default())
        .unwrap();
    assert!(flow.builder.pages.len() > 1);
    assert_eq!(reopened_text(&flow.builder), text);
    let counts = flow
        .builder
        .pages
        .iter()
        .map(|p| p.commands.len())
        .collect::<Vec<_>>();
    let cursor = flow.cursor_y;
    assert!(flow
        .add_list(
            ["ABC\nABC\nABC\nABC", "DEF"],
            false,
            &style,
            &ParagraphStyle::default()
        )
        .is_err());
    assert_eq!(
        flow.builder
            .pages
            .iter()
            .map(|p| p.commands.len())
            .collect::<Vec<_>>(),
        counts
    );
    assert_eq!(flow.cursor_y, cursor);

    let (mut builder, stack, _, _) = fixture();
    let mut table =
        TableBuilder::new(vec![TableColumn::new(120.0)]).body_style(TextStyle::new(stack, 12.0));
    table.add_row(["ABC אבג\r\nABC"]);
    table
        .draw_on_page(builder.add_page(PageSize::LETTER), 10.0, 700.0)
        .unwrap();
    assert_eq!(reopened_text(&builder), "ABC אבג\r\nABC");
}

#[test]
fn control_only_lines_report_private_carriers_not_stack_paint_fonts() {
    let (mut builder, stack, _, _) = fixture();
    let style = TextStyle::new(stack, 12.0);
    let text = "\u{200d}\n\u{e0100}\r\n\u{2067}\u{2069}";
    let page = builder.add_page(PageSize::LETTER);
    let preview = page.preview_font_stack(text, 200.0, &style).unwrap();
    assert!(preview
        .iter()
        .all(|l| l.logical_carrier && l.width == 0.0 && l.runs.is_empty()));
    page.draw_paragraph(text, 10.0, 700.0, 200.0, &style, &ParagraphStyle::default())
        .unwrap();
    assert!(page
        .commands
        .iter()
        .all(|c| matches!(c, PageCommand::LogicalBreak { .. })));
    assert_eq!(reopened_text(&builder), text);
}

#[test]
fn borrowed_font_core_matches_transaction_asset_entry_points() {
    let assets = vec![
        ApprovedFontAsset {
            lookup_name: "Latin".into(),
            bytes: subset("ABC "),
        },
        ApprovedFontAsset {
            lookup_name: "Hebrew".into(),
            bytes: subset("אבג "),
        },
    ];
    let programs = assets
        .iter()
        .map(|a| a.bytes.as_slice())
        .collect::<Vec<_>>();
    let ranked = [(0, 0.0), (1, 1.0)];
    let text = "ABC אבג ABC";
    let options = ShapeOptions::default();
    let settings = Default::default();
    let owned =
        engine::resolve_contextual_fonts(text, &assets, &ranked, options, &settings).unwrap();
    let borrowed = engine::resolve_contextual_programs(
        text,
        &programs,
        &ranked,
        options,
        &settings,
        WritingMode::HorizontalTb,
    )
    .unwrap();
    assert_eq!(owned, borrowed);
    let paragraph = PreparedParagraph::new(text, options).unwrap();
    let bidi = paragraph.bidi.line(0..text.len()).unwrap();
    let a = engine::shape_line(text, &bidi, &owned, &assets, &settings).unwrap();
    let b = engine::shape_line_programs(text, &bidi, &borrowed, &programs, &settings).unwrap();
    assert_eq!(a.len(), b.len());
    for (a, b) in a.iter().zip(&b) {
        assert_eq!(
            (&a.range, a.font_index, &a.shaped),
            (&b.range, b.font_index, &b.shaped)
        );
    }
    let a = engine::measure_line(&a, &assets, 12.0).unwrap();
    let b = engine::measure_line_programs(&b, &programs, 12.0).unwrap();
    assert_eq!(
        (a.advance, a.width(), a.ascent, a.descent),
        (b.advance, b.width(), b.ascent, b.descent)
    );
}

#[test]
fn signed_contextual_advances_are_measured_as_the_writer_emits_them() {
    let bytes = builtin_unicode_font_bytes().unwrap();
    let gid = ttf_parser::Face::parse(bytes, 0)
        .unwrap()
        .glyph_index(' ')
        .unwrap()
        .0;
    let shaped = ShapedRun {
        direction: TextDirection::LeftToRight,
        used_complex_shaping: true,
        glyphs: vec![
            crate::fonts::ShapedGlyph {
                glyph_id: gid,
                cluster: 0,
                advance: -1000.0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
            crate::fonts::ShapedGlyph {
                glyph_id: gid,
                cluster: 1,
                advance: 500.0,
                offset_x: 0.0,
                offset_y: 0.0,
            },
        ],
    };
    let metrics = crate::fonts::line_layout::measure_signed_run(bytes, &shaped, 10.0).unwrap();
    assert_eq!(metrics.advance, -5.0);
    assert_eq!(metrics.left_pad, 10.0);
    assert_eq!(metrics.right_pad, 5.0);
    assert_eq!(metrics.width(), 10.0);
    layout::checked_metrics(metrics).unwrap();
    let runs = vec![engine::FontRun {
        range: 0..2,
        font_index: 0,
        shaped: shaped.clone(),
    }];
    let combined = engine::measure_signed_line_programs(&runs, &[bytes], 10.0).unwrap();
    assert_eq!((combined.advance, combined.width()), (-5.0, 10.0));
    let bidi = PreparedParagraph::new("  ", ShapeOptions::default())
        .unwrap()
        .bidi
        .line(0..2)
        .unwrap();
    let mut builder = PdfBuilder::new();
    let style = TextStyle::unicode(10.0);
    builder
        .add_page(PageSize::LETTER)
        .commands
        .push(PageCommand::Text {
            text: "  ".into(),
            x: 20.0,
            y: 100.0,
            style: style.clone(),
            bidi: Some(bidi.clone()),
            logical_text: None,
            suppress_actual_text: false,
            font_asset: None,
            shaped: Some(Arc::new(shaped)),
        });
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    let mut content = Vec::new();
    write_text_command_resolved(
        &mut content,
        "  ",
        20.0,
        100.0,
        &style,
        &plan,
        Some(&bidi),
        None,
    )
    .unwrap();
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("1 0 0 1 20 100 Tm"));
    assert!(content.contains("1 0 0 1 10 100 Tm"));
}
