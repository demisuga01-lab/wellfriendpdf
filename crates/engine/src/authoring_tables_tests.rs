//! Source-only regression cases. No builds, tests, or PDF workloads were run.
use super::*;
use sha2::{Digest, Sha256};

fn flow() -> FlowDocument {
    FlowDocument::new(PageSize::custom(220.0, 100.0), Margins::all(10.0))
}

fn logical(page: &PdfPageBuilder) -> String {
    page.commands
        .iter()
        .filter_map(|c| match c {
            PageCommand::Text {
                text, logical_text, ..
            } => Some(logical_text.as_deref().unwrap_or(text)),
            PageCommand::TextGroup { logical_text, .. } => Some(logical_text.as_str()),
            PageCommand::LogicalBreak { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn assert_source_partition(table: &TableBuilder, report: &TableFlowReport) {
    let grid = table.body_cell_placements().unwrap();
    for (row_index, row) in table.rows.iter().enumerate() {
        let placements = &grid[row_index];
        let fragments = report
            .fragments
            .iter()
            .filter(|f| f.row == Some(row_index))
            .collect::<Vec<_>>();
        assert!(!fragments.is_empty());
        assert!(!fragments.last().unwrap().continues);
        for fragment in &fragments {
            assert_eq!(fragment.cells.len(), placements.len());
            for (actual, placement) in fragment.cells.iter().zip(placements) {
                assert_eq!(actual.source_cell, placement.source.map(|(index, _)| index));
                assert_eq!(actual.column_start, placement.columns.start);
                assert_eq!(actual.column_span, placement.columns.len());
                assert_eq!(actual.row_start, placement.row_start);
                assert_eq!(actual.row_span, placement.row_span);
                if placement.source.is_none() {
                    assert_eq!(actual.utf8_range, [0, 0]);
                }
            }
        }
        for (source_index, source) in row.cells.iter().enumerate() {
            let mut end = 0;
            let mut reconstructed = String::new();
            for fragment in &fragments {
                let cell = fragment
                    .cells
                    .iter()
                    .find(|cell| cell.source_cell == Some(source_index))
                    .expect("source cell receipt");
                let [start, next] = cell.utf8_range;
                assert_eq!(start, end);
                reconstructed.push_str(source.text.get(start..next).unwrap());
                end = next;
                assert!(fragment.height > 0.0);
            }
            assert_eq!(end, source.text.len());
            assert_eq!(reconstructed, source.text);
        }
    }
}

#[test]
fn column_spans_aggregate_geometry_ranges_and_table_semantics() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![
        TableColumn::new(50.0),
        TableColumn::new(60.0),
        TableColumn::new(70.0),
    ]);
    table.push_row(TableRow::new(vec![
        TableCell::text("Merged logical cell").column_span(2),
        TableCell::text("Last"),
    ]));

    let mut budget = Budget::default();
    let prepared = PreparedRow::new(
        &table,
        flow.current_page_ref(),
        &table.rows[0],
        false,
        &mut budget,
    )
    .unwrap();
    assert_eq!(prepared.cells.len(), 2);
    assert_eq!(prepared.cells[0].source_cell, Some(0));
    assert_eq!(prepared.cells[0].column_start, 0);
    assert_eq!(prepared.cells[0].column_span, 2);
    assert!((prepared.cells[0].width - 110.0).abs() < EPS);
    assert!((prepared.cells[1].left - 110.0).abs() < EPS);

    let report = flow.add_table_with_report(&table).unwrap();
    assert_source_partition(&table, &report);
    let body = report
        .fragments
        .iter()
        .find(|fragment| fragment.row == Some(0))
        .unwrap();
    assert_eq!(body.cells.len(), 2);
    assert_eq!(body.cells[0].column_span, 2);
    assert_eq!(body.cells[0].utf8_range, [0, "Merged logical cell".len()]);
    let rects = flow.builder.pages[0]
        .commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Rect { x, width, .. } => Some((*x, *width)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(rects
        .iter()
        .any(|(x, width)| (*x - 10.0).abs() < EPS && (*width - 110.0).abs() < EPS));
    assert!(rects
        .iter()
        .any(|(x, width)| (*x - 120.0).abs() < EPS && (*width - 70.0).abs() < EPS));

    flow.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&flow.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/ColSpan 2"));
}

#[test]
fn invalid_column_spans_fail_before_painting() {
    let mut page = PdfPageBuilder::new(PageSize::custom(220.0, 100.0));
    let commands = page.commands.len();
    let mut zero = TableBuilder::new(vec![TableColumn::new(80.0)]);
    zero.add_row([TableCell::text("bad").column_span(0)]);
    assert!(zero.draw_on_page(&mut page, 10.0, 90.0).is_err());
    let mut overflow = TableBuilder::new(vec![TableColumn::new(80.0)]);
    overflow.add_row([TableCell::text("bad").column_span(2)]);
    assert!(overflow.draw_on_page(&mut page, 10.0, 90.0).is_err());
    assert_eq!(page.commands.len(), commands);
}

#[test]
fn row_spans_skip_occupied_slots_move_as_blocks_and_publish_semantics() {
    let mut table = TableBuilder::new(vec![
        TableColumn::new(50.0),
        TableColumn::new(50.0),
        TableColumn::new(50.0),
    ]);
    table.push_row(TableRow::new(vec![
        TableCell::text("A").row_span(2),
        TableCell::text("B"),
        TableCell::text("C"),
    ]));
    table.push_row(TableRow::new(vec![TableCell::text("D").column_span(2)]));
    table.push_row(TableRow::new(vec![TableCell::text("E").column_span(3)]));
    table.validate().unwrap();
    let grid = table.body_cell_placements().unwrap();
    assert_eq!(grid.len(), 3);
    assert_eq!(grid[0][0].columns, 0..1);
    assert_eq!(grid[0][0].row_start, 0);
    assert_eq!(grid[0][0].row_span, 2);
    assert_eq!(grid[1].len(), 1);
    assert_eq!(grid[1][0].source.map(|(index, _)| index), Some(0));
    assert_eq!(grid[1][0].columns, 1..3);
    assert_eq!(grid[2][0].columns, 0..3);

    let mut rendered = flow();
    // Leave only one point, strictly less than the two-row span group's
    // measured height. An exact fit is valid and must not be forced forward.
    rendered.add_spacer(79.0);
    let report = rendered.add_table_with_report(&table).unwrap();
    assert_source_partition(&table, &report);
    assert_eq!(report.fragments[0].page, 2);
    assert_eq!(report.fragments[1].page, 2);
    let spanning = report.fragments[0]
        .cells
        .iter()
        .find(|cell| cell.source_cell == Some(0))
        .unwrap();
    assert_eq!((spanning.row_start, spanning.row_span), (0, 2));
    let span_fragments = report
        .fragments
        .iter()
        .filter(|fragment| matches!(fragment.row, Some(0 | 1)))
        .collect::<Vec<_>>();
    let span_top = span_fragments
        .iter()
        .map(|fragment| fragment.top)
        .fold(f64::NEG_INFINITY, f64::max);
    let span_bottom = span_fragments
        .iter()
        .map(|fragment| fragment.top - fragment.height)
        .fold(f64::INFINITY, f64::min);
    let span_height = span_top - span_bottom;
    let spanning_rectangles = rendered.builder.pages[1]
        .commands
        .iter()
        .filter_map(|command| match command {
            PageCommand::Rect {
                x, width, height, ..
            } if (*x - 10.0).abs() < EPS && (*width - 50.0).abs() < EPS => Some(*height),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(spanning_rectangles.len(), 1);
    assert!(
        (spanning_rectangles[0] - span_height).abs() < EPS,
        "spanning rectangle {} != fragment height {}",
        spanning_rectangles[0],
        span_height
    );
    rendered.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&rendered.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/RowSpan 2"));

    let mut overflow = TableBuilder::new(vec![TableColumn::new(50.0)]);
    overflow.push_row(TableRow::new(vec![TableCell::text("A").row_span(2)]));
    assert!(overflow.validate().is_err());

    let mut header = TableBuilder::new(vec![TableColumn::new(50.0)]);
    header.set_header([TableCell::text("H").row_span(2)]);
    header.add_row(["body"]);
    assert!(header.validate().is_err());
}

#[test]
fn oversized_row_span_cut_solver_uses_only_safe_shaped_line_boundaries() {
    let page = PdfPageBuilder::new(PageSize::custom(220.0, 100.0));
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)]);
    table.push_row(TableRow::new(vec![
        TableCell::text("Long spanning line\n".repeat(18)).row_span(2),
        TableCell::text("top"),
    ]));
    table.push_row(TableRow::new(vec![TableCell::text("bottom")]));
    table.validate().unwrap();
    let grid = PreparedGrid::new(&table, &page, &mut Budget::default()).unwrap();
    let group = grid.groups().into_iter().next().unwrap();
    assert_eq!(group, 0..2);
    let end = grid.prefix[group.end];
    let mut start = grid.prefix[group.start];
    let mut cuts = Vec::new();
    while start < end - EPS {
        let cut = grid
            .safe_cut(group.clone(), start, 30.0, table.row_split_policy)
            .unwrap()
            .expect("safe continuation cut");
        assert!(cut > start + EPS);
        assert!(cut <= (start + 30.0).min(end) + EPS);
        for row in &grid.rows[group.clone()] {
            for cell in &row.cells {
                let top = grid.prefix[cell.row_start] + row.padding;
                assert!(!cell
                    .heights
                    .windows(2)
                    .any(|window| { top + window[0] < cut - EPS && top + window[1] > cut + EPS }));
            }
        }
        cuts.push(cut);
        start = cut;
        assert!(cuts.len() < 100);
    }
    assert!(cuts.len() > 1);
    assert!((start - end).abs() < EPS);

    let mut rendered = flow();
    let report = rendered.add_table_with_report(&table).unwrap();
    assert!(report.added_pages > 0);
    assert_source_partition(&table, &report);
    assert_page_bounds(&report, &rendered);
    let spanning_rectangles = rendered
        .builder
        .pages
        .iter()
        .flat_map(|page| &page.commands)
        .filter(|command| {
            matches!(command, PageCommand::Rect { x, width, .. }
                if (*x - 10.0).abs() < EPS && (*width - 80.0).abs() < EPS)
        })
        .count();
    assert!(spanning_rectangles > 1);
    rendered.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&rendered.builder.to_bytes().unwrap()).into_owned();
    assert!(pdf.contains("/RowSpan 2"));
}

#[test]
fn fresh_typed_tables_reuse_exact_decimal_dependency_evaluation() {
    use crate::typed_tables::{DecimalValue, TableFormula, TableValue};

    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("invoice-totals");
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "subtotal",
            TableValue::Decimal {
                value: DecimalValue {
                    coefficient: "1234".into(),
                    scale: 2,
                },
            },
        ),
        TableCell::typed(
            "total",
            TableValue::Formula {
                expression: TableFormula::Add {
                    left: Box::new(TableFormula::Cell {
                        id: "subtotal".into(),
                    }),
                    right: Box::new(TableFormula::Constant {
                        value: DecimalValue {
                            coefficient: "66".into(),
                            scale: 2,
                        },
                    }),
                },
                display_scale: 2,
            },
        ),
    ]));
    let mut rendered = flow();
    let report = rendered.add_table_with_report(&table).unwrap();
    assert_eq!(report.evaluated_values["subtotal"], "12.34");
    assert_eq!(report.evaluated_values["total"], "13.00");
    assert_eq!(logical(&rendered.builder.pages[0]), "12.3413.00");
    assert!(table.rows[0].cells.iter().all(|cell| cell.text.is_empty()));
    let bytes = rendered.builder.to_bytes().unwrap();
    let models = load_authored_typed_tables(&bytes).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "invoice-totals");
    assert_eq!(models[0].cells.len(), 2);
    assert_eq!(models[0].cells[0].evaluated, "12.34");
    assert_eq!(models[0].cells[1].evaluated, "13.00");
    assert_eq!(models[0].cells[1].column, 1);
    let sources = inspect_authored_typed_table_sources(&bytes).unwrap();
    assert!(sources.exact_source_ownership);
    assert_eq!(sources.cells.len(), 2);
    assert_eq!(sources.cells[0].table_id, "invoice-totals");
    assert_eq!(sources.cells[0].cell_id, "subtotal");
    assert_eq!(sources.cells[0].fragments[0].logical_text, "12.34");
    assert!(sources.cells[0].fragments[0].region.is_some_and(|region| {
        region.iter().all(|value| value.is_finite())
            && region[0] < region[2]
            && region[1] < region[3]
    }));
    assert_eq!(sources.cells[1].cell_id, "total");
    assert_eq!(sources.cells[1].fragments[0].logical_text, "13.00");
    assert!(sources.cells[1].fragments[0].region.is_some_and(|region| {
        region.iter().all(|value| value.is_finite())
            && region[0] < region[2]
            && region[1] < region[3]
    }));
    let grid = inspect_authored_typed_table_grid_paint(&bytes, "invoice-totals").unwrap();
    assert!(grid.complete_typed_grid_ownership);
    assert_eq!(grid.fragments.len(), 2);
    assert!(grid.fragments.iter().all(|fragment| {
        fragment.decoded_range[0] < fragment.decoded_range[1]
            && fragment.rect[0] < fragment.rect[2]
            && fragment.rect[1] < fragment.rect[3]
    }));
    let typed_owners = rendered
        .builder
        .structures
        .iter()
        .filter_map(|element| {
            element
                .typed_table_identity
                .as_deref()
                .zip(element.typed_cell_identity.as_deref())
        })
        .collect::<Vec<_>>();
    assert_eq!(
        typed_owners,
        vec![("invoice-totals", "subtotal"), ("invoice-totals", "total")]
    );
    rendered.builder.writer_mode = WriterMode::ClassicXref;
    let owned_pdf = String::from_utf8_lossy(&rendered.builder.to_bytes().unwrap()).into_owned();
    // Each cell identity is published once in its content owner and once in
    // its structure-tree owner so source editing and accessibility resolve the
    // same stable identity after reopen.
    assert_eq!(owned_pdf.matches("/WFTableID <FEFF").count(), 4);
    assert_eq!(owned_pdf.matches("/WFCellID <FEFF").count(), 4);
    assert_eq!(owned_pdf.matches("/WFGridTableID <FEFF").count(), 2);
    assert_eq!(owned_pdf.matches("/WFRowGrid true").count(), 2);
    assert!(owned_pdf.contains("/WFRow 0 /WFColumn 0 /WFRowSpan 1 /WFColSpan 1"));
    assert!(owned_pdf.contains("/WFRow 0 /WFColumn 1 /WFRowSpan 1 /WFColSpan 1"));
    assert!(owned_pdf.contains("/WFTableID"));
    assert!(owned_pdf.contains("/WFCellID"));
    let pages = rendered.builder.pages.len();
    let commands = rendered
        .builder
        .pages
        .iter()
        .map(|page| page.commands.len())
        .collect::<Vec<_>>();
    let structures = rendered.builder.structures.len();
    assert!(rendered.add_table_with_report(&table).is_err());
    assert_eq!(rendered.builder.pages.len(), pages);
    assert_eq!(
        rendered
            .builder
            .pages
            .iter()
            .map(|page| page.commands.len())
            .collect::<Vec<_>>(),
        commands
    );
    assert_eq!(rendered.builder.structures.len(), structures);
    assert_eq!(rendered.builder.authored_typed_tables.len(), 1);

    let mut cyclic = table.clone();
    cyclic.rows[0].cells[0].typed_value = Some(TableValue::Formula {
        expression: TableFormula::Cell { id: "total".into() },
        display_scale: 2,
    });
    let mut page = PdfPageBuilder::new(PageSize::custom(220.0, 100.0));
    let commands = page.commands.len();
    assert!(cyclic.draw_on_page(&mut page, 10.0, 90.0).is_err());
    assert_eq!(page.commands.len(), commands);
}

