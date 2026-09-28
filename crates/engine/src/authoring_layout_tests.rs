//! Unexecuted source regressions; no build, PDF workload or rendering was run.
use super::*;
use crate::fonts::line_layout::measure_run;

fn logical_commands(page: &PdfPageBuilder) -> String {
    page.commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::LogicalBreak { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn control_only_authored_runs_survive_without_the_selected_fonts_glyphs() {
    for style in [TextStyle::default(), TextStyle::unicode(12.0)] {
        for text in ["\u{00ad}", "\u{200d}", "\u{2067}\u{2069}", "\u{e0100}"] {
            let mut builder = PdfBuilder::new();
            let page = builder.add_page(PageSize::LETTER);
            assert_eq!(page.text_width(text, &style).unwrap(), 0.0);
            page.draw_text(text, 10.0, 700.0, &style).unwrap();
            assert!(matches!(page.commands[0], PageCommand::LogicalBreak { .. }));
            let engine = crate::ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
            let chunks = engine.collect_page_text_chunks(1).unwrap();
            assert_eq!(chunks.len(), 1);
            assert_eq!(chunks[0].text, text);
            assert_eq!(chunks[0].width, 0.0);
            assert!(!chunks[0].is_invisible);
        }
    }
}

#[test]
fn paragraph_controls_between_painted_lines_keep_zero_advance_source_ownership() {
    let text = "A\n\u{200d}\n\u{e0100}\r\nB";
    for style in [TextStyle::default(), TextStyle::unicode(12.0)] {
        let mut builder = PdfBuilder::new();
        let page = builder.add_page(PageSize::LETTER);
        page.draw_paragraph(text, 10.0, 700.0, 200.0, &style, &ParagraphStyle::default())
            .unwrap();
        assert_eq!(logical_commands(page), text);
        let output = builder.to_bytes().unwrap();
        let engine = crate::ContentEngine::open_bytes(output.clone()).unwrap();
        assert_eq!(
            engine
                .collect_page_text_chunks(1)
                .unwrap()
                .into_iter()
                .map(|chunk| chunk.text)
                .collect::<String>(),
            text
        );
        assert_eq!(
            crate::advanced_editing::analyze_multi_run_text_range(&output, 1)
                .unwrap()
                .logical_text,
            text
        );
    }
}

#[test]
fn registered_font_metrics_and_final_glyphs_use_the_same_program() {
    let mut builder = PdfBuilder::new();
    let bytes = get_fallback_font("Courier").unwrap();
    let font = builder.register_font_bytes("ActualCourier", bytes).unwrap();
    let style = TextStyle::new(font, 14.0);
    let text = "AV office 123 Wiii";
    let page = builder.add_page(PageSize::LETTER);
    let run = TextShaper::shape(bytes, text, ShapeOptions::default()).unwrap();
    let expected = measure_run(bytes, &run, style.size).unwrap().width();
    assert_eq!(page.text_width(text, &style).unwrap(), expected);
    let lines = layout::prepare(page, text, expected / 2.0, &style).unwrap();
    assert!(lines.len() > 1);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.logical.as_str())
            .collect::<String>(),
        text
    );
    page.draw_paragraph(
        text,
        10.0,
        700.0,
        expected / 2.0,
        &style,
        &ParagraphStyle::default(),
    )
    .unwrap();
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    for command in &builder.pages[0].commands {
        let PageCommand::Text { text, bidi, .. } = command else {
            continue;
        };
        let shaped =
            TextShaper::shape_resolved(bytes, text, bidi.as_ref().unwrap(), &Default::default())
                .unwrap();
        let emitted = plan.shaped_run_resolved(font, text, bidi.as_ref()).unwrap();
        assert_eq!(emitted.glyphs.len(), shaped.glyphs.len());
        for (a, b) in emitted.glyphs.iter().zip(shaped.glyphs) {
            assert_eq!(
                (a.advance, a.offset_x, a.offset_y),
                (b.advance, b.offset_x, b.offset_y)
            );
        }
    }
}

#[test]
fn registration_updates_existing_pages_without_cloning_font_bytes() {
    let mut builder = PdfBuilder::new();
    builder.add_page(PageSize::LETTER);
    let font = builder
        .register_font_bytes("Late", get_fallback_font("Courier").unwrap())
        .unwrap();
    assert!(Arc::ptr_eq(
        &builder.custom_fonts,
        &builder.pages[0].custom_fonts
    ));
    let style = TextStyle::new(font, 12.0);
    builder.pages_mut()[0]
        .draw_paragraph("A B", 20.0, 50.0, 100.0, &style, &ParagraphStyle::default())
        .unwrap();
    let PageCommand::Text {
        font_asset: Some(asset),
        ..
    } = &builder.pages[0].commands[0]
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(asset, &builder.custom_fonts[0].bytes));
    let mut clone = builder.clone();
    clone
        .register_font_bytes("Second", get_fallback_font("Symbol").unwrap())
        .unwrap();
    assert_eq!(builder.custom_fonts.len(), 1);
    assert_eq!(clone.custom_fonts.len(), 2);
    assert!(Arc::ptr_eq(
        &builder.custom_fonts[0].bytes,
        &clone.custom_fonts[0].bytes
    ));
    FontBuildPlan::from_builder(&builder).unwrap();
    FontBuildPlan::from_builder(&clone).unwrap();
}

#[test]
fn transferred_page_cannot_silently_rebind_already_measured_custom_fonts() {
    let mut source = PdfBuilder::new();
    let font = source
        .register_font_bytes("Source", get_fallback_font("Courier").unwrap())
        .unwrap();
    source
        .add_page(PageSize::LETTER)
        .draw_paragraph(
            "AAA",
            10.0,
            40.0,
            100.0,
            &TextStyle::new(font, 12.0),
            &ParagraphStyle::default(),
        )
        .unwrap();
    let mut destination = PdfBuilder::new();
    destination
        .register_font_bytes("Different", get_fallback_font("Symbol").unwrap())
        .unwrap();
    destination.add_page(PageSize::LETTER);
    destination.pages_mut()[0] = source.pages()[0].clone();
    // Even a subsequent registration refreshing page registry snapshots must
    // not erase the immutable font asset captured by the old drawing command.
    destination
        .register_font_bytes("Extra", get_fallback_font("Helvetica").unwrap())
        .unwrap();
    assert!(FontBuildPlan::from_builder(&destination).is_err());
}

#[test]
fn standalone_page_does_not_measure_a_document_font_using_builtin_metrics() {
    let mut builder = PdfBuilder::new();
    let font = builder
        .register_font_bytes("Custom", get_fallback_font("Courier").unwrap())
        .unwrap();
    let mut page = PdfPageBuilder::new(PageSize::LETTER);
    let style = TextStyle::new(font, 12.0);
    assert!(page.text_width("A", &style).is_err());
    assert!(page.wrap_text("A", 100.0, &style).is_err());
    assert!(page.draw_text("A", 10.0, 40.0, &style).is_err());
    assert!(page.commands.is_empty());
}

#[test]
fn mixed_direction_soft_lines_retain_paragraph_levels_and_joining_context() {
    let mut builder = PdfBuilder::new();
    let style = TextStyle::unicode(12.0);
    let text = "العربية 123 ABC العربية 456 xyz";
    let page = builder.add_page(PageSize::LETTER);
    page.draw_paragraph(text, 10.0, 700.0, 70.0, &style, &ParagraphStyle::default())
        .unwrap();
    let paragraph =
        crate::fonts::shaper::ParagraphBidi::new(text, ShapeOptions::default()).unwrap();
    let mut offset = 0;
    assert!(page.commands.len() > 1);
    for command in &page.commands {
        let PageCommand::Text {
            text: line,
            bidi: Some(bidi),
            ..
        } = command
        else {
            panic!()
        };
        assert_eq!(*bidi, paragraph.line(offset..offset + line.len()).unwrap());
        offset += line.len();
    }
    assert_eq!(offset, text.len());
    assert_eq!(logical_commands(page), text);
    FontBuildPlan::from_builder(&builder).unwrap();
}