#[test]
fn empty_typed_cell_retains_a_zero_width_exact_source_carrier() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).identity("empty-table");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "empty-cell",
        TableValue::Text {
            text: String::new(),
        },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let bytes = rendered.builder.to_bytes().unwrap();
    let sources = inspect_authored_typed_table_sources(&bytes).unwrap();
    assert_eq!(sources.cells.len(), 1);
    assert_eq!(sources.cells[0].evaluated, "");
    assert_eq!(sources.cells[0].fragments.len(), 1);
    assert_eq!(
        sources.cells[0].fragments[0].logical_range[0],
        sources.cells[0].fragments[0].logical_range[1]
    );
    assert_eq!(sources.cells[0].fragments[0].logical_text, "");
    assert!(!sources.cells[0].fragments[0].source_span_ids.is_empty());
    assert!(sources.cells[0].fragments[0]
        .region
        .is_some_and(|region| region[0] < region[2] && region[1] < region[3]));
}

#[test]
fn typed_multirow_grid_paint_has_exact_relocatable_ownership() {
    use crate::typed_tables::TableValue;

    let typed = |id: &str, text: &str| {
        TableCell::typed(
            id,
            TableValue::Text {
                text: text.to_string(),
            },
        )
    };
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("owned-multirow-grid");
    table.push_row(TableRow::new(vec![
        typed("spanning", "A").row_span(2),
        typed("first", "B"),
    ]));
    table.push_row(TableRow::new(vec![typed("second", "C")]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let bytes = rendered.builder.to_bytes().unwrap();
    let models = load_authored_typed_tables(&bytes).unwrap();
    assert_eq!(models[0].pagination.as_ref().unwrap().body_rows, 2);
    assert_eq!(models[0].cells.len(), 3);
    let grid = inspect_authored_typed_table_grid_paint(&bytes, "owned-multirow-grid").unwrap();
    assert!(grid.complete_typed_grid_ownership);
    assert_eq!(grid.fragments.len(), 3);
    assert_eq!(
        grid.fragments
            .iter()
            .map(|fragment| (
                fragment.row,
                fragment.column,
                fragment.row_span,
                fragment.column_span,
            ))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([(0, 0, 2, 1), (0, 1, 1, 1), (1, 1, 1, 1)])
    );

    rendered.builder.writer_mode = WriterMode::ClassicXref;
    let pdf = String::from_utf8_lossy(&rendered.builder.to_bytes().unwrap()).into_owned();
    assert_eq!(pdf.matches("/WFRowGrid true").count(), 3);
    assert!(pdf.contains("/WFRow 0 /WFColumn 0 /WFRowSpan 2 /WFColSpan 1"));
    assert!(pdf.contains("/WFRow 0 /WFColumn 1 /WFRowSpan 1 /WFColSpan 1"));
    assert!(pdf.contains("/WFRow 1 /WFColumn 1 /WFRowSpan 1 /WFColSpan 1"));
}

#[test]
fn reopened_typed_table_mutation_recalculates_dependents_by_exact_owner() {
    use crate::typed_tables::{DecimalValue, TableFormula, TableValue};

    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("reopened-invoice");
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "subtotal",
            TableValue::Decimal {
                value: DecimalValue {
                    coefficient: "1000".into(),
                    scale: 2,
                },
            },
        ),
        TableCell::typed(
            "total",
            TableValue::Formula {
                expression: TableFormula::Add {
                    left: Box::new(TableFormula::Cell {
                        id: "subtotal".into(),
                    }),
                    right: Box::new(TableFormula::Constant {
                        value: DecimalValue {
                            coefficient: "66".into(),
                            scale: 2,
                        },
                    }),
                },
                display_scale: 2,
            },
        ),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "reopened-invoice".into(),
        updates: BTreeMap::from([(
            "subtotal".into(),
            TableValue::Decimal {
                value: DecimalValue {
                    coefficient: "2000".into(),
                    scale: 2,
                },
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert!(output.starts_with(&input));
    assert_eq!(report.previous_values["subtotal"], "10.00");
    assert_eq!(report.values["subtotal"], "20.00");
    assert_eq!(report.values["total"], "20.66");
    assert_eq!(report.changed_cells, vec!["subtotal", "total"]);
    assert_eq!(report.changed_pages, vec![1]);
    let reopened = load_authored_typed_tables(&output).unwrap();
    assert_eq!(reopened[0].cells[0].evaluated, "20.00");
    assert_eq!(reopened[0].cells[1].evaluated, "20.66");
    let sources = inspect_authored_typed_table_sources(&output).unwrap();
    assert_eq!(sources.cells[0].fragments[0].logical_text, "20.00");
    assert_eq!(sources.cells[1].fragments[0].logical_text, "20.66");
    assert!(sources
        .cells
        .iter()
        .flat_map(|cell| &cell.fragments)
        .all(|fragment| fragment
            .region
            .is_some_and(|region| region[0] < region[2] && region[1] < region[3])));
}

#[test]
fn multiple_zero_width_typed_cells_rebind_by_owner_between_insertions() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("empty-pair");
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "left",
            TableValue::Text {
                text: String::new(),
            },
        ),
        TableCell::typed(
            "right",
            TableValue::Text {
                text: String::new(),
            },
        ),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "empty-pair".into(),
        updates: BTreeMap::from([
            (
                "left".into(),
                TableValue::Text {
                    text: "LEFT".into(),
                },
            ),
            (
                "right".into(),
                TableValue::Text {
                    text: "RIGHT".into(),
                },
            ),
        ]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert_eq!(report.changed_cells, vec!["left", "right"]);
    let sources = inspect_authored_typed_table_sources(&output).unwrap();
    assert_eq!(sources.cells[0].cell_id, "left");
    assert_eq!(sources.cells[0].fragments[0].logical_text, "LEFT");
    assert_eq!(sources.cells[1].cell_id, "right");
    assert_eq!(sources.cells[1].fragments[0].logical_text, "RIGHT");
}

#[test]
fn typed_cell_can_be_cleared_and_refilled_without_losing_its_exact_owner() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).identity("clear-refill");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "value",
        TableValue::Text {
            text: "ORIGINAL".into(),
        },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let clear = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "clear-refill".into(),
        updates: BTreeMap::from([(
            "value".into(),
            TableValue::Text {
                text: String::new(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (cleared, cleared_report) = mutate_authored_typed_table(&input, &clear, None).unwrap();
    assert_eq!(cleared_report.values["value"], "");
    let cleared_sources = inspect_authored_typed_table_sources(&cleared).unwrap();
    assert_eq!(cleared_sources.cells[0].fragments.len(), 1);
    assert_eq!(cleared_sources.cells[0].fragments[0].logical_text, "");
    assert_eq!(
        cleared_sources.cells[0].fragments[0].logical_range[0],
        cleared_sources.cells[0].fragments[0].logical_range[1]
    );
    assert!(!cleared_sources.cells[0].fragments[0]
        .source_span_ids
        .is_empty());

    let refill = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&cleared)),
        table_id: "clear-refill".into(),
        updates: BTreeMap::from([(
            "value".into(),
            TableValue::Text {
                text: "REFILLED".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (refilled, refilled_report) = mutate_authored_typed_table(&cleared, &refill, None).unwrap();
    assert_eq!(refilled_report.values["value"], "REFILLED");
    let refilled_sources = inspect_authored_typed_table_sources(&refilled).unwrap();
    assert_eq!(
        refilled_sources.cells[0].fragments[0].logical_text,
        "REFILLED"
    );
}

#[test]
fn continued_typed_cell_can_collapse_into_its_first_owned_region() {
    use crate::typed_tables::TableValue;

    let original = (0..30)
        .map(|index| format!("original line {index:02}\n"))
        .collect::<String>();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).identity("continued-cell");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "description",
        TableValue::Text { text: original },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let before = inspect_authored_typed_table_sources(&input).unwrap();
    assert!(before.cells[0].fragments.len() > 1);
    assert!(before.cells[0]
        .fragments
        .iter()
        .all(|fragment| fragment.region.is_some()));

    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "continued-cell".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: "collapsed".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert_eq!(report.values["description"], "collapsed");
    assert!(report.changed_pages.len() > 1);
    let after = inspect_authored_typed_table_sources(&output).unwrap();
    assert_eq!(after.cells[0].evaluated, "collapsed");
    assert_eq!(
        after.cells[0]
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "collapsed"
    );
    assert_eq!(
        after.cells[0].fragments.len(),
        before.cells[0].fragments.len()
    );
    assert!(after.cells[0].fragments[1..]
        .iter()
        .all(
            |fragment| fragment.logical_range[0] == fragment.logical_range[1]
                && fragment.logical_text.is_empty()
                && !fragment.source_span_ids.is_empty()
        ));
}

#[test]
fn continued_typed_cell_redistributes_one_shaped_paragraph_across_existing_owners() {
    use crate::typed_tables::TableValue;

    let original = (0..34)
        .map(|index| format!("original row {index:02}\n"))
        .collect::<String>();
    let replacement = (0..24)
        .map(|index| format!("replacement row {index:02}\n"))
        .collect::<String>();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).identity("redistributed-cell");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "description",
        TableValue::Text { text: original },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let input_engine = crate::ContentEngine::open_bytes(input.clone()).unwrap();
    let input_contents = (1..=input_engine.page_count().unwrap())
        .map(|page| input_engine.document().get_page(page).unwrap().contents)
        .collect::<Vec<_>>();
    let before = inspect_authored_typed_table_sources(&input).unwrap();
    assert!(before.cells[0].fragments.len() > 1);
    let model = load_authored_typed_tables(&input).unwrap();
    let layout = model[0].cells[0]
        .layout
        .as_ref()
        .expect("new authored table retains mutation layout");
    assert!(layout.font_size > 0.0 && layout.line_spacing > 0.0);
    assert!(before.cells[0]
        .fragments
        .iter()
        .all(|fragment| (fragment.font_size - layout.font_size).abs() <= EPS));

    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "redistributed-cell".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: replacement.clone(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert!(output.starts_with(&input));
    assert_eq!(report.values["description"], replacement);
    let after = inspect_authored_typed_table_sources(&output).unwrap();
    assert_eq!(
        after.cells[0]
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        replacement
    );
    assert!(
        after.cells[0]
            .fragments
            .iter()
            .filter(|fragment| !fragment.logical_text.is_empty())
            .count()
            > 1
    );
    assert_eq!(
        after.cells[0].fragments.len(),
        before.cells[0].fragments.len()
    );
    let output_engine = crate::ContentEngine::open_bytes(output).unwrap();
    let output_contents = (1..=output_engine.page_count().unwrap())
        .map(|page| output_engine.document().get_page(page).unwrap().contents)
        .collect::<Vec<_>>();
    assert_eq!(
        output_contents, input_contents,
        "positioned owner rewrites must not append page-level overlay streams"
    );
}

#[test]
fn single_cell_typed_table_allocates_tagged_canonical_continuation_pages() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)])
        .identity("growing-single-cell")
        .style(
            TableStyle::new()
                .padding(6.0)
                .border(Color::device_rgb(0.1, 0.2, 0.3), 0.75)
                .row_fill(Some(Color::device_rgb(0.92, 0.94, 0.98))),
        );
    table.push_row(TableRow::new(vec![TableCell::typed(
        "description",
        TableValue::Text {
            text: "short".into(),
        },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let before_pages = crate::ContentEngine::open_bytes(input.clone())
        .unwrap()
        .page_count()
        .unwrap();
    let before = inspect_authored_typed_table_sources(&input).unwrap();
    assert_eq!(before.cells[0].fragments.len(), 1);
    let retained = load_authored_typed_tables(&input).unwrap();
    let pagination = retained[0]
        .pagination
        .as_ref()
        .expect("new authored table retains continuation geometry");
    assert_eq!(pagination.page_size, [220.0, 100.0]);
    assert!(retained[0].cells[0]
        .layout
        .as_ref()
        .and_then(|layout| layout.paint.as_ref())
        .is_some());

    let replacement = (0..36)
        .map(|index| format!("grown line {index:02}\n"))
        .collect::<String>();
    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "growing-single-cell".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: replacement.clone(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert!(!output.starts_with(&input));
    assert!(!report.original_prefix_preserved);
    let output_engine = crate::ContentEngine::open_bytes(output.clone()).unwrap();
    assert!(output_engine.page_count().unwrap() > before_pages);
    let after = inspect_authored_typed_table_sources(&output).unwrap();
    assert!(after.cells[0].fragments.len() > before.cells[0].fragments.len());
    assert_eq!(
        after.cells[0]
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        replacement
    );
    assert!(after.cells[0]
        .fragments
        .iter()
        .all(|fragment| fragment.region.is_some()));
    assert!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn fully_typed_row_growth_allocates_every_cell_owner_atomically() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![
        TableColumn::new(80.0),
        TableColumn::new(80.0).align(TextAlign::Right),
    ])
    .identity("growing-typed-row");
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "description",
            TableValue::Text {
                text: "short".into(),
            },
        )
        .row_header(),
        TableCell::typed(
            "amount",
            TableValue::Text {
                text: "12.34".into(),
            },
        ),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let retained = load_authored_typed_tables(&input).unwrap();
    let retained_description = retained[0]
        .cells
        .iter()
        .find(|cell| cell.id == "description")
        .unwrap();
    assert_eq!(
        retained_description.structure_role,
        AuthoredTypedCellRole::Header
    );
    assert_eq!(
        retained_description.header_scope,
        Some(AuthoredTypedHeaderScope::Row)
    );
    let retained_amount = retained[0]
        .cells
        .iter()
        .find(|cell| cell.id == "amount")
        .unwrap();
    assert_eq!(retained_amount.structure_role, AuthoredTypedCellRole::Data);
    assert_eq!(retained_amount.header_scope, None);
    let mut malformed = retained[0].clone();
    malformed
        .cells
        .iter_mut()
        .find(|cell| cell.id == "amount")
        .unwrap()
        .header_scope = Some(AuthoredTypedHeaderScope::Column);
    assert!(malformed.validate().is_err());
    let before = inspect_authored_typed_table_sources(&input).unwrap();
    assert!(before.cells.iter().all(|cell| cell.fragments.len() == 1));
    let replacement = (0..30)
        .map(|index| format!("expanded description {index:02}\n"))
        .collect::<String>();
    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "growing-typed-row".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: replacement.clone(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert_eq!(report.changed_cells, vec!["description"]);
    assert!(!report.original_prefix_preserved);
    let after = inspect_authored_typed_table_sources(&output).unwrap();
    let description = after
        .cells
        .iter()
        .find(|cell| cell.cell_id == "description")
        .unwrap();
    let amount = after
        .cells
        .iter()
        .find(|cell| cell.cell_id == "amount")
        .unwrap();
    assert_eq!(
        description
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        replacement
    );
    assert_eq!(
        amount
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "12.34"
    );
    assert_eq!(description.fragments.len(), amount.fragments.len());
    assert!(description.fragments.len() > 1);
    assert!(amount.fragments[1..].iter().all(|fragment| {
        fragment.logical_text.is_empty()
            && fragment.logical_range[0] == fragment.logical_range[1]
            && !fragment.source_span_ids.is_empty()
    }));
    let grid = inspect_authored_typed_table_grid_paint(&output, "growing-typed-row").unwrap();
    assert!(grid.complete_typed_grid_ownership);
    assert_eq!(
        grid.fragments.len(),
        description.fragments.len() + amount.fragments.len()
    );
    let output_engine = crate::ContentEngine::open_bytes(output.clone()).unwrap();
    for fragment in &description.fragments[1..] {
        let page = output_engine.document().get_page(fragment.page).unwrap();
        let dictionary = output_engine
            .document()
            .reader()
            .get_object(page.object_number, page.generation_number)
            .unwrap();
        assert!(is_authored_typed_table_continuation(
            dictionary.as_dict().unwrap(),
            "growing-typed-row",
            0,
            2,
        ));
    }
    assert!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn fully_typed_row_shrink_prunes_only_owned_empty_continuations() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("pruned-typed-row");
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "description",
            TableValue::Text {
                text: "short".into(),
            },
        ),
        TableCell::typed(
            "amount",
            TableValue::Text {
                text: "12.34".into(),
            },
        ),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let original = rendered.builder.to_bytes().unwrap();
    let original_pages = crate::ContentEngine::open_bytes(original.clone())
        .unwrap()
        .page_count()
        .unwrap();
    let long = (0..36)
        .map(|index| format!("continuation line {index:02}\n"))
        .collect::<String>();
    let grow = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&original)),
        table_id: "pruned-typed-row".into(),
        updates: BTreeMap::from([("description".into(), TableValue::Text { text: long })]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (grown, growth_report) = mutate_authored_typed_table(&original, &grow, None).unwrap();
    assert!(growth_report.removed_pages.is_empty());
    let grown_pages = crate::ContentEngine::open_bytes(grown.clone())
        .unwrap()
        .page_count()
        .unwrap();
    assert!(grown_pages > original_pages);

    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&grown)),
        table_id: "pruned-typed-row".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: "short again".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (shrunk, report) = mutate_authored_typed_table(&grown, &shrink, None).unwrap();
    assert_eq!(report.removed_pages.len(), grown_pages - original_pages);
    assert!(report.retained_continuation_pages.is_empty());
    assert!(!report.original_prefix_preserved);
    assert_eq!(
        crate::ContentEngine::open_bytes(shrunk.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        original_pages
    );
    let sources = inspect_authored_typed_table_sources(&shrunk).unwrap();
    assert!(sources.cells.iter().all(|cell| cell.fragments.len() == 1));
    assert_eq!(
        sources
            .cells
            .iter()
            .find(|cell| cell.cell_id == "description")
            .unwrap()
            .fragments[0]
            .logical_text,
        "short again"
    );
    assert!(
        crate::tagged_structure::validate_parent_tree(&shrunk)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn repeatable_header_growth_repaints_exact_artifact_and_remains_prunable() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![
        TableColumn::new(100.0),
        TableColumn::new(60.0).align(TextAlign::Right),
    ])
    .identity("repeatable-header-growth")
    .style(
        TableStyle::new()
            .padding(5.0)
            .border(Color::device_rgb(0.15, 0.2, 0.3), 0.8)
            .header_fill(Color::device_rgb(0.82, 0.9, 0.96)),
    )
    .header_style(TextStyle::unicode(10.0).fill(Color::device_rgb(0.05, 0.1, 0.2)));
    table.set_header([
        TableCell::text("Description").align(TextAlign::Center),
        TableCell::text("Amount").align(TextAlign::Right),
    ]);
    table.push_row(TableRow::new(vec![
        TableCell::typed(
            "description",
            TableValue::Text {
                text: "short".into(),
            },
        ),
        TableCell::typed(
            "amount",
            TableValue::Text {
                text: "12.34".into(),
            },
        ),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let original = rendered.builder.to_bytes().unwrap();
    let original_pages = crate::ContentEngine::open_bytes(original.clone())
        .unwrap()
        .page_count()
        .unwrap();
    let retained = load_authored_typed_tables(&original).unwrap();
    let header = retained[0]
        .pagination
        .as_ref()
        .and_then(|pagination| pagination.repeatable_header.as_ref())
        .expect("resolvable authored header is retained");
    assert_eq!(
        header
            .cells
            .iter()
            .map(|cell| cell.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Description", "Amount"]
    );
    assert_eq!(header.cells[0].column, 0);
    assert_eq!(header.cells[1].column, 1);
    assert!(header.cells.iter().all(|cell| matches!(
        &cell.font,
        AuthoredRetainedFont::Embedded { base_name }
            if base_name == BUILTIN_UNICODE_RESOURCE_NAME
    )));
    assert!(header.height > 10.0);

    let long = (0..42)
        .map(|index| format!("repeatable header line {index:02}\n"))
        .collect::<String>();
    let grow = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&original)),
        table_id: "repeatable-header-growth".into(),
        updates: BTreeMap::from([("description".into(), TableValue::Text { text: long })]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (grown, _) = mutate_authored_typed_table(&original, &grow, None).unwrap();
    let grown_engine = crate::ContentEngine::open_bytes(grown.clone()).unwrap();
    assert!(grown_engine.page_count().unwrap() > original_pages);
    let sources = inspect_authored_typed_table_sources(&grown).unwrap();
    assert_eq!(sources.cells.len(), 2);
    let description = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "description")
        .unwrap();
    for fragment in &description.fragments[1..] {
        let page_text = grown_engine
            .collect_page_text_chunks(fragment.page)
            .unwrap()
            .into_iter()
            .map(|chunk| chunk.text)
            .collect::<String>();
        assert!(page_text.contains("Description"));
        assert!(page_text.contains("Amount"));
        let page = grown_engine.document().get_page(fragment.page).unwrap();
        let dictionary = grown_engine
            .document()
            .reader()
            .get_object(page.object_number, page.generation_number)
            .unwrap();
        assert!(is_authored_typed_table_continuation(
            dictionary.as_dict().unwrap(),
            "repeatable-header-growth",
            0,
            2,
        ));
    }
    assert!(
        crate::tagged_structure::validate_parent_tree(&grown)
            .unwrap()
            .ownership_verified
    );

    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&grown)),
        table_id: "repeatable-header-growth".into(),
        updates: BTreeMap::from([(
            "description".into(),
            TableValue::Text {
                text: "short again".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (shrunk, report) = mutate_authored_typed_table(&grown, &shrink, None).unwrap();
    assert!(!report.removed_pages.is_empty());
    assert!(report.retained_continuation_pages.is_empty());
    assert_eq!(
        crate::ContentEngine::open_bytes(shrunk)
            .unwrap()
            .page_count()
            .unwrap(),
        original_pages
    );
}

#[test]
fn separated_multirow_typed_table_grows_and_prunes_before_downstream_rows() {
    use crate::typed_tables::TableValue;

    let typed = |id: &str, text: &str| {
        TableCell::typed(
            id,
            TableValue::Text {
                text: text.to_string(),
            },
        )
    };
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("separated-multirow-growth");
    table.push_row(TableRow::new(vec![
        typed("first-description", "short"),
        typed("first-amount", "12.34"),
    ]));
    table.push_row(
        TableRow::new(vec![
            typed("second-description", "downstream"),
            typed("second-amount", "56.78"),
        ])
        .page_break_before(FlowPageBreak::NextPage),
    );
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let original = rendered.builder.to_bytes().unwrap();
    let original_pages = crate::ContentEngine::open_bytes(original.clone())
        .unwrap()
        .page_count()
        .unwrap();
    assert_eq!(original_pages, 2);
    assert_eq!(
        load_authored_typed_tables(&original).unwrap()[0]
            .pagination
            .as_ref()
            .unwrap()
            .row_page_breaks
            .as_deref(),
        Some(&[None, Some(FlowPageBreak::NextPage)][..])
    );
    let before = inspect_authored_typed_table_sources(&original).unwrap();
    assert_eq!(
        before
            .cells
            .iter()
            .find(|cell| cell.cell_id == "first-description")
            .unwrap()
            .fragments[0]
            .page,
        1
    );
    assert_eq!(
        before
            .cells
            .iter()
            .find(|cell| cell.cell_id == "second-description")
            .unwrap()
            .fragments[0]
            .page,
        2
    );

    let replacement = (0..42)
        .map(|index| format!("first row continuation {index:02}\n"))
        .collect::<String>();
    let grow = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&original)),
        table_id: "separated-multirow-growth".into(),
        updates: BTreeMap::from([(
            "first-description".into(),
            TableValue::Text {
                text: replacement.clone(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (grown, _) = mutate_authored_typed_table(&original, &grow, None).unwrap();
    let grown_sources = inspect_authored_typed_table_sources(&grown).unwrap();
    let first = grown_sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "first-description")
        .unwrap();
    let second = grown_sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "second-description")
        .unwrap();
    assert_eq!(
        first
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        replacement
    );
    let continuation_pages = first
        .fragments
        .iter()
        .skip(1)
        .map(|fragment| fragment.page)
        .collect::<BTreeSet<_>>();
    assert!(!continuation_pages.is_empty());
    assert!(second.fragments[0].page > *continuation_pages.last().unwrap());
    let grown_engine = crate::ContentEngine::open_bytes(grown.clone()).unwrap();
    for &page_number in &continuation_pages {
        let page = grown_engine.document().get_page(page_number).unwrap();
        let dictionary = grown_engine
            .document()
            .reader()
            .get_object(page.object_number, page.generation_number)
            .unwrap();
        assert!(is_authored_typed_table_continuation(
            dictionary.as_dict().unwrap(),
            "separated-multirow-growth",
            0,
            2,
        ));
    }
    let grid =
        inspect_authored_typed_table_grid_paint(&grown, "separated-multirow-growth").unwrap();
    assert!(grid.complete_typed_grid_ownership);
    assert!(
        crate::tagged_structure::validate_parent_tree(&grown)
            .unwrap()
            .ownership_verified
    );

    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&grown)),
        table_id: "separated-multirow-growth".into(),
        updates: BTreeMap::from([(
            "first-description".into(),
            TableValue::Text {
                text: "short again".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (shrunk, report) = mutate_authored_typed_table(&grown, &shrink, None).unwrap();
    assert_eq!(
        report
            .removed_pages
            .iter()
            .copied()
            .collect::<BTreeSet<_>>(),
        continuation_pages
    );
    let shrunk_engine = crate::ContentEngine::open_bytes(shrunk.clone()).unwrap();
    assert_eq!(shrunk_engine.page_count().unwrap(), original_pages);
    let shrunk_sources = inspect_authored_typed_table_sources(&shrunk).unwrap();
    assert_eq!(
        shrunk_sources
            .cells
            .iter()
            .find(|cell| cell.cell_id == "second-description")
            .unwrap()
            .fragments[0]
            .page,
        2
    );
    assert!(
        crate::tagged_structure::validate_parent_tree(&shrunk)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn same_page_downstream_typed_rows_relocate_after_growing_row() {
    use crate::typed_tables::TableValue;

    let typed = |id: &str, text: &str| {
        TableCell::typed(
            id,
            TableValue::Text {
                text: text.to_string(),
            },
        )
    };
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("same-page-multirow-relocation");
    table.push_row(TableRow::new(vec![
        typed("first-left", "short"),
        typed("first-right", "A"),
    ]));
    table.push_row(TableRow::new(vec![
        typed("second-left", "second"),
        typed("second-right", "B"),
    ]));
    table.push_row(TableRow::new(vec![
        typed("third-left", "third"),
        typed("third-right", "C"),
    ]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let input = rendered.builder.to_bytes().unwrap();
    let before = inspect_authored_typed_table_sources(&input).unwrap();
    assert!(before
        .cells
        .iter()
        .all(|cell| cell.fragments.iter().all(|fragment| fragment.page == 1)));
    let request = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&input)),
        table_id: "same-page-multirow-relocation".into(),
        updates: BTreeMap::from([(
            "first-left".into(),
            TableValue::Text {
                text: "overflow\n".repeat(40),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (output, report) = mutate_authored_typed_table(&input, &request, None).unwrap();
    assert!(!report.original_prefix_preserved);
    assert_eq!(
        report
            .changed_cells
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "first-left",
            "second-left",
            "second-right",
            "third-left",
            "third-right",
        ])
    );
    let sources = inspect_authored_typed_table_sources(&output).unwrap();
    let first = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "first-left")
        .unwrap();
    let second_left = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "second-left")
        .unwrap();
    let second_right = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "second-right")
        .unwrap();
    let third_left = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "third-left")
        .unwrap();
    let third_right = sources
        .cells
        .iter()
        .find(|cell| cell.cell_id == "third-right")
        .unwrap();
    let last_first_page = first
        .fragments
        .iter()
        .map(|fragment| fragment.page)
        .max()
        .unwrap();
    assert!(first.fragments.len() > 1);
    assert!(second_left
        .fragments
        .iter()
        .all(|fragment| fragment.page > last_first_page));
    assert_eq!(
        second_left.fragments[0].page,
        second_right.fragments[0].page
    );
    assert_eq!(second_left.fragments[0].page, third_left.fragments[0].page);
    assert_eq!(third_left.fragments[0].page, third_right.fragments[0].page);
    assert_eq!(
        second_left
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "second"
    );
    assert_eq!(
        second_right
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "B"
    );
    assert_eq!(
        third_left
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "third"
    );
    assert_eq!(
        third_right
            .fragments
            .iter()
            .map(|fragment| fragment.logical_text.as_str())
            .collect::<String>(),
        "C"
    );
    let grid =
        inspect_authored_typed_table_grid_paint(&output, "same-page-multirow-relocation").unwrap();
    assert!(grid.complete_typed_grid_ownership);
    assert!(grid
        .fragments
        .iter()
        .filter(|fragment| fragment.row == 1 || fragment.row == 2)
        .all(|fragment| fragment.page == second_left.fragments[0].page));
    assert!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .ownership_verified
    );

    let mut grown_models = load_authored_typed_tables(&output).unwrap();
    let grown_model = grown_models.remove(0);
    let relocation = inspect_authored_typed_table_relocations(&output, &grown_model)
        .unwrap()
        .remove(0);
    let grown_engine = crate::ContentEngine::open_bytes(output.clone()).unwrap();
    let destination = grown_engine
        .document()
        .get_page(relocation.destination_page)
        .unwrap();
    let mut destination_dictionary = grown_engine
        .document()
        .reader()
        .get_object(destination.object_number, destination.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    destination_dictionary.insert("WFExternalPageState", PdfObject::Boolean(true));
    let decorated = crate::writer::write_incremental_update(
        grown_engine.document().reader(),
        vec![crate::writer::IncrementalObject {
            number: destination.object_number,
            generation: destination.generation_number,
            object: PdfObject::Dictionary(destination_dictionary),
        }],
    )
    .unwrap();
    let guarded_shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&decorated)),
        table_id: "same-page-multirow-relocation".into(),
        updates: BTreeMap::from([(
            "first-left".into(),
            TableValue::Text {
                text: "short guarded".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (guarded, guarded_report) =
        mutate_authored_typed_table(&decorated, &guarded_shrink, None).unwrap();
    assert!(guarded_report
        .retained_continuation_pages
        .iter()
        .any(|retained| retained.reason == "relocation_destination_page_features"));
    assert!(!inspect_authored_typed_table_relocations(
        &guarded,
        &load_authored_typed_tables(&guarded).unwrap()[0]
    )
    .unwrap()
    .is_empty());

    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&output)),
        table_id: "same-page-multirow-relocation".into(),
        updates: BTreeMap::from([(
            "first-left".into(),
            TableValue::Text {
                text: "short again".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (compacted, compacted_report) =
        mutate_authored_typed_table(&output, &shrink, None).unwrap();
    assert_eq!(
        crate::ContentEngine::open_bytes(compacted.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        1
    );
    assert!(!compacted_report.removed_pages.is_empty());
    assert!(compacted_report.retained_continuation_pages.is_empty());
    assert!(inspect_authored_typed_table_relocations(
        &compacted,
        &load_authored_typed_tables(&compacted).unwrap()[0]
    )
    .unwrap()
    .is_empty());
    let compacted_sources = inspect_authored_typed_table_sources(&compacted).unwrap();
    for (cell, value) in [
        ("first-left", "short again"),
        ("first-right", "A"),
        ("second-left", "second"),
        ("second-right", "B"),
        ("third-left", "third"),
        ("third-right", "C"),
    ] {
        let source = compacted_sources
            .cells
            .iter()
            .find(|candidate| candidate.cell_id == cell)
            .unwrap();
        assert_eq!(source.evaluated, value);
        assert!(source.fragments.iter().all(|fragment| fragment.page == 1));
    }
    assert!(
        inspect_authored_typed_table_grid_paint(&compacted, "same-page-multirow-relocation")
            .unwrap()
            .complete_typed_grid_ownership
    );
    assert!(
        crate::tagged_structure::validate_parent_tree(&compacted)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn nested_same_page_relocations_compact_child_before_parent() {
    use crate::typed_tables::TableValue;

    let typed = |id: &str, text: &str| {
        TableCell::typed(
            id,
            TableValue::Text {
                text: text.to_string(),
            },
        )
    };
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .identity("nested-row-relocation");
    for row in 0..5 {
        table.push_row(TableRow::new(vec![
            typed(&format!("r{row}-left"), &format!("row {row}")),
            typed(&format!("r{row}-right"), &format!("value {row}")),
        ]));
    }
    let mut rendered = FlowDocument::new(PageSize::custom(220.0, 160.0), Margins::all(10.0));
    rendered.add_table_with_report(&table).unwrap();
    let original = rendered.builder.to_bytes().unwrap();
    assert_eq!(
        crate::ContentEngine::open_bytes(original.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        1
    );

    let grow_first = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&original)),
        table_id: "nested-row-relocation".into(),
        updates: BTreeMap::from([(
            "r0-left".into(),
            TableValue::Text {
                // Several continuation pages plus the relocated downstream
                // row are sufficient to create a parent relocation receipt.
                // A much larger synthetic payload only repeats the same page
                // transaction and made this correctness test dominate the
                // entire workspace run; scale is covered by corpus benchmarks.
                text: "first overflow\n".repeat(8),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (first, _) = mutate_authored_typed_table(&original, &grow_first, None).unwrap();
    assert_eq!(
        inspect_authored_typed_table_relocations(
            &first,
            &load_authored_typed_tables(&first).unwrap()[0]
        )
        .unwrap()
        .len(),
        1
    );

    let grow_second = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&first)),
        table_id: "nested-row-relocation".into(),
        updates: BTreeMap::from([(
            "r1-left".into(),
            TableValue::Text {
                // Grow the already-relocated child row far enough to create a
                // second receipt whose origin is the parent's destination.
                text: "second overflow\n".repeat(8),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (second, _) = mutate_authored_typed_table(&first, &grow_second, None).unwrap();
    let second_model = &load_authored_typed_tables(&second).unwrap()[0];
    let receipts = inspect_authored_typed_table_relocations(&second, second_model).unwrap();
    assert_eq!(receipts.len(), 2);
    assert!(receipts[0].destination_page < receipts[1].destination_page);
    let parent_destination = receipts[0].destination_page;
    assert_eq!(receipts[1].origin_page, parent_destination);

    // An external change on the parent's destination must be detected before
    // child restoration mutates that same page. Otherwise refreshing the
    // parent receipt after the child transaction would legitimize the change.
    let guarded_engine = crate::ContentEngine::open_bytes(second.clone()).unwrap();
    let guarded_page = guarded_engine
        .document()
        .get_page(parent_destination)
        .unwrap();
    let mut guarded_dictionary = guarded_engine
        .document()
        .reader()
        .get_object(guarded_page.object_number, guarded_page.generation_number)
        .unwrap()
        .as_dict()
        .unwrap()
        .clone();
    guarded_dictionary.insert("WFNestedExternalPageState", PdfObject::Boolean(true));
    let guarded_input = crate::writer::write_incremental_update(
        guarded_engine.document().reader(),
        vec![crate::writer::IncrementalObject {
            number: guarded_page.object_number,
            generation: guarded_page.generation_number,
            object: PdfObject::Dictionary(guarded_dictionary),
        }],
    )
    .unwrap();
    let guarded_shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&guarded_input)),
        table_id: "nested-row-relocation".into(),
        updates: BTreeMap::from([
            (
                "r0-left".into(),
                TableValue::Text {
                    text: "first guarded".into(),
                },
            ),
            (
                "r1-left".into(),
                TableValue::Text {
                    text: "second guarded".into(),
                },
            ),
        ]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (guarded_output, guarded_report) =
        mutate_authored_typed_table(&guarded_input, &guarded_shrink, None).unwrap();
    assert!(guarded_report
        .retained_continuation_pages
        .iter()
        .any(|retained| retained.reason == "parent_relocation_destination_changed"));
    assert_eq!(
        inspect_authored_typed_table_relocations(
            &guarded_output,
            &load_authored_typed_tables(&guarded_output).unwrap()[0]
        )
        .unwrap()
        .len(),
        2
    );

    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&second)),
        table_id: "nested-row-relocation".into(),
        updates: BTreeMap::from([
            ("r0-left".into(), TableValue::Text { text: "one".into() }),
            ("r1-left".into(), TableValue::Text { text: "two".into() }),
        ]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (compacted, report) = mutate_authored_typed_table(&second, &shrink, None).unwrap();
    assert_eq!(
        crate::ContentEngine::open_bytes(compacted.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        1,
        "retained={:?}",
        report.retained_continuation_pages
    );
    assert!(report.retained_continuation_pages.is_empty());
    assert!(report.removed_pages.len() >= 4);
    assert!(inspect_authored_typed_table_relocations(
        &compacted,
        &load_authored_typed_tables(&compacted).unwrap()[0]
    )
    .unwrap()
    .is_empty());
    let sources = inspect_authored_typed_table_sources(&compacted).unwrap();
    assert!(sources
        .cells
        .iter()
        .all(|cell| cell.fragments.iter().all(|fragment| fragment.page == 1)));
    assert!(
        inspect_authored_typed_table_grid_paint(&compacted, "nested-row-relocation")
            .unwrap()
            .complete_typed_grid_ownership
    );
    assert!(
        crate::tagged_structure::validate_parent_tree(&compacted)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn typed_row_shrink_retains_continuations_with_added_page_state() {
    use crate::typed_tables::TableValue;

    let mut table =
        TableBuilder::new(vec![TableColumn::new(160.0)]).identity("retained-page-state");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "value",
        TableValue::Text {
            text: "short".into(),
        },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let original = rendered.builder.to_bytes().unwrap();
    let long = (0..30)
        .map(|index| format!("retained line {index:02}\n"))
        .collect::<String>();
    let grow = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&original)),
        table_id: "retained-page-state".into(),
        updates: BTreeMap::from([("value".into(), TableValue::Text { text: long })]),
        signature_policy_override: false,
        prune_empty_continuations: false,
    };
    let (grown, _) = mutate_authored_typed_table(&original, &grow, None).unwrap();
    let sources = inspect_authored_typed_table_sources(&grown).unwrap();
    let continuation_pages = sources.cells[0]
        .fragments
        .iter()
        .skip(1)
        .map(|fragment| fragment.page)
        .collect::<BTreeSet<_>>();
    assert!(!continuation_pages.is_empty());
    let engine = crate::ContentEngine::open_bytes(grown).unwrap();
    let reader = engine.document().reader();
    let mut updates = Vec::new();
    for &page_number in &continuation_pages {
        let page = engine.document().get_page(page_number).unwrap();
        let mut dictionary = reader
            .get_object(page.object_number, page.generation_number)
            .unwrap()
            .as_dict()
            .unwrap()
            .clone();
        dictionary.insert("WFExternalPageState", PdfObject::Boolean(true));
        updates.push(crate::writer::IncrementalObject {
            number: page.object_number,
            generation: page.generation_number,
            object: PdfObject::Dictionary(dictionary),
        });
    }
    let decorated = crate::writer::write_incremental_update(reader, updates).unwrap();
    let decorated_pages = engine.page_count().unwrap();
    let shrink = AuthoredTypedTableMutationRequest {
        input_sha256: format!("{:x}", Sha256::digest(&decorated)),
        table_id: "retained-page-state".into(),
        updates: BTreeMap::from([(
            "value".into(),
            TableValue::Text {
                text: "short again".into(),
            },
        )]),
        signature_policy_override: false,
        prune_empty_continuations: true,
    };
    let (output, report) = mutate_authored_typed_table(&decorated, &shrink, None).unwrap();
    assert!(report.removed_pages.is_empty());
    assert_eq!(
        report
            .retained_continuation_pages
            .iter()
            .map(|page| (page.page, page.reason.as_str()))
            .collect::<BTreeSet<_>>(),
        continuation_pages
            .iter()
            .map(|page| (*page, "additional_page_features"))
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        crate::ContentEngine::open_bytes(output.clone())
            .unwrap()
            .page_count()
            .unwrap(),
        decorated_pages
    );
    assert!(
        crate::tagged_structure::validate_parent_tree(&output)
            .unwrap()
            .ownership_verified
    );
}

#[test]
fn typed_cell_region_cannot_escape_its_authored_page() {
    use crate::typed_tables::TableValue;

    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).identity("region-guard");
    table.push_row(TableRow::new(vec![TableCell::typed(
        "value",
        TableValue::Text {
            text: "bounded".into(),
        },
    )]));
    let mut rendered = flow();
    rendered.add_table_with_report(&table).unwrap();
    let page_width = rendered.builder.pages[0].size.width;
    let command = rendered.builder.pages[0]
        .commands
        .iter_mut()
        .find(|command| matches!(command, PageCommand::BeginTypedCellStructure { .. }))
        .expect("typed cell structure command");
    let PageCommand::BeginTypedCellStructure { region, .. } = command else {
        unreachable!();
    };
    region[2] = page_width + 10.0;
    assert!(rendered.builder.to_bytes().is_err());
}

fn assert_page_bounds(report: &TableFlowReport, flow: &FlowDocument) {
    for fragment in &report.fragments {
        assert!(fragment.page > 0 && fragment.page <= flow.builder.pages.len());
        assert!(fragment.top <= flow.page_size.height - flow.margins.top + EPS);
        assert!(fragment.top - fragment.height >= flow.margins.bottom - EPS);
    }
}

#[test]
fn oversized_rows_split_without_clipping_and_reopen_with_every_logical_line() {
    let mut flow = flow();
    let text = (0..30)
        .map(|i| format!("Line {i:02}\r\n"))
        .collect::<String>();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row([text.as_str()]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert!(report.fragments.len() > 2);
    assert_eq!(report.added_pages, report.fragments.len() - 1);
    assert_source_partition(&table, &report);
    assert_page_bounds(&report, &flow);
    assert_eq!(
        flow.builder.pages.iter().map(logical).collect::<String>(),
        text
    );
    let engine = crate::ContentEngine::open_bytes(flow.builder.to_bytes().unwrap()).unwrap();
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
fn repeated_headers_are_whole_artifacts_and_stay_with_a_body_fragment() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.set_header(["Column Header"]);
    table.add_row(["Body\n".repeat(30)]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert_source_partition(&table, &report);
    assert_page_bounds(&report, &flow);
    let headers = report
        .fragments
        .iter()
        .filter(|f| f.row.is_none())
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), flow.builder.pages.len());
    assert!(!headers[0].repeated_header);
    assert!(headers[1..].iter().all(|f| f.repeated_header));
    for header in headers {
        assert!(report
            .fragments
            .iter()
            .any(|f| f.page == header.page && f.row == Some(0)));
        assert_eq!(header.cell_utf8_ranges, vec![[0, "Column Header".len()]]);
    }
    let font_plan = FontBuildPlan::from_builder(&flow.builder).unwrap();
    let image_plan = ImageBuildPlan::from_builder(&flow.builder).unwrap();
    let mut next_object = 10_000;
    let structures = structure::build(
        &flow.builder,
        1_000,
        &mut next_object,
        &[],
        flow.builder.pages.len() as u32,
    )
    .unwrap();
    for (index, page) in flow.builder.pages.iter().enumerate() {
        let mut artifact = false;
        let mut artifact_text = String::new();
        for command in &page.commands {
            match command {
                PageCommand::BeginArtifact => {
                    assert!(!artifact);
                    artifact = true;
                }
                PageCommand::EndArtifact => {
                    assert!(artifact);
                    artifact = false;
                }
                PageCommand::Text { text, .. } if artifact => artifact_text.push_str(text),
                _ => (),
            }
        }
        assert!(!artifact);
        assert_eq!(artifact_text, if index == 0 { "" } else { "Column Header" });
        let content = String::from_utf8(
            build_content_stream_with_structure(page, &font_plan, &image_plan, &structures)
                .unwrap(),
        )
        .unwrap();
        assert!(content.contains("/Artifact BMC"));
    }
}

#[test]
fn completed_short_cells_do_not_repeat_on_tall_neighbor_continuations() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![TableColumn::new(70.0), TableColumn::new(100.0)]);
    table.add_row(["Short".to_string(), "Tall\n".repeat(25)]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert!(report.fragments.len() > 1);
    assert_eq!(report.fragments[0].cell_utf8_ranges[0], [0, 5]);
    assert!(report.fragments[1..]
        .iter()
        .all(|f| f.cell_utf8_ranges[0] == [5, 5]));
    assert_source_partition(&table, &report);
    assert_eq!(
        flow.builder
            .pages
            .iter()
            .map(logical)
            .collect::<String>()
            .matches("Short")
            .count(),
        1
    );
}

#[test]
fn small_rows_move_intact_to_the_next_page_without_orphaning_headers() {
    let mut flow = flow();
    flow.add_spacer(60.0);
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.set_header(["Header"]);
    table.add_row(["Body"]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert_eq!(report.added_pages, 1);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert_eq!(report.fragments.len(), 2);
    assert!(report.fragments.iter().all(|f| f.page == 2));
    assert_eq!(logical(&flow.builder.pages[1]), "HeaderBody");
}

#[test]
fn row_owned_odd_page_break_accounts_for_parity_and_repeats_header_only_at_destination() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.set_header(["Header"]);
    table.add_row(["First"]);
    table.push_row(
        TableRow::new(vec![TableCell::text("Second")])
            .page_break_before(FlowPageBreak::NextOddPage),
    );

    let report = flow.add_table_with_report(&table).unwrap();
    assert_eq!(flow.builder.pages.len(), 3);
    assert_eq!(report.added_pages, 2);
    assert_eq!(report.page_breaks.len(), 1);
    assert_eq!(
        report.page_breaks[0],
        TableRowPageBreakInfo {
            row: 1,
            policy: FlowPageBreak::NextOddPage,
            from_page: 1,
            to_page: 3,
            added_pages: 2,
        }
    );
    assert_eq!(logical(&flow.builder.pages[0]), "HeaderFirst");
    assert_eq!(logical(&flow.builder.pages[1]), "");
    assert_eq!(logical(&flow.builder.pages[2]), "HeaderSecond");
    assert!(flow.builder.pages[1].suppress_section_master);
    assert!(!flow.builder.pages[2].suppress_section_master);
    assert_source_partition(&table, &report);
    assert_page_bounds(&report, &flow);
}

#[test]
fn page_local_table_refuses_row_page_commands_without_partial_paint() {
    let mut page = PdfPageBuilder::new(PageSize::custom(220.0, 100.0));
    page.draw_text("before", 10.0, 80.0, &TextStyle::default())
        .unwrap();
    let commands = page.commands.len();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.push_row(
        TableRow::new(vec![TableCell::text("Body")]).page_break_before(FlowPageBreak::NextPage),
    );
    assert!(table.draw_on_page(&mut page, 10.0, 60.0).is_err());
    assert_eq!(page.commands.len(), commands);
    assert_eq!(logical(&page), "before");
}

#[test]
fn failure_after_forced_row_transition_rolls_pages_receipts_and_content_back() {
    let mut flow = flow();
    flow.add_paragraph("before", &TextStyle::default(), &ParagraphStyle::default())
        .unwrap();
    let cursor = flow.cursor_y;
    let commands = flow.builder.pages[0].commands.len();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row(["valid"]);
    table.push_row(
        TableRow::new(vec![TableCell::text("🚀")]).page_break_before(FlowPageBreak::NextEvenPage),
    );
    assert!(flow.add_table_with_report(&table).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert_eq!(logical(&flow.builder.pages[0]), "before");
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.current_page, 0);
}

#[test]
fn explicit_keep_together_refuses_an_oversized_row_transactionally() {
    let mut flow = flow();
    flow.add_paragraph("before", &TextStyle::default(), &ParagraphStyle::default())
        .unwrap();
    let cursor = flow.cursor_y;
    let original = flow.builder.pages[0].commands.len();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)])
        .row_split_policy(TableRowSplitPolicy::KeepTogether);
    table.set_header(["Header"]);
    table.add_row(["A\n".repeat(30)]);
    assert!(flow.add_table(&table).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert_eq!(flow.builder.pages[0].commands.len(), original);
    assert_eq!(logical(&flow.builder.pages[0]), "before");
    assert_eq!(flow.cursor_y, cursor);
}

fn synthetic_heights(heights: &[f64]) -> PreparedRow {
    let page = PdfPageBuilder::new(PageSize::LETTER);
    let source = "A\n".repeat(heights.len());
    let mut table = TableBuilder::new(vec![TableColumn::new(100.0)])
        .style(TableStyle::default().padding(0.0))
        .body_style(TextStyle::unicode(1.0));
    table.add_row([source]);
    let mut row =
        PreparedRow::new(&table, &page, &table.rows[0], false, &mut Budget::default()).unwrap();
    let mut prefix = vec![0.0];
    for &height in heights {
        prefix.push(prefix.last().unwrap() + height);
    }
    row.cells[0].heights = prefix;
    row.minimum_height = 1.0;
    row
}

#[test]
fn suffix_feasibility_avoids_greedy_dead_ends_with_unequal_line_heights() {
    // Greedy takes three short lines, then strands [9, 1, 1] with a two-line
    // minimum. The feasible partition is [1,1] / [1,9] / [1,1].
    let row = synthetic_heights(&[1.0, 1.0, 1.0, 9.0, 1.0, 1.0]);
    let policy = TableRowSplitPolicy::Lines {
        min_fragment_lines: 2,
        min_final_lines: 2,
    };
    let plan = row.continuations(10.0, policy).unwrap();
    let a = row.fragment(&[0], 10.0, &plan).unwrap().unwrap();
    let b = row
        .fragment(&[a.ranges[0].end], 10.0, &plan)
        .unwrap()
        .unwrap();
    let c = row
        .fragment(&[b.ranges[0].end], 10.0, &plan)
        .unwrap()
        .unwrap();
    assert_eq!(a.ranges[0], 0..2);
    assert_eq!(b.ranges[0], 2..4);
    assert_eq!(c.ranges[0], 4..6);
    assert!(c.complete);
}

#[test]
fn continuation_feasibility_matches_exhaustive_small_partition_oracle() {
    fn oracle(
        prefix: &[f64],
        start: usize,
        capacity: f64,
        minimum: usize,
        final_minimum: usize,
    ) -> bool {
        let n = prefix.len() - 1;
        if prefix[n] - prefix[start] <= capacity && (start == 0 || n - start >= final_minimum) {
            return true;
        }
        (start + minimum..n).any(|end| {
            prefix[end] - prefix[start] <= capacity
                && n - end >= final_minimum
                && oracle(prefix, end, capacity, minimum, final_minimum)
        })
    }
    for n in 1..=8 {
        let mut row = synthetic_heights(&vec![1.0; n]);
        for mask in 0..(1usize << n) {
            let mut prefix = vec![0.0];
            for index in 0..n {
                prefix.push(
                    prefix.last().unwrap() + if mask & (1 << index) == 0 { 1.0 } else { 3.0 },
                );
            }
            row.cells[0].heights = prefix.clone();
            for capacity in [3.0, 5.0, 7.0] {
                for minimum in 1..=3 {
                    for final_minimum in 1..=3 {
                        let expected = oracle(&prefix, 0, capacity, minimum, final_minimum);
                        let actual = row.continuations(
                            capacity,
                            TableRowSplitPolicy::Lines {
                                min_fragment_lines: minimum,
                                min_final_lines: final_minimum,
                            },
                        );
                        assert_eq!(
                            actual.is_ok(),
                            expected,
                            "n={n},mask={mask},capacity={capacity},min={minimum},last={final_minimum}"
                        );
                        if let Ok(plan) = actual {
                            let mut start = 0;
                            for _ in 0..n {
                                let fragment =
                                    row.fragment(&[start], capacity, &plan).unwrap().unwrap();
                                assert!(fragment.ranges[0].end > start);
                                start = fragment.ranges[0].end;
                                if fragment.complete {
                                    break;
                                }
                            }
                            assert_eq!(start, n);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn final_line_minimum_reserves_lines_and_rejects_impossible_policies() {
    let row = synthetic_heights(&[1.0; 5]);
    let policy = TableRowSplitPolicy::Lines {
        min_fragment_lines: 2,
        min_final_lines: 2,
    };
    let plan = row.continuations(4.0, policy).unwrap();
    assert_eq!(
        row.fragment(&[0], 4.0, &plan).unwrap().unwrap().ranges[0],
        0..3
    );
    assert!(row.fragment(&[0], 1.0, &plan).unwrap().is_none());
    assert!(row.continuations(1.0, policy).is_err());
    assert!(TableRowSplitPolicy::Lines {
        min_fragment_lines: 0,
        min_final_lines: 1
    }
    .validate()
    .is_err());
}

#[test]
fn large_header_or_single_tall_line_cannot_loop_allocating_pages() {
    let mut flow = flow();
    let cursor = flow.cursor_y;
    let mut header = TableBuilder::new(vec![TableColumn::new(160.0)]);
    header.set_header(["Heading\n".repeat(30)]);
    header.add_row(["Body"]);
    assert!(flow.add_table(&header).is_err());
    let mut body = TableBuilder::new(vec![TableColumn::new(160.0)]);
    body.add_row([TableCell::text("A").style(TextStyle::unicode(120.0))]);
    assert!(flow.add_table(&body).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert!(flow.builder.pages[0].commands.is_empty());
    assert_eq!(flow.cursor_y, cursor);
}

#[test]
fn later_cell_failure_removes_prior_rows_fragments_headers_and_pages() {
    let mut flow = flow();
    flow.add_paragraph("before", &TextStyle::default(), &ParagraphStyle::default())
        .unwrap();
    let cursor = flow.cursor_y;
    let commands = flow.builder.pages[0].commands.len();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.set_header(["Header"]);
    table.add_row(["A\n".repeat(30)]);
    table.add_row(["🚀"]);
    assert!(flow.add_table_with_report(&table).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert_eq!(flow.builder.pages[0].commands.len(), commands);
    assert_eq!(flow.cursor_y, cursor);
    assert_eq!(flow.current_page, 0);
}

#[test]
fn mixed_font_rtl_cells_keep_full_paragraph_context_across_fragments() {
    let mut flow = flow();
    let bytes = get_fallback_font("Symbol").unwrap();
    let run = TextShaper::shape(bytes, "ABC 123 ", ShapeOptions::default()).unwrap();
    let gids = run
        .glyphs
        .iter()
        .map(|g| g.glyph_id)
        .collect::<BTreeSet<_>>();
    let subset = subset_glyf_preserving_gids(bytes, &gids).unwrap().bytes;
    let primary = flow.builder.register_font_bytes("Latin", subset).unwrap();
    let full = flow
        .builder
        .register_font_bytes("Full", get_fallback_font("Symbol").unwrap())
        .unwrap();
    let stack = flow.builder.register_font_stack(&[primary, full]).unwrap();
    let style = TextStyle::new(stack, 12.0);
    let text = "ABC \u{2067}אבג 123 ABC אבג ABC 123 אבג\u{2069}\r\n".repeat(8);
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]).body_style(style);
    table.add_row([text.as_str()]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert_source_partition(&table, &report);
    assert_page_bounds(&report, &flow);
    assert!(flow.builder.pages.len() > 2);
    assert_eq!(
        flow.builder.pages.iter().map(logical).collect::<String>(),
        text
    );
    FontBuildPlan::from_builder(&flow.builder).unwrap();
    let used = flow
        .builder
        .pages
        .iter()
        .flat_map(|page| page.fonts_used())
        .collect::<BTreeSet<_>>();
    assert!(used.contains(&primary) && used.contains(&full));
    let engine = crate::ContentEngine::open_bytes(flow.builder.to_bytes().unwrap()).unwrap();
    assert_eq!(
        (1..=flow.builder.pages.len())
            .map(|page| engine
                .collect_page_text_chunks(page)
                .unwrap()
                .into_iter()
                .map(|c| c.text)
                .collect::<String>())
            .collect::<String>(),
        text
    );
}

#[test]
fn blank_and_control_only_cells_keep_exact_source_ranges_and_carriers() {
    let mut flow = flow();
    let text = "\r\n\u{200d}\n\u{e0100}\r\n".repeat(20);
    let mut table = TableBuilder::new(vec![TableColumn::new(80.0), TableColumn::new(80.0)])
        .body_style(TextStyle::unicode(10.0));
    table.add_row(["", text.as_str()]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert_source_partition(&table, &report);
    assert!(report
        .fragments
        .iter()
        .all(|f| f.cell_utf8_ranges[0] == [0, 0]));
    assert_eq!(
        flow.builder.pages.iter().map(logical).collect::<String>(),
        text
    );
    assert!(flow
        .builder
        .pages
        .iter()
        .flat_map(|p| &p.commands)
        .any(|c| matches!(c, PageCommand::LogicalBreak { .. })));
}

#[test]
fn continuation_rectangles_preserve_cell_fill_border_alignment_and_font_assets() {
    let mut flow = flow();
    let font = flow
        .builder
        .register_font_bytes("Cell", get_fallback_font("Courier").unwrap())
        .unwrap();
    let fill = Color::device_rgb(0.8, 0.9, 1.0);
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row([TableCell::text("Styled\n".repeat(25))
        .style(TextStyle::new(font, 12.0))
        .background(fill.clone())
        .align(TextAlign::Right)]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert!(report.fragments.len() > 2);
    for page in &flow.builder.pages {
        for command in &page.commands {
            if let PageCommand::Rect { style, .. } = command {
                assert_eq!(style.fill, Some(fill.clone()));
            }
            if let PageCommand::Text {
                x,
                style,
                font_asset,
                ..
            } = command
            {
                assert_eq!(style.font, font);
                assert!(*x > flow.margins.left + 40.0);
                assert!(Arc::ptr_eq(
                    font_asset.as_ref().unwrap(),
                    &flow.builder.custom_fonts[0].bytes
                ));
            }
        }
    }
}

#[test]
fn header_only_and_empty_tables_do_not_invent_body_fragments() {
    let mut flow = flow();
    let empty = TableBuilder::new(Vec::new());
    assert_eq!(
        flow.add_table_with_report(&empty).unwrap(),
        TableFlowReport::default()
    );
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.set_header(["Header"]);
    let report = flow.add_table_with_report(&table).unwrap();
    assert_eq!(report.fragments.len(), 1);
    assert_eq!(report.fragments[0].row, None);
    assert!(!report.fragments[0].repeated_header);
    assert_eq!(logical(&flow.builder.pages[0]), "Header");
}

#[test]
fn narrow_cells_use_actual_inner_width_and_invalid_state_fails_without_mutation() {
    let mut flow = flow();
    let mut table =
        TableBuilder::new(vec![TableColumn::new(8.5)]).body_style(TextStyle::unicode(1.0));
    table.add_row(["W"]);
    assert!(flow.add_table(&table).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
    let mut invalid = TableBuilder::new(vec![TableColumn::new(50.0)]).style(TableStyle {
        line_width: f64::NAN,
        ..TableStyle::default()
    });
    invalid.add_row(["A"]);
    assert!(flow.add_table(&invalid).is_err());
    flow.current_page = usize::MAX;
    assert!(flow.add_table(&table).is_err());
}

#[test]
fn cell_form_feed_is_rejected_before_any_table_page_is_mutated() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row(["A\u{000c}B"]);
    assert!(flow.add_table_with_report(&table).is_err());
    assert_eq!(flow.builder.pages.len(), 1);
    assert!(flow.builder.pages[0].commands.is_empty());
}

#[test]
fn cancellation_budget_and_unbalanced_artifacts_are_explicit_errors() {
    let mut flow = flow();
    let mut table = TableBuilder::new(vec![TableColumn::new(160.0)]);
    table.add_row(["A"]);
    let token = crate::cancel::CancelToken::new();
    token.cancel();
    assert!(token.scope(|| flow.add_table(&table).map(|_| ())).is_err());
    assert!(flow.builder.pages[0].commands.is_empty());
    let mut budget = Budget {
        fragments: MAX_FRAGMENTS,
        ..Budget::default()
    };
    let fragment = Fragment {
        height: 1.0,
        ranges: vec![0..1],
        complete: true,
    };
    assert!(budget.fragment(&fragment).is_err());
    budget.cells = MAX_CELLS;
    assert!(budget.cell(1).is_err());
    let mut expanded = Budget {
        output_cells: MAX_CELLS,
        ..Budget::default()
    };
    assert!(expanded.fragment(&fragment).is_err());
    let mut repeated = Budget {
        output_lines: MAX_LINES,
        ..Budget::default()
    };
    assert!(repeated.fragment(&fragment).is_err());
    let fonts = FontBuildPlan::from_builder(&flow.builder).unwrap();
    let images = ImageBuildPlan::from_builder(&flow.builder).unwrap();
    flow.builder.pages[0]
        .commands
        .push(PageCommand::BeginArtifact);
    assert!(build_content_stream(&flow.builder.pages[0], &fonts, &images).is_err());
    flow.builder.pages[0].commands[0] = PageCommand::EndArtifact;
    assert!(build_content_stream(&flow.builder.pages[0], &fonts, &images).is_err());
}