#[test]
fn hard_boundaries_and_blank_lines_survive_authoring_and_reopening() {
    for style in [TextStyle::default(), TextStyle::unicode(12.0)] {
        for separator in [
            "\r", "\n", "\r\n", "\u{000b}", "\u{0085}", "\u{2028}", "\u{2029}",
        ] {
            let text = format!("{separator}A{separator}{separator}B{separator}");
            let mut builder = PdfBuilder::new();
            let page = builder.add_page(PageSize::LETTER);
            let visible = page
                .draw_paragraph(
                    &text,
                    10.0,
                    700.0,
                    200.0,
                    &style,
                    &ParagraphStyle::default(),
                )
                .unwrap();
            assert_eq!(visible, ["", "A", "", "B"]);
            assert_eq!(logical_commands(page), text);
            let output = builder.to_bytes().unwrap();
            let engine = crate::ContentEngine::open_bytes(output).unwrap();
            let chunks = engine.collect_page_text_chunks(1).unwrap();
            assert_eq!(
                chunks
                    .iter()
                    .map(|chunk| chunk.text.as_str())
                    .collect::<String>(),
                text
            );
            let blank = chunks
                .iter()
                .filter(|chunk| {
                    chunk
                        .text
                        .chars()
                        .all(crate::fonts::hard_break::is_hard_break)
                })
                .collect::<Vec<_>>();
            assert_eq!(blank.len(), 2);
            assert!(blank
                .iter()
                .all(|chunk| chunk.width == 0.0 && !chunk.is_invisible));
        }
    }
}

#[test]
fn form_feed_is_page_owned_in_flow_and_rejected_by_page_local_paragraphs() {
    let style = TextStyle::unicode(12.0);
    let mut page = PdfPageBuilder::new(PageSize::LETTER);
    assert!(page
        .draw_paragraph(
            "A\u{000c}B",
            10.0,
            700.0,
            200.0,
            &style,
            &ParagraphStyle::default(),
        )
        .is_err());
    assert!(page.commands.is_empty());

    let mut flow = FlowDocument::new(PageSize::LETTER, Margins::all(36.0));
    flow.add_paragraph("A\u{000c}B\u{000c}", &style, &ParagraphStyle::default())
        .unwrap();
    assert_eq!(flow.builder.pages.len(), 3);
    assert_eq!(logical_commands(&flow.builder.pages[0]), "A\u{000c}");
    assert_eq!(logical_commands(&flow.builder.pages[1]), "B\u{000c}");
    assert!(flow.builder.pages[2].commands.is_empty());
}

#[test]
fn explicit_authoring_page_parity_materializes_owned_blank_pages() {
    let mut flow = FlowDocument::new(PageSize::LETTER, Margins::all(36.0));
    flow.add_page_break_to(FlowPageBreak::NextOddPage);
    assert_eq!(flow.builder.pages.len(), 3);
    flow.add_page_break_to(FlowPageBreak::NextEvenPage);
    assert_eq!(flow.builder.pages.len(), 4);
}

#[test]
fn blank_carrier_widths_are_zero_and_use_empty_outlines_with_exact_unicode() {
    let text = "\r\n\u{000b}\u{000c}\u{0085}\u{2028}\u{2029}";
    let mut flow = FlowDocument::new(PageSize::LETTER, Margins::all(36.0));
    flow.add_paragraph(text, &TextStyle::default(), &ParagraphStyle::default())
        .unwrap();
    let builder = flow.into_builder();
    let plan = FontBuildPlan::from_builder(&builder).unwrap();
    let embedded = plan.embedded_plan(FontFace::BuiltinUnicode).unwrap();
    let bytes = builtin_unicode_font_bytes().unwrap();
    let face = ttf_parser::Face::parse(bytes, 0).unwrap();
    for entry in &embedded.entries {
        assert_eq!(entry.width, 0.0);
        assert_ne!(entry.glyph_id, 0);
        assert!(face
            .glyph_bounding_box(ttf_parser::GlyphId(entry.glyph_id))
            .is_none());
        assert!(entry
            .unicode
            .chars()
            .all(crate::fonts::hard_break::is_hard_break));
    }
    assert_eq!(embedded.entries.len(), 7);
    let widths = unicode_width_array(
        FontFace::BuiltinUnicode,
        &plan,
        &TrueTypeMetrics::parse(bytes).unwrap(),
    )
    .unwrap();
    let PdfObject::Array(widths) = &widths[1] else {
        panic!()
    };
    assert!(widths.iter().all(|width| width.as_number() == Some(0.0)));
}

#[test]
fn paragraph_wrapping_retains_repeated_spaces_and_rejects_an_overwide_grapheme() {
    let mut page = PdfPageBuilder::new(PageSize::LETTER);
    let style = TextStyle::default();
    let text = "  alpha  beta   gamma  ";
    let lines = page
        .draw_paragraph(text, 10.0, 700.0, 65.0, &style, &ParagraphStyle::default())
        .unwrap();
    assert_eq!(lines.concat(), text);
    assert_eq!(logical_commands(&page), text);
    for line in lines {
        assert!(page.text_width(&line, &style).unwrap() <= 65.000001);
    }
    let count = page.commands.len();
    assert!(page
        .draw_paragraph("W", 10.0, 50.0, 0.001, &style, &ParagraphStyle::default())
        .is_err());
    assert_eq!(page.commands.len(), count);
}

#[test]
fn invalid_or_multiline_single_run_requests_do_not_append_commands() {
    let mut page = PdfPageBuilder::new(PageSize::LETTER);
    for separator in [
        "\r", "\n", "\r\n", "\u{000b}", "\u{000c}", "\u{0085}", "\u{2028}", "\u{2029}",
    ] {
        assert!(page
            .draw_text(
                format!("A{separator}B"),
                0.0,
                0.0,
                &TextStyle::unicode(12.0)
            )
            .is_err());
    }
    for size in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(page
            .draw_text("A", 0.0, 0.0, &TextStyle::unicode(size))
            .is_err());
    }
    assert!(page
        .draw_text("A", f64::INFINITY, 0.0, &TextStyle::default())
        .is_err());
    assert!(page.commands.is_empty());
}

#[test]
fn paragraph_planning_errors_and_cancellation_leave_the_page_unchanged() {
    let mut page = PdfPageBuilder::new(PageSize::LETTER);
    page.draw_text("before", 10.0, 20.0, &TextStyle::default())
        .unwrap();
    let before = logical_commands(&page);
    assert!(page
        .draw_paragraph(
            "A\n🚀",
            10.0,
            100.0,
            100.0,
            &TextStyle::default(),
            &ParagraphStyle::default()
        )
        .is_err());
    assert!(page
        .draw_paragraph(
            "A",
            10.0,
            100.0,
            100.0,
            &TextStyle::default(),
            &ParagraphStyle::new().line_height(f64::INFINITY)
        )
        .is_err());
    let cancel = crate::cancel::CancelToken::new();
    cancel.cancel();
    assert!(cancel
        .scope(|| page.draw_paragraph(
            "A",
            10.0,
            100.0,
            100.0,
            &TextStyle::default(),
            &ParagraphStyle::default()
        ))
        .is_err());
    assert_eq!(logical_commands(&page), before);
    assert_eq!(page.commands.len(), 1);
}

#[test]
fn flow_preserves_blank_lines_across_pages_and_uses_registered_fonts() {
    let mut flow = FlowDocument::new(PageSize::custom(120.0, 80.0), Margins::all(10.0));
    let font = flow
        .builder_mut()
        .register_font_bytes("Flow", get_fallback_font("Courier").unwrap())
        .unwrap();
    let text = "A\n\nB\r\nC\n\nD\nE\n\nF";
    flow.add_paragraph(
        text,
        &TextStyle::new(font, 12.0),
        &ParagraphStyle::default(),
    )
    .unwrap();
    assert!(flow.builder.pages.len() > 1);
    assert_eq!(
        flow.builder
            .pages
            .iter()
            .map(logical_commands)
            .collect::<String>(),
        text
    );
    let output = flow.builder.to_bytes().unwrap();
    let engine = crate::ContentEngine::open_bytes(output).unwrap();
    let actual = (1..=flow.builder.pages.len())
        .map(|page| {
            engine
                .collect_page_text_chunks(page)
                .unwrap()
                .into_iter()
                .map(|chunk| chunk.text)
                .collect::<String>()
        })
        .collect::<String>();
    assert_eq!(actual, text);
}

#[test]
fn failed_list_rolls_back_prior_items_added_pages_and_cursor() {
    let mut flow = FlowDocument::new(PageSize::custom(120.0, 80.0), Margins::all(10.0));
    flow.add_paragraph("before", &TextStyle::default(), &ParagraphStyle::default())
        .unwrap();
    let page_count = flow.builder.pages.len();
    let command_counts = flow
        .builder
        .pages
        .iter()
        .map(|p| p.commands.len())
        .collect::<Vec<_>>();
    let cursor = flow.cursor_y;
    assert!(flow
        .add_list(
            ["A\nA\nA\nA\nA\nA", "🚀"],
            false,
            &TextStyle::default(),
            &ParagraphStyle::default()
        )
        .is_err());
    assert_eq!(flow.builder.pages.len(), page_count);
    assert_eq!(
        flow.builder
            .pages
            .iter()
            .map(|p| p.commands.len())
            .collect::<Vec<_>>(),
        command_counts
    );
    assert_eq!(flow.cursor_y, cursor);
}

#[test]
fn table_cells_share_font_plans_and_preserve_hard_separators() {
    let mut builder = PdfBuilder::new();
    let font = builder
        .register_font_bytes("Cells", get_fallback_font("Courier").unwrap())
        .unwrap();
    let style = TextStyle::new(font, 12.0);
    let text = " A  B\r\n\r\nC ";
    let mut table = TableBuilder::new(vec![TableColumn::new(100.0)]).body_style(style);
    table.add_row([text]);
    let page = builder.add_page(PageSize::LETTER);
    let height = table.measure_row(page, &table.rows[0], false).unwrap();
    assert!((table.draw_on_page(page, 10.0, 700.0).unwrap() - height).abs() < 1e-7);
    assert_eq!(logical_commands(page), text);
    let engine = crate::ContentEngine::open_bytes(builder.to_bytes().unwrap()).unwrap();
    assert_eq!(
        engine
            .collect_page_text_chunks(1)
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.text)
            .collect::<String>(),
        text
    );
}

#[test]
fn invalid_and_overheight_tables_do_not_leave_partial_headers_or_rows() {
    let mut flow = FlowDocument::new(PageSize::custom(120.0, 80.0), Margins::all(10.0));
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0)])
        .row_split_policy(TableRowSplitPolicy::KeepTogether);
    table.set_header(["Header"]);
    table.add_row(["A\n".repeat(50)]);
    let cursor = flow.cursor_y;
    assert!(flow.add_table(&table).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert_eq!(flow.cursor_y, cursor);
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0)]);
    table.add_row(["fits", "must not disappear"]);
    assert!(table
        .draw_on_page(&mut flow.builder.pages[0], 10.0, 50.0)
        .is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn table_header_moves_with_the_first_row_instead_of_being_orphaned() {
    let mut flow = FlowDocument::new(PageSize::custom(120.0, 80.0), Margins::all(10.0));
    flow.add_paragraph(
        "before\nbefore",
        &TextStyle::default(),
        &ParagraphStyle::default(),
    )
    .unwrap();
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0)]);
    table.set_header(["Header"]);
    table.add_row(["Body"]);
    flow.add_table(&table).unwrap();
    assert_eq!(flow.builder.pages.len(), 2);
    assert_eq!(logical_commands(&flow.builder.pages[0]), "before\nbefore");
    assert_eq!(logical_commands(&flow.builder.pages[1]), "HeaderBody");
}

#[test]
fn paragraph_tabs_are_positioned_fields_and_remain_one_logical_text_value() {
    let mut builder = PdfBuilder::new();
    let page = builder.add_page(PageSize::LETTER);
    let style = TextStyle::standard(StandardFont::Courier, 12.0);
    let paragraph = ParagraphStyle::new().tab_stops(TabStops {
        stops: vec![
            TabStop {
                position: 72.0,
                alignment: TabAlignment::Left,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::None,
                bar: false,
            },
            TabStop {
                position: 144.0,
                alignment: TabAlignment::Left,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::None,
                bar: false,
            },
        ],
        default_interval: 36.0,
    });
    assert!(page.text_width("A\tB", &style).is_err());
    assert!(page.draw_text("A\tB", 10.0, 700.0, &style).is_err());
    assert!(page.commands.is_empty());
    page.draw_paragraph("A\tB\tC", 10.0, 700.0, 200.0, &style, &paragraph)
        .unwrap();
    let PageCommand::TextGroup { logical_text, runs } = &page.commands[0] else {
        panic!("tabbed paragraph was not emitted as one logical text group")
    };
    assert_eq!(logical_text, "A\tB\tC");
    assert_eq!(runs.len(), 3);
    let positions = runs
        .iter()
        .map(|run| match run {
            PageCommand::Text { x, .. } => *x,
            _ => panic!("tab field emitted a non-text command"),
        })
        .collect::<Vec<_>>();
    assert!((positions[0] - 10.0).abs() < 1e-7);
    assert!((positions[1] - 82.0).abs() < 1e-7);
    assert!((positions[2] - 154.0).abs() < 1e-7);
    let output = builder.to_bytes().unwrap();
    let engine = crate::ContentEngine::open_bytes(output).unwrap();
    assert!(engine.get_page_text(1).unwrap().contains("A\tB\tC"));
}

#[test]
fn positioned_tabs_retain_exact_semantic_owners_and_field_origins() {
    let mut builder = PdfBuilder::new();
    let page = builder.add_page(PageSize::LETTER);
    let style = TextStyle::standard(StandardFont::Courier, 12.0);
    let tabs = TabStops {
        stops: vec![
            TabStop {
                position: 72.0,
                alignment: TabAlignment::Left,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::None,
                bar: false,
            },
            TabStop {
                position: 144.0,
                alignment: TabAlignment::Left,
                decimal: '.',
                decimal_token: None,
                leader: TabLeader::None,
                bar: false,
            },
        ],
        default_interval: 36.0,
    };
    let lines = layout::prepare_with_tabs(page, "A\tB\tC", 200.0, &style, &tabs).unwrap();
    assert_eq!(lines.len(), 1);
    let commands = lines[0]
        .owned_commands(
            page,
            10.0,
            700.0,
            &style,
            &[
                layout::OwnedTextSpan {
                    range: 0..2,
                    element: 11,
                },
                layout::OwnedTextSpan {
                    range: 2..5,
                    element: 12,
                },
            ],
        )
        .unwrap();
    let logical = commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::LogicalBreak { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<String>();
    assert_eq!(logical, "A\tB\tC");
    let positions = commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text { x, .. } => Some(*x),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(positions.len(), 3);
    assert!((positions[0] - 10.0).abs() < 1e-7);
    assert!((positions[1] - 82.0).abs() < 1e-7);
    assert!((positions[2] - 154.0).abs() < 1e-7);
    assert!(commands
        .iter()
        .any(|command| matches!(command, PageCommand::LogicalBreak { text, .. } if text == "\t")));
}

#[test]
fn tab_leaders_and_bars_are_artifacts_without_changing_logical_text() {
    let mut builder = PdfBuilder::new();
    let page = builder.add_page(PageSize::LETTER);
    let style = TextStyle::standard(StandardFont::Courier, 12.0);
    let paragraph = ParagraphStyle::new().tab_stops(TabStops {
        stops: vec![TabStop {
            position: 96.0,
            alignment: TabAlignment::Left,
            decimal: '.',
            decimal_token: None,
            leader: TabLeader::Dots,
            bar: true,
        }],
        default_interval: 36.0,
    });
    page.draw_paragraph("Title\t12", 10.0, 700.0, 200.0, &style, &paragraph)
        .unwrap();
    let PageCommand::TextGroup { logical_text, runs } = &page.commands[0] else {
        panic!("decorated tab row was not emitted as one logical text group")
    };
    assert_eq!(logical_text, "Title\t12");
    assert_eq!(
        runs.iter()
            .filter(|command| matches!(command, PageCommand::BeginArtifact))
            .count(),
        1
    );
    assert_eq!(
        runs.iter()
            .filter(|command| matches!(command, PageCommand::Path { .. }))
            .count(),
        2
    );
    assert_eq!(
        runs.iter()
            .filter(|command| matches!(command, PageCommand::EndArtifact))
            .count(),
        1
    );
    let positions = runs
        .iter()
        .filter_map(|command| match command {
            PageCommand::Text { x, .. } => Some(*x),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(positions.len(), 2);
    assert!((positions[0] - 10.0).abs() < 1e-7);
    assert!((positions[1] - 106.0).abs() < 1e-7);
    let output = builder.to_bytes().unwrap();
    let pdf = String::from_utf8_lossy(&output);
    let artifact = pdf.find("/Artifact BMC").unwrap();
    let logical = pdf.find("/Span << /ActualText").unwrap();
    assert!(
        artifact < logical,
        "artifact must precede the logical text scope"
    );
    let engine = crate::ContentEngine::open_bytes(output).unwrap();
    assert!(engine.get_page_text(1).unwrap().contains("Title\t12"));
}
